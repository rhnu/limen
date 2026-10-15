//! Synthetic inherited variables; never mutate the multithreaded parent's environment.

use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptErrorKind, ScriptRuntime};
use serde_json::json;

use super::{Fixture, ProbeResult, call, ensure, hook, stop};

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

pub(super) async fn run(fixture: &Fixture) -> ProbeResult {
    fixture.write(
        "token.mjs",
        r"
        export const token = limen.env('LIMEN_PROBE_ENV_TOKEN');
        export const frozen = Object.isFrozen(limen)
            && !Reflect.defineProperty(limen, 'moduleExtra', { value: true });
    ",
    )?;
    let path = fixture.write("config.js", r"
        import { token, frozen } from './token.mjs';
        const unicode = await Promise.resolve(limen.env('LIMEN_PROBE_ENV_UNICODE'));
        export default {
            providers: { p: { auth: { bearer: token } } },
            frozen,
            unicode,
            missing: limen.env('LIMEN_PROBE_ENV_MISSING') === undefined,
            empty: limen.env('LIMEN_PROBE_ENV_EMPTY'),
            async onRequest(ctx) {
                await Promise.resolve();
                if (ctx.action === 'read') {
                    ctx.token = limen.env('LIMEN_PROBE_ENV_TOKEN');
                    ctx.fallback = limen.env('LIMEN_PROBE_ENV_MISSING') ?? 'fallback';
                    return;
                }
                if (ctx.action === 'uncaught') { limen.env('A=B'); return; }
                const rejected = (run, type) => {
                    try { run(); } catch (error) {
                        if (!(error instanceof type)) throw new Error('wrong error type');
                        return;
                    }
                    throw new Error('invalid environment access succeeded');
                };
                for (const name of ['', 'A=B', 'A\0B']) {
                    rejected(() => limen.env(name), TypeError);
                }
                for (const name of [null, undefined, 1, {}, { toString() { throw new Error(); } }]) {
                    rejected(() => limen.env(name), TypeError);
                }
                rejected(() => limen.env(), TypeError);
                rejected(() => limen.env('x'.repeat(1025)), RangeError);
                rejected(() => limen.env('LIMEN_PROBE_ENV_LARGE'), RangeError);
                rejected(() => limen.env('LIMEN_PROBE_ENV_INVALID'), Error);
                rejected(() => { limen.env = () => 'changed'; }, TypeError);
                rejected(() => { globalThis.limen = {}; }, TypeError);
                rejected(() => { delete limen.env; }, TypeError);
                rejected(() => { delete globalThis.limen; }, TypeError);
                rejected(() => { limen.extra = true; }, TypeError);
                rejected(() => Object.setPrototypeOf(limen, {}), TypeError);
                ctx.checked = true;
            }
        };
    ")?;
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
        .await?;
    ensure(
        output["checked"] == true,
        "environment error contracts were not checked",
    )?;
    let error = runtime
        .invoke(call(id, 2, json!({"action":"uncaught"})))
        .await
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
        .await?;
    ensure(
        output["token"] == "synthetic-upstream-token" && output["fallback"] == "fallback",
        "async hook environment lookup failed after a rejected access",
    )?;
    stop(&runtime).await
}
