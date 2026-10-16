use serde_json::{Value, json};

use super::{Fixture, TestResult, call, ensure, hook, stop, wait_for};
use crate::{RuntimeStatus, ScriptErrorKind};

mod client;
mod error_response;
mod rejection;

fn draft(phase: &str) -> Value {
    let mut value = json!({
        "meta": {"phase": phase, "requestId": "synthetic", "clientStream": false},
        "state": {"nested": {"value": 1}}
    });
    match phase {
        "onRequest" | "beforeSend" | "onResponse" => {
            value["body"] = json!({"model": "synthetic", "nested": {"value": 1}});
        },
        "afterReceive" => value["status"] = json!(200),
        "onErrorResponse" => {
            value["status"] = json!(502);
            value["body"] = json!("synthetic upstream error");
            value["contentType"] = json!("text/plain");
            value["error"] = json!({"code": "upstream_failed", "status": 502});
        },
        "onUpstreamEvent" | "onStreamEvent" => value["event"] = json!({"text": "synthetic"}),
        "onError" | "onFinish" => {
            value["outcome"] = json!("success");
            value["stats"] = json!({"outputBytes": 0});
            if phase == "onError" {
                value["error"] = json!({"code": "synthetic"});
            }
        },
        _ => panic!("unsupported test phase"),
    }
    if matches!(phase, "beforeSend" | "afterReceive" | "onErrorResponse") {
        value["headers"] = json!([
            {"name": "X-Example", "value": "first"},
            {"name": "x-example", "value": "second"}
        ]);
    }
    if phase == "beforeSend" {
        value["url"] = json!("https://fictional.invalid/v1/chat/completions");
    }
    value
}

#[tokio::test]
async fn headers() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/context/headers.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    let mut input = draft("beforeSend");
    input["meta"]["mode"] = json!("valid");
    let output = runtime.invoke(call(id, 1, input)).await.into_context()?;
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
            let output = runtime.invoke(call(id, 2, input)).await.into_context();
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
            runtime
                .invoke(call(id, 3, input))
                .await
                .into_context()
                .is_err(),
            "invalid header value was accepted",
        )?;
    }
    let mut input = draft("afterReceive");
    input["meta"]["mode"] = json!("get");
    input["headers"] = json!([{"name": "Host", "value": "fictional.invalid"}]);
    let output = runtime.invoke(call(id, 4, input)).await.into_context()?;
    ensure(
        output["headers"] == json!([{"name": "host", "value": "fictional.invalid"}]),
        "read access to transport header failed",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn readonly() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/context/readonly.js"))
        .await?;
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
            let result = runtime.invoke(call(id, 1, input)).await.into_context();
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

#[tokio::test]
async fn isolation() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/context/isolation.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    let mut input = draft("onRequest");
    input["meta"]["pending"] = json!(true);
    let pending = call(id, 1, input.clone());
    let cancellation = pending.cancellation.clone();
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await.into_context() });
    wait_for(
        &runtime,
        hook(&config, "onError")?,
        json!({"started": true}),
    )
    .await?;
    let mut second = draft("onRequest");
    second["meta"]["requestId"] = json!("independent");
    let mut output = runtime.invoke(call(id, 2, second)).await.into_context()?;
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
        .await
        .into_context()?;
    let mut next = draft("beforeSend");
    next["state"] = output["state"].take();
    next["body"] = output["body"].take();
    output = next;
    let provider = config["providers"]["p"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("missing provider hook")?;
    let model = config["models"]["m"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("missing model hook")?;
    let output = runtime
        .invoke(call(provider, 4, output))
        .await
        .into_context()?;
    let output = runtime
        .invoke(call(model, 5, output))
        .await
        .into_context()?;
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
        let output = runtime.invoke(call(id, 6, input)).await.into_context();
        ensure(
            output.is_err_and(|error| error.kind() == expected),
            "failed async hook exposed partial state",
        )?;
    }
    let output = runtime
        .invoke(call(id, 7, draft("onRequest")))
        .await
        .into_context()?;
    ensure(
        output["state"]["order"] == json!(["request", "completed"]),
        "failed or late hook polluted later request state",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn stream() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/context/stream.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for phase in ["onUpstreamEvent", "onStreamEvent"] {
        for (operation, expected) in [
            ("edit", json!([{"text":"initial edited"}])),
            ("replace", json!([{"text":"replacement"}])),
            ("drop", json!([])),
            ("split", json!([{"text":"a"}, {"text":"b"}])),
        ] {
            let input = json!({"meta":{"phase":phase,"operation":operation},"state":{},"event":{"text":"initial"}});
            let output = runtime.invoke(call(id, 1, input)).await.into_context()?;
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
                runtime
                    .invoke(call(id, 2, input))
                    .await
                    .into_context()
                    .is_err(),
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
        .await
        .into_context()?;
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
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await.into_context() });
    wait_for(
        &runtime,
        hook(&config, "onError")?,
        json!({"started": true}),
    )
    .await?;
    cancellation.cancel();
    ensure(
        waiting
            .await?
            .is_err_and(|error| error.kind() == ScriptErrorKind::Cancelled),
        "cancelled replacement escaped its draft",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 5, json!({})))
        .await
        .into_context()?;
    let output = runtime
        .invoke(call(
            id,
            6,
            json!({"meta":{"phase":"onStreamEvent","operation":"drop"},"state":{},"event":{}}),
        ))
        .await
        .into_context()?;
    ensure(
        output["events"] == json!([]) && output["state"].get("late").is_none(),
        "late stream output polluted a later call",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn stream_envelope_is_an_owned_readonly_view_independent_of_data_edits() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(
            r"
        export default { providers:{}, models:{}, onRequest(ctx) {
            if (ctx.meta.operation === 'field') ctx.sse.event = 'changed';
            else if (ctx.meta.operation === 'replace') ctx.sse = null;
            else { ctx.state.envelope = ctx.sse; ctx.event = {annotations:[]}; }
        }};
    ",
        )
        .await?;
    let id = hook(&config, "onRequest")?;
    let envelope = json!({"event":"annotation","id":"frame-1","retry":"1500"});
    for phase in ["onUpstreamEvent", "onStreamEvent"] {
        let input =
            json!({"meta":{"phase":phase,"operation":"read"},"state":{},"event":{},"sse":envelope});
        let output = runtime.invoke(call(id, 1, input)).await.into_context()?;
        ensure(
            output["sse"] == envelope && output["state"]["envelope"] == envelope,
            "stream envelope was not preserved",
        )?;
        ensure(
            output["events"] == json!([{"annotations":[]}]),
            "data replacement changed the envelope",
        )?;
        for operation in ["field", "replace"] {
            let input = json!({"meta":{"phase":phase,"operation":operation},"state":{},"event":{},"sse":envelope});
            ensure(
                runtime
                    .invoke(call(id, 2, input))
                    .await
                    .into_context()
                    .is_err(),
                "stream envelope mutation committed",
            )?;
        }
    }
    stop(&runtime).await
}
