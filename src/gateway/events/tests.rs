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
    let size =
        validate_event_payload(&event, &limits, ErrorCode::HookFailed, "onStreamEvent").unwrap();
    assert_eq!(size, source.len());
    let mut budget = EventBudget::new();
    budget.charge(1, size, &limits, "onUpstreamEvent").unwrap();
    budget.charge(1, size, &limits, "onStreamEvent").unwrap();
    assert!(budget.charge(1, size, &limits, "onStreamEvent").is_err());
    assert_eq!(
        validate_event_payload(
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
            &json!({"choices":[{"index":-1}]}),
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
fn outgoing_usage_tail_cannot_gain_content_or_tools_from_hooks() {
    let finished = json!({"id":"c1","object":"chat.completion.chunk","created":1,
        "model":"alias","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
    for delta in [
        json!({"content":"late"}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":"late"}}]}),
    ] {
        let mut tracker = limen_chat::ChunkTracker::new(1);
        tracker.push(&finished).unwrap();
        let mut frames = Vec::new();
        let mut tail = finished.clone();
        tail["usage"] = json!({"total_tokens":3});
        EventConsumer::Stream {
            tracker: &mut tracker,
            frames: &mut frames,
            max_event_bytes: 4096,
            frame: None,
            stream_started: false,
        }
        .consume(
            &[tail.clone()],
            &mut EventBudget::new(),
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        tail["choices"][0]["delta"] = delta;
        let failure = EventConsumer::Stream {
            tracker: &mut tracker,
            frames: &mut frames,
            max_event_bytes: 4096,
            frame: None,
            stream_started: false,
        }
        .consume(&[tail], &mut EventBudget::new(), &ResourceLimits::default())
        .unwrap_err();
        assert_eq!(failure.code, ErrorCode::HookFailed);
        assert_eq!(failure.phase, "onStreamEvent");
        assert_eq!(frames.len(), 1);
    }
}

fn data_frame(bytes: &[u8]) -> limen_chat::Frame {
    let mut decoder = limen_chat::FrameDecoder::new(4096);
    for byte in bytes {
        if let Some(frame) = decoder.push(*byte).unwrap()
            && frame.data.is_some()
        {
            return frame;
        }
    }
    panic!("complete synthetic frame required")
}

#[test]
fn expanded_frames_emit_a_source_bom_once_and_preserve_both_data_events() {
    let first = json!({"choices":[{"index":0,"delta":{"content":"first"}}]});
    let last = json!({"choices":[{"index":0,"delta":{"content":"last"},"finish_reason":"stop"}]});
    let frame = data_frame(format!("\u{feff}id: source\ndata: {first}\n\n").as_bytes());
    let mut tracker = limen_chat::ChunkTracker::new(1);
    let mut frames = Vec::new();
    EventConsumer::Stream {
        tracker: &mut tracker,
        frames: &mut frames,
        max_event_bytes: 4096,
        frame: Some(&frame),
        stream_started: false,
    }
    .consume(
        &[first.clone(), last.clone()],
        &mut EventBudget::new(),
        &ResourceLimits::default(),
    )
    .unwrap();
    tracker.finish().unwrap();
    let bytes = frames.concat();
    let mut decoder = limen_chat::FrameDecoder::new(4096);
    let mut data = Vec::new();
    for byte in bytes {
        if let Some(frame) = decoder.push(byte).unwrap()
            && let limen_chat::Event::Chunk(value) = frame.event().unwrap()
        {
            data.push(value);
        }
    }
    decoder.finish().unwrap();
    assert_eq!(data, [first, last]);
}

#[test]
fn preserved_outer_fields_consume_the_shared_encoded_output_budget() {
    let event = json!({"choices":[{"index":0,"delta":{"content":"x"}}]});
    let frame = data_frame(format!(": {}\ndata: {event}\n\n", "x".repeat(700)).as_bytes());
    let limits = ResourceLimits {
        max_output_bytes: 1000,
        ..ResourceLimits::default()
    };
    let mut tracker = limen_chat::ChunkTracker::new(1);
    let mut frames = Vec::new();
    let failure = EventConsumer::Stream {
        tracker: &mut tracker,
        frames: &mut frames,
        max_event_bytes: 4096,
        frame: Some(&frame),
        stream_started: false,
    }
    .consume(&[event.clone(), event], &mut EventBudget::new(), &limits)
    .unwrap_err();
    assert_eq!(failure.code, ErrorCode::HookFailed);
    assert_eq!(frames, [] as [Vec<u8>; 0]);
}
