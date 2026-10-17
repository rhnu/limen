//! Deep JSON exercises stay behind the external process and stack-failure boundary.

use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptErrorKind, ScriptRuntime};
use serde_json::{Value, json};

use super::{Fixture, TestResult, call, ensure, hook, stop};

fn options() -> RuntimeOptions {
    RuntimeOptions {
        stack_limit: 8 * 1024 * 1024,
        json_limits: JsonLimits {
            max_depth: 128,
            ..JsonLimits::default()
        },
        ..RuntimeOptions::default()
    }
}

fn arrays(mut value: &Value) -> usize {
    let mut depth = 0;
    while let Some(array) = value.as_array() {
        assert_eq!(array.len(), 1);
        depth += 1;
        value = &array[0];
    }
    assert!(value.is_null());
    depth
}

fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

pub(super) async fn startup(fixture: &Fixture) -> TestResult {
    // Root/providers/provider/defaults consume four levels before the arrays.
    for (depth, accepted) in [(128, true), (129, false)] {
        let source = format!(
            "let nested = null; for (let i = 0; i < {}; i++) nested = [nested];\n\
             export default {{providers: {{p: {{endpoint: 'https://example.invalid', defaults: {{nested}}}}}},\n\
             models: {{m: {{provider: 'p', upstream: 'm'}}}}}};",
            depth - 4,
        );
        let path = fixture.write("config.js", &source)?;
        match ScriptRuntime::start(path, options()).await {
            Ok((runtime, config)) => {
                ensure(
                    accepted,
                    "startup accepted JSON beyond the configured depth",
                )?;
                ensure(
                    arrays(&config["providers"]["p"]["defaults"]["nested"]) == depth - 4,
                    "startup lost nested JSON data",
                )?;
                stop(&runtime).await?;
            },
            Err(error) if !accepted && error.kind() == ScriptErrorKind::Json => {},
            Err(error) => return Err(format!("startup depth {depth}: {error}").into()),
        }
    }
    Ok(())
}

pub(super) async fn hooks(fixture: &Fixture) -> TestResult {
    let path = fixture.write(
        "config.js",
        r"
        function deep(ctx) {
            if (ctx.meta.input) {
                let value = (ctx.meta.phase === 'onStreamEvent' ? ctx.event : ctx.body).nested, depth = 0;
                while (Array.isArray(value)) { value = value[0]; depth++; }
                ctx.state.inputArrays = depth;
                if (ctx.meta.phase === 'onStreamEvent') ctx.event = {};
                else ctx.body = {};
                return;
            }
            let nested = null;
            for (let i = 0; i < ctx.meta.arrays; i++) nested = [nested];
            if (ctx.meta.phase === 'onStreamEvent') ctx.event = {nested};
            else ctx.body = {nested};
        }
        export default {providers: {p: {endpoint: 'https://example.invalid',
            pipeline: [{name: 'deep', onStreamEvent: deep}]}},
            models: {m: {provider: 'p', upstream: 'm'}}, onRequest: deep};
        ",
    )?;
    let (runtime, config) = ScriptRuntime::start(path, options()).await?;
    let stream_hook = config["providers"]["p"]["pipeline"][0]["onStreamEvent"]
        .as_u64()
        .ok_or("missing stream hook")?;
    for (phase, id, overhead) in [
        ("onRequest", hook(&config, "onRequest")?, 3),
        // The output envelope adds context/events/event before the nested field.
        ("onStreamEvent", stream_hook, 4),
    ] {
        // The hook replaces this maximum-depth input before projection, because
        // its private output envelope adds depth beyond the incoming context.
        for (depth, accepted) in [(128, true), (129, false)] {
            let mut context = json!({"meta": {"phase": phase, "input": true}, "state": {}});
            context[if phase == "onRequest" {
                "body"
            } else {
                "event"
            }] = json!({"nested": nested(depth - 2)});
            let report = runtime.invoke(call(id, 1, context)).await;
            match report.outcome {
                Ok(output) => {
                    ensure(
                        accepted,
                        "hook accepted an owned input beyond its depth limit",
                    )?;
                    ensure(
                        output.context["state"]["inputArrays"] == depth - 2,
                        "owned input did not survive VM parsing and preparation",
                    )?;
                },
                Err(error) if !accepted && error.kind() == ScriptErrorKind::Json => {},
                Err(error) => return Err(format!("{phase} input depth {depth}: {error}").into()),
            }
        }
        for (depth, accepted) in [(128, true), (129, false)] {
            let mut context =
                json!({"meta": {"phase": phase, "arrays": depth - overhead}, "state": {}});
            context[if phase == "onRequest" {
                "body"
            } else {
                "event"
            }] = json!({});
            let report = runtime.invoke(call(id, 1, context)).await;
            match report.outcome {
                Ok(output) => {
                    ensure(accepted, "hook accepted JSON beyond the configured depth")?;
                    let nested = if phase == "onRequest" {
                        &output.context["body"]["nested"]
                    } else {
                        &output.context["events"][0]["nested"]
                    };
                    ensure(
                        arrays(nested) == depth - overhead,
                        "hook lost nested JSON data",
                    )?;
                },
                Err(error) if !accepted && error.kind() == ScriptErrorKind::Json => {},
                Err(error) => return Err(format!("{phase} depth {depth}: {error}").into()),
            }
            ensure(
                matches!(runtime.status(), RuntimeStatus::Ready),
                "bounded JSON rejection retired the VM",
            )?;
        }
    }
    stop(&runtime).await
}

