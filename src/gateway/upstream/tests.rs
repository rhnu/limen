use std::error::Error as _;

use serde_json::json;

use super::*;

mod keepalive;

fn choices(count: u64, limits: &ResourceLimits) -> Aggregator {
    let mut aggregate = Aggregator::new(limits.max_aggregate_bytes, limits.max_aggregate_events);
    for index in 0..count {
        let event = json!({"id":"completion", "object":"chat.completion.chunk", "created":1,
            "model":"upstream", "choices":[{"index":index,
                "delta":{"role":"assistant","content":"hello"}, "finish_reason":"stop"}]});
        validate_json(&event, limits.max_json_depth, limits.max_json_nodes).unwrap();
        aggregate.push(event).unwrap();
    }
    aggregate
}

#[test]
fn aggregation_rejects_combined_nodes_even_when_every_source_event_fits() {
    let limits = ResourceLimits {
        max_json_nodes: 32,
        ..ResourceLimits::default()
    };
    let failure = finish_aggregate(choices(5, &limits), &limits).unwrap_err();
    assert_eq!(failure.code, ErrorCode::UpstreamProtocol);
    assert_eq!(failure.phase, "sse_to_json");
    assert!(failure.source().unwrap().is::<limen_config::ConfigError>());
}

#[test]
fn bounded_aggregation_preserves_the_complete_response() {
    let limits = ResourceLimits {
        max_json_nodes: 32,
        ..ResourceLimits::default()
    };
    let body = finish_aggregate(choices(2, &limits), &limits).unwrap();
    assert_eq!(body["id"], "completion");
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["model"], "upstream");
    assert_eq!(body["choices"].as_array().unwrap().len(), 2);
    assert_eq!(body["choices"][0]["index"], 0);
    assert_eq!(body["choices"][1]["index"], 1);
    assert_eq!(body["choices"][0]["message"]["content"], "hello");
    assert_eq!(body["choices"][1]["finish_reason"], "stop");
}
