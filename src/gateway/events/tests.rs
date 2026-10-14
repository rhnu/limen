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
    budget
        .charge(&[json!({}), json!({})], &limits, "onUpstreamEvent")
        .unwrap();
    budget
        .charge(&[json!({})], &limits, "onStreamEvent")
        .unwrap();
    assert_eq!(
        budget
            .charge(&[json!({})], &limits, "onStreamEvent")
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
    budget
        .charge(&[json!({})], &limits, "onUpstreamEvent")
        .unwrap();
    budget.charge(&[], &limits, "onUpstreamEvent").unwrap();
    budget
        .charge(&[json!({})], &limits, "onStreamEvent")
        .unwrap();
    assert!(
        budget
            .charge(&[json!({})], &limits, "onStreamEvent")
            .is_err()
    );
    let mut separate_source_event = EventBudget::new();
    assert!(
        separate_source_event
            .charge(&[json!({})], &limits, "onStreamEvent")
            .is_ok()
    );
}
