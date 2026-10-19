use super::*;

mod ordered;

fn limits() -> NativeContentLimits {
    NativeContentLimits {
        max_output_events: 128,
        max_output_bytes: 1_000_000,
        max_retained_bytes: 1_000_000,
        max_retained_records: 10_000,
    }
}

fn replace_policy(search: &str, replacement: &str) -> NativeContentPolicy {
    NativeContentTemplate::compile(
        ContentPolicy::ReplaceText(&ReplaceText {
            search: search.into(),
            replacement: replacement.into(),
            max_buffered_chars: 1024,
            max_parts: 64,
        }),
        &limits(),
        &mut || true,
    )
    .unwrap()
    .instantiate()
}

fn synthetic_policy() -> NativeContentPolicy { synthetic_options(|_| {}) }

fn synthetic_options(change: impl FnOnce(&mut SyntheticTextTool)) -> NativeContentPolicy {
    let mut options = SyntheticTextTool {
        tool_name: "limen_emit_text".into(),
        prompt: None,
        injection_depth: 0,
        max_buffered_chars: 4096,
        max_parts: 64,
        max_pending_events: 64,
    };
    change(&mut options);
    NativeContentTemplate::compile(
        ContentPolicy::SyntheticTextTool(&options),
        &limits(),
        &mut || true,
    )
    .unwrap()
    .instantiate()
}

fn start(part: &str, kind: &str) -> Value {
    json!({"type":"part_start","choice":0,"part":part,"kind":kind})
}

fn tool(part: &str, name: &str, index: u64) -> Value {
    json!({"type":"part_start","choice":0,"part":part,"kind":"tool","tool":{"type":"function","index":index,"name":name}})
}

fn delta(part: &str, text: &str) -> Value {
    json!({"type":"part_delta","choice":0,"part":part,"text":text})
}

fn end(part: &str, reason: &str) -> Value {
    json!({"type":"part_end","choice":0,"part":part,"finishReason":reason})
}

fn apply(policy: &mut NativeContentPolicy, event: Value) -> Vec<Value> {
    policy.apply(event, &limits(), &mut || true).unwrap()
}

fn text(events: &[Value]) -> String {
    events
        .iter()
        .filter_map(|event| event["text"].as_str())
        .collect()
}

#[test]
fn replacement_handles_every_utf8_split_and_overlapping_prefixes() {
    for (search, replacement, input, expected) in [
        ("abc", "X", "0abcabcab1", "0XXab1"),
        ("aab", "R", "aaaabaaab", "aaRaR"),
        ("abab", "!", "abababababa", "!!aba"),
        ("😀é", "替换", "a😀é😀😀éb😀", "a替换😀替换b😀"),
        ("ééx", "", "éééxéé", "ééé"),
    ] {
        for split in (0..=input.len()).filter(|at| input.is_char_boundary(*at)) {
            let mut policy = replace_policy(search, replacement);
            apply(&mut policy, start("p", "text"));
            let mut events = apply(&mut policy, delta("p", &input[..split]));
            events.extend(apply(&mut policy, delta("p", &input[split..])));
            events.extend(apply(&mut policy, end("p", "stop")));
            assert_eq!(text(&events), expected, "search {search}, split {split}");
            assert_eq!(policy.retained_parts(), 0);
        }
    }
}

#[test]
fn replacement_keeps_parts_independent_and_preserves_other_kinds() {
    let mut policy = replace_policy("ab", "X");
    apply(&mut policy, start("a", "text"));
    apply(&mut policy, start("b", "text"));
    assert_eq!(apply(&mut policy, delta("a", "a")), [] as [Value; 0]);
    assert_eq!(text(&apply(&mut policy, delta("b", "b"))), "b");
    assert_eq!(text(&apply(&mut policy, delta("a", "b"))), "X");
    let reasoning = start("r", "reasoning");
    assert_eq!(apply(&mut policy, reasoning.clone()), [reasoning]);
    assert_eq!(text(&apply(&mut policy, delta("r", "ab"))), "ab");
    assert!(
        policy
            .apply(json!({"type":"response_end"}), &limits(), &mut || true)
            .is_err()
    );
}

