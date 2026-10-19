use super::*;

fn tool_delta(function: Value) -> Value {
    let mut event = json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"type":"function"
    }]}}]});
    event["choices"][0]["delta"]["tool_calls"][0]["function"] = function;
    event
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_source_metadata_cannot_bypass_the_shared_record_limit() {
    let (_directory, loaded, mut gateway) = configured_with(include_str!(
        "../../../../fixtures/gateway/content-unit/ordered.js"
    ))
    .await;
    Arc::get_mut(&mut gateway).unwrap().limits.max_json_nodes = 512;
    let route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let mut pipeline = gateway.content_projector(&route).unwrap();
    let mut lifecycle = lifecycle();
    gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(tool_delta(
                json!({"name":"limen_","arguments":"{\"text\":\"A"}),
            )),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap();
    let metadata = json!({"choices":[],"vendor":{"opaque":vec![0;128]}});
    let mut rejected = false;
    for _ in 0..8 {
        match gateway
            .content_payload(
                &mut pipeline,
                ContentInput::Chunk(metadata.clone()),
                None,
                &mut EventBudget::new(),
                &mut lifecycle,
            )
            .await
        {
            Ok(output) => assert!(output.is_empty()),
            Err(failure) => {
                assert_eq!(failure.code, ErrorCode::HookFailed);
                assert_eq!(failure.phase, "onContent");
                rejected = true;
                break;
            },
        }
    }
    assert!(
        rejected,
        "waiting opaque source payloads escaped the request record limit"
    );
    loaded.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn failed_release_and_cancellation_preserve_waiting_frames_and_native_candidate() {
    let (_directory, loaded, mut gateway) = configured_with(include_str!(
        "../../../../fixtures/gateway/content-unit/ordered.js"
    ))
    .await;
    let route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let mut pipeline = gateway.content_projector(&route).unwrap();
    let mut lifecycle = lifecycle();
    for input in [
        tool_delta(json!({"name":"limen_","arguments":"{\"text\":\"A"})),
        chunk("B"),
    ] {
        assert!(
            gateway
                .content_payload(
                    &mut pipeline,
                    ContentInput::Chunk(input),
                    None,
                    &mut EventBudget::new(),
                    &mut lifecycle,
                )
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(pipeline.stages[0].pending.len(), 2);
    let completion = tool_delta(json!({"name":"emit_text","arguments":"\"}"}));
    Arc::get_mut(&mut gateway).unwrap().limits.max_event_bytes = 1;
    // Attach a source envelope so the release checks the actual wire limit.
    let mut decoder = limen_chat::FrameDecoder::new(4096);
    let envelope = format!("id: last\ndata: {completion}\n\n")
        .bytes()
        .find_map(|byte| decoder.push(byte).unwrap())
        .unwrap();
    let failure = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(completion.clone()),
            Some(&envelope),
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::HookFailed);
    assert_eq!(pipeline.stages[0].pending.len(), 2);
    Arc::get_mut(&mut gateway).unwrap().limits.max_event_bytes = 4096;
    lifecycle.cancellation.cancel();
    let failure = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(completion.clone()),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::Cancelled);
    assert_eq!(pipeline.stages[0].pending.len(), 2);
    lifecycle.cancellation = Cancellation::new();
    let outputs = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(completion),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap();
    let text: String = outputs
        .iter()
        .filter_map(|output| output.value["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "AB");
    assert!(pipeline.stages[0].pending.is_empty());
    loaded.shutdown().await.unwrap();
}
