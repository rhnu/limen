"""Opt-in local HTTP experiments, isolated from pure logic unit tests.

Each case runs in an externally timed Python child. Synthetic upstreams bind
only loopback and use placeholder secrets; no real provider is contacted.
"""

import argparse
import contextlib
import http.client
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time


CASES = ("auth-routing", "pipeline", "failures", "deadline", "disconnect", "shutdown", "retirement",
         "resources", "observations", "transport-contracts")
REQUEST = {"model": "chat", "messages": [{"role": "user", "content": "synthetic-body-secret"}]}


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Upstream(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), UpstreamHandler)
        self.records = []
        self.started = threading.Event()
        self.disconnected = threading.Event()
        self.hold = threading.Event()
        self.status = 200
        self.media = "application/json"
        self.delay = False


class UpstreamHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def handle(self):
        try:
            super().handle()
        except (ConnectionError, TimeoutError):
            # The experiments deliberately close pooled and in-flight sockets.
            pass

    def log_message(self, *_args):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers["content-length"]))
        self.server.records.append((self.path, self.headers, json.loads(raw)))
        self.server.started.set()
        if self.server.delay:
            self.connection.settimeout(0.05)
            while not self.server.hold.is_set():
                try:
                    if self.connection.recv(1, socket.MSG_PEEK) == b"":
                        self.server.disconnected.set()
                        return
                except socket.timeout:
                    continue
                except OSError:
                    self.server.disconnected.set()
                    return
        response = {"id": "synthetic", "model": "real-model", "choices": [], "echo": json.loads(raw)}
        body = json.dumps(response).encode()
        self.send_response(self.server.status)
        self.send_header("Content-Type", self.server.media)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Set-Cookie", "synthetic-cookie-secret=1")
        if self.server.status == 307:
            self.send_header("Location", "/redirect-target")
        self.end_headers()
        try:
            self.wfile.write(body)
        except OSError:
            self.server.disconnected.set()


def free_port():
    with socket.socket() as stream:
        stream.bind(("127.0.0.1", 0))
        return stream.getsockname()[1]


class Fixture:
    def __init__(self, executable, config=None, limits="", auth=True):
        self.upstream = Upstream()
        self.thread = threading.Thread(target=self.upstream.serve_forever, daemon=True)
        self.thread.start()
        self.temp = tempfile.TemporaryDirectory(prefix="limen-http-probe-")
        self.directory = Path(self.temp.name)
        self.port = free_port()
        endpoint = f"http://127.0.0.1:{self.upstream.server_port}/custom/completion?key=synthetic-query-secret"
        business = config or """export default {
          providers: {p: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'},
            defaults: {temperature: 0.2, extra: {provider: true}}}},
          models: {chat: {provider: 'p', upstream: 'real-model', defaults: {temperature: 0.4}}}
        };"""
        (self.directory / "config.js").write_text(business.replace("ENDPOINT", json.dumps(endpoint)), encoding="utf-8")
        authentication = "bearer: synthetic-client-secret" if auth else "none: true"
        (self.directory / "server.yaml").write_text(
            f"listen: 127.0.0.1:{self.port}\nclient_auth: {{{authentication}}}\n"
            "log: {level: debug, format: json}\nlimits:\n  request_ms: 1500\n  hook_ms: 1000\n"
            "  cleanup_ms: 200\n  shutdown_ms: 1500\n  concurrency: 2\n  queue_capacity: 2\n" + limits,
            encoding="utf-8",
        )
        self.log = open(self.directory / "process.log", "w+", encoding="utf-8")
        flags = subprocess.CREATE_NEW_CONSOLE if os.name == "nt" else 0
        startup = None
        if os.name == "nt":
            startup = subprocess.STARTUPINFO()
            startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            startup.wShowWindow = 0
        self.process = subprocess.Popen([os.fspath(executable), "serve", "--config", os.fspath(self.directory / "server.yaml")],
            stdout=self.log, stderr=self.log, creationflags=flags, startupinfo=startup)
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            require(self.process.poll() is None, "gateway failed during startup")
            try:
                with socket.create_connection(("127.0.0.1", self.port), timeout=0.1):
                    return
            except OSError:
                time.sleep(0.02)
        raise AssertionError("gateway did not listen")

    def request(self, body=REQUEST, method="POST", path="/v1/chat/completions", headers=None):
        fields = {"Authorization": "Bearer synthetic-client-secret", "Content-Type": "application/json"}
        fields.update(headers or {})
        fields = {key: value for key, value in fields.items() if value is not None}
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=4)
        try:
            connection.request(method, path, json.dumps(body) if method == "POST" else None, fields)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), json.loads(response.read())
        finally:
            connection.close()

    def logs(self):
        self.log.flush()
        return (self.directory / "process.log").read_text(encoding="utf-8")

    def wait_event(self, predicate):
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            # Read complete JSON lines only while the child is still logging.
            for line in self.logs().splitlines(keepends=True):
                if line.endswith("\n") and predicate(json.loads(line)["fields"]):
                    return
            time.sleep(0.01)
        raise AssertionError("expected bounded observation did not finish")

    def close(self):
        self.upstream.hold.set()
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=3)
        text = self.logs()
        for secret in ("synthetic-client-secret", "synthetic-upstream-secret", "synthetic-query-secret", "synthetic-body-secret", "synthetic-cookie-secret"):
            require(secret not in text, "process log exposed synthetic secret")
        self.log.close()
        self.upstream.shutdown()
        self.upstream.server_close()
        self.thread.join(timeout=2)
        self.temp.cleanup()


