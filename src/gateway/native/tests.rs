use super::*;

fn apply(policy: Value, body: Value) -> (Value, ObservationBatch) {
    let policy = serde_json::from_value(policy).unwrap();
    let mut context = json!({"body":null,"client":{"headers":[]},"headers":[],"state":{}});
    context["body"] = body;
    let observations = before_send(
        &policy,
        &mut context,
        &["changed".to_owned()],
        "policy",
        &mut || Ok(()),
    )
    .unwrap();
    (context, observations)
}

#[test]
fn token_cap_preserves_lower_values_sets_absent_and_rejects_conflicts() {
    let policy = json!({"kind":"capCompletionTokens","max":10,"onChange":"changed"});
    for (body, expected, changed) in [
        (json!({}), 10, true),
        (json!({"max_completion_tokens":2}), 2, false),
        (json!({"max_completion_tokens":20}), 10, true),
    ] {
        let (context, observations) = apply(policy.clone(), body);
        assert_eq!(context["body"]["max_completion_tokens"], expected);
        assert_eq!(!observations.events.is_empty(), changed);
    }
    let (context, observations) = apply(policy.clone(), json!({"max_completion_tokens":2.0}));
    assert_eq!(context["body"]["max_completion_tokens"].as_f64(), Some(2.0));
    assert_eq!(observations.events, Vec::<u8>::new());
    for body in [
        json!({"max_tokens":2}),
        json!({"max_completion_tokens":null}),
        json!({"max_completion_tokens":-1}),
        json!({"max_completion_tokens":1.5}),
    ] {
        let (context, observations) = apply(policy.clone(), body.clone());
        assert_eq!(context["rejection"]["status"], 422);
        assert_eq!(context["body"], body);
        assert_eq!(observations.events, Vec::<u8>::new());
    }
}

#[test]
fn field_removal_records_once_only_when_present() {
    let policy =
        json!({"kind":"dropRequestFields","fields":["temperature","seed"],"onChange":"changed"});
    let (context, observations) = apply(
        policy.clone(),
        json!({"temperature":null,"seed":7,"vendor":1}),
    );
    assert_eq!(context["body"], json!({"vendor":1}));
    assert_eq!(observations.events, vec![0]);
    let (_, observations) = apply(policy, json!({}));
    assert_eq!(observations.events, Vec::<u8>::new());
}

#[test]
fn forwarding_uses_exposed_headers_and_rejects_multiple_values() {
    let policy = serde_json::from_value(
        json!({"kind":"forwardClientHeader","from":"X-Tenant","to":"X-Upstream"}),
    )
    .unwrap();
    let mut context = json!({"client":{"headers":[{"name":"x-tenant","value":"a"}]},"headers":[{"name":"x-upstream","value":"old"},{"name":"other","value":"b"}]});
    before_send(&policy, &mut context, &[], "policy", &mut || Ok(())).unwrap();
    assert_eq!(
        context["headers"],
        json!([{"name":"other","value":"b"},{"name":"x-upstream","value":"a"}])
    );
    context["client"]["headers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"x-tenant","value":"duplicate"}));
    let previous = context["headers"].clone();
    before_send(&policy, &mut context, &[], "policy", &mut || Ok(())).unwrap();
    assert_eq!(context["headers"], previous);
    assert_eq!(context["rejection"]["status"], 422);
}

#[test]
fn synthetic_tool_appends_instruction_and_preserves_original_history() {
    let (context, _) = apply(
        json!({"kind":"syntheticTextTool","maxBufferedChars":128}),
        json!({"messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"real"}}]}),
    );
    assert_eq!(
        context["body"]["messages"][0],
        json!({"role":"user","content":"hello"})
    );
    assert_eq!(context["body"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        context["body"]["tools"][1]["function"]["name"],
        "limen_emit_text"
    );
    assert_eq!(context["body"]["tool_choice"], "required");
    assert_eq!(context["body"]["parallel_tool_calls"], false);
    for extra in [
        json!({"functions":[]}),
        json!({"tool_choice":"none"}),
        json!({"n":2}),
        json!({"tools":[{"type":"function","function":{"name":"limen_emit_text"}}]}),
    ] {
        let mut body = json!({"messages":[{"role":"user","content":"hello"}]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (context, _) = apply(
            json!({"kind":"syntheticTextTool","maxBufferedChars":128}),
            body.clone(),
        );
        assert_eq!(context["body"], body);
        assert_eq!(context["rejection"]["status"], 422);
    }
}

#[test]
fn cancellation_and_unregistered_observation_do_not_publish_a_draft() {
    let policy = serde_json::from_value(
        json!({"kind":"dropRequestFields","fields":["x"],"onChange":"unregistered"}),
    )
    .unwrap();
    let mut context = json!({"body":{"x":1}});
    let result = before_send(&policy, &mut context, &[], "policy", &mut || Ok(()));
    assert_eq!(result.unwrap_err().code, ErrorCode::HookFailed);
    let result = before_send(&policy, &mut context, &[], "policy", &mut || {
        Err(Failure::new(ErrorCode::Cancelled, "beforeSend"))
    });
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
}
