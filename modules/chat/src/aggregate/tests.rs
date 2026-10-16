use serde_json::{Value, json};

use crate::{Aggregator, Error};

fn event(delta: Value, finish: Value) -> Value {
    let mut event = json!({"id":"c", "object":"chat.completion.chunk", "model":"m", "created":1,
        "choices":[{"index":0}]});
    event["choices"][0]["delta"] = delta;
    event["choices"][0]["finish_reason"] = finish;
    event
}

fn aggregate(events: Vec<Value>) -> Value {
    let mut aggregate = Aggregator::new(1_000_000, 100);
    for event in events {
        aggregate.push(event).unwrap();
    }
    aggregate.finish().unwrap()
}

#[test]
fn reasoning_refusal_legacy_and_custom_fields_reduce_fragments() {
    let output = aggregate(vec![
        event(
            json!({"content":null,"refusal":"no ","reasoning":"think ","reasoning_content":"a",
            "function_call":{"name":"leg","arguments":"{"},
            "tool_calls":[{"index":3,"id":"c","type":"custom","custom":{"name":"shell","input":"echo "}}]}),
            Value::Null,
        ),
        event(
            json!({"refusal":"thanks","reasoning":"more","reasoning_content":"b",
            "function_call":{"name":"acy","arguments":"}"},
            "tool_calls":[{"index":3,"custom":{"input":"ok"}}]}),
            json!("stop"),
        ),
    ]);
    assert_eq!(
        output["choices"][0]["message"],
        json!({"role":"assistant","content":null,
        "refusal":"no thanks","reasoning":"think more","reasoning_content":"ab",
        "function_call":{"name":"legacy","arguments":"{}"},
        "tool_calls":[{"id":"c","type":"custom","custom":{"name":"shell","input":"echo ok"}}]})
    );
}

#[test]
fn content_blocks_retain_order_across_string_array_transitions() {
    let output = aggregate(vec![
        event(json!({"content":"before"}), Value::Null),
        event(
            json!({"content":[{"type":"thinking","thinking":[{"type":"text","text":"reason"}],"signature":"opaque"}]}),
            Value::Null,
        ),
        event(json!({"content":"after"}), json!("stop")),
    ]);
    assert_eq!(
        output["choices"][0]["message"]["content"],
        json!([
            {"type":"text","text":"before"},
            {"type":"thinking","thinking":[{"type":"text","text":"reason"}],"signature":"opaque"},
            {"type":"text","text":"after"}
        ])
    );
}

#[test]
fn reasoning_details_merge_identified_text_but_keep_opaque_payloads() {
    let output = aggregate(vec![
        event(
            json!({"reasoning_details":[{"type":"reasoning.text","id":"r","index":0,"text":"a"},
            {"type":"reasoning.encrypted","id":"e","data":"opaque","format":"v1"}]}),
            Value::Null,
        ),
        event(
            json!({"reasoning_details":[{"type":"reasoning.text","id":"r","text":"b"},
            {"type":"reasoning.summary","index":2,"summary":"short"}]}),
            json!("stop"),
        ),
    ]);
    assert_eq!(
        output["choices"][0]["message"]["reasoning_details"],
        json!([
            {"type":"reasoning.text","id":"r","index":0,"text":"ab"},
            {"type":"reasoning.encrypted","id":"e","data":"opaque","format":"v1"},
            {"type":"reasoning.summary","index":2,"summary":"short"}
        ])
    );
}