@contextlib.contextmanager
def fixture(executable, **options):
    item = Fixture(executable, **options)
    try:
        yield item
    finally:
        item.close()


def auth_routing(executable):
    with fixture(executable) as item:
        for method, path in [("GET", "/v1/models"), ("POST", "/v1/chat/completions")]:
            status, _, body = item.request(method=method, path=path, headers={"Authorization": None})
            require(status == 401 and body["error"]["code"] == "unauthorized", "both endpoints must authenticate")
        status, _, body = item.request(method="GET", path="/v1/models")
        require(status == 200 and [model["id"] for model in body["data"]] == ["chat"], "explicit model list")
        status, _, body = item.request({**REQUEST, "model": "unknown"})
        require(status == 400 and body["error"]["code"] == "unknown_model", "unknown route rejected")
        require(not item.upstream.records, "rejections must not send upstream")
        status, headers, body = item.request({**REQUEST, "temperature": None, "unknownExtension": {"keep": True}}, headers={"Cookie": "synthetic-cookie-secret=1", "X-Unrelated": "hidden"})
        require(status == 200 and body["model"] == "chat", "response alias")
        path, upstream_headers, sent = item.upstream.records[0]
        require(path.endswith("/custom/completion?key=synthetic-query-secret"), "complete configured URL")
        require(upstream_headers["Authorization"] == "Bearer synthetic-upstream-secret", "independent upstream auth")
        require(not upstream_headers.get("Cookie") and not upstream_headers.get("X-Unrelated"), "client headers isolated")
        require(sent["model"] == "real-model" and sent["temperature"] is None and sent["unknownExtension"]["keep"], "merge null and unknown fields")
        require(not any(key.lower() == "set-cookie" for key in headers), "upstream cookies excluded")
        item.upstream.status = 307
        status, _, body = item.request()
        require(status == 502 and body["error"]["code"] == "upstream_failed", "redirect is an error")
        require(len(item.upstream.records) == 2, "one upstream send per request")


