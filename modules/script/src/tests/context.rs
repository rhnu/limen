use serde_json::{Value, json};

use super::{Fixture, TestResult, call, ensure, hook, stop, wait_for};
use crate::{RuntimeStatus, ScriptErrorKind};

mod client;
mod content;
mod error_response;
mod rejection;

pub(super) fn draft(phase: &str) -> Value {
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
        "onContent" | "onUpstreamEvent" | "onStreamEvent" => {
            value["event"] = json!({"text": "synthetic"});
            value["sse"] = Value::Null;
        },
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
    let output = runtime
        .invoke_fixture(call(id, 1, input))
        .await
        .into_context()?;
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
            let output = runtime
                .invoke_fixture(call(id, 2, input))
                .await
                .into_context();
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
                .invoke_fixture(call(id, 3, input))
                .await
                .into_context()
                .is_err(),
            "invalid header value was accepted",
        )?;
    }
    let mut input = draft("afterReceive");
    input["meta"]["mode"] = json!("get");
    input["headers"] = json!([{"name": "Host", "value": "fictional.invalid"}]);
    let output = runtime
        .invoke_fixture(call(id, 4, input))
        .await
        .into_context()?;
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
            let result = runtime
                .invoke_fixture(call(id, 1, input))
                .await
                .into_context();
            if allowed {
                let output = result?;
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
    let waiting =
        tokio::spawn(async move { waiting_runtime.invoke_fixture(pending).await.into_context() });
    wait_for(
        &runtime,
        hook(&config, "onError")?,
        json!({"started": true}),
    )
    .await?;
    let mut second = draft("onRequest");
    second["meta"]["requestId"] = json!("independent");
    let mut output = runtime
        .invoke_fixture(call(id, 2, second))
        .await
        .into_context()?;
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
        .invoke_fixture(call(hook(&config, "onFinish")?, 3, json!({})))
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
        .invoke_fixture(call(provider, 4, output))
        .await
        .into_context()?;
    let mut next = draft("beforeSend");
    for key in ["state", "body", "url", "headers"] {
        next[key] = output[key].clone();
    }
    let output = runtime
        .invoke_fixture(call(model, 5, next))
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
        let output = runtime
            .invoke_fixture(call(id, 6, input))
            .await
            .into_context();
        ensure(
            output.is_err_and(|error| error.kind() == expected),
            "failed async hook exposed partial state",
        )?;
    }
    let output = runtime
        .invoke_fixture(call(id, 7, draft("onRequest")))
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
    for phase in ["onContent", "onUpstreamEvent", "onStreamEvent"] {
        for (operation, expected) in [
            ("edit", json!([{"text":"initial edited"}])),
            ("replace", json!([{"text":"replacement"}])),
            ("drop", json!([])),
            ("split", json!([{"text":"a"}, {"text":"b"}])),
        ] {
            let input = json!({"meta":{"phase":phase,"operation":operation},"state":{},"event":{"text":"initial"}});
            let output = runtime
                .invoke_fixture(call(id, 1, input))
                .await
                .into_context()?;
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
                    .invoke_fixture(call(id, 2, input))
                    .await
                    .into_context()
                    .is_err(),
                "invalid stream output committed",
            )?;
        }
    }
    let output = runtime
        .invoke_fixture(call(
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
    let waiting =
        tokio::spawn(async move { waiting_runtime.invoke_fixture(pending).await.into_context() });
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
        .invoke_fixture(call(hook(&config, "onFinish")?, 5, json!({})))
        .await
        .into_context()?;
    let output = runtime
        .invoke_fixture(call(
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
        .start(include_str!("../../fixtures/context/stream-envelope.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    let envelope = json!({"event":"annotation","id":"frame-1","retry":"1500"});
    for phase in ["onContent", "onUpstreamEvent", "onStreamEvent"] {
        let input =
            json!({"meta":{"phase":phase,"operation":"read"},"state":{},"event":{},"sse":envelope});
        let output = runtime
            .invoke_fixture(call(id, 1, input))
            .await
            .into_context()?;
        ensure(
            output["state"]["envelope"] == envelope,
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
                    .invoke_fixture(call(id, 2, input))
                    .await
                    .into_context()
                    .is_err(),
                "stream envelope mutation committed",
            )?;
        }
    }
    stop(&runtime).await
}

#[tokio::test]
async fn captured_intrinsics_survive_config_extraction_and_capability_projection() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write(
        "gateway.js",
        include_str!("../../fixtures/context/intrinsics.js"),
    )?;
    let options = crate::RuntimeOptions {
        observation_names: vec!["changed".to_owned()],
        ..crate::RuntimeOptions::default()
    };
    let (runtime, config) = crate::ScriptRuntime::start(path, options).await?;
    let id = hook(&config, "onRequest")?;
    for phase in ["beforeSend", "onContent", "onFinish"] {
        let input = draft(phase);
        let output = runtime
            .invoke_fixture(call(id, 1, input.clone()))
            .await
            .outcome?;
        ensure(
            output.observations.events == [0],
            "captured observation operations failed",
        )?;
        match phase {
            "beforeSend" => {
                ensure(
                    output.context["headers"]
                        == json!([{ "name": "x-example", "value": "isolated" }]),
                    "captured header operations failed",
                )?;
                ensure(
                    output.context["rejection"] == json!({ "status": 403 }),
                    "captured rejection operations failed",
                )?;
                ensure(
                    output.context["state"]["checked"] == true,
                    "request state did not commit",
                )?;
            },
            "onContent" => {
                ensure(
                    output.context["events"] == json!([{ "text": "captured" }]),
                    "captured replacement operations failed",
                )?;
                ensure(
                    output.context["state"]["checked"] == true,
                    "content state did not commit",
                )?;
            },
            "onFinish" => ensure(
                output.context["state"] == input["state"],
                "observer capability changed final facts",
            )?,
            _ => unreachable!(),
        }
    }
    stop(&runtime).await
}

#[test]
fn private_projection_preserves_readonly_facts_for_every_stage() -> TestResult {
    use rquickjs::{Context, Function, Object, Runtime};
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> TestResult {
        let options = crate::RuntimeOptions::default();
        crate::host::install(&ctx, options.json_limits.max_bytes)?;
        let bridge = crate::bridge::install(&ctx, &options)?;
        for phase in ["onRequest", "beforeSend", "afterReceive", "onResponse", "onContent", "onUpstreamEvent", "onStreamEvent", "onErrorResponse", "onError", "onFinish"] {
            let mut input = draft(phase);
            input["client"] = json!({"headers":[{"name":"X-Original","value":"first"},{"name":"x-original","value":"second"}]});
            let original = input.clone();
            let input = ctx.json_parse(input.to_string())?;
            let prepare = bridge.prepare.clone().restore(&ctx)?;
            let draft: rquickjs::Value = prepare.call((input, phase))?;
            let inspect: Function = ctx.eval(include_str!("../../fixtures/context/inspect-readonly.js"))?;
            inspect.call::<_, ()>((draft.clone(),))?;
            let project = bridge.project.clone().restore(&ctx)?;
            let envelope: Object = project.call((draft,))?;
            let encode = bridge.encode.clone().restore(&ctx)?;
            let encoded: String = encode.call((envelope,))?;
            let projected: Value = serde_json::from_str(&encoded)?;
            for key in ["meta", "client", "sse", "error", "stats", "outcome"] {
                ensure(projected["context"][key] == original[key], "private projection changed readonly facts")?;
            }
        }
        Ok(())
    })
}

#[test]
fn private_context_rejects_unprepared_roots_missing_meta_and_forged_phase() -> TestResult {
    use rquickjs::{Context, Runtime};
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> TestResult {
        let options = crate::RuntimeOptions::default();
        crate::host::install(&ctx, options.json_limits.max_bytes)?;
        let bridge = crate::bridge::install(&ctx, &options)?;
        for input in [json!({}), json!({"state":{},"body":{}}), draft("onFinish")] {
            let prepare = bridge.prepare.clone().restore(&ctx)?;
            let input = ctx.json_parse(input.to_string())?;
            let error = prepare
                .call::<_, rquickjs::Value>((input, "onRequest"))
                .unwrap_err();
            ensure(
                bridge.failure(&ctx, error).kind() == ScriptErrorKind::JavaScript,
                "missing metadata or forged phase entered the VM",
            )?;
        }
        let project = bridge.project.clone().restore(&ctx)?;
        let input = ctx.json_parse("{\"state\":{},\"body\":{}}")?;
        let error = project.call::<_, rquickjs::Value>((input,)).unwrap_err();
        ensure(
            bridge.failure(&ctx, error).kind() == ScriptErrorKind::JavaScript,
            "unprepared draft was projected",
        )
    })
}
