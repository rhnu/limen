use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};
use crate::{RuntimeOptions, ScriptErrorKind, ScriptRuntime};

#[tokio::test]
async fn embedded_factories_snapshot_frozen_native_descriptors() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(r#"
        import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "limen:policies";
        import { replaceText, syntheticTextTool } from "limen:content";
        const fields = ["store", "metadata"];
        const options = { name: "drop", fields, onChange: "changed" };
        const drop = dropRequestFields(options);
        fields[0] = "messages"; fields.push("stream"); options.onChange = "later";
        const capOptions = { name: "cap", max: 16 };
        const cap = capCompletionTokens(capOptions); capOptions.max = 1;
        const steps = [cap, drop,
          forwardClientHeader({ name: "header", from: "X-Profile", to: "x-upstream" }),
          replaceText({ name: "replace", search: "abc", replacement: "x", maxBufferedChars: 2 }),
          syntheticTextTool({ name: "tool", maxBufferedChars: 512 })];
        export default { steps, frozen: steps.every(step => Object.isFrozen(step) && Object.isFrozen(step.builtin))
          && Object.isFrozen(drop.builtin.fields) };
    "#).await?;
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
    let (runtime, config) = fixture.start(r#"
        import * as policies from "limen:policies";
        import * as content from "limen:content";
        const factories = [
          [policies.capCompletionTokens, {name:"cap", max:1}],
          [policies.dropRequestFields, {name:"drop", fields:["store"]}],
          [policies.forwardClientHeader, {name:"header", from:"x-profile", to:"x-upstream"}],
          [content.replaceText, {name:"replace", search:"a", replacement:"b", maxBufferedChars:1}],
          [content.syntheticTextTool, {name:"tool", maxBufferedChars:512}]];
        let rejected = 0;
        for (const [factory, options] of factories) {
          for (const value of [null, [], 1, "private-value", Object.create({name:"inherited"}),
              {...options, stateKey:"old-state"}, {...options, unknown:true}, {...options, [Symbol()]:true}]) {
            try { factory(value); } catch (error) { if (error instanceof TypeError) rejected++; }
          }
        }
        export default {rejected, hidden: policies.descriptor === undefined && content.descriptor === undefined};
    "#).await?;
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
    let source = r#"
        import * as first from "limen:policies";
        import { capCompletionTokens as repeated } from "./reexport.js";
        import { capCompletionTokens as local } from "./policies.js";
        const dynamic = await import("limen:policies");
        export default { same: first === dynamic && first.capCompletionTokens === repeated,
          local, kind: repeated({ name:"cap", max:1 }).builtin.kind };
    "#;
    let path = fixture.write("config.js", source)?;
    let options = RuntimeOptions {
        max_modules: 4,
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
        "limen:policies/extra",
        "limen:content?query",
        "Limen:content",
    ] {
        let source = format!("import {{ anything }} from '{name}'; export default {{}};");
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
    let source = "import {capCompletionTokens} from 'limen:policies'; export default capCompletionTokens({name:'cap',max:1});";
    let path = fixture.write("config.js", source)?;
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
async fn embedded_factory_failures_keep_sanitized_module_coordinates() -> TestResult {
    let fixture = Fixture::new()?;
    let result = fixture
        .start(
            r#"
        import { replaceText } from "limen:content";
        export default replaceText({name:"private-value", stateKey:"removed"});
    "#,
        )
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
        "{}\nexport default {{onRequest(ctx) {{ctx.state.checked = runContentPolicyTests();}}}};",
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
