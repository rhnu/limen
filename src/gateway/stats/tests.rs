use limen_script::{HookOutput, ObservationBatch, ScriptError};

use super::*;

#[test]
fn native_timings_count_real_execution_without_js_bridge_samples() {
    let timings = Timings::default();
    timings.native("beforeSend").finish(&Ok::<_, Failure>(()));
    timings
        .native("beforeSend")
        .finish(&Err::<(), _>(Failure::new(
            ErrorCode::HookFailed,
            "beforeSend",
        )));
    drop(timings.native("onContent"));
    let snapshot = timings.snapshot();
    assert!(snapshot["hooks"].as_object().unwrap().is_empty());
    assert_eq!(snapshot["native"]["beforeSend"]["calls"], 2);
    assert_eq!(snapshot["native"]["beforeSend"]["failedCalls"], 1);
    assert_eq!(snapshot["native"]["beforeSend"]["interruptedCalls"], 0);
    assert_eq!(snapshot["native"]["onContent"]["interruptedCalls"], 1);
    assert!(snapshot["native"]["beforeSend"]["queue"].is_null());
    timings.freeze(Duration::ZERO);
    let frozen = timings.snapshot();
    drop(timings.native("onContent"));
    assert_eq!(timings.snapshot(), frozen);
}

#[test]
fn milestones_use_their_documented_origins_and_keep_the_first_event() {
    let start = Instant::now();
    let timings = Timings::default();
    timings.start_upstream(start + Duration::from_millis(10));
    timings.headers(start + Duration::from_millis(30));
    timings.first_byte(start + Duration::from_millis(35));
    timings.first_byte(start + Duration::from_millis(80));
    timings.first_chunk(start + Duration::from_millis(40));
    timings.first_output(start, start + Duration::from_millis(50));
    timings.first_output(start, start + Duration::from_millis(90));
    timings.freeze(Duration::from_millis(100));
    let value = timings.snapshot();
    assert_eq!(value["upstreamHeadersWaitMs"], 20.0);
    assert_eq!(value["upstreamFirstByteMs"], 25.0);
    assert_eq!(value["upstreamFirstChunkMs"], 30.0);
    assert_eq!(value["downstreamFirstEventMs"], 50.0);
    assert_eq!(value["requestMs"], 100.0);
    assert!(value["downstreamDrainMs"].is_null());
}

#[test]
fn samples_distinguish_absence_from_measured_zero_and_accumulate_durations() {
    let mut sample = Sample::default();
    sample.add(None);
    assert_eq!(sample.json(), json!({"totalMs":0.0,"samples":0}));
    sample.add(Some(Duration::ZERO));
    sample.add(Some(Duration::from_micros(250)));
    assert_eq!(sample.json(), json!({"totalMs":0.25,"samples":2}));
}

#[test]
fn dropped_hook_is_accounted_without_fabricating_runtime_samples() {
    let timings = Timings::default();
    drop(timings.hook("beforeSend"));
    let hook = &timings.snapshot()["hooks"]["beforeSend"];
    assert_eq!(hook["calls"], 1);
    assert_eq!(hook["interruptedCalls"], 1);
    assert_eq!(hook["reportedCalls"], 0);
    assert_eq!(hook["queue"]["samples"], 0);
}

#[test]
fn returned_reports_include_errors_and_only_available_measurements() {
    let timings = Timings::default();
    for outcome in [
        Ok(HookOutput {
            context: json!({}),
            observations: ObservationBatch::default(),
        }),
        Err(ScriptError::new(ScriptErrorKind::Deadline)),
    ] {
        let guard = timings.hook("onRequest");
        guard.finish(&HookReport {
            outcome,
            timings: HookTimings {
                queue: Some(Duration::from_millis(2)),
                ..HookTimings::default()
            },
        });
    }
    let hook = &timings.snapshot()["hooks"]["onRequest"];
    assert_eq!(hook["calls"], 2);
    assert_eq!(hook["failedCalls"], 1);
    assert_eq!(hook["interruptedCalls"], 1);
    assert_eq!(hook["reportedCalls"], 2);
    assert_eq!(hook["queue"], json!({"totalMs":4.0,"samples":2}));
    assert_eq!(hook["pending"]["samples"], 0);
}

#[test]
fn freezing_request_statistics_excludes_cleanup_and_late_work() {
    let start = Instant::now();
    let mut stats = Statistics::default();
    stats.usage.observe(Some(
        &json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}),
    ));
    stats.source = Some("json");
    stats.upstream_completed = true;
    stats.freeze(start, start + Duration::from_millis(10));
    let frozen = stats.snapshot();
    drop(stats.timings.measure(WaitKind::Queue));
    drop(stats.timings.hook("onFinish"));
    stats.usage.observe(Some(&json!({"total_tokens":99})));
    assert_eq!(stats.snapshot(), frozen);
    assert_eq!(frozen["usage"]["totalTokens"], 3);
    assert_eq!(frozen["timings"]["requestMs"], 10.0);
    assert!(frozen["timings"]["hooks"].as_object().unwrap().is_empty());
}

#[test]
fn usage_quality_and_upstream_completion_are_independent_frozen_facts() {
    let start = Instant::now();
    for completed in [false, true] {
        for (usage, quality) in [
            (
                json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}),
                "reported",
            ),
            (json!({"prompt_tokens":"invalid"}), "invalid"),
        ] {
            let mut stats = Statistics {
                source: Some("sse"),
                upstream_completed: completed,
                ..Statistics::default()
            };
            stats.usage.observe(Some(&usage));
            stats.freeze(start, start);
            stats.upstream_completed = !completed;
            stats.usage.observe(Some(&json!({"total_tokens":99})));
            let frozen = stats.snapshot();
            assert_eq!(frozen["usage"]["quality"], quality);
            assert_eq!(frozen["usage"]["upstreamCompleted"], completed);
            assert_eq!(frozen["usage"]["source"], "sse");
        }
    }
}
