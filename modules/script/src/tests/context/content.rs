use serde_json::{Value, json};

use super::draft;
use crate::{
    RuntimeOptions, ScriptErrorKind, ScriptRuntime,
    tests::{Fixture, TestResult, call, ensure, hook, stop, wait_for},
};

const SOURCE: &str = include_str!("../../../fixtures/context/content.js");

fn input(operation: &str) -> Value {
    let mut context = draft("onContent");
    context["meta"]["operation"] = json!(operation);
    context["event"] = json!({"nested": {"value": 1}});
    context
}

fn content_hook(config: &Value) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    hook(&config["providers"]["p"]["pipeline"][0], "onContent")
}

#[tokio::test]
async fn content_registration_rejects_non_functions_on_both_pipeline_layers() -> TestResult {
    for layer in ["providers", "models"] {
        for value in [
            "null",
            "1",
            "{}",
            "'private-function'",
            "new Proxy(() => {}, {})",
        ] {
            let fixture = Fixture::new()?;
            let source = format!(
                "export default {{ {layer}: {{ p: {{ pipeline: [{{ name: 'content', onContent: {value} }}] }} }} }};"
            );
            match fixture.start(&source).await {
                Err(error) => ensure(
                    error
                        .downcast_ref::<crate::ScriptError>()
                        .is_some_and(|error| error.kind() == ScriptErrorKind::Json),
                    "invalid content registration returned an unexpected failure",
                )?,
                Ok((runtime, _)) => {
                    stop(&runtime).await?;
                    return Err("non-function content hook registered".into());
                },
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn registered_content_hooks_project_owned_events_and_transactional_state() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let provider = content_hook(&config)?;
    let model = hook(&config["models"]["m"]["pipeline"][0], "onContent")?;
    for envelope in [Value::Null, json!({"event":"message", "id":"source-1"})] {
        let mut context = input("edit");
        context["sse"] = envelope.clone();
        let output = runtime
            .invoke(call(provider, 1, context))
            .await
            .into_context()?;
        ensure(
            output["events"]
                == json!([{
                    "nested": {"value": 2}, "extensions": {"custom": [true, null, "owned"]}
                }])
                && output["state"]["visits"] == 1
                && output["state"]["source"] == envelope
                && output["sse"] == envelope,
            "content edits, source view or request state did not commit",
        )?;
        let mut next = input("noop");
        next["state"] = output["state"].clone();
        next["event"] = output["events"][0].clone();
        let output = runtime.invoke(call(model, 2, next)).await.into_context()?;
        ensure(
            output["state"]["visits"] == 2 && output["events"][0]["model"] == true,
            "registered model content hook did not receive the previous committed draft",
        )?;
    }
    let step = &config["providers"]["p"]["pipeline"][0];
    for (phase, field, expected) in [
        ("onResponse", "body", "response"),
        ("onUpstreamEvent", "events", "upstream"),
        ("onStreamEvent", "events", "stream"),
    ] {
        let output = runtime
            .invoke(call(hook(step, phase)?, 3, draft(phase)))
            .await
            .into_context()?;
        let lower = if field == "events" {
            &output[field][0]["lower"]
        } else {
            &output[field]["lower"]
        };
        ensure(
            lower == expected,
            "transport hook registration was replaced by onContent",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn content_failures_discard_event_and_state_drafts_and_preserve_readonly_facts() -> TestResult
{
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let id = content_hook(&config)?;
    for (operation, expected) in [
        ("throw", ScriptErrorKind::JavaScript),
        ("reject", ScriptErrorKind::JavaScript),
        ("return", ScriptErrorKind::Contract),
        ("invalid", ScriptErrorKind::Json),
        ("body", ScriptErrorKind::JavaScript),
        ("meta", ScriptErrorKind::JavaScript),
        ("sse-ref", ScriptErrorKind::JavaScript),
        ("sse-field", ScriptErrorKind::JavaScript),
        ("sse-nested", ScriptErrorKind::JavaScript),
        ("state-ref", ScriptErrorKind::JavaScript),
    ] {
        let mut context = input(operation);
        context["sse"] = json!({"id":"source-1", "extra":{"value":1}});
        let original = context.clone();
        ensure(
            runtime
                .invoke(call(id, 1, context))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == expected),
            "failed content hook exposed a partial event or state draft",
        )?;
        let mut next = original;
        next["meta"]["operation"] = json!("noop");
        let output = runtime.invoke(call(id, 2, next)).await.into_context()?;
        ensure(
            output["state"]["visits"] == 1
                && output["events"] == json!([{"nested":{"value":1}}])
                && output["sse"] == json!({"id":"source-1", "extra":{"value":1}}),
            "failed content draft affected the next invocation",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn content_replacement_count_is_bounded_at_call_and_commit() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let options = RuntimeOptions {
        max_output_events: 1,
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path, options).await?;
    let id = content_hook(&config)?;
    for count in [0, 1, 2] {
        let mut context = input("count");
        context["meta"]["count"] = json!(count);
        let result = runtime.invoke(call(id, 1, context)).await.into_context();
        if count <= 1 {
            ensure(
                result?["events"]
                    .as_array()
                    .is_some_and(|events| events.len() == count),
                "bounded replacement lost zero or one event",
            )?;
        } else {
            ensure(
                result.is_err(),
                "content replacement exceeded the configured limit",
            )?;
        }
    }
    ensure(
        runtime
            .invoke(call(id, 2, input("late-count")))
            .await
            .into_context()
            .is_err(),
        "retained replacement array grew beyond its limit at commit",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn cancelled_content_cannot_commit_late_events_or_request_state() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let id = content_hook(&config)?;
    let pending = call(id, 1, input("pending"));
    let cancellation = pending.cancellation.clone();
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await.into_context() });
    let status = hook(&config, "onError")?;
    wait_for(&runtime, status, json!({"started":true,"finished":false})).await?;
    cancellation.cancel();
    ensure(
        waiting
            .await?
            .is_err_and(|error| error.kind() == ScriptErrorKind::Cancelled),
        "cancelled content call committed a draft",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 2, json!({})))
        .await
        .into_context()?;
    wait_for(&runtime, status, json!({"started":true,"finished":true})).await?;
    let output = runtime
        .invoke(call(id, 3, input("noop")))
        .await
        .into_context()?;
    ensure(
        output["state"]["visits"] == 1
            && output["state"].get("late").is_none()
            && output["events"] == json!([{"nested":{"value":1}}]),
        "late content completion polluted a later invocation",
    )?;
    stop(&runtime).await
}
