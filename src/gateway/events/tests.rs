use serde_json::json;

use super::*;

#[test]
fn expansion_budget_accumulates_across_all_steps_and_both_phases() {
    let limits = ResourceLimits {
        max_output_events: 3,
        max_output_bytes: 100,
        ..ResourceLimits::default()
    };
    let mut budget = EventBudget::new();
    budget.charge(2, 4, &limits, "onUpstreamEvent").unwrap();
    budget.charge(1, 2, &limits, "onStreamEvent").unwrap();
    assert_eq!(
        budget
            .charge(1, 2, &limits, "onStreamEvent")
            .unwrap_err()
            .code,
        ErrorCode::HookFailed
    );
}

#[test]
fn expansion_bytes_count_default_outputs_and_dropped_outputs_cost_nothing() {
    let limits = ResourceLimits {
        max_output_events: 100,
        max_output_bytes: 4,
        ..ResourceLimits::default()
    };
    let mut budget = EventBudget::new();
    budget.charge(1, 2, &limits, "onUpstreamEvent").unwrap();
    budget.charge(0, 0, &limits, "onUpstreamEvent").unwrap();
    budget.charge(1, 2, &limits, "onStreamEvent").unwrap();
    assert!(budget.charge(1, 2, &limits, "onStreamEvent").is_err());
    let mut separate_source_event = EventBudget::new();
    assert!(
        separate_source_event
            .charge(1, 2, &limits, "onStreamEvent")
            .is_ok()
    );
}

#[test]
fn event_size_respects_single_and_cumulative_limits() {
    let source =
        r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"世\n","choices":[]}"#;
    let event = serde_json::from_str(source).unwrap();
    let limits = ResourceLimits {
        max_event_bytes: source.len(),
        max_output_bytes: source.len() * 2,
        ..ResourceLimits::default()
    };
    let size = validate_event(&event, &limits, ErrorCode::HookFailed, "onStreamEvent").unwrap();
    assert_eq!(size, source.len());
    let mut budget = EventBudget::new();
    budget.charge(1, size, &limits, "onUpstreamEvent").unwrap();
    budget.charge(1, size, &limits, "onStreamEvent").unwrap();
    assert!(budget.charge(1, size, &limits, "onStreamEvent").is_err());
    assert_eq!(
        validate_event(
            &event,
            &ResourceLimits {
                max_event_bytes: size - 1,
                ..limits
            },
            ErrorCode::HookFailed,
            "onStreamEvent",
        )
        .unwrap_err()
        .code,
        ErrorCode::HookFailed
    );
}

#[test]
fn hook_output_events_still_reject_malformed_chunks() {
    assert_eq!(
        validate_event_payload(
            &json!({}),
            &ResourceLimits::default(),
            ErrorCode::HookFailed,
            "onStreamEvent"
        )
        .unwrap_err()
        .code,
        ErrorCode::HookFailed
    );
}

#[test]
fn source_events_keep_generic_json_bounds() {
    let event = json!({"id":"c1", "object":"chat.completion.chunk", "created":1,
        "model":"alias", "choices":[], "extension":{"value":1}});
    for limits in [
        ResourceLimits {
            max_json_depth: 1,
            ..ResourceLimits::default()
        },
        ResourceLimits {
            max_json_nodes: 3,
            ..ResourceLimits::default()
        },
    ] {
        assert_eq!(
            validate_event(
                &event,
                &limits,
                ErrorCode::UpstreamProtocol,
                "onUpstreamEvent"
            )
            .unwrap_err()
            .code,
            ErrorCode::UpstreamProtocol
        );
    }
    let mut unsafe_number = event;
    unsafe_number["extension"]["value"] = json!(9_007_199_254_740_992_u64);
    assert!(
        validate_event(
            &unsafe_number,
            &ResourceLimits::default(),
            ErrorCode::UpstreamProtocol,
            "onUpstreamEvent",
        )
        .is_err()
    );
}
