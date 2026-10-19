use super::*;

fn request(body: Value) -> RequestDraft {
    RequestDraft {
        body,
        url: "https://fictional.invalid/chat".into(),
        headers: Vec::new(),
    }
}

fn apply(policy: Value, body: Value) -> (RequestDraft, Option<RejectionStatus>, ObservationBatch) {
    let policy: limen_config::BuiltinPolicy = serde_json::from_value(policy).unwrap();
    let policy =
        NativeBeforeSend::compile(policy.stages().before_send.unwrap(), &["changed".into()])
            .unwrap();
    let mut request = request(body);
    let (rejection, observations) = before_send(
        &policy,
        &mut request,
        &ClientContext::default(),
        "policy",
        &mut || Ok(()),
    )
    .unwrap();
    (request, rejection, observations)
}

#[test]
fn token_cap_preserves_lower_values_sets_absent_and_rejects_conflicts() {
    let policy = json!({"kind":"capCompletionTokens","max":10,"onChange":"changed"});
    for (body, expected, changed) in [
        (json!({}), 10, true),
        (json!({"max_completion_tokens":2}), 2, false),
        (json!({"max_completion_tokens":20}), 10, true),
    ] {
        let (context, rejection, observations) = apply(policy.clone(), body);
        assert_eq!(rejection, None);
        assert_eq!(context.body["max_completion_tokens"], expected);
        assert_eq!(!observations.events.is_empty(), changed);
    }
    let (context, rejection, observations) =
        apply(policy.clone(), json!({"max_completion_tokens":2.0}));
    assert_eq!(rejection, None);
    assert_eq!(context.body["max_completion_tokens"].as_f64(), Some(2.0));
    assert_eq!(observations.events, Vec::<u8>::new());
    for body in [
        json!({"max_tokens":2}),
        json!({"max_completion_tokens":null}),
        json!({"max_completion_tokens":-1}),
        json!({"max_completion_tokens":1.5}),
    ] {
        let (context, rejection, observations) = apply(policy.clone(), body.clone());
        assert_eq!(rejection, Some(RejectionStatus::UnprocessableEntity));
        assert_eq!(context.body, body);
        assert_eq!(observations.events, Vec::<u8>::new());
    }
}

#[test]
fn field_removal_records_once_only_when_present() {
    let policy =
        json!({"kind":"dropRequestFields","fields":["temperature","seed"],"onChange":"changed"});
    let (context, rejection, observations) = apply(
        policy.clone(),
        json!({"temperature":null,"seed":7,"vendor":1}),
    );
    assert_eq!(rejection, None);
    assert_eq!(context.body, json!({"vendor":1}));
    assert_eq!(observations.events, vec![0]);
    let (_, _, observations) = apply(policy, json!({}));
    assert_eq!(observations.events, Vec::<u8>::new());
}

#[test]
fn forwarding_uses_exposed_headers_and_rejects_multiple_values() {
    let options: limen_config::BuiltinPolicy = serde_json::from_value(
        json!({"kind":"forwardClientHeader","from":"X-Tenant","to":"X-Upstream"}),
    )
    .unwrap();
    let policy = NativeBeforeSend::compile(options.stages().before_send.unwrap(), &[]).unwrap();
    let mut client = ClientContext {
        headers: vec![HeaderField {
            name: "x-tenant".into(),
            value: "a".into(),
        }],
    };
    let mut request = request(json!({}));
    request.headers = vec![
        HeaderField {
            name: "x-upstream".into(),
            value: "old".into(),
        },
        HeaderField {
            name: "other".into(),
            value: "b".into(),
        },
    ];
    let (rejection, _) =
        before_send(&policy, &mut request, &client, "policy", &mut || Ok(())).unwrap();
    assert_eq!(rejection, None);
    assert_eq!(
        request.headers,
        vec![
            HeaderField {
                name: "other".into(),
                value: "b".into()
            },
            HeaderField {
                name: "x-upstream".into(),
                value: "a".into()
            }
        ]
    );
    client.headers.push(HeaderField {
        name: "x-tenant".into(),
        value: "duplicate".into(),
    });
    let previous = request.headers.clone();
    let (rejection, _) =
        before_send(&policy, &mut request, &client, "policy", &mut || Ok(())).unwrap();
    assert_eq!(request.headers, previous);
    assert_eq!(rejection, Some(RejectionStatus::UnprocessableEntity));
}

