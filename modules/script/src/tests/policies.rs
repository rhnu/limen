use std::path::Path;

use serde_json::{Value, json};

use super::{TestResult, call, ensure, hook, stop};
use crate::{RuntimeOptions, ScriptRuntime};

async fn start() -> Result<(ScriptRuntime, u64), Box<dyn std::error::Error + Send + Sync>> {
    // The production loader resolves the fixture's relative import to the real library.
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gateway/policies/config.js");
    let options = RuntimeOptions {
        observation_names: ["capped", "scrubbed"].map(str::to_owned).to_vec(),
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path, options).await?;
    let id = hook(&config, "onRequest")?;
    Ok((runtime, id))
}

fn context(unit: &str) -> Value {
    json!({
        "meta":{"phase":"beforeSend","requestId":"synthetic"},
        "state":{},
        "body":{"unit":unit,"model":"synthetic","messages":[],"vendor":{"keep":[1,null]}},
        "url":"https://upstream.invalid/chat/completions",
        "headers":[{"name":"x-upstream-profile","value":"static"}],
        "client":{"headers":[]}
    })
}

#[tokio::test]
async fn factory_validation_is_early_safe_and_compatible_with_quickjs() -> TestResult {
    let (runtime, id) = start().await?;
    let output = runtime
        .invoke(call(id, 1, context("parameters")))
        .await
        .outcome?;
    ensure(
        output.context["state"]["checked"]
            .as_u64()
            .is_some_and(|count| count > 100),
        "factory validation did not reject the invalid parameter matrix",
    )?;
    ensure(
        output.observations.events.is_empty(),
        "constructing policies emitted observations",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn token_caps_preserve_extensions_and_only_record_changes() -> TestResult {
    let (runtime, id) = start().await?;
    for unit in ["cap", "legacy"] {
        let field = if unit == "cap" {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        for (input, expected, changed) in [
            (None, 16, true),
            (Some(1), 1, false),
            (Some(16), 16, false),
            (Some(17), 16, true),
            (Some(9_007_199_254_740_991_u64), 16, true),
        ] {
            let mut draft = context(unit);
            if let Some(value) = input {
                draft["body"][field] = json!(value);
            }
            let output = runtime.invoke(call(id, 1, draft)).await.outcome?;
            ensure(
                output.context["body"][field] == expected,
                "token cap changed the wrong value",
            )?;
            ensure(
                output.context["body"]["vendor"] == json!({"keep":[1,null]}),
                "token cap lost extensions",
            )?;
            ensure(
                output.context.get("rejection").is_none(),
                "valid token budget was rejected",
            )?;
            let expected_events = if changed { vec![0] } else { Vec::new() };
            ensure(
                output.observations.events == expected_events,
                "token observation differed from actual change",
            )?;
        }
    }
    stop(&runtime).await
}

#[tokio::test]
async fn invalid_and_ambiguous_token_budgets_reject_without_mutation_or_events() -> TestResult {
    let (runtime, id) = start().await?;
    for unit in ["cap", "legacy"] {
        let (field, other) = if unit == "cap" {
            ("max_completion_tokens", "max_tokens")
        } else {
            ("max_tokens", "max_completion_tokens")
        };
        for invalid in [
            json!(null),
            json!(0),
            json!(-1),
            json!(0.5),
            json!("16"),
            json!(true),
            json!({}),
        ] {
            let mut draft = context(unit);
            draft["body"][field] = invalid;
            let body = draft["body"].clone();
            let output = runtime.invoke(call(id, 1, draft)).await.outcome?;
            ensure(
                output.context["rejection"]["status"] == 422,
                "invalid budget was not a business rejection",
            )?;
            ensure(
                output.context["body"] == body,
                "rejected budget was changed",
            )?;
            ensure(
                output.observations.events.is_empty(),
                "rejected budget emitted a change event",
            )?;
        }
        for selected in [None, Some(json!(8))] {
            let mut draft = context(unit);
            draft["body"][other] = Value::Null;
            if let Some(value) = selected {
                draft["body"][field] = value;
            }
            let output = runtime.invoke(call(id, 1, draft)).await.outcome?;
            ensure(
                output.context["rejection"]["status"] == 422,
                "alternate token field was silently accepted",
            )?;
            ensure(
                output.observations.events.is_empty(),
                "ambiguous budget emitted a change event",
            )?;
        }
    }
    stop(&runtime).await
}

#[tokio::test]
async fn copied_field_configuration_and_shared_steps_keep_requests_isolated() -> TestResult {
    let (runtime, id) = start().await?;
    for remove in [true, false, true] {
        let mut draft = context("drop");
        if remove {
            draft["body"]["store"] = json!(false);
            draft["body"]["metadata"] = Value::Null;
        }
        let output = runtime.invoke(call(id, 1, draft)).await.outcome?;
        ensure(
            output.context["body"].get("store").is_none(),
            "copied policy failed to remove store",
        )?;
        ensure(
            output.context["body"].get("metadata").is_none(),
            "copied policy failed to remove null metadata",
        )?;
        ensure(
            output.context["body"]["vendor"] == json!({"keep":[1,null]}),
            "mutating options changed the captured field list",
        )?;
        ensure(
            output.context["body"]["messages"] == json!([]),
            "mutable options introduced a reserved deletion",
        )?;
        let expected = if remove { vec![1] } else { Vec::new() };
        ensure(
            output.observations.events == expected,
            "field deletion emitted the wrong observation",
        )?;
        ensure(
            output.context["state"] == json!({}),
            "policy retained per-request state",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn header_forwarding_uses_host_normalization_and_rejects_multiple_values() -> TestResult {
    let (runtime, id) = start().await?;
    for (headers, expected, rejected) in [
        (json!([]), "static", false),
        (
            json!([{"name":"X-PrOfIlE","value":"editor"}]),
            "editor",
            false,
        ),
        (json!([{"name":"x-profile","value":""}]), "", false),
        (
            json!([{"name":"X-Profile","value":"one"},{"name":"x-profile","value":"two"}]),
            "static",
            true,
        ),
    ] {
        let mut draft = context("header");
        draft["client"]["headers"] = headers.clone();
        let output = runtime.invoke(call(id, 1, draft)).await.outcome?;
        ensure(
            output.context["headers"] == json!([{"name":"x-upstream-profile","value":expected}]),
            "forwarded header changed normalization or missing-value behavior",
        )?;
        ensure(
            output.context["client"]["headers"] == headers,
            "forwarding changed the readonly client snapshot",
        )?;
        ensure(
            output.context.get("rejection").is_some() == rejected,
            "repeated client header was not rejected",
        )?;
        if rejected {
            ensure(
                output.context["rejection"]["status"] == 422,
                "header multiplicity used the wrong status",
            )?;
        }
        ensure(
            output.observations.events.is_empty(),
            "header forwarding invented observations",
        )?;
    }
    stop(&runtime).await
}
