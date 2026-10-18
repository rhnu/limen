use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};
use crate::{RuntimeOptions, ScriptErrorKind, ScriptRuntime};

#[tokio::test]
async fn embedded_factories_snapshot_frozen_native_descriptors() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/policies/descriptors.js"))
        .await?;
    ensure(
        config
            == json!({
                "steps": [
                    {"name":"cap","builtin":{"kind":"capCompletionTokens","max":16}},
                    {"name":"drop","builtin":{"kind":"dropRequestFields","fields":["store","metadata"],"onChange":"changed"}},
                    {"name":"header","builtin":{"kind":"forwardClientHeader","from":"X-Profile","to":"x-upstream"}},
                    {"name":"replace","builtin":{"kind":"replaceText","search":"abc","replacement":"x","maxBufferedChars":2}},
                    {"name":"tool","builtin":{"kind":"syntheticTextTool","maxBufferedChars":512}}
                ], "frozen":true
            }),
        "embedded factories changed descriptors, captured mutable options or registered JS hooks",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn embedded_factories_reject_unknown_options_without_exporting_private_helpers() -> TestResult
{
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/policies/invalid-options.js"))
        .await?;
    ensure(
        config == json!({"rejected":40,"hidden":true}),
        "native factories accepted malformed or unknown options, or exported private helpers",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn embedded_modules_share_esm_cache_and_reserve_exact_names() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("policies.js", "export const capCompletionTokens = 'local';")?;
    fixture.write(
        "reexport.js",
        "export { capCompletionTokens } from 'limen:policies';",
    )?;
    let source = include_str!("../../fixtures/policies/module-cache.js");
    let path = fixture.write("gateway.js", source)?;
    let options = RuntimeOptions {
        max_modules: 5,
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path, options).await?;
    ensure(
        config == json!({"same":true,"local":"local","kind":"capCompletionTokens"}),
        "embedded module identity or local-module separation failed",
    )?;
    stop(&runtime).await?;
    for name in [
        "limen:unknown",
        "limen:descriptor",
        "limen:internal/descriptor",
        "limen:policies/internal/descriptor.js",
        "limen:policies/extra",
        "limen:content?query",
        "limen:gateway?query",
        "limen:gateway/extra",
        "Limen:content",
    ] {
        let source = format!("import {{ anything }} from '{name}'; export default () => ({{}});");
        match fixture.start(&source).await {
            Err(error) => ensure(
                error
                    .downcast_ref::<crate::ScriptError>()
                    .is_some_and(|error| error.kind() == ScriptErrorKind::Module),
                "unknown embedded name used the wrong diagnostic",
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("unknown embedded module was accepted".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn embedded_module_sources_and_count_share_startup_budgets() -> TestResult {
    let fixture = Fixture::new()?;
    let source = "import {capCompletionTokens} from 'limen:policies'; export default () => (capCompletionTokens({name:'cap',max:1}));";
    let path = fixture.write("gateway.js", source)?;
    for options in [
        RuntimeOptions {
            max_modules: 1,
            ..RuntimeOptions::default()
        },
        RuntimeOptions {
            max_source_bytes: source.len() + 1,
            ..RuntimeOptions::default()
        },
    ] {
        match ScriptRuntime::start(path.clone(), options).await {
            Err(error) => ensure(
                error.kind() == ScriptErrorKind::Module,
                "embedded budget used the wrong failure",
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("embedded source bypassed startup budgets".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn bundled_private_dependencies_share_the_public_modules_byte_and_count_budget() -> TestResult
{
    let fixture = Fixture::new()?;
    let source = include_str!("../../fixtures/policies/bundle-budget.js");
    let path = fixture.write("gateway.js", source)?;
    // Measure the embedded data, so build formatting can change without changing the contract.
    let bytes = source.len()
        + crate::builtins::find("limen:policies")
            .ok_or("missing policies module")?
            .source()
            .len()
        + crate::builtins::find("limen:content")
            .ok_or("missing content module")?
            .source()
            .len();
    let options = RuntimeOptions {
        max_modules: 3,
        max_source_bytes: bytes,
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path.clone(), options.clone()).await?;
    ensure(
        config == json!({"same":true,"policy":"capCompletionTokens","content":"replaceText"}),
        "private dependencies or cached imports changed public module accounting",
    )?;
    stop(&runtime).await?;
    for options in [
        RuntimeOptions {
            max_source_bytes: bytes - 1,
            ..options.clone()
        },
        RuntimeOptions {
            max_modules: 2,
            ..options
        },
    ] {
        match ScriptRuntime::start(path.clone(), options).await {
            Err(error) => ensure(
                error.kind() == ScriptErrorKind::Module,
                "bundle budget used the wrong failure",
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("bundled module escaped the shared source or count budget".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn embedded_factory_failures_keep_sanitized_module_coordinates() -> TestResult {
    let fixture = Fixture::new()?;
    let result = fixture
        .start(include_str!("../../fixtures/policies/invalid-factory.js"))
        .await;
    match result {
        Err(error) => {
            let error = error
                .downcast_ref::<crate::ScriptError>()
                .ok_or("factory failure lost its script diagnostic")?;
            let diagnostic = error.diagnostic().to_string();
            ensure(
                error.kind() == ScriptErrorKind::Module
                    && diagnostic.contains("exception=TypeError")
                    && diagnostic.contains("module=import#1, line=")
                    && !diagnostic.contains("private-value")
                    && !diagnostic.contains("limen:content"),
                "embedded module failure lost coordinates or exposed source text",
            )
        },
        Ok((runtime, _)) => {
            stop(&runtime).await?;
            Err("invalid factory startup succeeded".into())
        },
    }
}

#[tokio::test]
async fn complete_part_callback_runs_in_quickjs_with_bounded_serialized_state() -> TestResult {
    let fixture = Fixture::new()?;
    let source = format!(
        "{}\nexport default () => ({{onRequest(ctx) {{ctx.state.checked = runContentPolicyTests();}}}});",
        include_str!("../../../../sdk/tests/content-runtime.js")
    );
    let (runtime, config) = fixture.start(&source).await?;
    let result = runtime
        .invoke_fixture(call(
            hook(&config, "onRequest")?,
            1,
            json!({"meta":{"phase":"onRequest"},"state":{},"body":{}}),
        ))
        .await
        .into_context()?;
    ensure(
        result["state"]["checked"] == true,
        "complete-part callback suite failed",
    )?;
    stop(&runtime).await
}