#[test]
fn synthetic_tool_appends_instruction_and_preserves_original_history() {
    let (context, rejection, _) = apply(
        json!({"kind":"syntheticTextTool","maxBufferedChars":128}),
        json!({"messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"real"}}]}),
    );
    assert_eq!(rejection, None);
    assert_eq!(
        context.body["messages"][0],
        json!({"role":"user","content":"hello"})
    );
    assert_eq!(context.body["messages"].as_array().unwrap().len(), 2);
    assert_eq!(context.body["messages"][1]["role"], "user");
    assert!(
        context.body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("limen_emit_text")
    );
    assert_eq!(
        context.body["tools"][1]["function"]["name"],
        "limen_emit_text"
    );
    assert_eq!(context.body["tool_choice"], "required");
    assert_eq!(context.body["parallel_tool_calls"], false);
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
        let (context, rejection, _) = apply(
            json!({"kind":"syntheticTextTool","maxBufferedChars":128}),
            body.clone(),
        );
        assert_eq!(context.body, body);
        assert_eq!(rejection, Some(RejectionStatus::UnprocessableEntity));
    }
}

#[test]
fn synthetic_tool_inserts_verbatim_prompts_at_message_depth() {
    let history = json!([
        {"role":"system","content":"rules"},
        {"role":"user","content":"first"},
        {"role":"assistant","content":"answer"},
        {"role":"user","content":"last"}
    ]);
    for (depth, index) in [(0, 4), (1, 3), (2, 2), (4, 0), (100, 0)] {
        for prompt in ["Use {toolName} literally.\n原样保留提示。", ""] {
            let (context, rejection, _) = apply(
                json!({"kind":"syntheticTextTool","toolName":"custom_emit","prompt":prompt,"injectionDepth":depth,"maxBufferedChars":128}),
                json!({"messages":history}),
            );
            assert_eq!(rejection, None);
            let mut messages = context.body["messages"].as_array().unwrap().clone();
            assert_eq!(
                messages.remove(index),
                json!({"role":"user","content":prompt})
            );
            assert_eq!(json!(messages), history);
            assert_eq!(context.body["tools"][0]["function"]["name"], "custom_emit");
        }
    }
    let (context, rejection, _) = apply(
        json!({"kind":"syntheticTextTool","prompt":"instruction","injectionDepth":5,"maxBufferedChars":128}),
        json!({"messages":[]}),
    );
    assert_eq!(rejection, None);
    assert_eq!(
        context.body["messages"],
        json!([{"role":"user","content":"instruction"}])
    );
}

#[test]
fn synthetic_tool_default_prompt_uses_configured_tool_name() {
    let (context, rejection, _) = apply(
        json!({"kind":"syntheticTextTool","toolName":"custom_emit","maxBufferedChars":128}),
        json!({"messages":[{"role":"user","content":"hello"}]}),
    );
    assert_eq!(rejection, None);
    assert!(
        context.body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("custom_emit")
    );
}

#[test]
fn cancellation_does_not_publish_a_draft() {
    let options: limen_config::BuiltinPolicy =
        serde_json::from_value(json!({"kind":"dropRequestFields","fields":["x"]})).unwrap();
    let policy = NativeBeforeSend::compile(options.stages().before_send.unwrap(), &[]).unwrap();
    let mut context = request(json!({"x":1}));
    let result = before_send(
        &policy,
        &mut context,
        &ClientContext::default(),
        "policy",
        &mut || Err(Failure::new(ErrorCode::Cancelled, "beforeSend")),
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(context.body, json!({"x":1}));
}
