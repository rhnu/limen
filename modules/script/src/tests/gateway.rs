use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};
use crate::ScriptErrorKind;

#[tokio::test]
async fn composition_preserves_pipeline_positions_and_seals_registration_scopes() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/gateway/composition.js"))
        .await?;
    let provider = &config["providers"]["__proto__"];
    ensure(
        provider["pipeline"]
            .as_array()
            .is_some_and(|steps| steps.len() == 3),
        "failed step setup left a partial pipeline entry",
    )?;
    ensure(
        provider["pipeline"][0] == config["models"]["constructor"]["pipeline"][0]
            && provider["pipeline"][0] == config["models"]["__proto__"]["pipeline"][0]
            && provider["pipeline"][0]["builtin"]["max"] == 16
            && provider["pipeline"][1]["beforeSend"].is_u64()
            && provider["pipeline"][1]["onResponse"].is_u64()
            && config["models"]["constructor"]["provider"] == "__proto__",
        "native/JS declarations or reserved identifiers changed during capture",
    )?;
    let output = runtime
        .invoke_fixture(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output == json!({"builds":1,"rejected":3,"sealed":7,"aborted":4}),
        "setup ran repeatedly or escaped scopes remained open",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn startup_rejects_invalid_factories_duplicates_and_setup_completion_values() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "cases.mjs",
        include_str!("../../fixtures/gateway/cases.mjs"),
    )?;
    for index in 0..28 {
        let source = format!("import {{cases}} from './cases.mjs'; export default cases[{index}];");
        match fixture.start(&source).await {
            Err(error) => ensure(
                error
                    .downcast_ref::<crate::ScriptError>()
                    .is_some_and(|error| {
                        error.kind() == ScriptErrorKind::Json
                            && !error
                                .diagnostic()
                                .to_string()
                                .contains("private setup failure")
                    }),
                &format!("invalid composition case {index} changed its controlled failure"),
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err(format!("invalid composition case {index} was accepted").into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn startup_calls_the_factory_once_without_arguments_before_capturing_hooks() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start("let calls=0; export default function() { if (++calls !== 1 || arguments.length !== 0) throw Error(); return {onRequest(ctx) {ctx.state.calls=calls;}}; }")
        .await?;
    let output = runtime
        .invoke_fixture(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output == json!({"calls":1}),
        "factory invocation contract changed",
    )?;
    stop(&runtime).await
}
