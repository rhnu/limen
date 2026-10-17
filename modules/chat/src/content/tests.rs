use super::*;

fn text_delta(events: &mut [Value], replacement: &str) {
    let delta = events
        .iter_mut()
        .find(|event| event["type"] == "part_delta")
        .unwrap();
    delta["text"] = json!(replacement);
}

#[test]
fn complete_json_keeps_opaque_fields_without_conversion() {
    let original = json!({"vendor":{"opaque":true},"choices":[{"index":0,
        "message":{"role":"assistant","content":"hello","audio":{"id":"a"},"vendor":{"x":1}},
        "finish_reason":"stop","vendor":[1,2]}]});
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_json(original.clone()).unwrap();
    let events = frame.events().to_vec();
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["part_start", "part_delta", "part_end", "choice_end"]
    );
    assert_eq!(events[2]["finishReason"], "stop");
    assert_eq!(projector.render(frame, events).unwrap(), original);
    let end = projector.project_end().unwrap();
    let events = end.events().to_vec();
    assert!(projector.render_end(end, events).unwrap().is_none());
}

#[test]
fn rewritten_blocks_keep_order_unknown_blocks_and_residuals() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_json(json!({"vendor":"root","choices":[{"index":0,"message":{
        "content":[{"type":"text","text":"a","vendor":3},
            {"type":"image","url":"opaque"},{"type":"text","text":"b"}],
        "audio":{"id":"voice"}},"finish_reason":"stop"}]}))
        .unwrap();
    let mut events = frame.events().to_vec();
    text_delta(&mut events, "A");
    let output = projector.render(frame, events).unwrap();
    assert_eq!(
        output["choices"][0]["message"]["content"],
        json!([
        {"type":"text","text":"A","vendor":3},{"type":"image","url":"opaque"},{"type":"text","text":"b"}])
    );
    assert_eq!(
        output["choices"][0]["message"]["audio"],
        json!({"id":"voice"})
    );
    assert_eq!(output["vendor"], "root");
}

#[test]
fn tools_can_be_removed_entirely_and_replaced_by_text() {
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_json(json!({"choices":[{"index":0,"message":{"role":"assistant",
        "tool_calls":[{"id":"call","type":"function","function":{"name":"tool","arguments":"{unfinished"}}]},
        "finish_reason":"tool_calls"}]})).unwrap();
    let events = vec![
        json!({"type":"part_start","choice":0,"part":"replacement","kind":"text"}),
        json!({"type":"part_delta","choice":0,"part":"replacement","text":"answer"}),
        json!({"type":"part_end","choice":0,"part":"replacement"}),
        json!({"type":"choice_end","choice":0,"finishReason":"stop"}),
    ];
    let output = projector.render(frame, events).unwrap();
    assert!(output["choices"][0]["message"].get("tool_calls").is_none());
    assert_eq!(output["choices"][0]["message"]["content"], "answer");
    assert_eq!(output["choices"][0]["finish_reason"], "stop");
}

#[test]
fn text_can_be_replaced_by_parallel_function_and_custom_tools() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_json(
            json!({"choices":[{"index":0,"message":{"content":"source"},"finish_reason":"stop"}]}),
        )
        .unwrap();
    let mut events = Vec::new();
    for (index, kind, argument) in [(0, "function", "{bad json"), (1, "custom", "raw input")] {
        let part = format!("synthetic:{index}");
        events.extend([
            json!({"type":"part_start","choice":0,"part":part,"kind":"tool",
            "tool":{"index":index,"type":kind,"id":format!("id{index}"),"name":"f"}}),
            json!({"type":"part_delta","choice":0,"part":part,"text":argument}),
            json!({"type":"part_end","choice":0,"part":part}),
        ]);
    }
    events.push(json!({"type":"choice_end","choice":0,"finishReason":"tool_calls"}));
    let output = projector.render(frame, events).unwrap();
    assert!(output["choices"][0]["message"].get("content").is_none());
    assert_eq!(
        output["choices"][0]["message"]["tool_calls"],
        json!([
        {"type":"function","id":"id0","function":{"name":"f","arguments":"{bad json"}},
        {"type":"custom","id":"id1","custom":{"name":"f","input":"raw input"}}])
    );
}

