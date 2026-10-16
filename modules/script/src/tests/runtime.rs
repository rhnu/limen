use std::time::{Duration, Instant};

use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop, wait_for};
use crate::{RuntimeOptions, RuntimeStatus, ScriptErrorKind, ScriptRuntime};

#[tokio::test]
async fn sync_and_async_void_hooks_commit_and_preserve_pipeline_order() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/hooks/void.js"))
        .await?;
    let sync = runtime
        .invoke(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        sync == json!({"sync": true}),
        "sync void draft did not commit",
    )?;
    let asynchronous = runtime
        .invoke(call(hook(&config, "onFinish")?, 2, json!({})))
        .await
        .into_context()?;
    ensure(
        asynchronous == json!({"async": true}),
        "async void draft did not commit",
    )?;
    let provider = config["providers"]["p"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("provider hook not registered")?;
    let model = config["models"]["m"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("model hook not registered")?;
    let provider_output = runtime
        .invoke(call(provider, 3, json!({"order": []})))
        .await
        .into_context()?;
    let model_output = runtime
        .invoke(call(model, 4, provider_output))
        .await
        .into_context()?;
    ensure(
        model_output == json!({"order": ["provider", "model"]}),
        "saved provider/model functions did not preserve sequential input",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn pending_promises_allow_other_calls_and_wait_failure_keeps_runtime_ready() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/scheduler/pending.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for (index, deadline) in [false, true].into_iter().enumerate() {
        let mut pending = call(id, if deadline { 3 } else { 1 }, json!({"pending": true}));
        if deadline {
            pending.deadline = Instant::now() + Duration::from_millis(500);
        }
        let cancellation = pending.cancellation.clone();
        let pending_runtime = runtime.clone();
        let waiting =
            tokio::spawn(async move { pending_runtime.invoke(pending).await.into_context() });
        wait_for(
            &runtime,
            hook(&config, "onError")?,
            json!({"started": index + 1}),
        )
        .await?;
        let output = runtime
            .invoke(call(id, if deadline { 4 } else { 2 }, json!({})))
            .await
            .into_context()?;
        ensure(
            output == json!({"done": true}),
            "pending Promise blocked a new call",
        )?;
        if !deadline {
            cancellation.cancel();
        }
        let expected = if deadline {
            ScriptErrorKind::Deadline
        } else {
            ScriptErrorKind::Cancelled
        };
        ensure(
            waiting.await?.is_err_and(|error| error.kind() == expected),
            "pending cancellation/deadline was not delivered",
        )?;
        ensure(
            matches!(runtime.status(), RuntimeStatus::Ready),
            "ordinary wait failure retired the VM",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn cancelled_and_expired_calls_discard_drafts_but_allow_late_closure_effects() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/scheduler/late-completion.js"))
        .await?;
    for (index, deadline) in [false, true].into_iter().enumerate() {
        let base = u64::try_from(index)? * 10;
        let mut pending = call(
            hook(&config, "onRequest")?,
            base,
            json!({"draft": "original"}),
        );
        if deadline {
            pending.deadline = Instant::now() + Duration::from_millis(500);
        }
        let cancellation = pending.cancellation.clone();
        let pending_runtime = runtime.clone();
        let waiting =
            tokio::spawn(async move { pending_runtime.invoke(pending).await.into_context() });
        wait_for(
            &runtime,
            hook(&config, "onError")?,
            json!({"changes": index, "started": index + 1}),
        )
        .await?;
        if !deadline {
            cancellation.cancel();
        }
        let expected = if deadline {
            ScriptErrorKind::Deadline
        } else {
            ScriptErrorKind::Cancelled
        };
        ensure(
            waiting.await?.is_err_and(|error| error.kind() == expected),
            "failed call exposed a draft instead of an error",
        )?;
        let trigger = runtime
            .invoke(call(hook(&config, "onFinish")?, base + 1, json!({})))
            .await
            .into_context()?;
        ensure(trigger == json!({}), "late draft crossed into a new call")?;
        wait_for(
            &runtime,
            hook(&config, "onError")?,
            json!({"changes": index + 1, "started": index + 1}),
        )
        .await?;
        ensure(
            matches!(runtime.status(), RuntimeStatus::Ready),
            "late completion retired the VM",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn non_void_completions_and_thenables_fail_the_hook_contract() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/hooks/invalid-return.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for kind in 0..=12 {
        let output = runtime
            .invoke(call(id, kind, json!({"kind": kind})))
            .await
            .into_context();
        ensure(
            output.is_err_and(|error| error.kind() == ScriptErrorKind::Contract),
            "non-void completion or thenable was accepted",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn startup_awaits_top_level_promise_before_publishing_ready() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/loader/top-level-await.js"))
        .await?;
    ensure(
        config["answer"] == 7,
        "top-level await was not awaited before extraction",
    )?;
    let output = runtime
        .invoke(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output == json!({"answer": 7, "hostApis": ["undefined", "undefined", "undefined"]}),
        "TLA closure or host API boundary failed",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn json_bridge_rejects_invalid_values_and_preserves_safe_integer_boundary() -> TestResult {
    let fixture = Fixture::new()?;
    for source in [
        include_str!("../../fixtures/json/invalid-1.js"),
        include_str!("../../fixtures/json/invalid-2.js"),
        include_str!("../../fixtures/json/invalid-3.js"),
        include_str!("../../fixtures/json/invalid-4.js"),
        include_str!("../../fixtures/json/invalid-5.js"),
        include_str!("../../fixtures/json/invalid-6.js"),
        include_str!("../../fixtures/json/invalid-7.js"),
        include_str!("../../fixtures/json/invalid-8.js"),
        include_str!("../../fixtures/json/invalid-9.js"),
        include_str!("../../fixtures/json/invalid-10.js"),
        include_str!("../../fixtures/json/invalid-11.js"),
        include_str!("../../fixtures/json/invalid-12.js"),
    ] {
        let path = fixture.write("config.js", source)?;
        let result = ScriptRuntime::start(path, RuntimeOptions::default()).await;
        match result {
            Err(error) if error.kind() == ScriptErrorKind::Json => {},
            Err(error) => {
                return Err(format!("invalid static JSON used unexpected error: {error}").into());
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid static JSON was accepted".into());
            },
        }
    }
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/json/invalid-draft.js"))
        .await?;
    for kind in 0..4 {
        let result = runtime
            .invoke(call(
                hook(&config, "onRequest")?,
                kind,
                json!({"kind": kind}),
            ))
            .await
            .into_context();
        ensure(
            result.is_err_and(|error| error.kind() == ScriptErrorKind::Json),
            "invalid hook JSON was accepted",
        )?;
    }
    let result = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            5,
            json!({"unsafe": 9_007_199_254_740_992_u64}),
        ))
        .await
        .into_context();
    ensure(
        result.is_err_and(|error| error.kind() == ScriptErrorKind::Json),
        "unsafe Rust input integer entered JS",
    )?;
    let result = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            6,
            json!({"safe": 9_007_199_254_740_991_u64, "null": null}),
        ))
        .await
        .into_context()?;
    ensure(
        result == json!({"safe": 9_007_199_254_740_991_u64, "null": null}),
        "safe integer or null failed to round trip",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn loader_normalizes_local_modules_and_rejects_non_local_imports() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "shared/state.mjs",
        include_str!("../../fixtures/loader/state.mjs"),
    )?;
    fixture.write(
        "app/first.js",
        include_str!("../../fixtures/loader/reexport.js"),
    )?;
    let path = fixture.write(
        "app/config.js",
        include_str!("../../fixtures/loader/config.js"),
    )?;
    let (runtime, config) = ScriptRuntime::start(path, RuntimeOptions::default()).await?;
    ensure(
        config == json!({"same": true, "loads": 1}),
        "relative parent imports or normalized module cache failed",
    )?;
    stop(&runtime).await?;
    for source in [
        include_str!("../../fixtures/loader/forbidden-node.js"),
        include_str!("../../fixtures/loader/forbidden-package.js"),
        include_str!("../../fixtures/loader/forbidden-url.js"),
    ] {
        let path = fixture.write("config.js", source)?;
        match ScriptRuntime::start(path, RuntimeOptions::default()).await {
            Err(error) if error.kind() == ScriptErrorKind::Module => {},
            Err(error) => {
                return Err(
                    format!("forbidden module import used unexpected error: {error}").into(),
                );
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("forbidden module import was accepted".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn shutdown_discards_an_entered_pending_hook() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/scheduler/shutdown-pending.js"))
        .await?;
    let pending_runtime = runtime.clone();
    let pending = call(hook(&config, "onRequest")?, 1, json!({}));
    let waiting = tokio::spawn(async move { pending_runtime.invoke(pending).await.into_context() });
    wait_for(
        &runtime,
        hook(&config, "onError")?,
        json!({"started": true}),
    )
    .await?;
    stop(&runtime).await?;
    ensure(
        waiting.await?.is_err(),
        "shutdown committed a pending draft",
    )
}
