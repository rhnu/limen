use std::error::Error as _;

use serde_json::{Value, json};

use crate::{
    Aggregator, ChunkTracker, DONE, Decoder, Error, Event, JsonChunks, encode_chunk, encode_error,
    validate_chunk,
};

mod compatibility;

fn chunk(choices: Value) -> Value {
    let mut value = json!({"id":"c1", "object":"chat.completion.chunk", "created":1,
        "model":"alias"});
    value["choices"] = choices;
    value
}

fn one(index: u64, delta: Value, finish: Value) -> Value {
    let mut choice = json!({"index":index});
    choice["delta"] = delta;
    choice["finish_reason"] = finish;
    chunk(Value::Array(vec![choice]))
}

fn response(message: Value) -> Value {
    let mut value = json!({"id":"c1", "object":"chat.completion", "created":1,
        "model":"explicit-hook-model", "choices":[{"index":0, "finish_reason":"stop"}]});
    value["choices"][0]["message"] = message;
    value
}

fn decode(input: &[u8], limit: usize) -> Result<Vec<Event>, Error> {
    let mut decoder = Decoder::new(limit);
    let mut events = Vec::new();
    for &byte in input {
        if let Some(event) = decoder.push(byte)? {
            events.push(event);
        }
    }
    decoder.finish()?;
    Ok(events)
}

fn nullable_tool_chunks() -> Vec<Value> {
    vec![
        one(
            0,
            json!({"role":"assistant","tool_calls":[
                {"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":"{"}}
            ]}),
            Value::Null,
        ),
        one(
            0,
            json!({"tool_calls":[
                {"index":0,"id":null,"type":null,"function":{"name":null,"arguments":null}}
            ]}),
            Value::Null,
        ),
        one(
            0,
            json!({"tool_calls":[
                {"index":0,"id":null,"type":null,"function":{"name":null,"arguments":"\"city\":\"Paris\"}"}}
            ]}),
            Value::Null,
        ),
        one(0, json!({}), json!("tool_calls")),
    ]
}

#[test]
fn nullable_tool_deltas_survive_decoding_encoding_and_completion_tracking() {
    let chunks = nullable_tool_chunks();
    let mut input = Vec::new();
    for chunk in &chunks {
        input.extend_from_slice(format!("data: {chunk}\n\n").as_bytes());
    }
    input.extend_from_slice(DONE);
    let decoded = decode(&input, 4096).unwrap();
    assert_eq!(decoded.len(), chunks.len() + 1);
    let mut tracker = ChunkTracker::new(1);
    let mut encoded = Vec::new();
    for (event, expected) in decoded.iter().zip(&chunks) {
        let Event::Chunk(event) = event else {
            panic!("expected tool delta before DONE")
        };
        assert_eq!(event, expected);
        tracker.push(event).unwrap();
        encoded.extend_from_slice(&encode_chunk(event, 4096).unwrap());
    }
    assert!(matches!(decoded.last(), Some(Event::Done)));
    tracker.finish().unwrap();
    encoded.extend_from_slice(DONE);
    assert_eq!(encoded, input);
}

