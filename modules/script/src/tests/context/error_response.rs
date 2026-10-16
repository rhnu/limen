use serde_json::json;

use super::draft;
use crate::{
    ScriptErrorKind,
    tests::{Fixture, TestResult, call, ensure, hook, stop},
};

const SOURCE: &str = include_str!("../../../fixtures/context/error-response.js");

#[tokio::test]
async fn registered_global_and_pipeline_error_response_hooks_commit_owned_drafts() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let mut input = draft("onErrorResponse");
    input["state"]["order"] = json!([]);
    for layer in ["providers", "models"] {
        let name = if layer == "providers" { "p" } else { "m" };
        let id = config[layer][name]["pipeline"][0]["onErrorResponse"]
            .as_u64()
            .ok_or("pipeline error response hook missing")?;
        input = runtime.invoke(call(id, 1, input)).await.into_context()?;
    }
    input["meta"]["operation"] = json!("edit");
    let original = input.clone();
    let output = runtime
        .invoke(call(hook(&config, "onErrorResponse")?, 2, input))
        .await
        .into_context()?;
    ensure(
        output["state"]["order"] == json!(["provider", "model"])
            && output["state"]["nested"]["value"] == 2
            && output["status"] == 503
            && output["contentType"] == "application/json"
            && output["headers"][2] == json!({"name": "retry-after", "value": "5"}),
        "error response edits or registered pipeline order were lost",
    )?;
    let body: serde_json::Value =
        serde_json::from_str(output["body"].as_str().ok_or("edited body was not text")?)?;
    ensure(
        body == json!({"message": "translated", "original": "synthetic upstream error"})
            && output["meta"] == original["meta"]
            && output["error"] == original["error"],
        "text body editing changed original error facts",
    )?;
    for (operation, body, content_type) in [
        ("null", serde_json::Value::Null, serde_json::Value::Null),
        (
            "async-edit",
            json!("asynchronous translation"),
            json!("text/plain"),
        ),
    ] {
        let mut input = draft("onErrorResponse");
        input["meta"]["operation"] = json!(operation);
        let output = runtime
            .invoke(call(hook(&config, "onErrorResponse")?, 3, input))
            .await
            .into_context()?;
        ensure(
            output["body"] == body && output["contentType"] == content_type,
            "nullable or asynchronous error response edits were lost",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn error_response_readonly_fields_transport_headers_and_completion_values_are_enforced()
-> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let id = hook(&config, "onErrorResponse")?;
    for operation in [
        "meta",
        "meta-ref",
        "error",
        "error-ref",
        "state-ref",
        "headers-ref",
        "headers-method",
        "reject",
        "return",
        "async-return",
    ] {
        let mut input = draft("onErrorResponse");
        input["meta"]["operation"] = json!(operation);
        let expected = if matches!(operation, "return" | "async-return") {
            ScriptErrorKind::Contract
        } else {
            ScriptErrorKind::JavaScript
        };
        ensure(
            runtime
                .invoke(call(id, 1, input))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == expected),
            "error response hook accepted a forbidden edit or return value",
        )?;
    }
    for header in [
        "Content-Type",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
    ] {
        let mut input = draft("onErrorResponse");
        input["meta"]["operation"] = json!("transport-header");
        input["meta"]["header"] = json!(header);
        ensure(
            runtime
                .invoke(call(id, 2, input))
                .await
                .into_context()
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "error response hook changed a Rust-owned transport header",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn error_response_registration_rejects_non_functions() -> TestResult {
    for source in [
        "export default { onErrorResponse: null };",
        "export default { providers: { p: { pipeline: [{ name: 'error', onErrorResponse: 1 }] } } };",
        "export default { models: { m: { pipeline: [{ name: 'error', onErrorResponse: {} }] } } };",
    ] {
        let fixture = Fixture::new()?;
        ensure(
            fixture.start(source).await.is_err(),
            "non-function error response hook registered",
        )?;
    }
    Ok(())
}
