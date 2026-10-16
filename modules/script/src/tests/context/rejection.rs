use serde_json::{Value, json};

use super::draft;
use crate::{
    RuntimeStatus, ScriptErrorKind,
    tests::{Fixture, TestResult, call, ensure, hook, stop, wait_for},
};

fn input(phase: &str, operation: &str, status: Value) -> Value {
    let mut context = draft(phase);
    context["meta"]["operation"] = json!(operation);
    context["meta"]["status"] = status;
    context
}

#[tokio::test]
async fn successful_void_hooks_project_owned_rejection_and_draft() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../../fixtures/context/rejection.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for phase in ["onRequest", "beforeSend"] {
        for operation in ["sync", "async"] {
            for status in [400, 403, 422] {
                let output = runtime
                    .invoke(call(id, 1, input(phase, operation, json!(status))))
                    .await
                    .into_context()?;
                ensure(
                    output["rejection"] == json!({"status": status})
                        && output["state"]["before"] == true
                        && output["state"]["after"] == true
                        && output.get("reject").is_none(),
                    "business rejection did not project the owned effect and completed draft",
                )?;
                if operation == "async" {
                    ensure(
                        output["state"]["completed"] == true,
                        "business rejection bypassed Promise completion",
                    )?;
                }
            }
        }
        let mut context = input(phase, "sync", Value::Null);
        context["meta"]["defaultStatus"] = json!(true);
        let output = runtime.invoke(call(id, 2, context)).await.into_context()?;
        ensure(
            output["rejection"] == json!({"status": 400}),
            "missing rejection status did not use 400",
        )?;
        let output = runtime
            .invoke(call(id, 3, input(phase, "inspect", json!(422))))
            .await
            .into_context()?;
        ensure(
            output["rejection"] == json!({"status": 422}),
            "frozen nonenumerable reject method did not return undefined",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn rejection_arguments_and_reserved_fields_are_enforced() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../../fixtures/context/rejection.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for status in [
        json!(200),
        json!(401),
        json!(404),
        json!(500),
        json!(400.5),
        Value::Null,
        json!("422"),
        json!(true),
        json!({}),
        json!([]),
    ] {
        ensure(
            runtime
                .invoke(call(id, 1, input("onRequest", "sync", status)))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "invalid business rejection status was accepted",
        )?;
    }
    for operation in ["extra", "twice", "replace", "method", "forge"] {
        ensure(
            runtime
                .invoke(call(id, 2, input("beforeSend", operation, json!(422))))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "duplicate, extra, or forged rejection operation was accepted",
        )?;
    }
    for reserved in ["reject", "rejection"] {
        let mut context = json!({});
        context[reserved] = json!({"status": 422});
        ensure(
            runtime
                .invoke(call(id, 3, context))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "plain scheduler input forged a rejection effect",
        )?;
    }
    for phase in [
        "onRequest",
        "beforeSend",
        "afterReceive",
        "onResponse",
        "onUpstreamEvent",
        "onStreamEvent",
        "onError",
        "onFinish",
    ] {
        for reserved in ["reject", "rejection"] {
            let mut context = input(phase, "noop", json!(422));
            context[reserved] = json!({"status": 422});
            ensure(
                runtime
                    .invoke(call(id, 3, context))
                    .await
                    .into_context()
                    .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
                "reserved input field forged a rejection effect",
            )?;
        }
        if !matches!(phase, "onRequest" | "beforeSend") {
            let output = runtime
                .invoke(call(id, 4, input(phase, "noop", json!(422))))
                .await
                .into_context()?;
            ensure(
                output.get("reject").is_none() && output.get("rejection").is_none(),
                "restricted phase exposed a rejection effect",
            )?;
            ensure(
                runtime
                    .invoke(call(id, 5, input(phase, "inspect", json!(422))))
                    .await
                    .into_context()
                    .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
                "restricted phase exposed the rejection method",
            )?;
        }
    }
    ensure(
        matches!(runtime.status(), RuntimeStatus::Ready),
        "contract errors retired the VM",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn hook_errors_and_cancellation_prevent_rejection_commit() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../../fixtures/context/rejection.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for (operation, expected) in [
        ("throw", ScriptErrorKind::JavaScript),
        ("async-throw", ScriptErrorKind::JavaScript),
        ("return", ScriptErrorKind::Contract),
        ("async-return", ScriptErrorKind::Contract),
        ("invalid-draft", ScriptErrorKind::Json),
    ] {
        ensure(
            runtime
                .invoke(call(id, 1, input("onRequest", operation, json!(403))))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == expected),
            "a recorded business rejection replaced a hook failure",
        )?;
    }
    let pending = call(id, 2, input("beforeSend", "pending", json!(422)));
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
        "a recorded business rejection replaced cancellation",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 3, json!({})))
        .await
        .into_context()?;
    let original = input("onRequest", "noop", json!(400));
    let output = runtime
        .invoke(call(id, 4, original.clone()))
        .await
        .into_context()?;
    ensure(
        output == original && output.get("rejection").is_none(),
        "failed or late rejection draft polluted the next invocation",
    )?;
    stop(&runtime).await
}
