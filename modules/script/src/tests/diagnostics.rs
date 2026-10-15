//! Synthetic exceptions exercise diagnostics without exposing script-controlled text.

use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};
use crate::{RuntimeOptions, ScriptError, ScriptErrorKind, ScriptRuntime};

fn check(error: &ScriptError, kind: ScriptErrorKind, class: &str) -> TestResult {
    ensure(
        error.kind() == kind,
        "exception changed its policy classification",
    )?;
    let diagnostic = error.diagnostic().to_string();
    ensure(
        diagnostic.contains(&format!("exception={class}"))
            && diagnostic.contains("quickjs=Exception")
            && !diagnostic.contains("synthetic-secret")
            && !diagnostic.contains("private-module.mjs"),
        &format!("exception metadata missing or exposed script text: {diagnostic}"),
    )
}

async fn startup(fixture: &Fixture) -> TestResult {
    for (source, class) in [
        (
            include_str!("../../fixtures/diagnostics/startup-syntax.js"),
            "SyntaxError",
        ),
        (
            include_str!("../../fixtures/diagnostics/startup-reference.js"),
            "ReferenceError",
        ),
        (
            include_str!("../../fixtures/diagnostics/startup-rejection.js"),
            "RangeError",
        ),
    ] {
        let path = fixture.write("config.js", source)?;
        match ScriptRuntime::start(path, RuntimeOptions::default()).await {
            Err(error) => {
                check(&error, ScriptErrorKind::Module, class)?;
                ensure(
                    error
                        .diagnostic()
                        .to_string()
                        .contains("module=config.js, line=1"),
                    "startup exception lost its entry coordinates",
                )?;
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid startup configuration succeeded".into());
            },
        }
    }
    for (source, class) in [
        (
            include_str!("../../fixtures/diagnostics/import-error.mjs"),
            "Error",
        ),
        (
            include_str!("../../fixtures/diagnostics/import-syntax.mjs"),
            "SyntaxError",
        ),
        (
            include_str!("../../fixtures/diagnostics/import-rejection.mjs"),
            "TypeError",
        ),
    ] {
        fixture.write("private-module.mjs", source)?;
        let path = fixture.write(
            "config.js",
            include_str!("../../fixtures/diagnostics/config.js"),
        )?;
        match ScriptRuntime::start(path, RuntimeOptions::default()).await {
            Err(error) => {
                check(&error, ScriptErrorKind::Module, class)?;
                ensure(
                    error
                        .diagnostic()
                        .to_string()
                        .contains("module=import#1, line=2"),
                    "import exception lost its module coordinates",
                )?;
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid import succeeded".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn rejection() -> TestResult {
    let fixture = Fixture::new()?;
    startup(&fixture).await?;
    fixture.write(
        "private-module.mjs",
        include_str!("../../fixtures/diagnostics/private-module.mjs"),
    )?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/diagnostics/hooks.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for (index, (mode, class)) in [
        ("sync", "Error"),
        ("async", "ReferenceError"),
        ("import", "TypeError"),
        ("string", "ThrownValue"),
        ("proxy", "ThrownValue"),
        ("revoked", "ThrownValue"),
        ("subclass", "RangeError"),
        ("getters", "Error"),
        ("prototype-proxy", "Error"),
        ("custom-stack", "Error"),
        ("unicode-stack", "Error"),
        ("huge-stack", "Error"),
        ("intrinsics", "Error"),
    ]
    .into_iter()
    .enumerate()
    {
        let error = runtime
            .invoke(call(id, index as u64, json!({"mode": mode})))
            .await
            .err()
            .ok_or("throw or rejection succeeded")?;
        check(&error, ScriptErrorKind::JavaScript, class)?;
        let diagnostic = error.diagnostic().to_string();
        if mode == "import" {
            ensure(
                diagnostic.contains("module=import#1, line=2"),
                "hook lost imported exception coordinates",
            )?;
        }
        if matches!(mode, "custom-stack" | "unicode-stack" | "huge-stack") {
            ensure(
                !diagnostic.contains("line="),
                "unavailable coordinates were fabricated",
            )?;
        }
    }
    let output = runtime
        .invoke(call(hook(&config, "onFinish")?, 100, json!({})))
        .await?;
    ensure(
        output == json!({"touched": 0, "ok": true}),
        "diagnostics executed script accessors or poisoned the VM",
    )?;
    stop(&runtime).await
}
