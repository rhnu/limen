use limen_script::{Cancellation, HookPhase, ObservationBatch, ResponseHead, ScriptState};
use serde_json::json;

use super::{super::upstream::media_kind, *};

#[test]
fn committed_state_is_owned() {
    let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
    let mut state = ScriptState::new();
    state.insert("count".into(), json!(1));
    commit_effects(
        &mut lifecycle,
        HookEffects {
            state: state.clone(),
            observations: ObservationBatch::default(),
        },
    );
    state.insert("count".into(), json!(2));
    assert_eq!(lifecycle.state["count"], 1);
}

#[test]
fn metadata_retains_client_intent_and_original_upstream_facts() {
    let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
    lifecycle.client_stream = Some(true);
    lifecycle.alias = Some("chat".into());
    lifecycle.configured_model = Some("upstream".into());
    lifecycle.upstream_status = Some(401);
    lifecycle.upstream_media = Some("json".into());
    let context = lifecycle.context(HookInput::AfterReceive(ResponseHead {
        status: 200,
        headers: Vec::new(),
    }));
    assert_eq!(context.input.phase(), HookPhase::AfterReceive);
    assert_eq!(context.meta.client_stream, Some(true));
    assert_eq!(context.meta.upstream_status, Some(401));
    assert_eq!(context.meta.model.as_deref(), Some("chat"));
    assert_eq!(
        context.meta.configured_upstream_model.as_deref(),
        Some("upstream")
    );
}

#[test]
fn media_selection_ignores_case_and_parameters_but_not_other_types() {
    assert_eq!(media_kind(" Application/JSON ; charset=utf-8"), "json");
    assert_eq!(media_kind("TEXT/EVENT-STREAM;charset=utf-8"), "sse");
    assert_eq!(media_kind("application/problem+json"), "other");
    assert_eq!(media_kind("text/plain"), "other");
}

#[test]
fn business_rejection_commits_state_and_observations_preserving_phase_and_step() {
    use super::super::observations::Observations;
    for status in [
        RejectionStatus::BadRequest,
        RejectionStatus::Forbidden,
        RejectionStatus::UnprocessableEntity,
    ] {
        let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
        lifecycle.phase = "beforeSend";
        lifecycle.step = Some("policy".into());
        lifecycle.observations = Observations::new(4);
        let mut state = ScriptState::new();
        state.insert("checked".into(), json!(true));
        let effects = HookEffects {
            state,
            observations: ObservationBatch {
                events: vec![0],
                dropped: 0,
            },
        };
        let failure = commit_request(&mut lifecycle, effects, Some(status)).unwrap_err();
        assert_eq!(failure.code, ErrorCode::RequestRejected);
        assert_eq!(failure.http_status(), status.as_u16());
        assert_eq!(failure.phase, "beforeSend");
        assert_eq!(failure.step.as_deref(), Some("policy"));
        assert_eq!(lifecycle.state["checked"], true);
        assert_eq!(
            lifecycle.observations.snapshot(&["policy_applied".into()])["events"][0]["count"],
            1
        );
    }
}
