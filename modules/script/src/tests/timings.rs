use std::time::{Duration, Instant};

use serde_json::json;

use super::{Fixture, TestResult, call, ensure, hook, stop, wait_for};
use crate::{HookTimings, ScriptErrorKind};

fn check_completed(timings: HookTimings, asynchronous: bool, output: bool) -> TestResult {
    ensure(timings.queue.is_some(), "admitted call lost queue timing")?;
    ensure(
        timings.initial_call.is_some(),
        "entered hook lost initial call timing",
    )?;
    ensure(
        timings.pending.is_some() == asynchronous,
        "Promise timing presence differed from execution",
    )?;
    ensure(
        timings.output_bridge.is_some() == output,
        "output bridge timing presence differed from conversion",
    )?;
    // These measured intervals are disjoint even though Promise waiting includes jobs
    // belonging to other calls. Their sum cannot exceed this invocation's wall time.
    let sampled = timings.input_bridge
        + timings.queue.unwrap_or_default()
        + timings.initial_call.unwrap_or_default()
        + timings.pending.unwrap_or_default()
        + timings.output_bridge.unwrap_or_default();
    ensure(sampled <= timings.wall, "sampled phases exceeded wall time")
}

#[tokio::test]
async fn success_and_failure_reports_keep_each_attempted_phase() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/hooks/timings.js"))
        .await?;
    for (name, asynchronous) in [("onRequest", false), ("onFinish", true)] {
        let id = hook(&config, name)?;
        for (mode, expected, output) in [
            ("success", None, true),
            ("throw", Some(ScriptErrorKind::JavaScript), false),
            ("invalid", Some(ScriptErrorKind::Json), true),
            ("contract", Some(ScriptErrorKind::Contract), false),
        ] {
            let report = runtime.invoke(call(id, 1, json!({"mode": mode}))).await;
            check_completed(report.timings, asynchronous, output)?;
            match expected {
                Some(expected) => ensure(
                    report.outcome.is_err_and(|error| error.kind() == expected),
                    "hook returned an unexpected outcome",
                )?,
                None => ensure(
                    report.outcome?.context == json!({"mode": mode, "done": true}),
                    "hook report lost its committed draft",
                )?,
            }
        }
    }
    stop(&runtime).await
}

#[tokio::test]
async fn rejection_before_a_phase_leaves_it_unsampled() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/hooks/timings.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    for (mut input, expected) in [
        (
            call(id, 1, json!({"unsafe": 9_007_199_254_740_992_u64})),
            ScriptErrorKind::Json,
        ),
        (call(id, 2, json!({})), ScriptErrorKind::Cancelled),
        (call(id, 3, json!({})), ScriptErrorKind::Deadline),
        (call(0, 4, json!({})), ScriptErrorKind::UnknownHook),
    ] {
        match expected {
            ScriptErrorKind::Cancelled => input.cancellation.cancel(),
            ScriptErrorKind::Deadline => input.deadline = Instant::now(),
            _ => {},
        }
        let report = runtime.invoke(input).await;
        ensure(
            report.outcome.is_err_and(|error| error.kind() == expected),
            "wrong hook failure",
        )?;
        ensure(
            report.timings.queue.is_some() == (expected == ScriptErrorKind::UnknownHook),
            "queue sample presence differed from admission",
        )?;
        ensure(
            report.timings.initial_call.is_none(),
            "unentered hook had a call sample",
        )?;
        ensure(
            report.timings.pending.is_none(),
            "unentered hook had a Promise sample",
        )?;
        ensure(
            report.timings.output_bridge.is_none(),
            "unconverted draft had an output sample",
        )?;
        ensure(
            report.timings.input_bridge <= report.timings.wall,
            "input bridge exceeded wall time",
        )?;
    }
    stop(&runtime).await?;
    let report = runtime.invoke(call(id, 5, json!({}))).await;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Retired),
        "stopped runtime accepted a hook",
    )?;
    ensure(
        report.timings.queue.is_none(),
        "rejected enqueue had a queue sample",
    )?;
    ensure(
        report.timings.input_bridge <= report.timings.wall,
        "input bridge exceeded wall time",
    )?;
    Ok(())
}

#[tokio::test]
async fn shutdown_reports_observed_pending_failure_without_output_conversion() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/scheduler/shutdown-pending.js"))
        .await?;
    let pending = call(hook(&config, "onRequest")?, 1, json!({}));
    let waiting_runtime = runtime.clone();
    let waiting = tokio::spawn(async move { waiting_runtime.invoke(pending).await });
    wait_for(
        &runtime,
        hook(&config, "onError")?,
        json!({"started": true}),
    )
    .await?;
    stop(&runtime).await?;
    let report = waiting.await?;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Stopped),
        "shutdown committed a pending draft",
    )?;
    check_completed(report.timings, true, false)
}

#[test]
fn default_timings_do_not_fabricate_unobserved_phases() {
    let timings = HookTimings::default();
    assert_eq!(timings.wall, Duration::ZERO);
    assert_eq!(timings.input_bridge, Duration::ZERO);
    assert!(timings.queue.is_none());
    assert!(timings.initial_call.is_none());
    assert!(timings.pending.is_none());
    assert!(timings.output_bridge.is_none());
}