#[test]
fn replacement_enforces_output_state_and_utf16_bounds() {
    let mut policy = replace_policy("a", &"x".repeat(100_000));
    apply(&mut policy, start("p", "text"));
    let mut bound = limits();
    bound.max_output_bytes = 200;
    assert_eq!(
        policy.apply(delta("p", "aa"), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = replace_policy("😀x", "y");
    if let Policy::Replace { options, .. } = &mut policy.policy {
        Arc::make_mut(options).max_buffered_chars = 1;
    }
    apply(&mut policy, start("p", "text"));
    assert_eq!(
        policy.apply(delta("p", "😀"), &limits(), &mut || true),
        Err(NativeContentError::Limit)
    );
    let mut policy = replace_policy("a", "b");
    bound = limits();
    bound.max_retained_records = 0;
    assert_eq!(
        policy.apply(start("p", "text"), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );
}

#[test]
fn synthetic_decodes_every_split_in_escaped_and_unicode_arguments() {
    let args = " \t{ \"text\" : \"a\\n\\t\\b\\f\\r\\/\\\\\\\"\\u4e2d\\ud83d\\ude00é😀\" }\r\n";
    for split in (0..=args.len()).filter(|at| args.is_char_boundary(*at)) {
        let mut policy = synthetic_policy();
        assert_eq!(
            apply(&mut policy, tool("p", "limen_emit_text", 4)),
            [] as [Value; 0]
        );
        let mut events = apply(&mut policy, delta("p", &args[..split]));
        events.extend(apply(&mut policy, delta("p", &args[split..])));
        events.extend(apply(&mut policy, end("p", "tool_calls")));
        assert_eq!(
            text(&events),
            "a\n\t\u{8}\u{c}\r/\\\"中😀é😀",
            "split {split}"
        );
        assert_eq!(events[0]["kind"], "text");
        let choice = apply(
            &mut policy,
            json!({"type":"choice_end","choice":0,"finishReason":"tool_calls"}),
        );
        assert_eq!(choice[0]["finishReason"], "stop");
        apply(&mut policy, json!({"type":"response_end"}));
        assert_eq!(policy.retained_parts(), 0);
        assert!(policy.retained_bytes() < 3000);
    }
}

#[test]
fn synthetic_supports_one_character_deltas_without_argument_retention() {
    let mut policy = synthetic_policy();
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    let mut events = Vec::new();
    for ch in "{\"text\":\"\\ud83d\\ude00hello\"}".chars() {
        events.extend(apply(&mut policy, delta("p", &ch.to_string())));
        assert!(policy.retained_bytes() < 10_000);
    }
    assert_eq!(text(&events), "😀hello");
    let retained = policy.retained_bytes();
    // Completed decoding retains no complete argument or decoded-text history.
    assert!(retained < 10_000);
    apply(&mut policy, end("p", "tool_calls"));
}

#[test]
fn synthetic_rejects_malformed_json_for_every_split() {
    for args in [
        "{}",
        "{\"other\":\"x\"}",
        "{\"te\\u0078t\":\"x\"}",
        "{\"text\":1}",
        "{\"text\":\"x\",\"x\":1}",
        "{\"text\":\"x\"}{}",
        "{\"text\":\"\\q\"}",
        "{\"text\":\"\\ud800x\"}",
        "{\"text\":\"\\udc00\"}",
        "{\"text\":\"\\ud800\"}",
        "{\"text\":\"\\uZZZZ\"}",
        "{\"text\":\"\n\"}",
        "{\"text\":\"unfinished",
        "{\"text\":\"x\"",
    ] {
        for split in 0..=args.len() {
            let mut policy = synthetic_policy();
            apply(&mut policy, tool("p", "limen_emit_text", 0));
            let result = policy
                .apply(delta("p", &args[..split]), &limits(), &mut || true)
                .and_then(|_| policy.apply(delta("p", &args[split..]), &limits(), &mut || true))
                .and_then(|_| policy.apply(end("p", "tool_calls"), &limits(), &mut || true));
            assert_eq!(
                result.err(),
                Some(NativeContentError::InvalidInput),
                "{args} split {split}"
            );
        }
    }
}

#[test]
fn synthetic_fragmented_names_and_mixed_real_tools_reindex_contiguously() {
    let mut policy = synthetic_policy();
    apply(&mut policy, tool("synthetic", "limen_", 0));
    let mut name = delta("synthetic", "");
    name["tool"] = json!({"name":"emit_text"});
    assert_eq!(apply(&mut policy, name), [] as [Value; 0]);
    apply(&mut policy, delta("synthetic", "{\"text\":\"ok\"}"));
    apply(&mut policy, end("synthetic", "tool_calls"));

    apply(&mut policy, tool("real", "limen_emit_text", 9));
    let mut real_name = delta("real", "");
    real_name["tool"] = json!({"name":"_real"});
    let events = apply(&mut policy, real_name);
    assert_eq!(events[0]["tool"]["index"], 0);
    assert_eq!(events[1]["tool"]["name"], "_real");
    assert_eq!(text(&apply(&mut policy, delta("real", "raw"))), "raw");
    apply(&mut policy, end("real", "tool_calls"));
    assert_eq!(
        apply(&mut policy, tool("real2", "other", 50))[0]["tool"]["index"],
        1
    );
    apply(&mut policy, end("real2", "tool_calls"));
    assert_eq!(
        apply(
            &mut policy,
            json!({"type":"choice_end","choice":0,"finishReason":"tool_calls"})
        )[0]["finishReason"],
        "tool_calls"
    );
}

#[test]
fn synthetic_argument_before_name_resolution_is_decoded() {
    let mut policy = synthetic_policy();
    apply(&mut policy, tool("p", "limen_", 0));
    assert_eq!(
        apply(&mut policy, delta("p", "{\"text\":\"hello\"}")),
        [] as [Value; 0]
    );
    let mut name = delta("p", "");
    name["tool"] = json!({"name":"emit_text"});
    let events = apply(&mut policy, name);
    assert_eq!(text(&events), "hello");
    apply(&mut policy, end("p", "tool_calls"));
}

#[test]
fn synthetic_pending_metadata_is_bounded_in_events_utf16_and_shared_bytes() {
    let mut policy = synthetic_options(|options| options.max_pending_events = 1);
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    assert_eq!(
        policy.apply(delta("p", ""), &limits(), &mut || true),
        Err(NativeContentError::Limit)
    );

    let event = tool("😀", "limen_", 0);
    let mut policy =
        synthetic_options(|options| options.max_buffered_chars = json_size(&event, true));
    assert_eq!(apply(&mut policy, event), [] as [Value; 0]);
    assert!(
        policy
            .apply(delta("😀", ""), &limits(), &mut || true)
            .is_err()
    );
    let mut policy = synthetic_policy();
    let mut bound = limits();
    bound.max_retained_bytes = 5;
    assert_eq!(
        policy.apply(tool("p", "limen_", 0), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );
}

#[test]
fn synthetic_truncation_and_finish_reason_rules_are_explicit() {
    for reason in ["length", "content_filter"] {
        let mut policy = synthetic_policy();
        apply(&mut policy, tool("p", "limen_emit_text", 0));
        apply(&mut policy, delta("p", "{\"text\":\"partial\\u"));
        assert_eq!(
            apply(&mut policy, end("p", reason)).last().unwrap()["finishReason"],
            reason
        );
        assert_eq!(
            apply(
                &mut policy,
                json!({"type":"choice_end","choice":0,"finishReason":reason})
            )[0]["finishReason"],
            reason
        );
    }
    let mut policy = synthetic_policy();
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    apply(&mut policy, delta("p", "{\"text\":\"ok\"}"));
    apply(&mut policy, end("p", "stop"));
    assert_eq!(
        policy.apply(
            json!({"type":"choice_end","choice":0,"finishReason":"stop"}),
            &limits(),
            &mut || true
        ),
        Err(NativeContentError::InvalidInput)
    );
}

#[test]
fn synthetic_rejects_custom_and_late_name_metadata_and_bounds_output() {
    for custom in [false, true] {
        let mut policy = synthetic_policy();
        let mut event = tool("p", "limen_emit_text", 0);
        if custom {
            event["tool"]["type"] = json!("custom");
        }
        apply(&mut policy, event);
        if custom {
            assert!(
                policy
                    .apply(delta("p", "{\"text\":\"ok\"}"), &limits(), &mut || true)
                    .is_err()
            );
        } else {
            apply(&mut policy, delta("p", "{\"text\":\""));
            let mut event = delta("p", "ok");
            event["tool"] = json!({"name":"late"});
            assert!(policy.apply(event, &limits(), &mut || true).is_err());
        }
    }
    let mut policy = synthetic_policy();
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    let mut bound = limits();
    bound.max_output_events = 1;
    assert_eq!(
        policy.apply(delta("p", "{\"text\":\"ok\"}"), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );
}

#[test]
fn cancellation_and_failed_candidates_leave_committed_state_usable() {
    let mut committed = replace_policy("abc", "X");
    apply(&mut committed, start("p", "text"));
    apply(&mut committed, delta("p", "ab"));
    let mut candidate = committed.clone();
    let mut calls = 0;
    assert_eq!(
        candidate.apply(delta("p", &"x".repeat(100_000)), &limits(), &mut || {
            calls += 1;
            calls < 4
        }),
        Err(NativeContentError::Interrupted)
    );
    assert_eq!(text(&apply(&mut committed, delta("p", "c"))), "X");

    let mut committed = synthetic_policy();
    apply(&mut committed, tool("p", "limen_emit_text", 0));
    apply(&mut committed, delta("p", "{\"text\":\""));
    let mut candidate = committed.clone();
    let mut calls = 0;
    assert_eq!(
        candidate.apply(delta("p", &"x".repeat(100_000)), &limits(), &mut || {
            calls += 1;
            calls < 4
        }),
        Err(NativeContentError::Interrupted)
    );
    assert_eq!(text(&apply(&mut committed, delta("p", "ok\"}"))), "ok");
    apply(&mut committed, end("p", "tool_calls"));
}

#[test]
fn startup_compilation_checks_table_capacity_and_execution_budget() {
    let policy = ReplaceText {
        search: "a".repeat(100_000),
        replacement: "b".into(),
        max_buffered_chars: 100_000,
        max_parts: 64,
    };
    let mut bound = limits();
    bound.max_retained_bytes = 1024;
    assert!(matches!(
        NativeContentTemplate::compile(ContentPolicy::ReplaceText(&policy), &bound, &mut || true),
        Err(NativeContentError::Limit)
    ));
    let mut calls = 0;
    assert!(matches!(
        NativeContentTemplate::compile(ContentPolicy::ReplaceText(&policy), &limits(), &mut || {
            calls += 1;
            calls < 3
        }),
        Err(NativeContentError::Interrupted)
    ));
    let template =
        NativeContentTemplate::compile(ContentPolicy::ReplaceText(&policy), &limits(), &mut || {
            true
        })
        .unwrap()
        .instantiate();
    assert!(template.retained_bytes() > 900_000);
    assert_eq!(template.retained_parts(), 0);
}

#[test]
fn synthetic_decodes_large_arguments_with_constant_retained_state() {
    let mut policy = synthetic_options(|options| options.max_buffered_chars = 256);
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    apply(&mut policy, delta("p", "{\"text\":\""));
    let bytes = policy.retained_bytes();
    let records = policy.retained_parts();
    for _ in 0..4 {
        let events = apply(&mut policy, delta("p", &"😀".repeat(10_000)));
        assert_eq!(text(&events).encode_utf16().count(), 20_000);
        assert_eq!(policy.retained_bytes(), bytes);
        assert_eq!(policy.retained_parts(), records);
    }
    apply(&mut policy, delta("p", "\"}"));
    apply(&mut policy, end("p", "tool_calls"));
}

#[test]
fn encoded_output_limit_covers_escaped_characters_and_event_metadata() {
    let mut policy = replace_policy("a", "\u{1}".repeat(100).as_str());
    apply(&mut policy, start("p", "text"));
    let mut bound = limits();
    bound.max_output_bytes = 200;
    assert_eq!(
        policy.apply(delta("p", "a"), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = synthetic_policy();
    apply(&mut policy, tool("p", "limen_emit_text", 0));
    apply(&mut policy, delta("p", "{\"text\":\""));
    bound.max_output_bytes = 5;
    assert_eq!(
        policy.apply(delta("p", "hello"), &bound, &mut || true),
        Err(NativeContentError::Limit)
    );
}

#[test]
fn compiled_template_always_instantiates_independent_mutable_state() {
    let options = ReplaceText {
        search: "ab".into(),
        replacement: "X".into(),
        max_buffered_chars: 1,
        max_parts: 2,
    };
    let template = NativeContentTemplate::compile(
        ContentPolicy::ReplaceText(&options),
        &limits(),
        &mut || true,
    )
    .unwrap();
    let mut first = template.instantiate();
    apply(&mut first, start("p", "text"));
    assert_eq!(apply(&mut first, delta("p", "a")), [] as [Value; 0]);
    let mut second = template.instantiate();
    apply(&mut second, start("p", "text"));
    assert_eq!(text(&apply(&mut second, delta("p", "b"))), "b");
    assert_eq!(text(&apply(&mut first, delta("p", "b"))), "X");
    assert_eq!(template.instantiate().retained_parts(), 0);
}
