use super::*;

fn batch(policy: &mut NativeContentPolicy, events: Vec<Value>) -> Vec<Vec<Value>> {
    policy.apply_batch(events, &limits(), &mut || true).unwrap()
}

fn name(part: &str, suffix: &str) -> Value {
    let mut event = delta(part, "");
    event["tool"] = json!({"name":suffix});
    event
}

#[test]
fn pending_tool_identity_holds_plain_text_and_preserves_source_batches() {
    let mut policy = synthetic_policy();
    assert_eq!(
        batch(&mut policy, vec![tool("s", "limen_", 0)]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![delta("s", "{\"text\":\"A\"}")]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![start("p", "text"), delta("p", "B")]),
        [] as [Vec<Value>; 0]
    );
    let released = batch(&mut policy, vec![name("s", "emit_text")]);
    assert_eq!(released.len(), 4);
    assert_eq!(released[0], [start("s", "text")]);
    assert_eq!(text(&released[1]), "A");
    assert_eq!(text(&released[2]), "B");
    assert_eq!(released[3], [] as [Value; 0]);
    assert_eq!(
        text(&released.into_iter().flatten().collect::<Vec<_>>()),
        "AB"
    );
    // The text is released while the tool is still open.
    assert_eq!(batch(&mut policy, vec![delta("p", "C")])[0][0]["text"], "C");
}

