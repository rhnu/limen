//! Synthetic inherited variables; never mutate the multithreaded parent's environment.

use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptErrorKind, ScriptRuntime};
use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop};

pub(crate) fn configure(command: &mut tokio::process::Command) {
    command
        .env("LIMEN_PROBE_ENV_TOKEN", "synthetic-upstream-token")
        .env("LIMEN_PROBE_ENV_EMPTY", "")
        .env("LIMEN_PROBE_ENV_UNICODE", "密钥-é")
        .env("LIMEN_PROBE_ENV_LARGE", "x".repeat(1025))
        .env_remove("LIMEN_PROBE_ENV_MISSING");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        command.env(
            "LIMEN_PROBE_ENV_INVALID",
            std::ffi::OsString::from_vec(vec![0xFF]),
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        command.env(
            "LIMEN_PROBE_ENV_INVALID",
            std::ffi::OsString::from_wide(&[0xD800]),
        );
    }
}

pub(super) async fn run(fixture: &Fixture) -> TestResult {
    fixture.write(
        "token.mjs",
        include_str!("../../fixtures/runtime/environment-run-1.js"),
    )?;
    let path = fixture.write(
        "config.js",
        include_str!("../../fixtures/runtime/environment-run-2.js"),
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
        config["providers"]["p"]["auth"]["bearer"] == "synthetic-upstream-token"
            && config["frozen"] == true
            && config["unicode"] == "密钥-é"
            && config["missing"] == true
            && config["empty"] == "",
        "module host API lost freezing, token, Unicode, absence or empty string",
    )?;
    let id = hook(&config, "onRequest")?;
    let output = runtime
        .invoke(call(id, 1, json!({"action":"check"})))
        .await
        .outcome?
        .context;
    ensure(
        output["checked"] == true,
        "environment error contracts were not checked",
    )?;
    let error = runtime
        .invoke(call(id, 2, json!({"action":"uncaught"})))
        .await
        .outcome
        .err()
        .ok_or("invalid environment name must fail the hook")?;
    ensure(
        error.kind() == ScriptErrorKind::JavaScript
            && !error.diagnostic().to_string().contains("A=B")
            && matches!(runtime.status(), RuntimeStatus::Ready),
        "environment failure leaked input or retired a usable VM",
    )?;
    let output = runtime
        .invoke(call(id, 3, json!({"action":"read"})))
        .await
        .outcome?
        .context;
    ensure(
        output["token"] == "synthetic-upstream-token" && output["fallback"] == "fallback",
        "async hook environment lookup failed after a rejected access",
    )?;
    stop(&runtime).await
}
