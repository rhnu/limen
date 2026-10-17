use std::time::Duration;

use serde_json::json;

use super::*;

fn committed(context: Json) -> HookOutput {
    let Json::Object(state) = context else {
        panic!("committed test state must be an object");
    };
    HookOutput {
        effects: crate::HookEffects {
            state,
            observations: ObservationBatch::default(),
        },
        candidate: crate::HookCandidate::OnRequest {
            body: json!({}),
            rejection: None,
        },
    }
}

#[test]
fn hook_lookup_rejects_zero_and_unregistered_ids() {
    let hooks = ["first", "second"];
    assert_eq!(lookup_hook(&hooks, 1), Some(&"first"));
    assert_eq!(lookup_hook(&hooks, 2), Some(&"second"));
    for hook_id in [0, 3, u64::MAX] {
        assert_eq!(lookup_hook(&hooks, hook_id), None);
    }
    assert_eq!(lookup_hook::<()>(&[], 1), None);
}

#[test]
fn completion_rechecks_cancellation_and_deadline_before_delivering_a_result() {
    for (cancelled, expired, expected) in [
        (false, false, None),
        (true, false, Some(ScriptErrorKind::Cancelled)),
        (false, true, Some(ScriptErrorKind::Deadline)),
        (true, true, Some(ScriptErrorKind::Cancelled)),
    ] {
        let cancellation = Cancellation::new();
        let (response, mut receive) = oneshot::channel();
        let mut completion = Completion {
            phase: HookPhase::OnRequest,
            cancellation: cancellation.clone(),
            deadline: Instant::now() + Duration::from_secs(60),
            response,
            timings: HookTimings {
                queue: Some(Duration::from_nanos(2)),
                initial_call: Some(Duration::from_nanos(3)),
                input_bridge: Duration::from_nanos(5),
                output_bridge: Some(Duration::from_nanos(7)),
                ..HookTimings::default()
            },
            pending_since: None,
        };
        if expired {
            completion.deadline = Instant::now();
        }
        if cancelled {
            cancellation.cancel();
        }
        completion.send(Ok(committed(json!({"committed": true}))));
        let report = receive.try_recv().unwrap();
        assert_eq!(report.timings.queue, Some(Duration::from_nanos(2)));
        assert_eq!(report.timings.initial_call, Some(Duration::from_nanos(3)));
        assert_eq!(report.timings.input_bridge, Duration::from_nanos(5));
        assert_eq!(report.timings.output_bridge, Some(Duration::from_nanos(7)));
        let result = report
            .outcome
            .map(|output| Json::Object(output.effects.state));
        match expected {
            None => assert_eq!(result.unwrap(), json!({"committed": true})),
            Some(expected) => assert_eq!(result.unwrap_err().kind(), expected),
        }
    }
}

#[test]
fn closed_receiver_revokes_completion_and_live_receiver_keeps_hook_failure() {
    let (response, mut receive) = oneshot::channel();
    let completion = Completion {
        phase: HookPhase::OnRequest,
        cancellation: Cancellation::new(),
        deadline: Instant::now() + Duration::from_secs(60),
        response,
        timings: HookTimings::default(),
        pending_since: None,
    };
    receive.close();
    assert_eq!(
        completion.invalid().unwrap().kind(),
        ScriptErrorKind::Cancelled
    );
    completion.send(Ok(committed(json!({"committed": true}))));
    assert!(matches!(
        receive.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));

    let (response, mut receive) = oneshot::channel();
    let completion = Completion {
        phase: HookPhase::OnRequest,
        cancellation: Cancellation::new(),
        deadline: Instant::now() + Duration::from_secs(60),
        response,
        timings: HookTimings::default(),
        pending_since: None,
    };
    completion.send(Err(ScriptError::new(ScriptErrorKind::Contract)));
    assert_eq!(
        receive.try_recv().unwrap().outcome.unwrap_err().kind(),
        ScriptErrorKind::Contract
    );
}

#[test]
fn observation_envelopes_validate_ids_capacity_and_safe_dropped_counts() {
    let options = RuntimeOptions {
        observation_names: vec!["first".into(), "second".into()],
        max_observations_per_hook: 1,
        ..RuntimeOptions::default()
    };
    let output = decode_output(json!({"context": {"state":{},"body":{}}, "observations": {"events": [1], "dropped": 9_007_199_254_740_991_u64}}), HookPhase::OnRequest, &options).unwrap();
    assert_eq!(output.effects.observations.events, [1]);
    assert_eq!(output.effects.observations.dropped, 9_007_199_254_740_991);
    for envelope in [
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [2], "dropped": 0}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [0, 1], "dropped": 0}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [0], "dropped": -1}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [], "dropped": 9_007_199_254_740_992_u64}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [0.5], "dropped": 0}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [256], "dropped": 0}}),
        json!({"context": {"state":{},"body":{}}, "observations": {"events": [], "dropped": 0, "extra": true}}),
        json!({"context": {"state":{},"body":{}}}),
    ] {
        assert_eq!(
            decode_output(envelope, HookPhase::OnRequest, &options)
                .unwrap_err()
                .kind(),
            ScriptErrorKind::Contract
        );
    }
    let options = RuntimeOptions {
        max_observations_per_hook: 2,
        ..options
    };
    assert_eq!(
        decode_output(
            json!({"context": {"state":{},"body":{}}, "observations": {"events": [0, 0], "dropped": 0}}),
            HookPhase::OnRequest, &options
        )
        .unwrap_err()
        .kind(),
        ScriptErrorKind::Contract
    );
}
