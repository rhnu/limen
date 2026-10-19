use limen_script::{HookOutput, ObservationBatch, ScriptError};

use super::*;

#[test]
fn native_batch_counts_semantic_inputs_without_multiplying_wall_samples() {
    let timings = Timings::default();
    timings
        .native_scoped("onContent", PhaseScope::Model)
        .finish_batch(&Ok::<(), Failure>(()), 3);
    timings
        .native_scoped("onContent", PhaseScope::Model)
        .finish_batch(&Ok::<(), Failure>(()), 0);
    let (_, _, phases) = timings.owned();
    let phase = phases
        .iter()
        .find(|phase| {
            phase.phase == "onContent"
                && phase.scope == PhaseScope::Model
                && phase.executor == PhaseExecutor::Native
        })
        .unwrap();
    assert_eq!(phase.calls, 3);
    assert_eq!(phase.reported_calls, 3);
    assert_eq!(phase.failed_calls, 0);
    assert!(phase.wall_us.is_some());
}

#[test]
fn native_timings_count_real_execution_without_js_bridge_samples() {
    let timings = Timings::default();
    timings
        .native_scoped("beforeSend", PhaseScope::Global)
        .finish(&Ok::<_, Failure>(()));
    timings
        .native_scoped("beforeSend", PhaseScope::Global)
        .finish(&Err::<(), _>(Failure::new(
            ErrorCode::HookFailed,
            "beforeSend",
        )));
    drop(timings.native_scoped("onContent", PhaseScope::Global));
    let snapshot = timings.snapshot();
    assert!(snapshot["hooks"].as_object().unwrap().is_empty());
    assert_eq!(snapshot["native"]["beforeSend"]["calls"], 2);
    assert_eq!(snapshot["native"]["beforeSend"]["failedCalls"], 1);
    assert_eq!(snapshot["native"]["beforeSend"]["interruptedCalls"], 0);
    assert_eq!(snapshot["native"]["onContent"]["interruptedCalls"], 1);
    assert!(snapshot["native"]["beforeSend"]["queue"].is_null());
    timings.freeze(Duration::ZERO);
    let frozen = timings.snapshot();
    drop(timings.native_scoped("onContent", PhaseScope::Global));
    assert_eq!(timings.snapshot(), frozen);
}

#[test]
fn milestones_use_their_documented_origins_and_keep_the_first_event() {
    let start = Instant::now();
    let timings = Timings::default();
    timings.start_upstream(start, start + Duration::from_millis(10));
    timings.headers(start + Duration::from_millis(30));
    timings.first_byte(start + Duration::from_millis(35));
    timings.first_byte(start + Duration::from_millis(80));
    timings.first_chunk(start + Duration::from_millis(40));
    timings.downstream(start, start + Duration::from_millis(50), false, false);
    timings.downstream(start, start + Duration::from_millis(90), false, false);
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
            candidate: limen_script::HookCandidate::OnFinish,
            effects: limen_script::HookEffects {
                state: limen_script::ScriptState::new(),
                observations: ObservationBatch::default(),
            },
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

#[test]
fn typed_facts_keep_integer_microseconds_and_group_one_network_observation() {
    let start = Instant::now();
    let timings = Timings::default();
    timings.start_upstream(start, start + Duration::from_micros(17));
    let role =
        limen_chat::observe_generated_content(&json!({"choices":[{"delta":{"role":"assistant"}}]}));
    timings.upstream_content(role, 1, start + Duration::from_micros(20));
    let thought = limen_chat::observe_generated_content(
        &json!({"choices":[{"delta":{"reasoning":"thinking"}}]}),
    );
    timings.upstream_content(thought, 1, start + Duration::from_micros(35));
    let text =
        limen_chat::observe_generated_content(&json!({"choices":[{"delta":{"content":"answer"}}]}));
    timings.upstream_content(text, 1, start + Duration::from_micros(35));
    timings.upstream_content(text, 1, start + Duration::from_micros(42));
    timings.upstream_completed(start + Duration::from_micros(60));
    timings.downstream(start, start + Duration::from_micros(50), false, false);
    timings.downstream(start, start + Duration::from_micros(55), true, false);
    timings.downstream(start, start + Duration::from_micros(57), true, true);
    timings.freeze(Duration::from_micros(70));
    let (owned, generation, _) = timings.owned();
    assert_eq!(owned.request_us, Some(70));
    assert_eq!(owned.pre_send_us, Some(17));
    assert_eq!(owned.upstream_total_us, Some(43));
    assert_eq!(owned.downstream_first_event_us, Some(50));
    assert_eq!(generation.upstream_first_content_us, Some(18));
    assert_eq!(generation.upstream_first_text_us, Some(18));
    assert_eq!(generation.upstream_last_content_us, Some(25));
    assert_eq!(generation.upstream_content_observations, 2);
    assert_eq!(generation.downstream_first_content_us, Some(55));
    assert_eq!(generation.downstream_first_text_us, Some(57));
    assert!(generation.observed_reasoning);
    timings.downstream(start, start + Duration::from_secs(2), true, true);
    assert_eq!(timings.owned().1.downstream_first_text_us, Some(57));
}

#[test]
fn phase_scopes_and_absent_cleanup_samples_remain_distinct() {
    let timings = Timings::default();
    drop(timings.hook_scoped("beforeSend", PhaseScope::Provider));
    drop(timings.hook_scoped("beforeSend", PhaseScope::Model));
    timings.skip("onFinish", PhaseScope::Global, false);
    timings.skip("onError", PhaseScope::Global, true);
    let (_, _, phases) = timings.owned();
    let before: Vec<_> = phases
        .iter()
        .filter(|phase| phase.phase == "beforeSend")
        .collect();
    assert_eq!(before.len(), 2);
    assert_ne!(before[0].scope, before[1].scope);
    assert_eq!(timings.snapshot()["hooks"]["beforeSend"]["calls"], 2);
    for phase in phases.iter().filter(|phase| phase.phase != "beforeSend") {
        assert_eq!(phase.calls, 0);
        assert_eq!(phase.wall_us, None);
        assert_eq!(phase.queue.total_us, None);
        assert_eq!(phase.queue.samples, 0);
    }
    assert_eq!(
        phases
            .iter()
            .find(|phase| phase.phase == "onFinish")
            .unwrap()
            .skipped_budget_calls,
        1
    );
    assert_eq!(
        phases
            .iter()
            .find(|phase| phase.phase == "onError")
            .unwrap()
            .unavailable_calls,
        1
    );
}