#[test]
fn aggregation_ignores_null_tool_fragments_without_erasing_identifiers() {
    let mut aggregate = Aggregator::new(100_000, 10);
    for chunk in nullable_tool_chunks() {
        aggregate.push(chunk).unwrap();
    }
    let output = aggregate.finish().unwrap();
    assert_eq!(
        output["choices"][0]["message"]["tool_calls"],
        json!([
            {"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}}
        ])
    );
}

#[test]
fn null_tool_fields_do_not_satisfy_complete_tool_requirements() {
    for pointer in ["/id", "/type", "/function/name", "/function/arguments"] {
        let mut tool = json!({"index":0,"id":"call_1","type":"function",
            "function":{"name":"weather","arguments":"{}"}});
        *tool.pointer_mut(pointer).unwrap() = Value::Null;
        let mut aggregate = Aggregator::new(10_000, 10);
        aggregate
            .push(one(
                0,
                json!({"role":"assistant","tool_calls":[tool]}),
                json!("tool_calls"),
            ))
            .unwrap();
        assert!(
            aggregate.finish().is_err(),
            "required tool field: {pointer}"
        );
    }
}

#[test]
fn tool_deltas_still_reject_non_strings_and_unsupported_conversion_types() {
    for pointer in ["/id", "/type", "/function/name", "/function/arguments"] {
        let mut tool = json!({"index":0,"id":"call_1","type":"function",
            "function":{"name":"weather","arguments":"{}"}});
        *tool.pointer_mut(pointer).unwrap() = json!(42);
        let chunk = one(0, json!({"tool_calls":[tool]}), Value::Null);
        assert!(
            matches!(validate_chunk(&chunk), Err(Error::InvalidChunk)),
            "invalid tool field: {pointer}"
        );
    }
    let chunk = one(
        0,
        json!({"tool_calls":[{"index":0,"type":"unsupported"}]}),
        Value::Null,
    );
    assert!(matches!(
        Aggregator::new(10_000, 10).push(chunk),
        Err(Error::Unsupported)
    ));
}

#[test]
fn decoder_accepts_utf8_bom_comments_multiline_and_all_line_endings() {
    let value = one(
        0,
        json!({"role":"assistant", "content":"世界🌍"}),
        json!("stop"),
    );
    for newline in ["\n", "\r", "\r\n"] {
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        let mut input = String::from("\u{feff}: heartbeat");
        input.push_str(newline);
        input.push_str(newline);
        input.push_str("event: ignored-without-data");
        input.push_str(newline);
        input.push_str(newline);
        for line in pretty.lines() {
            input.push_str("data: ");
            input.push_str(line);
            input.push_str(newline);
        }
        input.push_str(newline);
        input.push_str("data: [DONE]");
        input.push_str(newline);
        input.push_str(newline);
        let events = decode(input.as_bytes(), 4096).unwrap();
        assert_eq!(events.len(), 2);
        let Event::Chunk(decoded) = &events[0] else {
            panic!("expected JSON chunk")
        };
        assert_eq!(decoded, &value);
        assert!(matches!(events[1], Event::Done));
    }
}

#[test]
fn decoder_requires_delimited_done_even_after_finish_reason() {
    let value = one(0, json!({"role":"assistant"}), json!("stop"));
    let mut bytes = encode_chunk(&value, 1024).unwrap();
    assert!(matches!(decode(&bytes, 1024), Err(Error::Truncated)));
    bytes.extend_from_slice(b"data: [DONE]\n");
    assert!(matches!(decode(&bytes, 1024), Err(Error::Truncated)));
    bytes.push(b'\n');
    assert!(decode(&bytes, 1024).is_ok());
}

#[test]
fn decoder_rejects_oversized_events_and_unterminated_lines() {
    assert!(matches!(
        decode(b"data: 012345678901234567890", 12),
        Err(Error::EventLimit)
    ));
    assert!(matches!(
        decode(b": a\n: b\n: c\n: d\n\n", 12),
        Err(Error::EventLimit)
    ));
}

#[test]
fn decoder_does_not_hide_invalid_json_or_named_error_events() {
    let error = decode(b"data: {\n\n", 100).err().unwrap();
    assert!(matches!(error, Error::Json(_)));
    assert!(error.source().unwrap().is::<serde_json::Error>());
    assert!(matches!(
        decode(b"event: error\ndata: {}\n\n", 100),
        Err(Error::UpstreamError)
    ));
    assert!(matches!(
        decode(b"event: custom\ndata: {}\n\n", 100),
        Err(Error::InvalidChunk)
    ));
}

#[test]
fn decoder_rejects_data_after_done() {
    assert!(matches!(
        decode(b"data: [DONE]\n\ndata: [DONE]\n\n", 100),
        Err(Error::AfterDone)
    ));
}

#[test]
fn same_shape_extensions_survive_decode_and_encoding() {
    let mut value = one(
        0,
        json!({"role":"assistant", "vendor_audio":{"bytes":"extension"}}),
        json!("stop"),
    );
    value["vendor"] = json!({"key":[1,2,3]});
    let mut encoded = encode_chunk(&value, 4096).unwrap();
    encoded.extend_from_slice(DONE);
    let decoded = decode(&encoded, 4096).unwrap();
    let Event::Chunk(decoded) = &decoded[0] else {
        panic!("expected chunk")
    };
    assert_eq!(decoded, &value);
}

#[test]
fn chunk_validation_checks_indices_and_known_field_types() {
    let mut value = one(0, json!({"content":"a"}), Value::Null);
    value["choices"][0]["index"] = json!(-1);
    assert!(matches!(validate_chunk(&value), Err(Error::InvalidChunk)));
    let duplicate = json!({"index":0, "delta":{}, "finish_reason":null});
    assert!(validate_chunk(&chunk(json!([duplicate, duplicate]))).is_err());
    assert!(validate_chunk(&one(0, json!({"content":42}), Value::Null)).is_err());
    assert!(
        validate_chunk(&one(
            0,
            json!({"tool_calls":[{"index":0,"function":{"arguments":{}}}]}),
            Value::Null
        ))
        .is_err()
    );
}

#[test]
fn tracker_waits_for_all_choices_but_allows_usage_tail() {
    let mut tracker = ChunkTracker::new(2);
    tracker
        .push(&one(3, json!({"content":"a"}), Value::Null))
        .unwrap();
    tracker
        .push(&one(1, json!({"content":"b"}), json!("stop")))
        .unwrap();
    tracker.push(&one(3, json!({}), json!("length"))).unwrap();
    let mut usage = chunk(json!([]));
    usage["usage"] = json!({"completion_tokens":2});
    tracker.push(&usage).unwrap();
    tracker.finish().unwrap();
}

#[test]
fn tracker_rejects_unfinished_duplicate_finished_and_unbounded_choices() {
    let mut tracker = ChunkTracker::new(1);
    tracker.push(&one(0, json!({}), Value::Null)).unwrap();
    assert!(matches!(tracker.finish(), Err(Error::UnfinishedChoices)));
    let mut tracker = ChunkTracker::new(1);
    tracker.push(&one(0, json!({}), json!("stop"))).unwrap();
    assert!(matches!(
        tracker.push(&one(0, json!({}), Value::Null)),
        Err(Error::ChoiceFinished)
    ));
    assert!(matches!(
        tracker.push(&one(1, json!({}), Value::Null)),
        Err(Error::ChoiceLimit)
    ));
    assert!(matches!(
        ChunkTracker::new(1).finish(),
        Err(Error::UnfinishedChoices)
    ));
}

#[test]
fn aggregation_merges_tools_by_choice_and_call_index_and_keeps_usage() {
    let mut aggregate = Aggregator::new(100_000, 20);
    aggregate.push(one(2, json!({"role":"assistant", "content":"hello ", "tool_calls":[
        {"index":1,"id":"call_","type":"function","function":{"name":"lo","arguments":"{\""}},
        {"index":0,"id":"other","type":"function","function":{"name":"other","arguments":"{}"}}
    ]}), Value::Null)).unwrap();
    aggregate
        .push(one(
            0,
            json!({"role":"assistant","refusal":"cannot "}),
            Value::Null,
        ))
        .unwrap();
    aggregate
        .push(one(
            2,
            json!({"content":"world", "tool_calls":[
                {"index":1,"id":"x","function":{"name":"ok","arguments":"x\":1}"}}
            ]}),
            json!("tool_calls"),
        ))
        .unwrap();
    aggregate
        .push(one(0, json!({"refusal":"comply"}), json!("stop")))
        .unwrap();
    let mut usage = chunk(json!([]));
    usage["usage"] = json!({"prompt_tokens":2,"completion_tokens":3,"vendor_details":{"cache":1}});
    aggregate.push(usage.clone()).unwrap();
    let output = aggregate.finish().unwrap();
    assert_eq!(output["choices"][0]["index"], 0);
    assert_eq!(output["choices"][0]["message"]["refusal"], "cannot comply");
    let message = &output["choices"][1]["message"];
    assert_eq!(message["content"], "hello world");
    assert_eq!(message["tool_calls"][0]["id"], "other");
    assert_eq!(message["tool_calls"][1]["id"], "call_x");
    assert_eq!(
        message["tool_calls"][1]["function"],
        json!({"name":"look","arguments":"{\"x\":1}"})
    );
    assert_eq!(output["usage"], usage["usage"]);
}

#[test]
fn aggregation_has_event_and_cumulative_byte_limits() {
    let value = one(0, json!({"role":"assistant"}), json!("stop"));
    let bytes = serde_json::to_vec(&value).unwrap().len();
    let mut aggregate = Aggregator::new(bytes - 1, 10);
    assert!(matches!(
        aggregate.push(value.clone()),
        Err(Error::AggregateBytes)
    ));
    let mut aggregate = Aggregator::new(100_000, 1);
    aggregate.push(value.clone()).unwrap();
    assert!(matches!(
        aggregate.push(chunk(json!([]))),
        Err(Error::AggregateEvents)
    ));
    let mut aggregate = Aggregator::new(bytes + 1, 10);
    aggregate.push(value).unwrap();
    assert!(matches!(
        aggregate.push(chunk(json!([]))),
        Err(Error::AggregateBytes)
    ));
}

#[test]
fn one_aggregate_event_can_complete_multiple_choices() {
    let mut aggregate = Aggregator::new(10_000, 1);
    aggregate
        .push(chunk(json!([
            {"index":1,"delta":{"role":"assistant","content":"b"},"finish_reason":"stop"},
            {"index":0,"delta":{"role":"assistant","content":"a"},"finish_reason":"length"}
        ])))
        .unwrap();
    let response = aggregate.finish().unwrap();
    assert_eq!(response["choices"].as_array().unwrap().len(), 2);
    assert_eq!(response["choices"][0]["message"]["content"], "a");
    assert_eq!(response["choices"][1]["message"]["content"], "b");
}

#[test]
fn aggregation_rejects_conflicting_metadata_and_incomplete_tool_calls() {
    let mut aggregate = Aggregator::new(100_000, 10);
    aggregate
        .push(one(0, json!({"role":"assistant"}), Value::Null))
        .unwrap();
    let mut other = one(0, json!({}), json!("stop"));
    other["id"] = json!("other");
    assert!(matches!(
        aggregate.push(other),
        Err(Error::ConflictingMetadata)
    ));
    let mut aggregate = Aggregator::new(100_000, 10);
    aggregate
        .push(one(
            0,
            json!({"role":"assistant","tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}),
            json!("tool_calls"),
        ))
        .unwrap();
    assert!(aggregate.finish().is_err());
}

#[test]
fn cross_shape_conversion_rejects_extensions_instead_of_dropping_them() {
    let mut value = one(0, json!({"role":"assistant","content":"a"}), json!("stop"));
    value["extension"] = json!(true);
    assert!(matches!(
        Aggregator::new(100_000, 10).push(value),
        Err(Error::Unsupported)
    ));
    let full = response(json!({"role":"assistant","content":"a","audio":{}}));
    assert!(matches!(
        JsonChunks::new(full, 4096),
        Err(Error::Unsupported)
    ));
    let mut full = response(json!({"role":"assistant","content":"a"}));
    full["choices"][0]["logprobs"] = json!({"content":[]});
    assert!(matches!(
        JsonChunks::new(full, 4096),
        Err(Error::Unsupported)
    ));
}

#[test]
fn json_chunks_roundtrip_refusal_nulls_empty_tools_and_explicit_model() {
    let mut full = response(
        json!({"role":"assistant","content":null,"refusal":"cannot help","tool_calls":[]}),
    );
    full["choices"][0]["logprobs"] = Value::Null;
    full["usage"] = json!({"total_tokens":2,"details":{"extra":1}});
    full["system_fingerprint"] = Value::Null;
    let mut aggregate = Aggregator::new(100_000, 100);
    for value in JsonChunks::new(full.clone(), 4096).unwrap() {
        let value = value.unwrap();
        assert_eq!(value["model"], "explicit-hook-model");
        aggregate.push(value).unwrap();
    }
    assert_eq!(aggregate.finish().unwrap(), full);
}

#[test]
fn json_chunks_split_long_unicode_and_function_fragments_with_bounded_encoding() {
    let long = "世🌍\"\\".repeat(300);
    let full = response(json!({"role":"assistant","content":long,"tool_calls":[
        {"id":long,"type":"function","function":{"name":long,"arguments":long}}
    ]}));
    let mut aggregate = Aggregator::new(1_000_000, 10_000);
    let mut count = 0;
    for value in JsonChunks::new(full.clone(), 320).unwrap() {
        let value = value.unwrap();
        assert!(encode_chunk(&value, 320).unwrap().len() <= 320);
        aggregate.push(value).unwrap();
        count += 1;
    }
    assert!(count > 10);
    assert_eq!(aggregate.finish().unwrap(), full);
}

#[test]
fn too_small_json_chunk_limit_fails_once_and_terminates_iteration() {
    let mut chunks =
        JsonChunks::new(response(json!({"role":"assistant","content":"a"})), 10).unwrap();
    assert!(matches!(chunks.next(), Some(Err(Error::EventLimit))));
    assert!(chunks.next().is_none());
}

#[test]
fn error_frames_are_separate_and_never_include_success_done() {
    let error = json!({"error":{"code":"upstream_protocol","message":"upstream stream failed","request_id":"r1"}});
    let bytes = encode_error(&error, 1024).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("event: error\ndata: "));
    assert!(!text.contains("[DONE]"));
    assert!(matches!(encode_error(&error, 10), Err(Error::EventLimit)));
}

#[test]
fn bounded_encoders_count_utf8_escapes_and_sse_framing() {
    let source =
        r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"世\n","choices":[]}"#;
    let value = serde_json::from_str(source).unwrap();
    let frame_size = source.len() + b"data: \n\n".len();
    assert_eq!(encode_chunk(&value, frame_size).unwrap().len(), frame_size);
    assert!(matches!(
        encode_chunk(&value, frame_size - 1),
        Err(Error::EventLimit)
    ));
    assert!(matches!(
        encode_chunk(&value, source.len()),
        Err(Error::EventLimit)
    ));

    let error = json!({"error":"世\n"});
    let expected = "event: error\ndata: {\"error\":\"世\\n\"}\n\n".as_bytes();
    assert_eq!(encode_error(&error, expected.len()).unwrap(), expected);
    assert!(matches!(
        encode_error(&error, expected.len() - 1),
        Err(Error::EventLimit)
    ));
}

#[test]
fn aggregation_rejects_empty_unfinished_and_duplicate_choices() {
    assert!(matches!(
        Aggregator::new(4096, 10).finish(),
        Err(Error::UnfinishedChoices)
    ));
    let mut aggregate = Aggregator::new(4096, 10);
    aggregate.push(chunk(json!([]))).unwrap();
    assert!(matches!(aggregate.finish(), Err(Error::UnfinishedChoices)));

    let mut aggregate = Aggregator::new(4096, 10);
    aggregate
        .push(chunk(json!([
            {"index":0,"delta":{"role":"assistant"},"finish_reason":"stop"},
            {"index":1,"delta":{"role":"assistant"},"finish_reason":null}
        ])))
        .unwrap();
    assert!(matches!(aggregate.finish(), Err(Error::UnfinishedChoices)));

    let duplicate = json!({"index":0,"delta":{"role":"assistant"},"finish_reason":null});
    let mut aggregate = Aggregator::new(4096, 10);
    assert!(matches!(
        aggregate.push(chunk(json!([duplicate, duplicate]))),
        Err(Error::InvalidChunk)
    ));
    assert!(matches!(
        aggregate.push(chunk(json!([]))),
        Err(Error::InvalidChunk)
    ));
}

#[test]
fn aggregation_uses_choice_finish_before_conflicting_data_and_keeps_failure_terminal() {
    let mut aggregate = Aggregator::new(4096, 10);
    aggregate
        .push(one(0, json!({"role":"assistant"}), json!("stop")))
        .unwrap();
    let mut after_finish = one(0, json!({"content":"late"}), Value::Null);
    after_finish["id"] = json!("conflicting-id");
    assert!(matches!(
        aggregate.push(after_finish),
        Err(Error::ChoiceFinished)
    ));
    assert!(matches!(
        aggregate.push(chunk(json!([]))),
        Err(Error::InvalidChunk)
    ));
    assert!(matches!(aggregate.finish(), Err(Error::InvalidChunk)));
}
