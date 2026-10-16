use serde_json::{Value, json};

use crate::{Aggregator, Error, JsonChunks, encode_chunk};

fn response(message: Value) -> Value {
    let mut response = json!({"id":"c","object":"chat.completion","model":"m","created":1,
        "choices":[{"index":0,"finish_reason":"stop"}]});
    response["choices"][0]["message"] = message;
    response
}

fn roundtrip(source: Value, limit: usize) -> Value {
    let mut aggregate = Aggregator::new(10_000_000, 10_000);
    for chunk in JsonChunks::new(source, limit).unwrap() {
        let chunk = chunk.unwrap();
        assert!(encode_chunk(&chunk, limit).unwrap().len() <= limit);
        aggregate.push(chunk).unwrap();
    }
    aggregate.finish().unwrap()
}

#[test]
fn supported_fields_roundtrip_without_inventing_token_timing() {
    let mut source = response(json!({"role":"assistant","content":[
        {"type":"thinking","thinking":[{"type":"text","text":"reason"}],"signature":"opaque"},
        {"type":"text","text":"answer"}],"refusal":null,
        "reasoning":"first","reasoning_content":"second",
        "reasoning_details":[{"type":"reasoning.text","index":0,"id":"r","text":"detail"},
            {"type":"reasoning.encrypted","id":"e","data":"opaque"}],
        "function_call":{"name":"legacy","arguments":"{}"},
        "tool_calls":[{"id":"custom","type":"custom","custom":{"name":"shell","input":"echo hi"}}],
        "audio":{"id":"a","data":"YWJj","transcript":"abc","expires_at":4},
        "annotations":[{"url":"source"}]}));
    source["choices"][0]["logprobs"] =
        json!({"content":[{"token":"answer","logprob":-0.2}],"refusal":[]});
    source["choices"][0]["native_finish_reason"] = json!("STOP");
    source["prompt_annotations"] =
        json!([{"prompt_index":0,"content_filter_results":{"hate":{"filtered":false}}}]);
    source["provider"] = json!("provider-id");
    source["usage"] = json!({"completion_tokens":2});
    assert_eq!(roundtrip(source.clone(), 4096), source);
}

#[test]
fn small_events_split_text_tools_audio_and_token_arrays_without_repetition() {
    let text = "世🌍".repeat(100);
    let mut source = response(json!({"role":"assistant","content":text,
        "reasoning":text,"reasoning_content":text,
        "function_call":{"name":"legacy","arguments":text},
        "tool_calls":[{"id":"c","type":"custom","custom":{"name":"shell","input":text}}],
        "audio":{"id":"a","data":"YWJj".repeat(200),"transcript":text,"expires_at":4},
        "reasoning_details":[{"type":"reasoning.text","id":"r","text":text,"summary":text,"signature":"opaque"}]}));
    source["choices"][0]["logprobs"] = json!({"content":vec![json!({"token":"a","logprob":-0.1});50],
        "refusal":vec![json!({"token":"b","logprob":-0.2});50]});
    assert_eq!(roundtrip(source.clone(), 320), source);
}

#[test]
fn arrays_split_between_semantic_items_and_annotations_emit_once() {
    let mut source = response(
        json!({"role":"assistant","content":vec![json!({"type":"text","text":"segment"});30],
        "annotations":vec![json!({"url":"https://example.com"});30]}),
    );
    source["annotations"] = json!(vec![json!({"source":1}); 30]);
    source["choices"][0]["annotations"] = json!(vec![json!({"choice":1}); 30]);
    assert_eq!(roundtrip(source.clone(), 320), source);
}

#[test]
fn indivisible_payload_and_logprob_token_failure_is_terminal() {
    for mut source in [
        response(json!({"role":"assistant","audio":{"data":"YWJj"}})),
        response(
            json!({"role":"assistant","reasoning_details":[{"type":"reasoning.encrypted","data":"x".repeat(500)}]}),
        ),
    ] {
        source["choices"][0]["logprobs"] = json!({"content":[{"token":"x".repeat(500)}]});
        let mut chunks = JsonChunks::new(source, 240).unwrap();
        let mut failure = false;
        for chunk in chunks.by_ref() {
            if let Err(error) = chunk {
                assert!(matches!(error, Error::EventLimit));
                failure = true;
            }
        }
        assert!(failure);
        assert!(chunks.next().is_none());
    }
}

#[test]
fn unknown_response_choice_message_and_nested_fields_are_rejected() {
    for pointer in [
        "/unknown",
        "/choices/0/unknown",
        "/choices/0/message/unknown",
    ] {
        let mut source = response(json!({"role":"assistant","content":"a"}));
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let parent = if parent.is_empty() {
            &mut source
        } else {
            source.pointer_mut(parent).unwrap()
        };
        parent[key] = json!(true);
        assert!(matches!(
            JsonChunks::new(source, 4096),
            Err(Error::Unsupported)
        ));
    }
}

#[test]
fn simple_chat_events_keep_role_content_tools_and_terminal_order() {
    let mut source = response(json!({"role":"assistant","content":"hello","tool_calls":[
        {"id":"call","type":"function","function":{"name":"run","arguments":"{}"}}]}));
    source["choices"][0]["logprobs"] = Value::Null;
    let chunks: Vec<_> = JsonChunks::new(source, 4096)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let choices: Vec<_> = chunks
        .iter()
        .map(|chunk| chunk["choices"][0].clone())
        .collect();
    assert_eq!(choices,json!([
        {"index":0,"delta":{"role":"assistant"},"finish_reason":null},
        {"index":0,"delta":{"content":"hello"},"finish_reason":null},
        {"index":0,"delta":{"tool_calls":[{"index":0,"id":"call"}]},"finish_reason":null},
        {"index":0,"delta":{"tool_calls":[{"index":0,"type":"function"}]},"finish_reason":null},
        {"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"run"}}]},"finish_reason":null},
        {"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]},"finish_reason":null},
        {"index":0,"delta":{},"finish_reason":"stop","logprobs":null}
    ]).as_array().unwrap().clone());
}

#[test]
fn fixed_root_metadata_stays_on_each_chat_event_without_an_extra_event() {
    let mut source = response(json!({"role":"assistant","content":"hello"}));
    source["system_fingerprint"] = json!("fingerprint");
    source["service_tier"] = json!("default");
    source["usage"] = json!({"total_tokens":2});
    let chunks: Vec<_> = JsonChunks::new(source, 4096)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(chunks.len(), 4);
    for chunk in &chunks {
        assert_eq!(chunk["system_fingerprint"], "fingerprint");
        assert_eq!(chunk["service_tier"], "default");
    }
    assert_eq!(chunks[3]["usage"], json!({"total_tokens":2}));
}
