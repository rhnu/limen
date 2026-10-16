use serde_json::{Value, json};

use crate::{UsageQuality, UsageStats, UsageTracker};

fn observed(usage: &Value) -> UsageStats {
    let mut tracker = UsageTracker::default();
    tracker.observe(Some(usage));
    tracker.snapshot()
}

fn assert_suppressed(stats: UsageStats, quality: UsageQuality) {
    assert_eq!(stats.quality, quality);
    assert_eq!(stats.input_tokens, None);
    assert_eq!(stats.output_tokens, None);
    assert_eq!(stats.total_tokens, None);
    assert_eq!(stats.cached_input_tokens, None);
    assert_eq!(stats.reasoning_output_tokens, None);
}

#[test]
fn absent_null_and_empty_reports_are_missing() {
    let mut tracker = UsageTracker::default();
    tracker.observe(None);
    for value in [
        Value::Null,
        json!({}),
        json!({"prompt_tokens":null,"completion_tokens":null,"total_tokens":null}),
        json!({"prompt_tokens_details":null,"completion_tokens_details":{}}),
        json!({"prompt_tokens_details":{"cached_tokens":null}}),
        json!({"vendor_tokens":100,"vendor_details":{"private":"ignored"}}),
    ] {
        tracker.observe(Some(&value));
        assert_eq!(tracker.snapshot(), UsageStats::default());
    }
}

#[test]
fn zero_is_reported_and_details_map_to_owned_json() {
    let stats = observed(&json!({
        "prompt_tokens":0,"completion_tokens":0,"total_tokens":0,
        "prompt_tokens_details":{"cached_tokens":0},
        "completion_tokens_details":{"reasoning_tokens":0},
    }));
    assert_eq!(stats.quality, UsageQuality::Reported);
    assert_eq!(
        stats.to_json(),
        json!({
            "inputTokens":0,"outputTokens":0,"totalTokens":0,
            "cachedInputTokens":0,"reasoningOutputTokens":0,"quality":"reported",
        })
    );
}

#[test]
fn partial_reports_never_estimate_missing_counts() {
    for usage in [
        json!({"prompt_tokens":3}),
        json!({"completion_tokens":2}),
        json!({"total_tokens":5}),
        json!({"prompt_tokens_details":{"cached_tokens":1}}),
        json!({"completion_tokens_details":{"reasoning_tokens":1}}),
    ] {
        assert_eq!(observed(&usage).quality, UsageQuality::Partial);
    }
    let stats = observed(&json!({"prompt_tokens":3,"completion_tokens":2}));
    assert_eq!(stats.total_tokens, None);
    assert_eq!(stats.cached_input_tokens, None);
    assert_eq!(stats.reasoning_output_tokens, None);
}

