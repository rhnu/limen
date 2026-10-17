use serde_json::{Value, json};

use super::{Fixture, TestResult, call, ensure, hook, stop, wait_for};
use crate::{HookOutput, RuntimeOptions, ScriptErrorKind, ScriptRuntime};

const SOURCE: &str = include_str!("../../fixtures/hooks/observations.js");

fn options(capacity: usize) -> RuntimeOptions {
    RuntimeOptions {
        observation_names: ["first", "second", "third"].map(str::to_owned).to_vec(),
        max_observations_per_hook: capacity,
        ..RuntimeOptions::default()
    }
}

fn context(phase: &str, mode: &str) -> Value {
    let mut context = json!({"meta": {"phase": phase, "mode": mode}, "state": {}});
    if matches!(phase, "onContent" | "onUpstreamEvent" | "onStreamEvent") {
        context["event"] = json!({});
    }
    context
}

fn batch(output: &HookOutput, expected: &[u8], dropped: u64) -> TestResult {
    ensure(
        output.observations.events == expected,
        "observation IDs or order changed",
    )?;
    ensure(
        output.observations.dropped == dropped,
        "observation overflow count changed",
    )?;
    ensure(
        output.context.get("observe").is_none() && output.context.get("observations").is_none(),
        "private observation state escaped into context",
    )
}

#[tokio::test]
async fn sync_async_and_observer_phases_export_private_bounded_batches() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let (runtime, config) = ScriptRuntime::start(path, options(2)).await?;
    let id = hook(&config, "onRequest")?;
    for phase in [
        "onRequest",
        "beforeSend",
        "afterReceive",
        "onResponse",
        "onContent",
        "onUpstreamEvent",
        "onStreamEvent",
        "onErrorResponse",
        "onError",
        "onFinish",
    ] {
        for (mode, expected, dropped) in [
            ("success", &[0, 1][..], 2),
            ("async", &[1, 0][..], 0),
            ("empty", &[][..], 0),
        ] {
            let output = runtime
                .invoke(call(id, 1, context(phase, mode)))
                .await
                .outcome?;
            batch(&output, expected, dropped)?;
            ensure(
                output.context["state"] == json!({}),
                "observation changed request state",
            )?;
        }
    }
    let output = runtime
        .invoke(call(hook(&config, "onFinish")?, 2, json!({})))
        .await
        .outcome?;
    batch(&output, &[], 0)?;
    stop(&runtime).await
}

#[tokio::test]
async fn invalid_calls_mutation_and_forgery_fail_without_exporting_a_batch() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let (runtime, config) = ScriptRuntime::start(path, options(2)).await?;
    let id = hook(&config, "onRequest")?;
    for mode in [
        "no-arg",
        "extra",
        "unknown",
        "object",
        "number",
        "replace-api",
        "replace-method",
        "edit-function",
        "edit-api",
        "forge",
    ] {
        let report = runtime
            .invoke(call(id, 1, context("onRequest", mode)))
            .await;
        ensure(
            report
                .outcome
                .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "invalid observation API use succeeded",
        )?;
    }
    for mode in ["throw", "reject", "contract", "async-contract", "invalid"] {
        let expected = match mode {
            "contract" | "async-contract" => ScriptErrorKind::Contract,
            "invalid" => ScriptErrorKind::Json,
            _ => ScriptErrorKind::JavaScript,
        };
        let report = runtime
            .invoke(call(id, 2, context("onRequest", mode)))
            .await;
        ensure(
            report.outcome.is_err_and(|error| error.kind() == expected),
            "failed hook exported observations",
        )?;
    }
    for phase in [Some("onRequest"), None] {
        for reserved in ["observe", "observations"] {
            let mut input = phase.map_or_else(|| json!({}), |phase| context(phase, "empty"));
            input[reserved] = json!({"events": [0], "dropped": 0});
            let report = runtime.invoke(call(id, 3, input)).await;
            ensure(
                report.outcome.is_err(),
                "caller forged private observations",
            )?;
        }
    }
    let report = runtime
        .invoke(call(hook(&config, "onFinish")?, 4, json!({"forge": true})))
        .await;
    ensure(
        report.outcome.is_err(),
        "plain draft forged observations after prepare",
    )?;
    let output = runtime
        .invoke(call(id, 5, context("onRequest", "empty")))
        .await
        .outcome?;
    batch(&output, &[], 0)?;
    stop(&runtime).await
}

