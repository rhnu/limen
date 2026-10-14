"""Opt-in runtime experiments: each untrusted VM runs in a bounded child process."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


CASES = (
    "void",
    "pending-fairness",
    "late-completion",
    "invalid-return",
    "rejection",
    "top-level-await",
    "strict-json",
    "modules",
    "sync-loop",
    "microtask-loop",
    "memory-limit",
    "shutdown-pending",
    "startup-loop",
    "context-headers",
    "context-readonly",
    "context-isolation",
    "context-stream",
)


def build(workspace: Path) -> Path:
    """Read Cargo's artifact path so target-dir configuration remains supported."""
    result = subprocess.run(
        ["cargo", "build", "--locked", "--bin", "runtime_probe", "--message-format=json"],
        cwd=workspace,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    messages = [json.loads(line) for line in result.stdout.splitlines()]
    if result.returncode:
        for message in messages:
            if message.get("reason") == "compiler-message":
                rendered = message.get("message", {}).get("rendered")
                if rendered:
                    print(rendered, end="", file=sys.stderr)
        raise RuntimeError("runtime probe build failed")
    for message in messages:
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("target", {}).get("name") == "runtime_probe"
            and message.get("executable")
        ):
            return Path(message["executable"])
    raise RuntimeError("Cargo did not report the runtime_probe executable")


def run_case(executable: Path, case: str, timeout: float) -> bool:
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="limen-runtime-probe-") as fixture:
        try:
            # The Rust child creates OS threads only. No subprocess or host process
            # API is exposed to JS, so subprocess.run's timeout kills the entire VM,
            # including a blocked VM thread, on both Windows and Unix.
            result = subprocess.run(
                [os.fspath(executable), case, fixture],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired:
            print(f"FAIL {case}: killed after {timeout:g}s", flush=True)
            return False
        elapsed = time.monotonic() - started
        if result.returncode:
            print(f"FAIL {case}: child exit {result.returncode} ({elapsed:.2f}s)", flush=True)
            if result.stdout:
                print(result.stdout, end="")
            if result.stderr:
                print(result.stderr, end="", file=sys.stderr)
            return False
        print(f"PASS {case} ({elapsed:.2f}s)", flush=True)
        return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", nargs="*", choices=CASES)
    parser.add_argument("--timeout", type=float, default=12.0, help="seconds per child (1..60)")
    parser.add_argument("--list", action="store_true", help="list cases without building")
    args = parser.parse_args()
    if args.list:
        print("\n".join(CASES))
        return 0
    if not 1 <= args.timeout <= 60:
        parser.error("--timeout must be between 1 and 60 seconds")
    executable = build(Path(__file__).resolve().parents[1])
    cases = args.cases or CASES
    passed = sum(run_case(executable, case, args.timeout) for case in cases)
    print(f"{passed}/{len(cases)} runtime experiments passed", flush=True)
    return 0 if passed == len(cases) else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"runtime probes: {error}", file=sys.stderr)
        raise SystemExit(1) from error