#[test]
fn same_batch_events_keep_plain_and_synthetic_text_in_semantic_order() {
    let mut policy = synthetic_policy();
    let events = batch(
        &mut policy,
        vec![
            tool("s", "limen_emit_text", 0),
            delta("s", "{\"text\":\"A"),
            start("p", "text"),
            delta("p", "B"),
            delta("s", "C\"}"),
            delta("p", "D"),
        ],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(text(&events[0]), "ABCD");

    let mut policy = synthetic_policy();
    let events = batch(
        &mut policy,
        vec![
            tool("s", "limen_emit_text", 0),
            delta("s", "{\"text\":\"A\\"),
            start("p", "text"),
            delta("p", "B"),
            delta("s", "nC\"}"),
        ],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(text(&events[0]), "A\nBC");
}

#[test]
fn multiple_pending_tools_resolve_independently_and_real_indices_follow_source_starts() {
    let mut policy = synthetic_policy();
    assert_eq!(
        batch(&mut policy, vec![tool("a", "limen_", 20)]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![tool("b", "other", 21)]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![tool("c", "limen_", 22)]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![delta("c", "{\"text\":\"C\"}")]),
        [] as [Vec<Value>; 0]
    );
    assert_eq!(
        batch(&mut policy, vec![name("c", "emit_text")]),
        [] as [Vec<Value>; 0]
    );
    let events = batch(&mut policy, vec![name("a", "actual")]);
    assert_eq!(events.len(), 6);
    assert_eq!(events[0][0]["tool"]["index"], 0);
    assert_eq!(events[1][0]["tool"]["index"], 1);
    assert_eq!(events[2][0]["kind"], "text");
    assert_eq!(text(&events[3]), "C");
}

#[test]
fn split_escape_and_surrogate_characters_return_to_their_starting_event() {
    for (prefix, suffix, expected) in [
        ("{\"text\":\"A\\", "nC\"}", "A\nBC"),
        ("{\"text\":\"A\\u4", "e2dC\"}", "A中BC"),
        ("{\"text\":\"A\\ud83d", "\\ude00C\"}", "A😀BC"),
        ("{\"text\":\"A\\ud83d\\u", "de00C\"}", "A😀BC"),
    ] {
        let mut policy = synthetic_policy();
        batch(&mut policy, vec![tool("s", "limen_emit_text", 0)]);
        let first = batch(&mut policy, vec![delta("s", prefix)]);
        assert_eq!(first.len(), 1); // Only the resolved start batch is released.
        assert_eq!(
            batch(&mut policy, vec![start("p", "text"), delta("p", "B")]),
            [] as [Vec<Value>; 0]
        );
        let released = batch(&mut policy, vec![delta("s", suffix)]);
        assert_eq!(released.len(), 3);
        assert_eq!(
            text(&released.into_iter().flatten().collect::<Vec<_>>()),
            expected
        );
    }
}

#[test]
fn truncated_partial_character_drops_only_undecoded_tail_and_releases_ordered_prefix() {
    for reason in ["length", "content_filter"] {
        let mut policy = synthetic_policy();
        batch(&mut policy, vec![tool("s", "limen_emit_text", 0)]);
        batch(&mut policy, vec![delta("s", "{\"text\":\"A\\ud83d")]);
        assert_eq!(
            batch(&mut policy, vec![start("p", "text"), delta("p", "B")]),
            [] as [Vec<Value>; 0]
        );
        let released = batch(&mut policy, vec![end("s", reason)]);
        assert_eq!(released.len(), 3);
        assert_eq!(
            text(&released.iter().flatten().cloned().collect::<Vec<_>>()),
            "AB"
        );
        assert_eq!(released[2][0]["finishReason"], reason);
    }
}

#[test]
fn whole_waiting_window_obeys_event_character_and_shared_state_limits() {
    let mut policy = synthetic_options(|options| options.max_pending_events = 2);
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    batch(&mut policy, vec![start("p", "text")]);
    assert_eq!(
        policy.apply_batch(vec![delta("p", "B")], &limits(), &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = synthetic_options(|options| options.max_buffered_chars = 256);
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    assert_eq!(
        policy.apply_batch(vec![delta("p", &"😀".repeat(100))], &limits(), &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = synthetic_policy();
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    let mut bound = limits();
    bound.max_retained_bytes = policy.retained_bytes() + 1;
    assert_eq!(
        policy.apply_batch(vec![delta("p", "B")], &bound, &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = synthetic_policy();
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    bound = limits();
    bound.max_retained_records = policy.retained_records();
    assert_eq!(
        policy.apply_batch(vec![delta("p", "B")], &bound, &mut || true),
        Err(NativeContentError::Limit)
    );
}

#[test]
fn empty_source_batches_are_preserved_and_bounded_while_blocked() {
    let mut policy = synthetic_options(|options| options.max_pending_events = 2);
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    assert_eq!(batch(&mut policy, vec![]), [] as [Vec<Value>; 0]);
    assert_eq!(
        policy.apply_batch(vec![], &limits(), &mut || true),
        Err(NativeContentError::Limit)
    );

    let mut policy = synthetic_policy();
    batch(&mut policy, vec![tool("s", "limen_", 0)]);
    batch(&mut policy, vec![]);
    let released = batch(&mut policy, vec![name("s", "actual")]);
    assert_eq!(released.len(), 3);
    assert_eq!(released[1], [] as [Value; 0]);
    assert_eq!(
        batch(&mut replace_policy("a", "b"), vec![]),
        vec![Vec::<Value>::new()]
    );
}

#[test]
fn cancelled_window_scan_keeps_the_committed_candidate_usable() {
    let mut committed = synthetic_options(|options| options.max_buffered_chars = 200_000);
    batch(&mut committed, vec![tool("s", "limen_", 0)]);
    batch(
        &mut committed,
        vec![delta(
            "s",
            &format!("{{\"text\":\"{}\"}}", "x".repeat(100_000)),
        )],
    );
    let mut candidate = committed.clone();
    let mut checks = 0;
    assert_eq!(
        candidate.apply_batch(vec![], &limits(), &mut || {
            checks += 1;
            checks < 30
        }),
        Err(NativeContentError::Interrupted)
    );
    assert!(checks >= 30);
    let released = batch(&mut committed, vec![name("s", "emit_text")]);
    assert_eq!(
        text(&released.into_iter().flatten().collect::<Vec<_>>()).len(),
        100_000
    );
}

#[test]
fn eof_and_invalid_arguments_fail_without_releasing_a_blocked_candidate() {
    let mut committed = synthetic_policy();
    batch(&mut committed, vec![tool("s", "limen_", 0)]);
    batch(&mut committed, vec![delta("s", "{\"text\":\"A\"}")]);
    batch(&mut committed, vec![start("p", "text"), delta("p", "B")]);
    let mut candidate = committed.clone();
    assert_eq!(
        candidate.apply_batch(vec![json!({"type":"response_end"})], &limits(), &mut || {
            true
        }),
        Err(NativeContentError::InvalidInput)
    );
    let mut candidate = committed.clone();
    assert_eq!(
        candidate.apply_batch(
            vec![name("s", "emit_text"), delta("s", "invalid")],
            &limits(),
            &mut || true
        ),
        Err(NativeContentError::InvalidInput)
    );
    let released = batch(&mut committed, vec![name("s", "emit_text")]);
    assert_eq!(
        text(&released.into_iter().flatten().collect::<Vec<_>>()),
        "AB"
    );
}
