use std::{fs, sync::Arc};

use limen_script::Cancellation;
use serde_json::json;

use super::{super::observations::Observations, *};
use crate::{bootstrap::Loaded, gateway::FixtureResponse};

async fn configured() -> (tempfile::TempDir, Loaded, Arc<Gateway>) {
    configured_with(r"export default {
        providers:{p:{endpoint:'https://synthetic.invalid/chat',pipeline:[{name:'replace',builtin:{kind:'replaceText',search:'ab',replacement:'ZZ',maxBufferedChars:1}}]}},
        models:{chat:{provider:'p',upstream:'native'}}};").await
}

async fn configured_with(script: &str) -> (tempfile::TempDir, Loaded, Arc<Gateway>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("server.toml");
    fs::write(
        &path,
        "[auth]\napi_key='synthetic-client-secret-0123456789'\n",
    )
    .unwrap();
    fs::write(directory.path().join("config.js"), script).unwrap();
    let (path, settings) = limen_config::load_server(Some(&path)).unwrap();
    let loaded = Loaded::start(&path, settings).await.unwrap();
    let (gateway, _) = crate::offline::generation(
        &loaded,
        FixtureResponse {
            status: 200,
            headers: http::HeaderMap::new(),
            body: vec![],
        },
    );
    (directory, loaded, gateway)
}

fn lifecycle() -> Lifecycle {
    let mut lifecycle = Lifecycle::new("test".into(), Cancellation::new(), 1000);
    lifecycle.provider = Some("p".into());
    lifecycle.alias = Some("chat".into());
    lifecycle
}

fn chunk(text: &str) -> Value { json!({"choices":[{"index":0,"delta":{"content":text}}]}) }

#[tokio::test(flavor = "current_thread")]
async fn failed_native_render_discards_candidate_and_preserves_previous_prefix() {
    let (_directory, loaded, mut gateway) = configured().await;
    Arc::get_mut(&mut gateway).unwrap().limits.max_event_bytes = 256;
    let route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let mut lifecycle = lifecycle();
    lifecycle.state = json!({"previous":{"text":"retained"}})
        .as_object()
        .unwrap()
        .clone();
    lifecycle.observations = Observations::new(4);
    lifecycle.observations.commit(
        limen_script::ObservationBatch {
            events: vec![0],
            dropped: 0,
        },
        "beforeSend",
        Some("previous"),
    );
    let names = vec!["accepted".to_owned()];
    let observations = lifecycle.observations.snapshot(&names);
    let mut pipeline = gateway.content_projector(&route).unwrap();
    let first = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("a")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first["choices"][0]["delta"]["content"], "");
    let mut decoder = limen_chat::FrameDecoder::new(4096);
    let bytes = format!("id: {}\ndata: {}\n\n", "x".repeat(512), chunk("b"));
    let envelope = bytes
        .bytes()
        .find_map(|byte| decoder.push(byte).unwrap())
        .unwrap();
    let failure = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("b")),
            Some(&envelope),
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::HookFailed);
    let retry = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("b")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry["choices"][0]["delta"]["content"], "ZZ");
    assert_eq!(
        Value::Object(lifecycle.state),
        json!({"previous":{"text":"retained"}})
    );
    assert_eq!(lifecycle.observations.snapshot(&names), observations);
    loaded.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn native_occurrences_have_independent_state_and_share_retained_capacity() {
    let (_directory, loaded, gateway) = configured().await;
    let route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let mut first_lifecycle = lifecycle();
    let mut second_lifecycle = lifecycle();
    let mut first = gateway.content_projector(&route).unwrap();
    let mut second = gateway.content_projector(&route).unwrap();
    gateway
        .content_payload(
            &mut first,
            ContentInput::Chunk(chunk("a")),
            None,
            &mut EventBudget::new(),
            &mut first_lifecycle,
        )
        .await
        .unwrap();
    let value = gateway
        .content_payload(
            &mut second,
            ContentInput::Chunk(chunk("b")),
            None,
            &mut EventBudget::new(),
            &mut second_lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value["choices"][0]["delta"]["content"], "b");
    let ContentExecution::Native(native) = &first.stages[0].executor else {
        panic!("expected native");
    };
    let native_bytes = native.retained_bytes();
    let mut limits = gateway.limits.clone();
    limits.max_output_bytes = native_bytes;
    assert_eq!(
        first.capacity(None, &limits).unwrap_err().code,
        ErrorCode::HookFailed
    );
    let mut candidate = first.stages[0].clone();
    let ContentExecution::Native(native) = &mut candidate.executor else {
        panic!("expected native");
    };
    let retained = native.retained_parts();
    native
        .apply(
            json!({"type":"part_start","choice":1,"part":"extra","kind":"text"}),
            &NativeContentLimits {
                max_output_events: 10,
                max_output_bytes: 10000,
                max_retained_bytes: 10000,
                max_retained_records: 10,
            },
            &mut || true,
        )
        .unwrap();
    limits.max_output_bytes = usize::MAX;
    limits.max_json_nodes =
        first.source.retained_parts() + first.stages[0].projector.retained_parts() + retained;
    first.capacity(None, &limits).unwrap();
    assert!(first.capacity(Some((0, &candidate)), &limits).is_err());
    loaded.shutdown().await.unwrap();
}

