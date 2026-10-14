use std::time::Duration;

use limen_script::{RuntimeStatus, ScriptErrorKind};
use serde_json::{Value, json};

use super::{Fixture, ProbeResult, call, ensure, hook, stop};

fn draft(phase: &str) -> Value {
    json!({
        "meta": {"phase": phase, "requestId": "synthetic", "clientIntent": {"stream": false}},
        "state": {"nested": {"value": 1}},
        "body": {"model": "synthetic", "nested": {"value": 1}},
        "headers": [{"name": "X-Example", "value": "first"}, {"name": "x-example", "value": "second"}],
        "url": "https://fictional.invalid/v1/chat/completions",
        "status": 200,
        "outcome": {"success": true},
        "stats": {"events": 0},
        "error": {"category": "synthetic"}
    })
}

pub(super) async fn headers(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(r"
        export default { onRequest(ctx) {
            const mode = ctx.meta.mode;
            if (mode === 'valid') {
                const values = ctx.headers.getAll('X-EXAMPLE');
                if (values.length !== 2 || values[0] !== 'first' || values[1] !== 'second') throw new Error();
                values[0] = 'not committed';
                ctx.headers.append('x-Example', 'third,with,commas');
                ctx.headers.set('Authorization', 'synthetic');
                ctx.headers.append('authorization', 'second');
                ctx.headers.delete('X-EXAMPLE');
                ctx.headers.append('X-Example', 'final');
            } else if (mode === 'get') {
                if (ctx.headers.getAll('Host')[0] !== 'fictional.invalid') throw new Error();
            } else {
                ctx.headers[ctx.meta.operation](ctx.meta.name, ctx.meta.value);
            }
        } };
    ").await?;
    let id = hook(&config, "onRequest")?;
    let mut input = draft("beforeSend");
    input["meta"]["mode"] = json!("valid");
    let output = runtime.invoke(call(id, 1, input)).await?;
    ensure(
        output["headers"]
            == json!([
                {"name": "authorization", "value": "synthetic"},
                {"name": "authorization", "value": "second"},
                {"name": "x-example", "value": "final"}
            ]),
        "controlled headers lost multi-values or replacement/deletion semantics",
    )?;
    for name in [
        "Host",
        "Content-Length",
        "TRANSFER-encoding",
        "Connection",
        "Content-Encoding",
        "Content-Type",
        "Proxy-Connection",
        "Keep-Alive",
        "TE",
        "Trailer",
        "Upgrade",
        "bad name",
    ] {
        for operation in ["append", "set", "delete"] {
            let mut input = draft("beforeSend");
            input["meta"]["mode"] = json!("invalid");
            input["meta"]["operation"] = json!(operation);
            input["meta"]["name"] = json!(name);
            input["meta"]["value"] = json!("synthetic");
            let output = runtime.invoke(call(id, 2, input)).await;
            ensure(
                output.is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
                "invalid or transport header mutation was accepted",
            )?;
        }
    }
    for value in ["line\r\nbreak", "nul\0value", "control\u{7f}"] {
        let mut input = draft("beforeSend");
        input["meta"] = json!({"phase": "beforeSend", "mode": "invalid", "operation": "append", "name": "x-example", "value": value});
        ensure(
            runtime.invoke(call(id, 3, input)).await.is_err(),
            "invalid header value was accepted",
        )?;
    }
    let mut input = draft("afterReceive");
    input["meta"]["mode"] = json!("get");
    input["headers"] = json!([{"name": "Host", "value": "fictional.invalid"}]);
    let output = runtime.invoke(call(id, 4, input)).await?;
    ensure(
        output["headers"] == json!([{"name": "host", "value": "fictional.invalid"}]),
        "read access to transport header failed",
    )?;
    stop(&runtime).await
}

pub(super) async fn readonly(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(r"
        export default { onRequest(ctx) {
            switch (ctx.meta.operation) {
                case 'meta-value': ctx.meta.clientIntent.stream = true; break;
                case 'meta-ref': ctx.meta = {}; break;
                case 'state-ref': ctx.state = {}; break;
                case 'headers-ref': ctx.headers = []; break;
                case 'headers-method': ctx.headers.set = () => {}; break;
                case 'new-field': ctx.extra = true; break;
                case 'state': ctx.state.nested.value = 2; break;
                case 'body': ctx.body.nested.value = 2; break;
                case 'body-ref': ctx.body = { replacement: true }; break;
                case 'url': ctx.url = 'https://fictional.invalid/changed'; break;
                case 'status': ctx.status = 201; break;
                case 'outcome': ctx.outcome.success = false; break;
                case 'stats': ctx.stats.events = 1; break;
                case 'error': ctx.error.category = 'changed'; break;
                case 'headers': ctx.headers.append('x-example', 'changed'); break;
                case 'noop': if (ctx.headers.getAll('x-example').length !== 2) throw new Error(); break;
            }
        } };
    ").await?;
    let id = hook(&config, "onRequest")?;
    for phase in [
        "onRequest",
        "beforeSend",
        "afterReceive",
        "onResponse",
        "onError",
        "onFinish",
    ] {
        for operation in [
            "meta-value",
            "meta-ref",
            "state-ref",
            "headers-ref",
            "headers-method",
            "new-field",
            "state",
            "body",
            "body-ref",
            "url",
            "status",
            "outcome",
            "stats",
            "error",
            "headers",
            "noop",
        ] {
            let mut input = draft(phase);
            input["meta"]["operation"] = json!(operation);
            let allowed = operation == "noop"
                || (operation == "state" && !matches!(phase, "onError" | "onFinish"))
                || (matches!(operation, "body" | "body-ref")
                    && matches!(phase, "onRequest" | "beforeSend" | "onResponse"))
                || (operation == "url" && phase == "beforeSend")
                || (operation == "status" && phase == "afterReceive")
                || (operation == "headers" && matches!(phase, "beforeSend" | "afterReceive"));
            let original = input.clone();
            let result = runtime.invoke(call(id, 1, input)).await;
            if allowed {
                let output = result?;
                ensure(
                    output["meta"] == original["meta"],
                    "readonly metadata changed",
                )?;
                let changed = match operation {
                    "state" => output["state"]["nested"]["value"] == 2,
                    "body" => output["body"]["nested"]["value"] == 2,
                    "body-ref" => output["body"] == json!({"replacement": true}),
                    "url" => output["url"] == "https://fictional.invalid/changed",
                    "status" => output["status"] == 201,
                    "headers" => {
                        output["headers"][2] == json!({"name": "x-example", "value": "changed"})
                    },
                    _ => true,
                };
                ensure(changed, "allowed phase edit was lost at commit")?;
                if operation == "noop" {
                    ensure(
                        output["state"] == original["state"] && output["body"] == original["body"],
                        "observer changed its final view",
                    )?;
                }
            } else {
                ensure(
                    result.is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
                    "phase accepted a readonly context mutation",
                )?;
            }
        }
    }
    ensure(
        matches!(runtime.status(), RuntimeStatus::Ready),
        "readonly contract failures retired the VM",
    )?;
    stop(&runtime).await
}

pub(super) async fn isolation(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(r"
        let release;
        export default {
            async onRequest(ctx) {
                ctx.state.order = ['request'];
                ctx.body.private = ctx.meta.requestId;
                if (ctx.meta.pending) await new Promise(resolve => { release = resolve; });
                await Promise.resolve();
                ctx.state.order.push('completed');
                if (ctx.meta.failure === 'reject') throw new Error('synthetic');
                if (ctx.meta.failure === 'return') return {};
            },
            onFinish() { release(); },
            providers: { p: { pipeline: [{ name: 'provider', async beforeSend(ctx) {
                await Promise.resolve(); ctx.state.order.push('provider'); ctx.headers.append('x-step', 'provider');
            } }] } },
            models: { m: { pipeline: [{ name: 'model', async beforeSend(ctx) {
                await Promise.resolve(); ctx.state.order.push('model'); ctx.headers.append('x-step', 'model');
            } }] } }
        };
    ").await?;
    let id = hook(&config, "onRequest")?;
    let mut input = draft("onRequest");
    input["meta"]["pending"] = json!(true);
    let pending = call(id, 1, input.clone());
    let cancellation = pending.cancellation.clone();
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut second = draft("onRequest");
    second["meta"]["requestId"] = json!("independent");
    let mut output = runtime.invoke(call(id, 2, second)).await?;
    ensure(
        output["state"]["order"] == json!(["request", "completed"])
            && output["body"]["private"] == "independent",
        "concurrent request drafts shared state",
    )?;
    cancellation.cancel();
    ensure(
        waiting
            .await?
            .is_err_and(|error| error.kind() == ScriptErrorKind::Cancelled),
        "cancelled context draft committed",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 3, json!({})))
        .await?;
    output["meta"]["phase"] = json!("beforeSend");
    let provider = config["providers"]["p"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("missing provider hook")?;
    let model = config["models"]["m"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("missing model hook")?;
    let output = runtime.invoke(call(provider, 4, output)).await?;
    let output = runtime.invoke(call(model, 5, output)).await?;
    ensure(
        output["state"]["order"] == json!(["request", "completed", "provider", "model"]),
        "async hook outputs did not feed subsequent steps in order",
    )?;
    ensure(
        output["headers"]
            == json!([
                {"name": "x-example", "value": "first"}, {"name": "x-example", "value": "second"},
                {"name": "x-step", "value": "provider"}, {"name": "x-step", "value": "model"}
            ]),
        "headers did not persist across hook contexts",
    )?;
    for (failure, expected) in [
        ("reject", ScriptErrorKind::JavaScript),
        ("return", ScriptErrorKind::Contract),
    ] {
        let mut input = draft("onRequest");
        input["meta"]["failure"] = json!(failure);
        let output = runtime.invoke(call(id, 6, input)).await;
        ensure(
            output.is_err_and(|error| error.kind() == expected),
            "failed async hook exposed partial state",
        )?;
    }
    let output = runtime.invoke(call(id, 7, draft("onRequest"))).await?;
    ensure(
        output["state"]["order"] == json!(["request", "completed"]),
        "failed or late hook polluted later request state",
    )?;
    stop(&runtime).await
}

const STREAM_CONTEXT_CONFIG: &str = r"
        let release;
        export default {
            async onRequest(ctx) {
                switch (ctx.meta.operation) {
                    case 'edit': ctx.event.text += ' edited'; break;
                    case 'replace': ctx.event = { text: 'replacement' }; break;
                    case 'drop': ctx.replaceEvents([]); break;
                    case 'split': ctx.replaceEvents([{text:'a'}, {text:'b'}]); break;
                    case 'twice': ctx.replaceEvents([]); ctx.replaceEvents([]); break;
                    case 'invalid': ctx.replaceEvents([null]); break;
                    case 'sparse': ctx.replaceEvents(new Array(1)); break;
                    case 'count': ctx.replaceEvents(new Array(65).fill({})); break;
                    case 'late-count': {
                        const events = [{}]; ctx.replaceEvents(events);
                        await Promise.resolve(); events.length = 65; break;
                    }
                    case 'readonly': ctx.replaceEvents = () => {}; break;
                    case 'return': ctx.replaceEvents([]); return {};
                    case 'reject': ctx.replaceEvents([]); throw new Error('synthetic');
                    case 'pending':
                        ctx.replaceEvents([{text:'uncommitted'}]);
                        await new Promise(resolve => { release = resolve; });
                        ctx.state.late = true; break;
                    case 'missing': if (typeof ctx.replaceEvents !== 'undefined') throw new Error(); break;
                }
                ctx.state.visited = true;
            },
            onFinish() { release(); }
        };
    ";

/// Opt-in VM evidence stays outside pure unit tests and requires the timeout driver.
pub(super) async fn stream(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(STREAM_CONTEXT_CONFIG).await?;
    let id = hook(&config, "onRequest")?;
    for phase in ["onUpstreamEvent", "onStreamEvent"] {
        for (operation, expected) in [
            ("edit", json!([{"text":"initial edited"}])),
            ("replace", json!([{"text":"replacement"}])),
            ("drop", json!([])),
            ("split", json!([{"text":"a"}, {"text":"b"}])),
        ] {
            let input = json!({"meta":{"phase":phase,"operation":operation},"state":{},"event":{"text":"initial"}});
            let output = runtime.invoke(call(id, 1, input)).await?;
            ensure(
                output["events"] == expected && output.get("event").is_none(),
                "stream projection lost edits, drop or output order",
            )?;
            ensure(
                output["state"]["visited"] == true,
                "stream state did not commit",
            )?;
        }
        for operation in [
            "twice",
            "invalid",
            "sparse",
            "count",
            "late-count",
            "readonly",
            "return",
            "reject",
        ] {
            let input = json!({"meta":{"phase":phase,"operation":operation},"state":{},"event":{}});
            ensure(
                runtime.invoke(call(id, 2, input)).await.is_err(),
                "invalid stream output committed",
            )?;
        }
    }
    let output = runtime
        .invoke(call(
            id,
            3,
            json!({"meta":{"phase":"onRequest","operation":"missing"},"state":{},"body":{}}),
        ))
        .await?;
    ensure(
        output.get("events").is_none(),
        "non-stream context exposed event output",
    )?;
    let pending = call(
        id,
        4,
        json!({"meta":{"phase":"onStreamEvent","operation":"pending"},"state":{},"event":{}}),
    );
    let cancellation = pending.cancellation.clone();
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancellation.cancel();
    ensure(
        waiting
            .await?
            .is_err_and(|error| error.kind() == ScriptErrorKind::Cancelled),
        "cancelled replacement escaped its draft",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 5, json!({})))
        .await?;
    let output = runtime
        .invoke(call(
            id,
            6,
            json!({"meta":{"phase":"onStreamEvent","operation":"drop"},"state":{},"event":{}}),
        ))
        .await?;
    ensure(
        output["events"] == json!([]) && output["state"].get("late").is_none(),
        "late stream output polluted a later call",
    )?;
    stop(&runtime).await
}