#[test]
fn tool_names_ids_and_arguments_remain_incremental_raw_strings() {
    let mut projector = ContentProjector::new(8);
    for (id, name, arguments) in [("ca", "na", "{"), ("ll", "me", "bad}")] {
        let frame = projector
            .project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
            "index":3,"id":id,"function":{"name":name,"arguments":arguments}}]}}]}))
            .unwrap();
        let mut events = frame.events().to_vec();
        let delta = events
            .iter()
            .find(|event| event["type"] == "part_delta")
            .unwrap();
        assert_eq!(delta["text"], arguments);
        assert_eq!(delta["tool"], json!({"id":id,"name":name}));
        // Force template patch rendering, not just identity pass-through.
        text_delta(&mut events, &format!("{arguments}!"));
        let output = projector.render(frame, events).unwrap();
        let tool = &output["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tool["function"]["arguments"], format!("{arguments}!"));
        assert_eq!(tool["id"], id);
        assert_eq!(tool["function"]["name"], name);
    }
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"finish_reason":"length"}]}))
        .unwrap();
    assert_eq!(frame.events()[0]["type"], "part_end");
    assert_eq!(frame.events()[0]["finishReason"], "length");
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
}

#[test]
fn invalid_output_does_not_commit_lifecycle_state() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    let mut invalid = frame.events().to_vec();
    invalid.push(json!({"type":"choice_end","choice":0,"finishReason":"stop"}));
    assert!(matches!(
        projector.render(frame.clone(), invalid),
        Err(ContentError::Lifecycle)
    ));
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
}

#[test]
fn full_json_without_chat_envelope_stays_open_and_identity_preserving() {
    let mut projector = ContentProjector::new(8);
    let original = json!({"anything":[{"opaque":true}]});
    let frame = projector.project_json(original.clone()).unwrap();
    assert_eq!(frame.events(), &[] as &[Value]);
    assert_eq!(projector.render(frame, vec![]).unwrap(), original);
    let frame = projector.project_end().unwrap();
    assert!(
        projector
            .render_end(frame, vec![json!({"type":"response_end"})])
            .unwrap()
            .is_none()
    );
}

#[test]
fn full_json_without_finish_reason_closes_parts_without_inventing_choice_end() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_json(json!({"choices":[{"index":0,"message":{"content":"a"}}]}))
        .unwrap();
    assert_eq!(frame.events().last().unwrap()["type"], "part_end");
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    let frame = projector.project_end().unwrap();
    assert!(
        projector
            .render_end(frame, vec![json!({"type":"response_end"})])
            .unwrap()
            .is_none()
    );
}

#[test]
fn edits_reject_positional_metadata_in_current_or_later_payload() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(
            json!({"choices":[{"index":0,"delta":{"content":"a"},"logprobs":{"content":[{}]}}]}),
        )
        .unwrap();
    let mut events = frame.events().to_vec();
    text_delta(&mut events, "b");
    assert!(matches!(
        projector.render(frame, events),
        Err(ContentError::PositionalMetadata)
    ));
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    let mut events = frame.events().to_vec();
    text_delta(&mut events, "b");
    projector.render(frame, events).unwrap();
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"annotations":[{"offset":0}]}}]}))
        .unwrap();
    assert!(matches!(
        projector.render(frame, vec![]),
        Err(ContentError::PositionalMetadata)
    ));
}