#[test]
fn retained_grammar_budget_is_shared_by_source_and_all_steps() {
    let mut pipeline = ContentPipeline {
        source: ContentProjector::new(100),
        stages: vec![
            ContentStage {
                projector: ContentProjector::new(100),
                name: "script".into(),
                executor: ContentExecution::Script(0)
            };
            2
        ],
    };
    let frame = pipeline
        .source
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    for stage in &mut pipeline.stages {
        stage
            .projector
            .render(frame.clone(), frame.events())
            .unwrap();
    }
    let mut limits = ResourceLimits::default();
    pipeline.capacity(None, &limits).unwrap();
    limits.max_output_bytes = pipeline
        .source
        .retained_bytes()
        .max(pipeline.stages[0].projector.retained_bytes());
    assert_eq!(
        pipeline.capacity(None, &limits).unwrap_err().code,
        ErrorCode::HookFailed
    );
    limits.max_output_bytes = usize::MAX;
    limits.max_json_nodes = pipeline.source.retained_parts();
    assert_eq!(
        pipeline.capacity(None, &limits).unwrap_err().code,
        ErrorCode::HookFailed
    );
}

#[tokio::test(flavor = "current_thread")]
async fn same_named_provider_and_model_steps_keep_alias_and_request_state_separate() {
    let (_directory, loaded, gateway) = configured_with(r"export default {
        providers:{p:{endpoint:'https://synthetic.invalid/chat',pipeline:[{name:'same',builtin:{kind:'replaceText',search:'ab',replacement:'bc',maxBufferedChars:1}}]}},
        models:{chat:{provider:'p',upstream:'first',pipeline:[{name:'same',builtin:{kind:'replaceText',search:'bc',replacement:'ZZ',maxBufferedChars:1}}]},
        alias:{provider:'p',upstream:'second',pipeline:[{name:'same',builtin:{kind:'replaceText',search:'bc',replacement:'ZZ',maxBufferedChars:1}}]}}};").await;
    let first_route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let second_route = gateway.plans.route(&gateway.business, "alias").unwrap();
    let mut first = gateway.content_projector(&first_route).unwrap();
    let mut second = gateway.content_projector(&second_route).unwrap();
    let mut first_lifecycle = lifecycle();
    let mut second_lifecycle = lifecycle();
    for (pipeline, lifecycle, input, expected) in [
        (&mut first, &mut first_lifecycle, "a", ""),
        (&mut second, &mut second_lifecycle, "b", ""),
    ] {
        let output = gateway
            .content_payload(
                pipeline,
                ContentInput::Chunk(chunk(input)),
                None,
                &mut EventBudget::new(),
                lifecycle,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output["choices"][0]["delta"]["content"], expected);
    }
    let output = gateway
        .content_payload(
            &mut first,
            ContentInput::Chunk(chunk("b")),
            None,
            &mut EventBudget::new(),
            &mut first_lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output["choices"][0]["delta"]["content"], "ZZ");
    let output = gateway
        .content_payload(
            &mut second,
            ContentInput::Chunk(chunk("x")),
            None,
            &mut EventBudget::new(),
            &mut second_lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output["choices"][0]["delta"]["content"], "bx");
    let mut fresh = gateway.content_projector(&first_route).unwrap();
    let output = gateway
        .content_payload(
            &mut fresh,
            ContentInput::Chunk(chunk("ab")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output["choices"][0]["delta"]["content"], "ZZ");
    loaded.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_drops_native_candidate_and_preserves_committed_prefix() {
    let (_directory, loaded, gateway) = configured().await;
    let route = gateway.plans.route(&gateway.business, "chat").unwrap();
    let mut pipeline = gateway.content_projector(&route).unwrap();
    let mut lifecycle = lifecycle();
    gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("a")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap();
    lifecycle.cancellation.cancel();
    let error = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("b")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    lifecycle.cancellation = Cancellation::new();
    let output = gateway
        .content_payload(
            &mut pipeline,
            ContentInput::Chunk(chunk("b")),
            None,
            &mut EventBudget::new(),
            &mut lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output["choices"][0]["delta"]["content"], "ZZ");
    loaded.shutdown().await.unwrap();
}
