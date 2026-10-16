use super::*;

fn usage_tail(delta: Value, finish: &str) -> Value {
    let mut tail = one(0, delta, json!(finish));
    tail["usage"] = json!({"prompt_tokens":2,"completion_tokens":3,"total_tokens":5});
    tail
}

#[test]
fn accounting_choices_preserve_text_tools_and_finish_reasons() {
    for finish in ["stop", "length", "tool_calls"] {
        for delta in [
            json!({}),
            json!({"role":"assistant","content":""}),
            json!({"content":null,"refusal":null,"tool_calls":[]}),
        ] {
            let mut chunks = if finish == "tool_calls" {
                nullable_tool_chunks()
            } else {
                vec![one(
                    0,
                    json!({"role":"assistant","content":"hello"}),
                    json!(finish),
                )]
            };
            let message = if finish == "tool_calls" {
                json!({"role":"assistant","tool_calls":[{"id":"call_1","type":"function",
                    "function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}}]})
            } else {
                json!({"role":"assistant","content":"hello"})
            };
            let tail = usage_tail(delta, finish);
            let expected = json!({"id":"c1","object":"chat.completion","created":1,
                "model":"alias","choices":[{"index":0,"message":message,"finish_reason":finish}],
                "usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}});
            // Repeated accounting reports remain idempotent and do not reopen a choice.
            chunks.extend([tail.clone(), tail]);
            let mut tracker = ChunkTracker::new(1);
            let mut aggregate = Aggregator::new(100_000, 20);
            for chunk in chunks {
                tracker.push(&chunk).unwrap();
                aggregate.push(chunk).unwrap();
            }
            tracker.finish().unwrap();
            assert_eq!(aggregate.finish().unwrap(), expected);
        }
    }
}

#[test]
fn accounting_frames_do_not_complete_other_choices_or_require_all_to_finish_first() {
    let mut tracker = ChunkTracker::new(2);
    tracker.push(&one(1, json!({}), Value::Null)).unwrap();
    tracker
        .push(&one(0, json!({"content":"a"}), json!("stop")))
        .unwrap();
    tracker
        .push(&usage_tail(json!({"content":""}), "stop"))
        .unwrap();
    assert!(matches!(tracker.finish(), Err(Error::UnfinishedChoices)));

    // A choice first seen in a usage-bearing chunk still follows normal completion rules.
    let mut tracker = ChunkTracker::new(1);
    let mut initial = usage_tail(json!({}), "stop");
    initial["choices"][0]["finish_reason"] = Value::Null;
    tracker.push(&initial).unwrap();
    assert!(matches!(tracker.finish(), Err(Error::UnfinishedChoices)));
}

#[test]
fn accounting_frames_cannot_hide_new_data_or_changed_completion() {
    let finished = one(
        0,
        json!({"role":"assistant","content":"hello"}),
        json!("stop"),
    );
    let mut invalid = Vec::new();
    for delta in [
        json!({"content":"late"}),
        json!({"refusal":"late"}),
        json!({"role":"user"}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":"late"}}]}),
        json!({"tool_calls":[{"index":0}]}),
        json!({"reasoning":"late"}),
        json!({"vendor_audio":{"bytes":"late"}}),
    ] {
        invalid.push(usage_tail(delta, "stop"));
    }
    for finish in [json!("length"), Value::Null] {
        let mut tail = usage_tail(json!({}), "stop");
        tail["choices"][0]["finish_reason"] = finish;
        invalid.push(tail);
    }
    for usage in [None, Some(Value::Null)] {
        let mut tail = usage_tail(json!({}), "stop");
        tail.as_object_mut().unwrap().remove("usage");
        if let Some(usage) = usage {
            tail["usage"] = usage;
        }
        invalid.push(tail);
    }
    let mut logprobs = usage_tail(json!({}), "stop");
    logprobs["choices"][0]["logprobs"] = json!({"content":[]});
    invalid.push(logprobs);
    for tail in invalid {
        let mut tracker = ChunkTracker::new(1);
        tracker.push(&finished).unwrap();
        assert!(matches!(tracker.push(&tail), Err(Error::ChoiceFinished)));
        let mut aggregate = Aggregator::new(100_000, 20);
        aggregate.push(finished.clone()).unwrap();
        assert!(matches!(aggregate.push(tail), Err(Error::ChoiceFinished)));
        assert!(matches!(aggregate.finish(), Err(Error::InvalidChunk)));
    }
}

#[test]
fn native_finish_metadata_survives_same_shape_but_conversion_stays_explicit() {
    let mut tail = usage_tail(json!({"role":"assistant","content":""}), "stop");
    tail["choices"][0]["native_finish_reason"] = json!("end_turn");
    tail["provider"] = json!("synthetic-provider");
    let mut tracker = ChunkTracker::new(1);
    tracker.push(&one(0, json!({}), json!("stop"))).unwrap();
    tracker.push(&tail).unwrap();
    tracker.finish().unwrap();
    let mut encoded = encode_chunk(&tail, 4096).unwrap();
    encoded.extend_from_slice(DONE);
    let decoded = decode(&encoded, 4096).unwrap();
    let Event::Chunk(output) = &decoded[0] else {
        panic!("expected usage frame")
    };
    assert_eq!(output, &tail);
    tail.as_object_mut().unwrap().remove("provider");
    assert!(matches!(
        Aggregator::new(10_000, 10).push(tail),
        Err(Error::Unsupported)
    ));
}

#[test]
fn accounting_frames_still_consume_aggregation_budgets_and_require_done() {
    let finished = one(0, json!({"role":"assistant"}), json!("stop"));
    let tail = usage_tail(json!({}), "stop");
    let bytes =
        serde_json::to_vec(&finished).unwrap().len() + serde_json::to_vec(&tail).unwrap().len();
    for (max_bytes, max_events, byte_limit) in [(100_000, 1, false), (bytes - 1, 2, true)] {
        let mut aggregate = Aggregator::new(max_bytes, max_events);
        aggregate.push(finished.clone()).unwrap();
        assert!(matches!(
            (aggregate.push(tail.clone()), byte_limit),
            (Err(Error::AggregateBytes), true) | (Err(Error::AggregateEvents), false)
        ));
    }
    let mut bytes = encode_chunk(&finished, 4096).unwrap();
    bytes.extend_from_slice(&encode_chunk(&tail, 4096).unwrap());
    assert!(matches!(decode(&bytes, 4096), Err(Error::Truncated)));
}

#[test]
fn data_error_envelopes_fail_before_chat_validation_or_success_done() {
    let mut unified = one(0, json!({"content":""}), json!("error"));
    unified["error"] = json!({"code":"server_error","message":"synthetic-private-message"});
    for error in [
        unified,
        json!({"error":{"code":500,"message":"synthetic-private-message"}}),
    ] {
        assert!(matches!(validate_chunk(&error), Err(Error::UpstreamError)));
        let bytes = format!("data: {error}\n\ndata: [DONE]\n\n");
        assert!(matches!(
            decode(bytes.as_bytes(), 4096),
            Err(Error::UpstreamError)
        ));
        assert!(matches!(
            ChunkTracker::new(1).push(&error),
            Err(Error::UpstreamError)
        ));
    }
    let mut normal = one(0, json!({}), json!("stop"));
    normal["error"] = Value::Null;
    assert!(validate_chunk(&normal).is_ok());
}
