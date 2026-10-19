use limen_chat::Event;
use serde_json::{Value, json};

use super::{Outcome, chunk, config, encoded, run};

fn settings() -> String {
    config(
        "import {syntheticTextTool} from 'limen:content';",
        "{name:'source',onContent(ctx){if(ctx.event.type==='part_delta')ctx.state.inputCalls=(ctx.state.inputCalls||0)+1;}},syntheticTextTool({name:'transport',maxBufferedChars:4096})",
        "{name:'ordered-view',onContent(ctx){if(ctx.event.type==='part_delta'&&ctx.event.text)(ctx.state.seen??=[]).push([ctx.sse.id,ctx.event.text]);},onStreamEvent(ctx){const text=ctx.event.choices?.[0]?.delta?.content;if(text)(ctx.state.streamSeen??=[]).push([ctx.sse.id,text]);}}",
        "",
    )
}

fn framed(events: &[Value], done: bool) -> String {
    use std::fmt::Write as _;

    let mut input = String::new();
    for (index, event) in events.iter().enumerate() {
        writeln!(
            input,
            ": source-{index}\nid: source-{index}\nevent: chunk-{index}\nretry: {}\ndata: {event}\n",
            index + 100,
        )
        .unwrap();
    }
    if done {
        input.push_str("data: [DONE]\n\n");
    }
    input
}

fn text_frames(result: &Outcome) -> Vec<(String, String)> {
    result
        .frames
        .iter()
        .filter_map(|frame| {
            let Event::Chunk(value) = frame.event().unwrap() else {
                return None;
            };
            let text = value["choices"][0]["delta"]["content"].as_str()?;
            (!text.is_empty()).then(|| (frame.id.clone().unwrap(), text.to_owned()))
        })
        .collect()
}