pub(super) async fn stack_limit(fixture: &Fixture) -> TestResult {
    let path = fixture.write(
        "config.js",
        r"
        function recurse(left) { return left === 0 ? 0 : 1 + recurse(left - 1); }
        export default {onRequest(ctx) {
            if (ctx.meta.recurse) ctx.state.value = recurse(100000);
            else if (ctx.meta.deep) {
                let nested = null;
                for (let i = 0; i < 125; i++) nested = [nested];
                ctx.body = {nested};
            }
            else ctx.state.value = 1;
        }};
        ",
    )?;
    // Exercise the standalone default, service default and service upper bound.
    for stack_limit in [
        RuntimeOptions::default().stack_limit,
        512 * 1024,
        options().stack_limit,
    ] {
        let (runtime, config) = ScriptRuntime::start(
            path.clone(),
            RuntimeOptions {
                stack_limit,
                ..options()
            },
        )
        .await?;
        let id = hook(&config, "onRequest")?;
        let report = runtime
            .invoke(call(
                id,
                1,
                json!({"meta": {"phase": "onRequest", "recurse": true}, "state": {}, "body": {}}),
            ))
            .await;
        ensure(
            report
                .outcome
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "native stack budget did not return a controlled JavaScript error",
        )?;
        ensure(
            matches!(runtime.status(), RuntimeStatus::Ready),
            "stack budget failure retired the VM",
        )?;
        let report = runtime
            .invoke(call(
                id,
                2,
                json!({"meta": {"phase": "onRequest", "deep": true}, "state": {}, "body": {}}),
            ))
            .await;
        match report.outcome {
            Ok(output) => ensure(
                arrays(&output.context["body"]["nested"]) == 125,
                "deep output was incomplete",
            )?,
            Err(error)
                if stack_limit < options().stack_limit && error.kind() == ScriptErrorKind::Json => {
            },
            Err(error) => return Err(format!("stack budget {stack_limit}: {error}").into()),
        }
        let output = runtime
            .invoke(call(
                id,
                3,
                json!({"meta": {"phase": "onRequest", "recurse": false}, "state": {}, "body": {}}),
            ))
            .await
            .outcome?;
        ensure(
            output.context["state"]["value"] == 1,
            "stack budget failure poisoned a later call",
        )?;
        stop(&runtime).await?;
    }
    Ok(())
}
