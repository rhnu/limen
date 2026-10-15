//! Synthetic exceptions exercise diagnostics without exposing script-controlled text.

use limen_script::{RuntimeOptions, ScriptError, ScriptErrorKind, ScriptRuntime};
use serde_json::json;

use super::{Fixture, ProbeResult, call, ensure, hook, stop};

fn check(error: &ScriptError, kind: ScriptErrorKind, class: &str) -> ProbeResult {
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

async fn startup(fixture: &Fixture) -> ProbeResult {
    for (source, class) in [
        ("export default { value: ; };", "SyntaxError"),
        (
            "missingSyntheticBinding(); export default {};",
            "ReferenceError",
        ),
        (
            "await Promise.reject(new RangeError('synthetic-secret')); export default {};",
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
            "// synthetic module\nthrow new Error('synthetic-secret');\n",
            "Error",
        ),
        (
            "// synthetic module\nexport const value = ;\n",
            "SyntaxError",
        ),
        (
            "// synthetic module\nawait Promise.reject(new TypeError('synthetic-secret'));\n",
            "TypeError",
        ),
    ] {
        fixture.write("private-module.mjs", source)?;
        let path = fixture.write(
            "config.js",
            "import './private-module.mjs'; export default {};",
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

pub(super) async fn rejection(fixture: &Fixture) -> ProbeResult {
    startup(fixture).await?;
    fixture.write(
        "private-module.mjs",
        "export function fail() {\n  throw new TypeError('synthetic-secret');\n}\n",
    )?;
    let (runtime, config) = fixture.start(r"
        import { fail } from './private-module.mjs';
        let touched = 0;
        const trap = () => { touched++; throw new Error('synthetic-secret'); };
        export default {
            onRequest(ctx) {
                if (ctx.mode === 'import') fail();
                if (ctx.mode === 'async') return Promise.reject(new ReferenceError('synthetic-secret'));
                if (ctx.mode === 'string') throw 'synthetic-secret';
                if (ctx.mode === 'proxy') throw new Proxy({}, { get: trap, getPrototypeOf: trap, getOwnPropertyDescriptor: trap });
                if (ctx.mode === 'revoked') {
                    const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); throw proxy;
                }
                if (ctx.mode === 'subclass') { class Custom extends RangeError {} throw new Custom('synthetic-secret'); }
                if (ctx.mode === 'custom-stack' || ctx.mode === 'unicode-stack' || ctx.mode === 'huge-stack') {
                    Error.prepareStackTrace = () => ctx.mode === 'custom-stack' ? { get value() { trap(); } }
                        : ctx.mode === 'unicode-stack' ? '\uD800' : 'x'.repeat(8192);
                    const error = new Error('synthetic-secret');
                    Error.prepareStackTrace = undefined;
                    throw error;
                }
                const error = new Error('synthetic-secret');
                if (ctx.mode === 'prototype-proxy') Object.setPrototypeOf(error, new Proxy({}, { getPrototypeOf: trap, get: trap }));
                if (ctx.mode === 'getters') {
                    for (const key of ['message', 'name', 'stack', 'constructor']) Object.defineProperty(error, key, { get: trap });
                }
                if (ctx.mode === 'intrinsics') {
                    Object.getPrototypeOf = trap;
                    Object.getOwnPropertyDescriptor = trap;
                    Error.isError = trap;
                }
                throw error;
            },
            onFinish(ctx) { ctx.touched = touched; ctx.ok = true; }
        };
    ").await?;
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
