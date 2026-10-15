use limen_script::Cancellation;
use serde_json::json;

use super::{super::upstream::media_kind, *};

#[test]
fn committed_state_is_owned_and_failed_state_leaves_previous_value() {
    let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
    lifecycle.phase = "beforeSend";
    let mut output = json!({"state":{"count":1}});
    commit_state(&mut lifecycle, &mut output).unwrap();
    output["state"]["count"] = json!(2);
    assert_eq!(lifecycle.state["count"], 1);
    assert!(commit_state(&mut lifecycle, &mut json!({"state":[]})).is_err());
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
    let context = lifecycle.context("afterReceive");
    assert_eq!(context["meta"]["clientStream"], true);
    assert_eq!(context["meta"]["upstreamStatus"], 401);
    assert_eq!(context["meta"]["model"], "chat");
    assert_eq!(context["meta"]["configuredUpstreamModel"], "upstream");
}

#[test]
fn media_selection_ignores_case_and_parameters_but_not_other_types() {
    assert_eq!(media_kind(" Application/JSON ; charset=utf-8"), "json");
    assert_eq!(media_kind("TEXT/EVENT-STREAM;charset=utf-8"), "sse");
    assert_eq!(media_kind("application/problem+json"), "other");
    assert_eq!(media_kind("text/plain"), "other");
}

#[test]
fn response_hook_body_requires_an_object() {
    assert_eq!(
        require_body(&json!({"body":{"choices":[],"vendor":7}})).unwrap()["vendor"],
        7
    );
    assert_eq!(
        require_body(&json!({"body":[]})).unwrap_err().code,
        ErrorCode::HookFailed
    );
}

#[test]
fn business_rejection_commits_valid_state_and_preserves_phase_and_step() {
    for status in [400, 403, 422] {
        let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
        lifecycle.phase = "beforeSend";
        lifecycle.step = Some("policy".into());
        let mut output = json!({"state":{"checked":true},"rejection":{"status":status}});
        let failure = commit_request(&mut lifecycle, &mut output).unwrap_err();
        assert_eq!(failure.code, ErrorCode::RequestRejected);
        assert_eq!(failure.http_status(), status);
        assert_eq!(failure.phase, "beforeSend");
        assert_eq!(failure.step.as_deref(), Some("policy"));
        assert_eq!(lifecycle.state, json!({"checked":true}));
    }
}

#[test]
fn malformed_rejection_or_state_cannot_commit() {
    let mut lifecycle = Lifecycle::new("request-1".into(), Cancellation::new(), 1000);
    lifecycle.state = json!({"previous":true});
    for rejection in [
        json!(null),
        json!({}),
        json!({"status":500}),
        json!({"status":"400"}),
    ] {
        let failure = commit_request(
            &mut lifecycle,
            &mut json!({"state":{"changed":true},"rejection":rejection}),
        )
        .unwrap_err();
        assert_eq!(failure.code, ErrorCode::HookFailed);
        assert_eq!(lifecycle.state, json!({"previous":true}));
    }
    let failure = commit_request(
        &mut lifecycle,
        &mut json!({"state":[],"rejection":{"status":400}}),
    )
    .unwrap_err();
    assert_eq!(failure.code, ErrorCode::HookFailed);
    assert_eq!(lifecycle.state, json!({"previous":true}));
}
