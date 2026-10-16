use std::fmt::Write as _;

use super::{Report, Value, execute, fixtures, fs, json};

fn chunk(delta: Value, finish: Value) -> Value {
    let mut chunk = json!({"id":"compatibility","object":"chat.completion.chunk","created":1,
        "model":"real-model","choices":[{"index":0}]});
    chunk["choices"][0]["delta"] = delta;
    chunk["choices"][0]["finish_reason"] = finish;
    chunk
}

fn tools() -> Vec<Value> {
    vec![
        chunk(
            json!({"role":"assistant","tool_calls":[{"index":0,"id":"call-1",
            "type":"function","function":{"name":"lookup","arguments":"{"}}]}),
            Value::Null,
        ),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"}"}}]}),
            json!("tool_calls"),
        ),
    ]
}

fn usage_tail() -> Value {
    let mut tail = chunk(
        json!({"role":"assistant","content":""}),
        json!("tool_calls"),
    );
    tail["usage"] = json!({"prompt_tokens":1,"completion_tokens":2,"total_tokens":3});
    tail
}

async fn run_sse(events: &[Value], done: bool, stream: bool, expect: Value) -> Report {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.sse");
    let mut bytes = String::new();
    for event in events {
        writeln!(bytes, "data: {event}\n").unwrap();
    }
    if done {
        bytes.push_str("data: [DONE]\n\n");
    }
    fs::write(input, bytes).unwrap();
    let case = json!({"name":"stream-compatibility",
        "request":{"model":"chat","messages":[{"role":"user","content":"synthetic"}],"stream":stream},
        "upstream":{"status":200,"headers":{"content-type":["text/event-stream"]},"bodyFile":"input.sse"},
        "expect":expect});
    let path = directory.path().join("input.case.json");
    fs::write(&path, serde_json::to_vec(&case).unwrap()).unwrap();
    execute(&fixtures("transport").join("server.toml"), &path)
        .await
        .unwrap()
}

fn assert_passed(report: &Report) {
    assert!(report.passed(), "{}", report.mismatches.join(", "));
}

fn usage(completed: bool) -> Value {
    json!({"inputTokens":1,"outputTokens":2,"totalTokens":3,
        "cachedInputTokens":null,"reasoningOutputTokens":null,"quality":"reported",
        "source":"sse","upstreamCompleted":completed})
}

#[tokio::test(flavor = "current_thread")]
async fn usage_tail_reaches_downstream_and_aggregates_without_changing_tools() {
    let mut events = tools();
    events.push(usage_tail());
    let mut expected_events = events.clone();
    for event in &mut expected_events {
        event["model"] = json!("chat");
    }
    let body = json!({"id":"compatibility","object":"chat.completion","created":1,"model":"chat",
        "choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"call-1",
            "type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}});
    for stream in [false, true] {
        let response = if stream {
            json!({"status":200,"events":expected_events,"done":true})
        } else {
            json!({"status":200,"body":body})
        };
        assert_passed(
            &run_sse(
                &events,
                true,
                stream,
                json!({"sent":true,"response":response,"stats":{"usage":usage(true)}}),
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn usage_tail_keeps_openrouter_metadata_on_the_stream() {
    let mut events = tools();
    let mut tail = usage_tail();
    tail["provider"] = json!("synthetic-provider");
    tail["choices"][0]["native_finish_reason"] = json!("tool_use");
    events.push(tail);
    let mut expected = events.clone();
    for event in &mut expected {
        event["model"] = json!("chat");
    }
    assert_passed(&run_sse(&events, true, true,
        json!({"response":{"status":200,"events":expected,"done":true},"stats":{"usage":usage(true)}})).await);
}

#[tokio::test(flavor = "current_thread")]
async fn usage_tail_cannot_mask_late_content_tool_arguments_or_truncation() {
    let mut invalid = vec![None];
    for delta in [
        json!({"content":"late"}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":"late"}}]}),
    ] {
        let mut tail = usage_tail();
        tail["choices"][0]["delta"] = delta;
        invalid.push(Some(tail));
    }
    for stream in [false, true] {
        for tail in &invalid {
            let mut events = tools();
            events.push(tail.clone().unwrap_or_else(usage_tail));
            let mut response = json!({"status":if stream {200} else {502}});
            if stream {
                let mut expected = tools();
                if tail.is_none() {
                    expected.push(usage_tail());
                }
                for event in &mut expected {
                    event["model"] = json!("chat");
                }
                response["events"] = json!(expected);
                response["done"] = json!(false);
            }
            let mut expect = json!({"response":response,
                "error":{"code":"upstream_protocol","status":502,"phase":"upstream_sse"}});
            if tail.is_none() {
                expect["stats"] = json!({"usage":usage(false)});
            }
            assert_passed(&run_sse(&events, tail.is_some(), stream, expect).await);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn data_error_frames_fail_before_and_after_stream_start() {
    for first in [false, true] {
        for stream in [false, true] {
            let error =
                json!({"error":{"code":"server_error","message":"synthetic-private-error"}});
            let mut unified = chunk(json!({"content":""}), json!("error"));
            unified["error"] = error["error"].clone();
            for error in [error, unified] {
                let mut events = if first {
                    Vec::new()
                } else {
                    vec![chunk(
                        json!({"role":"assistant","content":"hello"}),
                        Value::Null,
                    )]
                };
                let mut response = json!({"status":if first || !stream {502} else {200}});
                if stream && !first {
                    let mut expected = events.clone();
                    for event in &mut expected {
                        event["model"] = json!("chat");
                    }
                    response["events"] = json!(expected);
                    response["done"] = json!(false);
                } else {
                    response["body"] = error.clone();
                }
                events.push(error);
                assert_passed(
                    &run_sse(
                        &events,
                        true,
                        stream,
                        json!({"response":response,
                    "error":{"code":"upstream_protocol","status":502,"phase":"upstream_sse"}}),
                    )
                    .await,
                );
            }
        }
    }
}