def pipeline(executable):
    config = """let finishes = 0, failures = 0, observed = null;
    export default {
      onRequest(ctx) { ctx.body.model = 'chat'; ctx.state.order = ['request']; },
      onError(ctx) { failures++; observed = ctx.state.order; },
      onFinish(ctx) { finishes++; if (!Object.isFrozen(ctx.state)) throw new Error('readonly'); },
      providers: {p: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'}, pipeline: [{name: 'provider',
        async beforeSend(ctx) { ctx.state.order.push('provider-before'); ctx.body.model = 'hook-model'; ctx.headers.delete('Authorization'); ctx.headers.append('X-Multiple','one'); ctx.headers.append('x-multiple','two'); ctx.url = ctx.url.replace('/custom/completion', '/modified'); },
        afterReceive(ctx) { ctx.state.order.push('provider-after'); ctx.status = 201; ctx.headers.set('X-Draft','provider'); },
        onResponse(ctx) { ctx.state.order.push('provider-response'); ctx.body.order = ctx.state.order; }}]}},
      models: {chat: {provider: 'p', upstream: 'real-model', pipeline: [{name: 'model',
        beforeSend(ctx) { if (ctx.body.fail) { ctx.state.order.push('discard'); throw new Error('synthetic-body-secret'); } ctx.state.order.push('model-before'); },
        afterReceive(ctx) { ctx.state.order.push('model-after'); ctx.headers.set('X-Draft','model'); },
        onResponse(ctx) { ctx.state.order.push('model-response'); ctx.body.order = ctx.state.order; ctx.body.counters = [finishes,failures]; ctx.body.observed = observed; ctx.body.model = 'explicit-alias'; }}]}}
    };"""
    with fixture(executable, config=config) as item:
        status, headers, body = item.request({**REQUEST, "model": "reroute-once"})
        require(status == 201 and body["model"] == "explicit-alias", "status and explicit response model")
        require(body["order"] == ["request", "provider-before", "model-before", "provider-after", "model-after", "provider-response", "model-response"], "provider then model in each stage")
        require(next(value for key, value in headers.items() if key.lower() == "x-draft") == "model", "later draft wins")
        path, fields, sent = item.upstream.records[0]
        require(path.startswith("/modified?") and sent["model"] == "hook-model" and fields.get("Authorization") is None, "final URL/model/headers respected")
        require(fields.get_all("X-Multiple") == ["one", "two"], "header multivalue preserved")
        status, _, body = item.request({**REQUEST, "fail": True})
        require(status == 500 and body["error"]["code"] == "hook_failed", "failed hook aborts")
        require(len(item.upstream.records) == 1, "failed hook never sends")
        status, _, body = item.request()
        require(status == 201 and body["counters"] == [2, 1], "each terminal outcome observed once")
        require(body["observed"] == ["request", "provider-before"], "failed draft not committed to observer")
        require(len(body["order"]) == 7, "state independently initialized")


def failures(executable):
    with fixture(executable) as item:
        item.upstream.status = 503
        status, _, body = item.request()
        require(status == 502 and body["error"]["code"] == "upstream_failed", "upstream failures classified")
        item.upstream.status = 200
        item.upstream.media = "text/event-stream"
        status, _, body = item.request()
        require(status == 502 and body["error"]["code"] == "upstream_protocol", "SSE explicitly refused")
        status, _, body = item.request({**REQUEST, "stream": True})
        require(status == 400 and body["error"]["code"] == "invalid_request", "client streaming explicitly refused")
    config = """export default {providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'invalid-status',afterReceive(ctx){ctx.status=200;}}]}},models:{chat:{provider:'p',upstream:'real-model'}}};"""
    with fixture(executable, config=config) as item:
        item.upstream.status = 500
        status, _, body = item.request()
        require(status == 500 and body["error"]["code"] == "hook_failed", "hook cannot turn failure into success")


def pending_config():
    return """let cancelled = 0, finishes = 0;
    export default {onRequest(ctx){if(ctx.body.wait)return new Promise(()=>{});},
      onFinish(ctx){finishes++;if(ctx.outcome==='cancelled')cancelled++;},
      providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'report',onResponse(ctx){ctx.body.cancelled=cancelled;ctx.body.finishes=finishes;}}]}},
      models:{chat:{provider:'p',upstream:'real-model'}}};"""


def deadline(executable):
    with fixture(executable, config=pending_config()) as item:
        started = time.monotonic()
        status, _, body = item.request({**REQUEST, "wait": True})
        require(status == 504 and body["error"]["code"] == "deadline_exceeded", "hook waiting deadline")
        require(time.monotonic() - started < 2, "deadline bounded")
        require(not item.upstream.records, "timed out request does not send")
        require(item.request()[0] == 200, "ordinary timeout retains healthy VM")
    with fixture(executable) as item:
        item.upstream.delay = True
        status, _, body = item.request()
        require(status == 504 and body["error"]["code"] == "deadline_exceeded", "upstream request deadline")
        require(item.upstream.disconnected.wait(1), "deadline aborts upstream read")