#[test]
fn success_end_cannot_flush_after_choice_completion_or_finish_unclosed_parts() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(
            json!({"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}),
        )
        .unwrap();
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    let end = projector.project_end().unwrap();
    assert!(matches!(
        projector.render_end(
            end,
            vec![
                json!({"type":"part_start","choice":0,"part":"late","kind":"text"}),
                json!({"type":"response_end"})
            ]
        ),
        Err(ContentError::Lifecycle)
    ));
    let mut projector = ContentProjector::new(8);
    projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    assert!(matches!(
        projector.project_end(),
        Err(ContentError::Lifecycle)
    ));
}

#[test]
fn semantic_validation_is_strict_and_part_state_is_bounded() {
    for event in [
        json!({"type":"part_delta","choice":0,"part":"x","text":{}}),
        json!({"type":"part_start","choice":0,"part":"x","kind":"tool","tool":{"index":0,"type":"unknown"}}),
        json!({"type":"response_end","source":"sse"}),
        json!({"type":"part_end","choice":0,"part":"x","finishReason":null}),
    ] {
        assert!(matches!(
            validate_content_event(&event),
            Err(ContentError::InvalidEvent)
        ));
    }
    let mut projector = ContentProjector::new(1);
    projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    assert!(matches!(
        projector.project_chunk(json!({"choices":[{"index":0,"delta":{"refusal":"b"}}]})),
        Err(ContentError::PartLimit)
    ));
}

#[test]
fn renumbered_tools_keep_opaque_attributes_from_the_source_template() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":4,"id":"call","type":"function","vendor":"tool",
        "function":{"name":"f","arguments":"raw","vendor":"function"}}]}}]}))
        .unwrap();
    let mut events = frame.events().to_vec();
    events[0]["tool"]["index"] = json!(0);
    let output = projector.render(frame, events).unwrap();
    assert_eq!(
        output["choices"][0]["delta"]["tool_calls"][0],
        json!({
        "index":0,"id":"call","type":"function","vendor":"tool",
        "function":{"name":"f","arguments":"raw","vendor":"function"}})
    );
}

#[test]
fn nested_thinking_leaves_are_reasoning_parts_and_keep_opaque_siblings() {
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_json(json!({"choices":[{"index":0,"message":{"content":[{
        "type":"thinking","thinking":[{"type":"text","text":"reason"},
            {"type":"encrypted","data":"opaque"},{"type":"thinking","thinking":[{"type":"text","text":"deep"}]}],
        "signature":"keep"}]},"finish_reason":"stop"}]})).unwrap();
    assert!(
        frame
            .events()
            .iter()
            .filter(|event| event["type"] == "part_start")
            .all(|event| event["kind"] == "reasoning")
    );
    let mut events = frame.events().to_vec();
    text_delta(&mut events, "updated");
    let output = projector.render(frame, events).unwrap();
    assert_eq!(
        output["choices"][0]["message"]["content"][0],
        json!({
        "type":"thinking","thinking":[{"type":"text","text":"updated"},
            {"type":"encrypted","data":"opaque"},{"type":"thinking","thinking":[{"type":"text","text":"deep"}]}],
        "signature":"keep"})
    );
}

#[test]
fn removing_one_nested_reasoning_part_keeps_other_parts_and_unknown_siblings() {
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_json(json!({"choices":[{"index":0,"message":{"content":[{
        "type":"thinking","thinking":[{"type":"text","text":"remove"},{"type":"encrypted","data":"keep"},{"type":"text","text":"survive"}]}]},"finish_reason":"stop"}]})).unwrap();
    let removed = frame.events()[0]["part"].clone();
    let events = frame
        .events()
        .iter()
        .filter(|event| event.get("part") != Some(&removed))
        .cloned()
        .collect();
    let output = projector.render(frame, events).unwrap();
    assert_eq!(
        output["choices"][0]["message"]["content"][0]["thinking"],
        json!([
        {"type":"encrypted","data":"keep"},{"type":"text","text":"survive"}])
    );
}

