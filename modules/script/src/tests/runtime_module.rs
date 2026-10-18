use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};
use crate::{JsonLimits, RuntimeOptions, ScriptErrorKind, ScriptRuntime};

#[tokio::test]
async fn native_runtime_exports_share_esm_identity_and_reject_namespace_mutation() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "runtime-reexport.mjs",
        "export { env } from 'limen:runtime';",
    )?;
    // A nearby local file must not replace the reserved builtin.
    fixture.write("runtime.js", "export const env = 'local';")?;
    let path = fixture.write(
        "gateway.js",
        include_str!("../../fixtures/loader/runtime.js"),
    )?;
    let (runtime, config) = ScriptRuntime::start(
        path,
        RuntimeOptions {
            json_limits: JsonLimits {
                max_bytes: 1024,
                ..JsonLimits::default()
            },
            ..RuntimeOptions::default()
        },
    )
    .await?;
    ensure(
        config["same"] == true,
        "native runtime import identities diverged",
    )?;
    let output = runtime
        .invoke_fixture(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output["checked"] == true,
        "native runtime binding or configured bound changed",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn native_runtime_counts_once_without_charging_javascript_source_bytes() -> TestResult {
    let fixture = Fixture::new()?;
    let source = "import {env} from 'limen:runtime'; import * as runtime from 'limen:runtime'; export default () => ({same:env===runtime.env});";
    let path = fixture.write("gateway.js", source)?;
    let options = RuntimeOptions {
        max_modules: 2,
        max_source_bytes: source.len(),
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path.clone(), options.clone()).await?;
    ensure(
        config["same"] == true,
        "native module repeated imports were not cached",
    )?;
    stop(&runtime).await?;
    let result = ScriptRuntime::start(
        path,
        RuntimeOptions {
            max_modules: 1,
            ..options
        },
    )
    .await;
    match result {
        Err(error) => ensure(
            error.kind() == ScriptErrorKind::Module,
            "native module count used the wrong diagnostic",
        ),
        Ok((runtime, _)) => {
            stop(&runtime).await?;
            Err("native module bypassed the module count budget".into())
        },
    }
}

#[tokio::test]
async fn native_runtime_rejects_unknown_exports_and_module_suffixes() -> TestResult {
    let fixture = Fixture::new()?;
    for source in [
        "import {missing} from 'limen:runtime'; export default () => ({});",
        "import 'limen:runtime/extra'; export default () => ({});",
        "import 'limen:runtime?query'; export default () => ({});",
        "import 'limen:runtime#fragment'; export default () => ({});",
        "import 'limen:runtime' with {type:'json'}; export default () => ({});",
    ] {
        let path = fixture.write("gateway.js", source)?;
        match ScriptRuntime::start(path, RuntimeOptions::default()).await {
            Err(error) => ensure(
                error.kind() == ScriptErrorKind::Module,
                "invalid native import used the wrong diagnostic",
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid native module import was accepted".into());
            },
        }
    }
    Ok(())
}