#[test]
fn logprobs_append_and_audio_expiry_updates_after_completion() {
    let mut first = event(
        json!({"audio":{"id":"a","data":"YWJj","transcript":"a"}}),
        Value::Null,
    );
    first["choices"][0]["logprobs"] =
        json!({"content":[{"token":"a","logprob":-0.1}],"refusal":null});
    let mut last = event(
        json!({"audio":{"data":"ZGVm","transcript":"b"}}),
        json!("stop"),
    );
    last["choices"][0]["logprobs"] = json!({"content":[{"token":"b","logprob":-0.2}],"refusal":[]});
    let tail = json!({"object":"", "id":"", "model":"", "created":0,
        "choices":[{"index":0,"delta":{"audio":{"expires_at":9}},"annotations":[{"url":"source"}],"content_filter_offsets":{"start_offset":0,"end_offset":2}}]});
    let output = aggregate(vec![first, last, tail]);
    assert_eq!(output["id"], "c");
    assert_eq!(
        output["choices"][0]["message"]["audio"],
        json!({"id":"a","data":"YWJjZGVm","transcript":"ab","expires_at":9})
    );
    assert_eq!(
        output["choices"][0]["logprobs"]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(output["choices"][0]["logprobs"]["refusal"], json!([]));
    assert_eq!(
        output["choices"][0]["annotations"],
        json!([{"url":"source"}])
    );
}

#[test]
fn accumulated_usage_is_latest_non_null_snapshot_and_transport_noise_is_removed() {
    let mut first = event(json!({"content":"a","obfuscation":"delta"}), Value::Null);
    first["usage"] = json!({"completion_tokens":1});
    first["obfuscation"] = json!("root");
    first["choices"][0]["obfuscation"] = json!("choice");
    let mut last = event(json!({"content":"b"}), json!("stop"));
    last["usage"] = json!({"completion_tokens":2,"total_tokens":5});
    let output = aggregate(vec![first, last, json!({"usage":null,"choices":[]})]);
    assert_eq!(
        output["usage"],
        json!({"completion_tokens":2,"total_tokens":5})
    );
    assert!(output.get("obfuscation").is_none());
    assert!(output["choices"][0].get("obfuscation").is_none());
    assert!(output["choices"][0]["message"].get("obfuscation").is_none());
}

#[test]
fn null_and_empty_content_are_distinct_and_null_does_not_erase_fragments() {
    for (values, expected) in [
        (vec![Value::Null], Value::Null),
        (vec![json!("")], json!("")),
        (vec![json!([])], json!([])),
        (vec![json!("a"), Value::Null], json!("a")),
    ] {
        let events = values
            .into_iter()
            .map(|content| event(json!({"content":content}), Value::Null));
        let output = aggregate(events.chain([event(json!({}), json!("stop"))]).collect());
        assert_eq!(output["choices"][0]["message"]["content"], expected);
    }
}

#[test]
fn unsupported_nested_extensions_and_conflicts_fail_explicitly() {
    for delta in [
        json!({"mystery":1}),
        json!({"content":[{"type":"unknown","data":"x"}]}),
        json!({"audio":{"unknown":"x"}}),
        json!({"reasoning_details":[{"type":"reasoning.text","unknown":true}]}),
    ] {
        assert!(matches!(
            Aggregator::new(100_000, 10).push(event(delta, json!("stop"))),
            Err(Error::Unsupported)
        ));
    }
    let mut aggregate = Aggregator::new(100_000, 10);
    aggregate
        .push(event(json!({"role":"assistant"}), Value::Null))
        .unwrap();
    let mut conflict = event(json!({}), json!("stop"));
    conflict["model"] = json!("other");
    assert!(matches!(
        aggregate.push(conflict),
        Err(Error::ConflictingMetadata)
    ));
}

#[test]
fn usage_snapshots_preserve_partial_detail_fields_and_ignore_null_updates() {
    let mut first = event(json!({"content":"a"}), Value::Null);
    first["usage"] = json!({"prompt_tokens":4,"completion_tokens":1,
        "prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}});
    let mut last = event(json!({}), json!("stop"));
    last["usage"] = json!({"prompt_tokens":null,"completion_tokens":3,
        "prompt_tokens_details":{"cached_tokens":null,"audio_tokens":1},"completion_tokens_details":null});
    let output = aggregate(vec![first, last]);
    assert_eq!(
        output["usage"],
        json!({"prompt_tokens":4,"completion_tokens":3,
        "prompt_tokens_details":{"cached_tokens":2,"audio_tokens":1},"completion_tokens_details":{"reasoning_tokens":1}})
    );
}

#[test]
fn distinct_scoped_filter_snapshots_require_explicit_cross_shape_adaptation() {
    for key in ["content_filter_offsets", "content_filter_results"] {
        let mut aggregate = Aggregator::new(100_000, 10);
        aggregate
            .push(event(json!({"content":"answer"}), json!("stop")))
            .unwrap();
        let mut first = json!({"choices":[{"index":0}]});
        let mut second = first.clone();
        first["choices"][0][key] = if key == "content_filter_offsets" {
            json!({"start_offset":0,"end_offset":3,"check_offset":3})
        } else {
            json!({"hate":{"filtered":false,"severity":"safe"}})
        };
        second["choices"][0][key] = if key == "content_filter_offsets" {
            json!({"start_offset":3,"end_offset":6,"check_offset":6})
        } else {
            json!({"hate":{"filtered":false,"severity":"low"}})
        };
        aggregate.push(first).unwrap();
        assert!(matches!(aggregate.push(second), Err(Error::Unsupported)));
    }
}

#[test]
fn audio_combines_independent_padded_and_unpadded_continuation_fragments() {
    for (fragments, expected) in [
        (vec!["YQ==", "Yg=="], "YWI="),
        (vec!["YW", "Jj"], "YWJj"),
        (vec!["YQ", "=", "=", "Y", "g=="], "YWI="),
    ] {
        let events = fragments
            .into_iter()
            .map(|data| event(json!({"audio":{"data":data}}), Value::Null));
        let output = aggregate(events.chain([event(json!({}), json!("stop"))]).collect());
        assert_eq!(output["choices"][0]["message"]["audio"]["data"], expected);
    }
    let mut aggregate = Aggregator::new(100_000, 10);
    aggregate
        .push(event(json!({"audio":{"data":"invalid?"}}), json!("stop")))
        .unwrap();
    assert!(matches!(aggregate.finish(), Err(Error::Audio(_))));
}