#[test]
fn empty_known_tail_fields_do_not_reopen_completed_parts() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(
            json!({"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}),
        )
        .unwrap();
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    let tail = json!({"choices":[{"index":0,"delta":{"content":"","refusal":null,"content_filter_results":{"safe":true}},"finish_reason":"stop"}]});
    let frame = projector.project_chunk(tail.clone()).unwrap();
    assert_eq!(frame.events(), &[] as &[Value]);
    assert_eq!(projector.render(frame, vec![]).unwrap(), tail);
}

#[test]
fn tool_indices_are_shared_by_function_and_custom_output_parts() {
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_chunk(json!({})).unwrap();
    let events = vec![
        json!({"type":"part_start","choice":0,"part":"a","kind":"tool","tool":{"index":0,"type":"function"}}),
        json!({"type":"part_start","choice":0,"part":"b","kind":"tool","tool":{"index":0,"type":"custom"}}),
    ];
    assert!(matches!(
        projector.render(frame, events),
        Err(ContentError::Lifecycle)
    ));
}

#[test]
fn streamed_tool_can_have_delayed_text_start_with_the_same_part_identity() {
    let mut projector = ContentProjector::new(8);
    for (arguments, name, transformed) in [
        ("", Some("limen_"), vec![]),
        (
            "{\"text\":\"hello \\uD83D",
            Some("emit_text"),
            vec![
                json!({"type":"part_start","choice":0,"part":"tool:0","kind":"text"}),
                json!({"type":"part_delta","choice":0,"part":"tool:0","text":"hello "}),
            ],
        ),
        (
            "\\uDE00\"}",
            None,
            vec![json!({"type":"part_delta","choice":0,"part":"tool:0","text":"😀"})],
        ),
    ] {
        let mut tool = json!({"index":0,"function":{"arguments":arguments}});
        if let Some(name) = name {
            tool["function"]["name"] = json!(name);
        }
        let frame = projector
            .project_chunk(
                json!({"choices":[{"index":0,"delta":{"tool_calls":[tool]},"finish_reason":null}]}),
            )
            .unwrap();
        let output = projector.render(frame, transformed).unwrap();
        assert!(output["choices"][0]["delta"].get("tool_calls").is_none());
    }
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}))
        .unwrap();
    let mut events = frame.events().to_vec();
    events.last_mut().unwrap()["finishReason"] = json!("stop");
    let output = projector.render(frame, events).unwrap();
    assert_eq!(output["choices"][0]["finish_reason"], "stop");
}

#[test]
fn id_only_tool_can_refine_to_custom_before_argument_data() {
    let mut projector = ContentProjector::new(8);
    let first = json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call"}]}}]});
    let frame = projector.project_chunk(first.clone()).unwrap();
    assert_eq!(frame.events()[0]["tool"]["type"], "function");
    let events = frame.events().to_vec();
    assert_eq!(projector.render(frame, events).unwrap(), first);
    let second = json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"custom","custom":{"name":"f","input":"raw"}}]}}]});
    let frame = projector.project_chunk(second.clone()).unwrap();
    assert_eq!(frame.events()[0]["tool"]["type"], "custom");
    assert_eq!(frame.events()[0]["text"], "raw");
    let events = frame.events().to_vec();
    assert_eq!(projector.render(frame, events).unwrap(), second);
}

#[test]
fn explicit_tool_type_cannot_conflict_and_output_type_cannot_change_after_arguments() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"type":"function","function":{"arguments":""}}]}}]}))
        .unwrap();
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    assert!(matches!(
        projector.project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"type":"custom","custom":{"input":"raw"}}]}}]})),
        Err(ContentError::Lifecycle)
    ));
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_chunk(json!({})).unwrap();
    let output = vec![
        json!({"type":"part_start","choice":0,"part":"tool","kind":"tool","tool":{"index":0,"type":"function"}}),
        json!({"type":"part_delta","choice":0,"part":"tool","text":"a"}),
        json!({"type":"part_delta","choice":0,"part":"tool","text":"b","tool":{"type":"custom"}}),
    ];
    assert!(matches!(
        projector.render(frame, output),
        Err(ContentError::Lifecycle)
    ));
}

#[test]
fn positional_metadata_before_a_semantic_edit_also_prevents_stale_positions() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"annotations":[{"start":0}]}}]}))
        .unwrap();
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    let mut events = frame.events().to_vec();
    text_delta(&mut events, "b");
    assert!(matches!(
        projector.render(frame, events),
        Err(ContentError::PositionalMetadata)
    ));
}