fn assert_text(result: &Outcome, stream: bool, expected: &str, sources: &[(&str, &str)]) {
    assert_eq!(
        result.status, 200,
        "{:?}; {}; {}",
        result.error, result.body, result.state
    );
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.done, stream);
    assert_eq!(result.body["choices"][0]["message"]["content"], expected);
    if stream {
        assert_eq!(result.state["streamSeen"], json!(sources));
        let expected: Vec<_> = sources
            .iter()
            .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
            .collect();
        assert_eq!(text_frames(result), expected);
        let observed: String = result
            .events
            .iter()
            .filter_map(|event| event["choices"][0]["delta"]["content"].as_str())
            .collect();
        assert_eq!(
            observed,
            result.body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_and_synthetic_text_keep_interleaving_and_same_frame_order() {
    let events = [
        chunk(json!({"role":"assistant","content":"A"}), None),
        chunk(
            json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_emit_text","arguments":"{\"text\":\"B"}}]}),
            None,
        ),
        chunk(
            json!({"content":"C","tool_calls":[{"index":0,"function":{"arguments":"D"}}]}),
            None,
        ),
        chunk(
            json!({"content":"E","tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ];
    let input = framed(&events, true);
    for stream in [true, false] {
        let result = run(&settings(), Value::Null, Some(&input), stream).await;
        assert_text(
            &result,
            stream,
            "ABCDE",
            &[
                ("source-0", "A"),
                ("source-1", "B"),
                ("source-2", "CD"),
                ("source-3", "E"),
            ],
        );
        assert_eq!(result.body["choices"][0]["finish_reason"], "stop");
        assert_eq!(result.state["inputCalls"], 6);
        assert_eq!(
            result.state["seen"],
            json!([
                ["source-0", "A"],
                ["source-1", "B"],
                ["source-2", "C"],
                ["source-2", "D"],
                ["source-3", "E"]
            ]),
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pending_tool_releases_original_envelopes_before_later_ordinary_text() {
    let events = [
        chunk(
            json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_","arguments":"{\"text\":\"A"}}]}),
            None,
        ),
        chunk(json!({"content":"B"}), None),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"name":"emit_text","arguments":"\"}"}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ];
    let input = framed(&events, true);
    for stream in [false, true] {
        let result = run(&settings(), Value::Null, Some(&input), stream).await;
        assert_text(
            &result,
            stream,
            "AB",
            &[("source-0", "A"), ("source-1", "B")],
        );
        assert_eq!(result.state["inputCalls"], 3);
        assert_eq!(
            result.state["seen"],
            json!([["source-0", "A"], ["source-1", "B"]])
        );
        if stream {
            for frame in &result.frames {
                let Some(index) = frame
                    .id
                    .as_deref()
                    .and_then(|id| id.strip_prefix("source-"))
                else {
                    continue;
                };
                let index: usize = index.parse().unwrap();
                assert_eq!(frame.event, format!("chunk-{index}"));
                assert_eq!(frame.retry, Some((index + 100).to_string()));
                assert!(
                    String::from_utf8_lossy(frame.raw()).contains(&format!(": source-{index}\n"))
                );
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resolving_later_pending_tool_cannot_overtake_earlier_sources() {
    let events = [
        chunk(
            json!({"tool_calls":[{"index":0,"id":"first","type":"function","function":{"name":"limen_","arguments":"{\"text\":\"A"}}]}),
            None,
        ),
        chunk(
            json!({"content":"B","tool_calls":[{"index":1,"id":"second","type":"function","function":{"name":"limen_","arguments":"{\"text\":\"C"}}]}),
            None,
        ),
        chunk(
            json!({"tool_calls":[{"index":1,"function":{"name":"emit_text","arguments":"\"}"}}]}),
            None,
        ),
        chunk(json!({"content":"D"}), None),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"name":"emit_text","arguments":"\"}"}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ];
    let input = framed(&events, true);
    for stream in [false, true] {
        let result = run(&settings(), Value::Null, Some(&input), stream).await;
        assert_text(
            &result,
            stream,
            "ABCD",
            &[("source-0", "A"), ("source-1", "BC"), ("source-3", "D")],
        );
        assert_eq!(result.state["inputCalls"], 6);
        assert_eq!(
            result.state["seen"],
            json!([
                ["source-0", "A"],
                ["source-1", "B"],
                ["source-1", "C"],
                ["source-3", "D"]
            ])
        );
        assert!(
            result.body["choices"][0]["message"]
                .get("tool_calls")
                .is_none()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn split_escape_and_surrogate_text_keep_their_starting_source() {
    for (first, continuation, old_text, expected) in [
        (r#"{"text":"A\"#, r#"nC"}"#, "A\n", "A\nBC"),
        (r#"{"text":"A\uD8"#, r#"3D\uDE00C"}"#, "A😀", "A😀BC"),
    ] {
        let events = [
            chunk(
                json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_emit_text","arguments":first}}]}),
                None,
            ),
            chunk(json!({"content":"B"}), None),
            chunk(
                json!({"tool_calls":[{"index":0,"function":{"arguments":continuation}}]}),
                None,
            ),
            chunk(json!({}), Some("tool_calls")),
        ];
        let input = framed(&events, true);
        for stream in [false, true] {
            let result = run(&settings(), Value::Null, Some(&input), stream).await;
            assert_text(
                &result,
                stream,
                expected,
                &[("source-0", old_text), ("source-1", "B"), ("source-2", "C")],
            );
            assert_eq!(result.state["inputCalls"], 3);
            let seen = result.state["seen"].as_array().unwrap();
            let sources: Vec<_> = seen
                .iter()
                .map(|event| event[0].as_str().unwrap())
                .collect();
            assert!(sources.windows(2).all(|pair| pair[0] <= pair[1]));
            let observed: String = seen
                .iter()
                .map(|event| event[1].as_str().unwrap())
                .collect();
            assert_eq!(observed, expected);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn truncated_synthetic_character_releases_known_prefix_without_reordering() {
    for finish in ["length", "content_filter"] {
        let events = [
            chunk(
                json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_emit_text","arguments":r#"{"text":"A\uD8"#}}]}),
                None,
            ),
            chunk(json!({"content":"B"}), None),
            chunk(json!({}), Some(finish)),
        ];
        let input = framed(&events, true);
        for stream in [false, true] {
            let result = run(&settings(), Value::Null, Some(&input), stream).await;
            assert_text(
                &result,
                stream,
                "AB",
                &[("source-0", "A"), ("source-1", "B")],
            );
            assert_eq!(result.body["choices"][0]["finish_reason"], finish);
            assert_eq!(
                result.state["seen"],
                json!([["source-0", "A"], ["source-1", "B"]])
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn eof_does_not_flush_unresolved_tool_or_blocked_ordinary_text() {
    let events = [
        chunk(
            json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_","arguments":"{\"text\":\"A"}}]}),
            None,
        ),
        chunk(json!({"content":"B"}), None),
    ];
    let input = encoded(&events, false);
    for stream in [false, true] {
        let result = run(&settings(), Value::Null, Some(&input), stream).await;
        assert!(!result.done);
        assert_eq!(result.error.as_ref().unwrap()["code"], "upstream_protocol");
        assert!(result.state.get("seen").is_none());
        assert!(
            result
                .events
                .iter()
                .all(|event| event["choices"][0]["delta"]["content"].is_null())
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_text_blocked_by_pending_tool_obeys_waiting_character_limit() {
    let settings = config(
        "import {syntheticTextTool} from 'limen:content';",
        "syntheticTextTool({name:'transport',maxBufferedChars:256})",
        "{name:'ordered-view',onContent(ctx){if(ctx.event.type==='part_delta'&&ctx.event.text)ctx.state.leaked=true;}}",
        "",
    );
    let events = [
        chunk(
            json!({"tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_","arguments":"{\"text\":\"A"}}]}),
            None,
        ),
        chunk(json!({"content":"B".repeat(512)}), None),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"name":"emit_text","arguments":"\"}"}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ];
    let input = encoded(&events, true);
    for stream in [false, true] {
        let result = run(&settings, Value::Null, Some(&input), stream).await;
        assert!(!result.done);
        assert_eq!(result.error.as_ref().unwrap()["code"], "hook_failed");
        assert_eq!(result.error.as_ref().unwrap()["phase"], "onContent");
        assert_eq!(result.state, json!({}));
        assert!(
            result
                .events
                .iter()
                .all(|event| event["choices"][0]["delta"]["content"].is_null())
        );
    }
}
