use serde_json::json;

use super::*;

#[test]
fn open_chat_objects_allow_optional_fields_and_content_blocks() {
    for value in [
        json!({}),
        json!({"usage":false}),
        json!({"usage":["invalid statistics remain observable"]}),
        json!({"object":"", "vendor":{"x":true}}),
        json!({"choices":[{"index":0,"annotations":[]}]}),
        json!({"choices":[{"index":1,"delta":{"content":[{"type":"text","text":"hello"},{"type":"image_url","image_url":{"url":"synthetic"}}]}}]}),
    ] {
        validate_chunk(&value).unwrap();
    }
    for value in [
        json!([]),
        json!({"id":null}),
        json!({"created":-1}),
        json!({"choices":null}),
        json!({"choices":[{"index":-1}]}),
        json!({"choices":[{"index":0,"delta":null}]}),
        json!({"choices":[{"index":0,"finish_reason":42}]}),
        json!({"choices":[{"index":0,"delta":{"content":["text"]}}]}),
    ] {
        assert!(matches!(validate_chunk(&value), Err(Error::InvalidChunk)));
    }
}

#[test]
fn annotations_and_auxiliary_events_do_not_invent_generation_completion() {
    let annotation = json!({"choices":[{"index":999,"content_filter_results":{"safe":true},"finish_reason":null}]});
    let mut tracker = ChunkTracker::new(1);
    tracker.push(&annotation).unwrap();
    tracker
        .push(&json!({"usage":{"total_tokens":1}, "vendor":"auxiliary"}))
        .unwrap();
    assert!(matches!(tracker.finish(), Err(Error::UnfinishedChoices)));
    let mut tracker = ChunkTracker::new(1);
    tracker.push(&annotation).unwrap();
    tracker
        .push(&json!({"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}]}))
        .unwrap();
    tracker.push(&annotation).unwrap();
    tracker.finish().unwrap();
}

#[test]
fn completed_choices_accept_audio_expiry_annotations_and_unknown_extensions() {
    let mut tracker = ChunkTracker::new(1);
    tracker
        .push(&json!({"choices":[{"index":0,"delta":{"audio":{"data":"YQ=="}}}]}))
        .unwrap();
    tracker
        .push(&json!({"choices":[{"index":0,"finish_reason":"stop"}]}))
        .unwrap();
    for tail in [
        json!({"choices":[{"index":0,"delta":{"audio":{"id":"audio1","expires_at":12}}}]}),
        json!({"choices":[{"index":0,"annotations":[{"kind":"citation"}]}]}),
        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":null,"vendor_audio":{"bytes":"opaque"}},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"content":[],"refusal":"","tool_calls":[]},"finish_reason":"stop"}]}),
        json!({"vendor":{"extension":true}}),
    ] {
        tracker.push(&tail).unwrap();
    }
    tracker.finish().unwrap();
}

#[test]
fn completed_choices_reject_all_known_generated_content() {
    for delta in [
        json!({"content":"late"}),
        json!({"content":[{"type":"text","text":"late"}]}),
        json!({"refusal":"late"}),
        json!({"tool_calls":[{"index":0}]}),
        json!({"function_call":{"arguments":"late"}}),
        json!({"reasoning":"late"}),
        json!({"reasoning_content":"late"}),
        json!({"reasoning_details":[{"text":"late"}]}),
        json!({"audio":{"data":"late"}}),
        json!({"audio":{"transcript":"late"}}),
    ] {
        let mut tracker = ChunkTracker::new(1);
        tracker
            .push(&json!({"choices":[{"index":0,"finish_reason":"stop"}]}))
            .unwrap();
        let mut tail = json!({"choices":[{"index":0}]});
        tail["choices"][0]["delta"] = delta;
        assert!(
            matches!(tracker.push(&tail), Err(Error::ChoiceFinished)),
            "{tail}"
        );
    }
    let mut tracker = ChunkTracker::new(1);
    tracker
        .push(&json!({"choices":[{"index":0,"finish_reason":"stop"}]}))
        .unwrap();
    assert!(matches!(
        tracker.push(&json!({"choices":[{"index":0,"finish_reason":"length"}]})),
        Err(Error::ChoiceFinished)
    ));
}

#[test]
fn completion_errors_remain_terminal_after_an_upstream_error() {
    let mut tracker = ChunkTracker::new(1);
    tracker
        .push(&json!({"choices":[{"index":0,"finish_reason":"stop"}]}))
        .unwrap();
    assert!(matches!(
        tracker.push(&json!({"error":{"code":"failed"}})),
        Err(Error::UpstreamError)
    ));
    assert!(matches!(
        tracker.push(&json!({"choices":[]})),
        Err(Error::InvalidChunk)
    ));
    assert!(matches!(tracker.finish(), Err(Error::InvalidChunk)));
}

#[test]
fn duplicate_or_negative_tool_indices_still_fail_same_shape_validation() {
    for calls in [json!([{"index":0},{"index":0}]), json!([{"index":-1}])] {
        let mut event = json!({"choices":[{"index":0,"delta":{}}]});
        event["choices"][0]["delta"]["tool_calls"] = calls;
        assert!(matches!(validate_chunk(&event), Err(Error::InvalidChunk)));
    }
}

#[test]
fn metadata_only_deltas_do_not_open_new_generation_choices() {
    let mut tracker = ChunkTracker::new(1);
    tracker
        .push(&json!({"choices":[{"index":0,"delta":{"content":"answer"},"finish_reason":"stop"}]}))
        .unwrap();
    for delta in [
        json!({"audio":{"expires_at":10}}),
        json!({"annotations":[{"source":1}]}),
    ] {
        tracker
            .push(&json!({"choices":[{"index":99,"delta":delta}]}))
            .unwrap();
    }
    tracker.finish().unwrap();
}

#[test]
fn finish_counts_classify_each_choice_once_and_keep_tail_reports_idempotent() {
    let mut tracker = ChunkTracker::new(8);
    let choices = json!({"choices":[
        {"index":0,"delta":{},"finish_reason":"stop"},
        {"index":1,"delta":{},"finish_reason":"length"},
        {"index":2,"delta":{},"finish_reason":"tool_calls"},
        {"index":3,"delta":{},"finish_reason":"function_call"},
        {"index":4,"delta":{},"finish_reason":"content_filter"},
        {"index":5,"delta":{},"finish_reason":"provider_secret_reason"}
    ]});
    tracker.push(&choices).unwrap();
    tracker.push(&choices).unwrap();
    assert_eq!(tracker.choice_count(), 6);
    assert_eq!(
        tracker.finish_counts(),
        super::FinishCounts {
            stop: 1,
            length: 1,
            tool: 2,
            filtered: 1,
            other: 1
        }
    );
    tracker.finish().unwrap();
}

#[test]
fn full_json_finish_observation_preserves_the_open_object_boundary() {
    let source = json!({"choices":[{"finish_reason":"stop"},{"finish_reason":"length"},{"finish_reason":null},{"finish_reason":12},{"vendor":"opaque"}]});
    assert_eq!(
        super::FinishCounts::from_response(&source),
        super::FinishCounts {
            stop: 1,
            length: 1,
            ..super::FinishCounts::default()
        }
    );
    assert_eq!(
        super::FinishCounts::from_response(&json!({"choices":"opaque"})),
        super::FinishCounts::default()
    );
}