#[tokio::test]
async fn empty_registry_is_available_but_cannot_accept_names() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture.start(SOURCE).await?;
    let id = hook(&config, "onRequest")?;
    let output = runtime
        .invoke(call(id, 1, context("onFinish", "empty")))
        .await
        .outcome?;
    batch(&output, &[], 0)?;
    let report = runtime
        .invoke(call(id, 2, context("onRequest", "success")))
        .await;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
        "unregistered name was accepted",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn concurrent_pending_and_cancelled_late_calls_keep_batches_separate() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let (runtime, config) = ScriptRuntime::start(path, options(3)).await?;
    let id = hook(&config, "onRequest")?;
    let status = hook(&config, "onError")?;
    let release = hook(&config, "onFinish")?;
    let first_runtime = runtime.clone();
    let first = tokio::spawn(async move {
        first_runtime
            .invoke(call(id, 1, context("onRequest", "pending-first")))
            .await
    });
    wait_for(&runtime, status, json!({"started": 1, "finished": 0})).await?;
    let second_runtime = runtime.clone();
    let second = tokio::spawn(async move {
        second_runtime
            .invoke(call(id, 2, context("onRequest", "pending-second")))
            .await
    });
    wait_for(&runtime, status, json!({"started": 2, "finished": 0})).await?;
    runtime.invoke(call(release, 3, json!({}))).await.outcome?;
    batch(&first.await?.outcome?, &[0, 2], 0)?;
    batch(&second.await?.outcome?, &[1, 2], 0)?;

    let pending = call(id, 4, context("onRequest", "pending-first"));
    let cancellation = pending.cancellation.clone();
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await });
    wait_for(&runtime, status, json!({"started": 3, "finished": 2})).await?;
    cancellation.cancel();
    ensure(
        waiting
            .await?
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Cancelled),
        "cancelled hook exported its batch",
    )?;
    runtime.invoke(call(release, 5, json!({}))).await.outcome?;
    wait_for(&runtime, status, json!({"started": 3, "finished": 3})).await?;
    let output = runtime
        .invoke(call(id, 6, context("onRequest", "empty")))
        .await
        .outcome?;
    batch(&output, &[], 0)?;
    stop(&runtime).await
}

#[tokio::test]
async fn private_envelope_is_subject_to_the_existing_json_depth_budget() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let mut limits = options(2);
    limits.json_limits.max_depth = 2;
    let (runtime, config) = ScriptRuntime::start(path, limits).await?;
    let report = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            1,
            context("onRequest", "empty"),
        ))
        .await;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Json),
        "private envelope bypassed output JSON budget",
    )?;
    stop(&runtime).await
}

#[test]
fn observation_options_require_bounded_unique_ascii_names_and_hook_capacity() {
    assert!(crate::valid_options(&RuntimeOptions::default()));
    for name in ["a".to_owned(), "a1_b".to_owned(), "a".repeat(64)] {
        let options = RuntimeOptions {
            observation_names: vec![name],
            ..RuntimeOptions::default()
        };
        assert!(crate::valid_options(&options));
    }
    for names in [
        vec![String::new()],
        vec!["a".repeat(65)],
        vec!["Upper".into()],
        vec!["1first".into()],
        vec!["_first".into()],
        vec!["a-b".into()],
        vec!["first name".into()],
        vec!["é".into()],
        vec!["a\0b".into()],
        vec!["same".into(), "same".into()],
        (0..65).map(|index| format!("event_{index}")).collect(),
    ] {
        let options = RuntimeOptions {
            observation_names: names,
            ..RuntimeOptions::default()
        };
        assert!(!crate::valid_options(&options));
    }
    let mut options = RuntimeOptions {
        observation_names: (0..64).map(|index| format!("event_{index}")).collect(),
        ..RuntimeOptions::default()
    };
    for capacity in [1, 64] {
        options.max_observations_per_hook = capacity;
        assert!(crate::valid_options(&options));
    }
    for capacity in [0, 65] {
        options.max_observations_per_hook = capacity;
        assert!(!crate::valid_options(&options));
    }
}

#[tokio::test]
async fn upper_registry_and_capacity_bounds_accept_all_64_ids() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let options = RuntimeOptions {
        observation_names: (0..64).map(|index| format!("event_{index}")).collect(),
        max_observations_per_hook: 64,
        ..RuntimeOptions::default()
    };
    let (runtime, config) = ScriptRuntime::start(path, options).await?;
    let output = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            1,
            context("onRequest", "boundary"),
        ))
        .await
        .outcome?;
    batch(&output, &(0..64).collect::<Vec<u8>>(), 0)?;
    stop(&runtime).await
}

#[tokio::test]
async fn deadline_failure_does_not_export_observations_from_late_work() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write("config.js", SOURCE)?;
    let (runtime, config) = ScriptRuntime::start(path, options(3)).await?;
    let id = hook(&config, "onRequest")?;
    let mut pending = call(id, 1, context("onRequest", "pending-first"));
    pending.deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await });
    let status = hook(&config, "onError")?;
    wait_for(&runtime, status, json!({"started": 1, "finished": 0})).await?;
    ensure(
        waiting
            .await?
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Deadline),
        "expired hook exported its batch",
    )?;
    runtime
        .invoke(call(hook(&config, "onFinish")?, 2, json!({})))
        .await
        .outcome?;
    wait_for(&runtime, status, json!({"started": 1, "finished": 1})).await?;
    let output = runtime
        .invoke(call(id, 3, context("onRequest", "empty")))
        .await
        .outcome?;
    batch(&output, &[], 0)?;
    stop(&runtime).await
}