#[test]
fn reported_quality_does_not_require_or_derive_details() {
    let stats = observed(&json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":9}));
    assert_eq!(stats.quality, UsageQuality::Reported);
    assert_eq!(stats.total_tokens, Some(9));
    assert_eq!(stats.cached_input_tokens, None);
    assert_eq!(stats.reasoning_output_tokens, None);
}

#[test]
fn repeated_cumulative_reports_are_not_added_and_nulls_do_not_erase() {
    let mut tracker = UsageTracker::default();
    let usage = json!({
        "prompt_tokens":4,"completion_tokens":6,"total_tokens":10,
        "prompt_tokens_details":{"cached_tokens":2},
        "completion_tokens_details":{"reasoning_tokens":3},
    });
    tracker.observe(Some(&usage));
    let initial = tracker.snapshot();
    tracker.observe(Some(&usage));
    tracker.observe(None);
    tracker.observe(Some(&Value::Null));
    tracker.observe(Some(&json!({
        "prompt_tokens":null,"completion_tokens":null,"total_tokens":null,
        "prompt_tokens_details":null,
        "completion_tokens_details":{"reasoning_tokens":null},
    })));
    assert_eq!(tracker.snapshot(), initial);
    assert_eq!(initial.input_tokens, Some(4));
    assert_eq!(initial.total_tokens, Some(10));
}

#[test]
fn fragments_fill_missing_fields_without_mutating_previous_snapshots() {
    let mut tracker = UsageTracker::default();
    tracker.observe(Some(&json!({"prompt_tokens":4})));
    let partial = tracker.snapshot();
    tracker.observe(Some(&json!({"completion_tokens":6})));
    tracker.observe(Some(&json!({"total_tokens":10})));
    tracker.observe(Some(&json!({"prompt_tokens_details":{"cached_tokens":2}})));
    tracker.observe(Some(
        &json!({"completion_tokens_details":{"reasoning_tokens":3}}),
    ));
    assert_eq!(partial.quality, UsageQuality::Partial);
    assert_eq!(partial.output_tokens, None);
    assert_eq!(
        tracker.snapshot().to_json(),
        json!({
            "inputTokens":4,"outputTokens":6,"totalTokens":10,
            "cachedInputTokens":2,"reasoningOutputTokens":3,"quality":"reported",
        })
    );
}

#[test]
fn increasing_snapshots_replace_each_count_without_adding_or_erasing() {
    let mut tracker = UsageTracker::default();
    tracker.observe(Some(&json!({"prompt_tokens":4,"completion_tokens":1})));
    let partial = tracker.snapshot();
    tracker.observe(Some(&json!({
        "prompt_tokens":4,"completion_tokens":3,"total_tokens":7,
        "prompt_tokens_details":{"cached_tokens":1},
        "completion_tokens_details":{"reasoning_tokens":1},
    })));
    let intermediate = tracker.snapshot();
    tracker.observe(Some(&json!({
        "prompt_tokens":null,"completion_tokens":6,"total_tokens":10,
        "prompt_tokens_details":{"cached_tokens":2},
        "completion_tokens_details":null,
    })));
    tracker.observe(Some(&json!({
        "prompt_tokens":5,"completion_tokens":null,"total_tokens":11,
        "completion_tokens_details":{"reasoning_tokens":3},
    })));
    assert_eq!(partial.quality, UsageQuality::Partial);
    assert_eq!(partial.output_tokens, Some(1));
    assert_eq!(intermediate.output_tokens, Some(3));
    assert_eq!(intermediate.quality, UsageQuality::Reported);
    assert_eq!(
        tracker.snapshot().to_json(),
        json!({
            "inputTokens":5,"outputTokens":6,"totalTokens":11,
            "cachedInputTokens":2,"reasoningOutputTokens":3,"quality":"reported",
        })
    );
}

#[test]
fn safe_integer_limits_and_integer_valued_floats_are_supported() {
    let stats = observed(&json!({
        "prompt_tokens":9_007_199_254_740_991_u64,
        "completion_tokens":1.0,
        "total_tokens":9_007_199_254_740_991.0,
    }));
    assert_eq!(stats.quality, UsageQuality::Reported);
    assert_eq!(stats.input_tokens, Some(9_007_199_254_740_991));
    assert_eq!(stats.output_tokens, Some(1));
    assert_eq!(
        observed(&json!({"prompt_tokens":-0.0})).input_tokens,
        Some(0)
    );
}

#[test]
fn malformed_counts_suppress_all_effective_counts_permanently() {
    for value in [
        json!(-1),
        json!(0.5),
        json!("1"),
        json!(true),
        json!([]),
        json!({}),
        json!(9_007_199_254_740_992_u64),
        json!(9_007_199_254_740_992.0),
        json!(u64::MAX),
    ] {
        for field in ["prompt_tokens", "completion_tokens", "total_tokens"] {
            let mut tracker = UsageTracker::default();
            tracker.observe(Some(
                &json!({"prompt_tokens":4,"completion_tokens":6,"total_tokens":10}),
            ));
            tracker.observe(Some(&json!({field:value})));
            tracker.observe(Some(
                &json!({"prompt_tokens":4,"completion_tokens":6,"total_tokens":10}),
            ));
            assert_suppressed(tracker.snapshot(), UsageQuality::Invalid);
        }
    }
}

#[test]
fn malformed_usage_and_known_details_are_invalid() {
    for usage in [json!([]), json!(1), json!("usage"), json!(false)] {
        assert_suppressed(observed(&usage), UsageQuality::Invalid);
    }
    for field in ["prompt_tokens_details", "completion_tokens_details"] {
        for details in [json!([]), json!(1), json!("details"), json!(false)] {
            assert_suppressed(observed(&json!({field:details})), UsageQuality::Invalid);
        }
    }
    assert_suppressed(
        observed(&json!({"prompt_tokens_details":{"cached_tokens":"1"}})),
        UsageQuality::Invalid,
    );
    assert_suppressed(
        observed(&json!({"completion_tokens_details":{"reasoning_tokens":-1}})),
        UsageQuality::Invalid,
    );
}

#[test]
fn unknown_extensions_are_ignored_and_not_copied() {
    let stats = observed(&json!({
        "prompt_tokens":3,"completion_tokens":2,"total_tokens":5,
        "private_extension":{"content":"not retained"},
        "prompt_tokens_details":{"cached_tokens":1,"vendor":[]},
        "completion_tokens_details":{"reasoning_tokens":1,"vendor":"ignored"},
    }));
    assert_eq!(
        stats.to_json(),
        json!({
            "inputTokens":3,"outputTokens":2,"totalTokens":5,
            "cachedInputTokens":1,"reasoningOutputTokens":1,"quality":"reported",
        })
    );
}

#[test]
fn regressing_core_and_detail_counts_suppress_all_counts_permanently() {
    for changed in [
        json!({"prompt_tokens":3}),
        json!({"completion_tokens":5}),
        json!({"total_tokens":9}),
        json!({"prompt_tokens_details":{"cached_tokens":1}}),
        json!({"completion_tokens_details":{"reasoning_tokens":2}}),
    ] {
        let mut tracker = UsageTracker::default();
        let usage = json!({
            "prompt_tokens":4,"completion_tokens":6,"total_tokens":10,
            "prompt_tokens_details":{"cached_tokens":2},
            "completion_tokens_details":{"reasoning_tokens":3},
        });
        tracker.observe(Some(&usage));
        tracker.observe(Some(&changed));
        tracker.observe(Some(&usage));
        tracker.observe(Some(&Value::Null));
        assert_suppressed(tracker.snapshot(), UsageQuality::Conflicting);
    }
}

#[test]
fn invalid_quality_takes_precedence_independent_of_fragment_order() {
    for fragments in [
        [
            json!({"prompt_tokens":2}),
            json!({"prompt_tokens":1}),
            json!({"total_tokens":"bad"}),
        ],
        [
            json!({"total_tokens":"bad"}),
            json!({"prompt_tokens":2}),
            json!({"prompt_tokens":1}),
        ],
    ] {
        let mut tracker = UsageTracker::default();
        for fragment in fragments {
            tracker.observe(Some(&fragment));
        }
        assert_suppressed(tracker.snapshot(), UsageQuality::Invalid);
    }
}