def disconnect(executable):
    with fixture(executable, config=pending_config()) as item:
        raw = json.dumps({**REQUEST, "wait": True}).encode()
        stream = socket.create_connection(("127.0.0.1", item.port), timeout=2)
        stream.sendall(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-client-secret\r\nContent-Type: application/json\r\nContent-Length: " + str(len(raw)).encode() + b"\r\n\r\n" + raw)
        time.sleep(0.08)
        stream.close()
        deadline = time.monotonic() + 0.8
        while time.monotonic() < deadline:
            status, _, body = item.request()
            if status == 200 and body["cancelled"] == 1:
                break
            time.sleep(0.02)
        else:
            raise AssertionError("disconnect did not cancel pending hook before deadline")
    with fixture(executable) as item:
        item.upstream.delay = True
        raw = json.dumps(REQUEST).encode()
        stream = socket.create_connection(("127.0.0.1", item.port), timeout=2)
        stream.sendall(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-client-secret\r\nContent-Type: application/json\r\nContent-Length: " + str(len(raw)).encode() + b"\r\n\r\n" + raw)
        require(item.upstream.started.wait(1), "upstream request started")
        stream.close()
        require(item.upstream.disconnected.wait(0.8), "disconnect promptly aborts upstream read")


def shutdown(executable):
    config = pending_config().replace("finishes++;if(ctx.outcome==='cancelled')cancelled++;",
        "finishes++;if(ctx.outcome==='cancelled')cancelled++;throw new Error('observer failure');")
    with fixture(executable, config=config) as item:
        raw = json.dumps({**REQUEST, "wait": True}).encode()
        stream = socket.create_connection(("127.0.0.1", item.port), timeout=2)
        stream.sendall(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-client-secret\r\nContent-Type: application/json\r\nContent-Length: " + str(len(raw)).encode() + b"\r\n\r\n" + raw)
        time.sleep(0.1)
        if os.name == "nt":
            # The gateway has its own hidden console. An isolated helper attaches
            # only to that console and ignores its own Ctrl-C before broadcasting.
            # This avoids signalling the user's terminal or the probe driver.
            helper = """import ctypes,sys,time
k=ctypes.WinDLL('kernel32',use_last_error=True)
k.FreeConsole()
assert k.AttachConsole(int(sys.argv[1])), ctypes.get_last_error()
assert k.SetConsoleCtrlHandler(None,True), ctypes.get_last_error()
assert k.GenerateConsoleCtrlEvent(0,0), ctypes.get_last_error()
time.sleep(0.1)
k.FreeConsole()
"""
            result = subprocess.run([sys.executable, "-c", helper, str(item.process.pid)],
                capture_output=True, text=True, check=False, timeout=2,
                creationflags=subprocess.CREATE_NO_WINDOW)
            require(result.returncode == 0, "isolated Ctrl-C helper failed")
        else:
            item.process.send_signal(signal.SIGTERM)
        require(item.process.wait(timeout=2) == 0, "shutdown signal drains and exits successfully")
        stream.close()
        require("HTTP listener stopped" in item.logs(), "listener shutdown observed")
        events = [json.loads(line)["fields"] for line in item.logs().splitlines()]
        require(sum(event.get("message") == "request finished" and event.get("outcome") == "cancelled"
            for event in events) == 1, "shutdown records one cancelled terminal outcome")
        require(sum(event.get("message") == "observation failed; terminal outcome retained"
            and event.get("phase") == "onFinish" for event in events) == 1,
            "shutdown attempts onFinish exactly once")


def retirement(executable):
    config = """export default {onRequest(){while(true){}},providers:{p:{endpoint:ENDPOINT}},models:{chat:{provider:'p',upstream:'real-model'}}};"""
    with fixture(executable, config=config, limits="  execution_ms: 20\n") as item:
        try:
            item.request()
        except (OSError, http.client.HTTPException):
            pass
        require(item.process.wait(timeout=3) != 0, "VM retirement terminates HTTP process with failure")
        require(not item.upstream.records, "retirement never sends generation")
        require("HTTP listener stopped" in item.logs(), "retirement stops listener")


def open_request(item, body):
    raw = json.dumps(body).encode()
    stream = socket.create_connection(("127.0.0.1", item.port), timeout=2)
    stream.sendall(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-client-secret\r\nContent-Type: application/json\r\nContent-Length: "
        + str(len(raw)).encode() + b"\r\n\r\n" + raw)
    return stream


def resources(executable):
    with fixture(executable, config=pending_config(), limits="  max_body_bytes: 300\n") as item:
        status, _, body = item.request({**REQUEST, "large": "x" * 400})
        require(status == 413 and body["error"]["code"] == "payload_too_large", "body limit rejects before upstream")
        require(not item.upstream.records, "oversized request never sent")
        held = [open_request(item, {**REQUEST, "wait": True}) for _ in range(2)]
        try:
            time.sleep(0.1)
            status, _, body = item.request()
            require(status == 429 and body["error"]["code"] == "busy", "bounded request admission")
            require(not item.upstream.records, "busy request never sent")
        finally:
            for stream in held:
                stream.close()
        deadline = time.monotonic() + 0.8
        while time.monotonic() < deadline:
            status, _, body = item.request()
            if status == 200 and body["cancelled"] == 2:
                break
            time.sleep(0.02)
        else:
            raise AssertionError("cancelled requests did not release admission permits")


def observations(executable):
    config = """let errors=0,finishes=0;
    export default {
      onRequest(ctx){ctx.state.mode=ctx.body.mode||'normal';},
      onError(ctx){errors++;if(ctx.state.mode==='error-wait')return new Promise(()=>{});throw new Error('synthetic-body-secret');},
      onFinish(ctx){finishes++;if(ctx.state.mode==='finish-wait')return new Promise(()=>{});if(ctx.state.mode==='bad')throw new Error('synthetic-body-secret');},
      providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'probe',
        beforeSend(ctx){if(ctx.state.mode==='bad'||ctx.state.mode==='error-wait')throw new Error('synthetic-body-secret');},
        onResponse(ctx){ctx.body.counts=[errors,finishes];}
      }]}},models:{chat:{provider:'p',upstream:'real-model'}}};"""
    with fixture(executable, config=config) as item:
        status, _, body = item.request({**REQUEST, "mode": "bad"})
        require(status == 500 and body["error"]["code"] == "hook_failed", "failed observers preserve original failure")
        request_id = body["error"]["request_id"]
        item.wait_event(lambda event: event.get("request_id") == request_id
            and event.get("phase") == "onFinish" and event.get("error_kind") == "JavaScript")
        status, _, body = item.request()
        require(status == 200 and body["counts"] == [1, 1], "failed observers run once without recursive onError")
        started = time.monotonic()
        status, _, body = item.request({**REQUEST, "mode": "error-wait"})
        require(status == 500 and body["error"]["code"] == "hook_failed", "pending error observer preserves failure")
        require(time.monotonic() - started < 0.8, "error observer cleanup deadline is finite")
        request_id = body["error"]["request_id"]
        item.wait_event(lambda event: event.get("request_id") == request_id
            and event.get("phase") == "onFinish" and event.get("reason") == "cleanup_budget")
        status, _, body = item.request()
        require(status == 200 and body["counts"] == [2, 2], "expired onError leaves onFinish unexecuted")
        started = time.monotonic()
        status, headers, body = item.request({**REQUEST, "mode": "finish-wait"})
        require(status == 200 and time.monotonic() - started < 0.8, "pending finish observer preserves bounded success")
        request_id = next(value for key, value in headers.items() if key.lower() == "x-limen-request-id")
        item.wait_event(lambda event: event.get("request_id") == request_id
            and event.get("phase") == "onFinish" and event.get("error_kind") == "Deadline")
        status, _, body = item.request()
        require(status == 200 and body["counts"] == [2, 4], "finish observer timeout never invokes onError")
        events = [json.loads(line)["fields"] for line in item.logs().splitlines()]
        require(any(event.get("phase") == "onFinish" and event.get("reason") == "cleanup_budget"
            for event in events), "exhausted cleanup reports skipped finish")


def transport_contracts(executable):
    config = """export default {
      onRequest(ctx){if(ctx.body.rewrite)ctx.body.stream=false;},
      providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'control',
        beforeSend(ctx){if(ctx.body.upstreamStream)ctx.body.stream=true;},
        afterReceive(ctx){if(ctx.meta.upstreamStatus>=300){ctx.status=409;ctx.headers.set('X-Mapped','failure');}
          else{ctx.headers.set('X-Original-Media',ctx.meta.upstreamContentType);}}
      }]}},models:{chat:{provider:'p',upstream:'real-model'}}};"""
    with fixture(executable, config=config) as item:
        status, _, body = item.request({**REQUEST, "stream": True, "rewrite": True})
        require(status == 400 and body["error"]["code"] == "invalid_request", "original client stream cannot be rewritten")
        status, _, body = item.request({**REQUEST, "upstreamStream": True})
        require(status == 400 and body["error"]["code"] == "invalid_request", "beforeSend streaming unsupported")
        require(not item.upstream.records, "stream rejections never send upstream")
        item.upstream.status = 500
        status, headers, body = item.request()
        require(status == 409 and body["error"]["code"] == "upstream_failed", "afterReceive maps error status without success classification")
        require(next(value for key, value in headers.items() if key.lower() == "x-mapped") == "failure", "error response preserves committed hook header")
        item.upstream.status = 200
        item.upstream.media = "text/event-stream"
        status, headers, body = item.request()
        require(status == 502 and body["error"]["code"] == "upstream_protocol", "original SSE parser fact remains unsupported")
        require(next(value for key, value in headers.items() if key.lower() == "x-original-media") == "sse", "original media metadata preserved on failure")


def build(workspace):
    result = subprocess.run(["cargo", "build", "--locked", "--bin", "limen", "--message-format=json"], cwd=workspace,
        capture_output=True, text=True, encoding="utf-8", check=False, timeout=240)
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    require(result.returncode == 0, "gateway build failed")
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            return Path(message["executable"])
    raise AssertionError("Cargo did not report executable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", nargs="*", choices=CASES)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--timeout", type=float, default=18, help="external seconds per child (5..60)")
    parser.add_argument("--executable", type=Path, help="reuse a built gateway executable")
    parser.add_argument("--worker", choices=CASES, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.list:
        print("\n".join(CASES))
        return 0
    if args.worker:
        globals()[args.worker.replace("-", "_")](args.executable)
        return 0
    if not 5 <= args.timeout <= 60:
        parser.error("--timeout must be between 5 and 60 seconds")
    executable = args.executable or build(Path(__file__).resolve().parents[1])
    failures_count = 0
    for case in args.cases or CASES:
        started = time.monotonic()
        try:
            worker = subprocess.Popen([sys.executable, os.fspath(Path(__file__).resolve()), "--worker", case,
                "--executable", os.fspath(executable)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, encoding="utf-8", start_new_session=os.name != "nt")
            stdout, stderr = worker.communicate(timeout=args.timeout)
            result = subprocess.CompletedProcess(worker.args, worker.returncode, stdout, stderr)
            success = result.returncode == 0
            if result.stdout:
                print(result.stdout, end="")
            if result.stderr:
                print(result.stderr, end="", file=sys.stderr)
        except subprocess.TimeoutExpired:
            # The worker owns a gateway subprocess. Kill the entire tree while the
            # parent is still alive, so an interrupted experiment leaves no VM.
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(worker.pid), "/T", "/F"],
                    capture_output=True, check=False, timeout=5)
            else:
                os.killpg(worker.pid, signal.SIGKILL)
            worker.kill()
            worker.communicate(timeout=3)
            success = False
            print(f"FAIL {case}: external timeout {args.timeout:g}s")
        failures_count += not success
        print(f"{'PASS' if success else 'FAIL'} {case} ({time.monotonic()-started:.2f}s)", flush=True)
    return int(failures_count != 0)


if __name__ == "__main__":
    raise SystemExit(main())