#[test]
fn repeated_source_finish_preserves_transformed_output_reason_and_tail_metadata() {
    let mut projector = ContentProjector::new(8);
    let frame = projector.project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"id":"call","type":"function","function":{"name":"emit_text","arguments":"raw"}}]},"finish_reason":"tool_calls"}]})).unwrap();
    let events = vec![
        json!({"type":"part_start","choice":0,"part":"text","kind":"text"}),
        json!({"type":"part_delta","choice":0,"part":"text","text":"answer"}),
        json!({"type":"part_end","choice":0,"part":"text"}),
        json!({"type":"choice_end","choice":0,"finishReason":"stop"}),
    ];
    let output = projector.render(frame, events).unwrap();
    assert_eq!(output["choices"][0]["finish_reason"], "stop");
    let tail = json!({"choices":[{"index":0,"delta":{"audio":{"expires_at":100},"vendor":"keep"},
        "finish_reason":"tool_calls","vendor":{"choice":true}}],"usage":{"total_tokens":7},"vendor":"root"});
    let frame = projector.project_chunk(tail.clone()).unwrap();
    assert_eq!(frame.events(), &[] as &[Value]);
    let output = projector.render(frame, vec![]).unwrap();
    let mut expected = tail;
    expected["choices"][0]["finish_reason"] = json!("stop");
    assert_eq!(output, expected);
    // Mapping output metadata never changes the source's established reason.
    assert!(matches!(
        projector.project_chunk(json!({"choices":[{"index":0,"finish_reason":"stop"}]})),
        Err(ContentError::Lifecycle)
    ));
}

#[test]
fn finished_tail_does_not_invent_a_finish_marker_for_absent_or_null_source_values() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"finish_reason":"tool_calls"}]}))
        .unwrap();
    projector
        .render(
            frame,
            vec![json!({"type":"choice_end","choice":0,"finishReason":"stop"})],
        )
        .unwrap();
    for finish in [None, Some(Value::Null)] {
        let mut tail = json!({"choices":[{"index":0,"vendor":"tail"}],"usage":{"total_tokens":1}});
        if let Some(finish) = finish {
            tail["choices"][0]["finish_reason"] = finish;
        }
        let frame = projector.project_chunk(tail.clone()).unwrap();
        assert_eq!(frame.events(), &[] as &[Value]);
        assert_eq!(projector.render(frame, vec![]).unwrap(), tail);
    }
}

#[test]
fn retained_cost_counts_both_lifecycles_and_large_identity_and_reason_strings() {
    let mut projector = ContentProjector::new(8);
    assert_eq!(projector.retained_parts(), 0);
    let empty_bytes = projector.retained_bytes();
    let frame = projector
        .project_chunk(
            json!({"choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"stop"}]}),
        )
        .unwrap();
    let events = frame.events().to_vec();
    projector.render(frame, events).unwrap();
    assert_eq!(projector.retained_parts(), 6);
    assert!(projector.retained_bytes() > empty_bytes);

    let mut projector = ContentProjector::new(8);
    let id = "p".repeat(2048);
    let reason = "r".repeat(4096);
    let frame = projector.project_chunk(json!({})).unwrap();
    projector
        .render(
            frame,
            vec![
                json!({"type":"part_start","choice":0,"part":id,"kind":"text"}),
                json!({"type":"part_end","choice":0,"part":id}),
                json!({"type":"choice_end","choice":0,"finishReason":reason}),
            ],
        )
        .unwrap();
    assert_eq!(projector.retained_parts(), 3);
    assert!(projector.retained_bytes() >= id.len() + reason.len() + empty_bytes);
}

#[test]
fn retained_cost_does_not_grow_with_argument_text() {
    fn retained(arguments: &str) -> usize {
        let mut projector = ContentProjector::new(8);
        let frame = projector
            .project_chunk(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
            "index":0,"type":"function","function":{"name":"f","arguments":arguments}}]}}]}))
            .unwrap();
        let events = frame.events().to_vec();
        projector.render(frame, events).unwrap();
        projector.retained_bytes()
    }
    assert_eq!(retained("{}"), retained(&"x".repeat(64 * 1024)));
}

#[test]
fn repeated_raw_finish_cannot_restore_a_dropped_semantic_end() {
    let mut projector = ContentProjector::new(8);
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"finish_reason":"stop"}]}))
        .unwrap();
    let output = projector.render(frame, vec![]).unwrap();
    assert!(output["choices"][0].get("finish_reason").is_none());
    let frame = projector
        .project_chunk(json!({"choices":[{"index":0,"finish_reason":"stop","vendor":"tail"}]}))
        .unwrap();
    let output = projector.render(frame, vec![]).unwrap();
    assert!(output["choices"][0].get("finish_reason").is_none());
    assert_eq!(output["choices"][0]["vendor"], "tail");
}
