use std::{fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request},
};
use limen_script::Cancellation;
use serde_json::json;

use super::*;
use crate::{bootstrap::Loaded, gateway::FixtureResponse};

const KEY: &str = "synthetic-client-secret-0123456789";

async fn configured(policy: &str) -> (tempfile::TempDir, Loaded) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("server.toml");
    fs::write(&path, format!("[auth]\napi_key='{KEY}'\n")).unwrap();
    fs::write(directory.path().join("gateway.js"),format!("import {{defineGateway}} from 'limen:gateway'; export default defineGateway(c=>{{ const p=c.provider('p',{{endpoint:'https://synthetic.invalid/chat'}}); const m=p.model('chat',{{upstream:'native'}}); {policy} }});")).unwrap();
    let (path, settings) = limen_config::load_server(Some(&path)).unwrap();
    let loaded = Loaded::start(&path, settings).await.unwrap();
    (directory, loaded)
}

fn request(stream: bool) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model":"chat","messages":[{}],"stream":stream}).to_string(),
        ))
        .unwrap()
}

fn incoming(stream: bool) -> FixtureResponse {
    let usage = json!({"prompt_tokens":3,"completion_tokens":0,"total_tokens":3});
    let body = if stream {
        format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",json!({"id":"synthetic","object":"chat.completion.chunk","created":1,"model":"native","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}]}),json!({"id":"synthetic","object":"chat.completion.chunk","created":1,"model":"native","choices":[],"usage":usage})).into_bytes()
    } else {
        serde_json::to_vec(&json!({"id":"synthetic","object":"chat.completion","created":1,"model":"native","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":usage})).unwrap()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static(if stream {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    FixtureResponse {
        status: 200,
        headers,
        body,
    }
}

#[tokio::test]
async fn native_usage_is_recorded_for_all_four_transport_shapes() {
    for upstream in [false, true] {
        for client in [false, true] {
            let (_directory, loaded) = configured("").await;
            let (gateway, _trace) = crate::offline::generation(&loaded, incoming(upstream));
            let response = Arc::clone(&gateway)
                .handle_generation(
                    request(client),
                    "limen-r-test-1".into(),
                    Cancellation::new(),
                )
                .await;
            assert_eq!(response.status(), 200);
            to_bytes(response.into_body(), 4096).await.unwrap();
            gateway.drain().await;
            let RecordSink::Memory(state) = &gateway.records else {
                panic!("offline recorder")
            };
            {
                let state = state.lock().unwrap();
                let pending = state.pending.as_ref().unwrap();
                let record = state.terminal.as_ref().unwrap();
                assert_eq!(pending.request_id, "limen-r-test-1");
                assert_eq!(record.request_id, "limen-r-test-1");
                assert_eq!(pending.client_stream, client);
                assert_eq!(record.input_tokens, Some(3));
                assert_eq!(record.output_tokens, Some(0));
                assert_eq!(record.total_tokens, Some(3));
                assert_eq!(record.cached_input_tokens, None);
                assert_eq!(record.upstream_completed, Some(true));
                assert_eq!(record.outcome, Some(Outcome::Success));
                assert_eq!(record.generation.finish_stop, 1);
                assert!(record.timings.request_us.is_some());
                assert!(record.timings.request_decode_us.is_some());
                assert!(record.timings.storage_begin_us.is_some());
                assert!(record.timings.pre_send_us.is_some());
                assert!(record.timings.upstream_total_us.is_some());
                assert!(record.timings.cleanup_us.is_some());
                assert_eq!(
                    record.generation.upstream_first_content_us.is_some(),
                    upstream
                );
                assert_eq!(
                    record.generation.downstream_first_content_us.is_some(),
                    client
                );
                assert_eq!(record.generation.downstream_buffered, !upstream && client);
                if upstream {
                    assert_eq!(record.generation.choice_count, 1);
                    assert_eq!(record.generation.upstream_content_observations, 1);
                }
                let upstream_parameters = record.upstream_parameters.as_ref().unwrap();
                assert_eq!(
                    pending.client_parameters.0.get("stream"),
                    Some(&json!(client))
                );
                assert_eq!(upstream_parameters.0.get("stream"), Some(&json!(client)));
            }
            loaded.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn initial_write_failure_sends_no_upstream_or_observer_hooks() {
    let (_directory,loaded)=configured("c.onRequest(()=>{throw Error('must not run')}); c.onFinish(()=>{throw Error('must not run')});").await;
    let (gateway, trace) = crate::offline::generation(&loaded, incoming(false));
    let RecordSink::Memory(state) = &gateway.records else {
        panic!("offline recorder")
    };
    state.lock().unwrap().fail_begin = true;
    let response = Arc::clone(&gateway)
        .handle_generation(request(false), "limen-r-test-1".into(), Cancellation::new())
        .await;
    assert_eq!(response.status(), 503);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["error"]["code"],
        "storage_unavailable"
    );
    gateway.drain().await;
    assert!(trace.lock().await.sent.is_none());
    assert!(!trace.lock().await.observation_failed);
    assert!(state.lock().unwrap().pending.is_none());
    loaded.shutdown().await.unwrap();
}

#[tokio::test]
async fn terminal_failure_preserves_success_and_unknown_record() {
    let (_directory, loaded) = configured("").await;
    let (gateway, trace) = crate::offline::generation(&loaded, incoming(false));
    let RecordSink::Memory(state) = &gateway.records else {
        panic!("offline recorder")
    };
    state.lock().unwrap().fail_finish = true;
    let response = Arc::clone(&gateway)
        .handle_generation(request(false), "limen-r-test-1".into(), Cancellation::new())
        .await;
    assert_eq!(response.status(), 200);
    to_bytes(response.into_body(), 4096).await.unwrap();
    gateway.drain().await;
    assert!(trace.lock().await.sent.is_some());
    assert!(state.lock().unwrap().pending.is_some());
    assert!(state.lock().unwrap().terminal.is_none());
    loaded.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_during_begin_keeps_confirmation_and_records_cancelled_without_upstream() {
    for shutdown in [false, true] {
        let (_directory, loaded) =
            configured("c.onRequest(ctx=>{ctx.state.requestHookRan=true});").await;
        let (gateway, trace) = crate::offline::generation(&loaded, incoming(false));
        let RecordSink::Memory(state) = &gateway.records else {
            panic!("offline recorder")
        };
        let entered = Arc::new(tokio::sync::Notify::new());
        let (release, receive) = tokio::sync::oneshot::channel();
        state.lock().unwrap().begin_gate = Some(BeginGate {
            entered: Arc::clone(&entered),
            release: receive,
        });
        let cancel = Cancellation::new();
        let handle_gateway = Arc::clone(&gateway);
        let handle_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            handle_gateway
                .handle_generation(request(false), "limen-r-test-1".into(), handle_cancel)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();
        if shutdown {
            gateway.shutdown.cancel();
        } else {
            cancel.cancel();
        }
        // Let the cancellation reach the tracked request before acknowledging the insert.
        tokio::task::yield_now().await;
        assert!(
            release.send(()).is_ok(),
            "cancellation discarded an in-flight database confirmation"
        );
        let response = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), 499);
        tokio::time::timeout(Duration::from_secs(1), gateway.drain())
            .await
            .unwrap();
        {
            let state = state.lock().unwrap();
            assert!(!state.fail_begin);
            assert!(state.pending.is_some());
            let final_record = state.terminal.as_ref().unwrap();
            assert_eq!(final_record.outcome, Some(Outcome::Cancelled));
            assert_eq!(final_record.error_phase.as_deref(), Some("storage_begin"));
        }
        {
            let trace = trace.lock().await;
            assert!(trace.sent.is_none());
            assert!(trace.state.get("requestHookRan").is_none());
            assert!(trace.stats["timings"]["hooks"].get("onRequest").is_none());
        }
        loaded.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn unconfirmed_begin_still_obeys_the_storage_operation_budget() {
    let (_directory, mut loaded) = configured("").await;
    loaded.server.storage.operation_ms = 50;
    let (gateway, trace) = crate::offline::generation(&loaded, incoming(false));
    let RecordSink::Memory(state) = &gateway.records else {
        panic!("offline recorder")
    };
    let (release, receive) = tokio::sync::oneshot::channel();
    state.lock().unwrap().begin_gate = Some(BeginGate {
        entered: Arc::new(tokio::sync::Notify::new()),
        release: receive,
    });
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        Arc::clone(&gateway).handle_generation(
            request(false),
            "limen-r-test-1".into(),
            Cancellation::new(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 503);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["error"]["code"],
        "storage_unavailable"
    );
    gateway.drain().await;
    assert!(release.send(()).is_err());
    {
        let state = state.lock().unwrap();
        assert!(state.fail_begin);
        assert!(state.pending.is_some());
        assert!(state.terminal.is_none());
    }
    assert!(trace.lock().await.sent.is_none());
    loaded.shutdown().await.unwrap();
}

#[tokio::test]
async fn drained_gateway_stops_admission_and_keeps_the_supplied_response_id() {
    let (_directory, loaded) = configured("").await;
    let (gateway, trace) = crate::offline::generation(&loaded, incoming(false));
    assert!(gateway.accepting());
    gateway.drain().await;
    assert!(!gateway.accepting());
    let response = Arc::clone(&gateway)
        .handle_generation(
            request(false),
            "external-generation-id".into(),
            Cancellation::new(),
        )
        .await;
    assert_eq!(response.status(), 503);
    assert_eq!(
        response.headers()["x-limen-request-id"],
        "external-generation-id"
    );
    let RecordSink::Memory(state) = &gateway.records else {
        panic!("offline recorder");
    };
    assert!(state.lock().unwrap().pending.is_none());
    assert!(trace.lock().await.sent.is_none());
    loaded.shutdown().await.unwrap();
}

async fn recorded(
    policy: &str,
    response: FixtureResponse,
    client_stream: bool,
) -> (PendingRecord, FinalRecord, Vec<u8>) {
    let (_directory, loaded) = configured(policy).await;
    let (gateway, _trace) = crate::offline::generation(&loaded, response);
    let response = Arc::clone(&gateway)
        .handle_generation(
            request(client_stream),
            "limen-r-test-1".into(),
            Cancellation::new(),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body = to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap()
        .to_vec();
    gateway.drain().await;
    let RecordSink::Memory(state) = &gateway.records else {
        panic!("offline recorder");
    };
    let (pending, terminal) = {
        let state = state.lock().unwrap();
        (
            state.pending.clone().unwrap(),
            state.terminal.clone().unwrap(),
        )
    };
    loaded.shutdown().await.unwrap();
    (pending, terminal, body)
}

#[tokio::test]
async fn final_delivery_metrics_follow_script_output_and_keep_raw_upstream_facts() {
    let (_, record, body) = recorded(
        "p.use({name:'drop',onStreamEvent(ctx){if(ctx.event.choices?.[0]?.delta) ctx.event.choices[0].delta={role:'assistant'};}});",
        incoming(true), true,
    ).await;
    assert!(std::str::from_utf8(&body).unwrap().contains("[DONE]"));
    assert!(!std::str::from_utf8(&body).unwrap().contains("hello"));
    assert!(record.generation.upstream_first_content_us.is_some());
    assert!(record.generation.downstream_first_content_us.is_none());
    assert!(record.timings.downstream_first_event_us.is_some());
    assert_eq!(record.generation.upstream_content_observations, 1);
    assert!(
        record
            .phases
            .iter()
            .any(|phase| phase.phase == "onStreamEvent"
                && phase.scope == limen_storage::PhaseScope::Provider)
    );

    let mut source = incoming(true);
    source.body = b"data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_vec();
    let (_, record, body) = recorded(
        "p.use({name:'generate',onStreamEvent(ctx){if(ctx.event.choices?.[0])ctx.event.choices[0].delta={content:'script answer'};}});",
        source, true,
    ).await;
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("script answer")
    );
    assert!(record.generation.upstream_first_content_us.is_none());
    assert!(record.generation.downstream_first_content_us.is_some());
}

#[tokio::test]
async fn phases_and_parameter_snapshots_preserve_scopes_and_cleanup_separately() {
    let (pending, record, _) = recorded(
        "c.onRequest(ctx=>{ctx.body.temperature=0.3}); p.use({name:'request',beforeSend(ctx){ctx.body.temperature=0.4;}}); m.use({name:'request',beforeSend(ctx){ctx.body.temperature=0.6;ctx.body.stream=false;}}); c.onFinish(()=>{});",
        incoming(false), true,
    ).await;
    assert!(!pending.client_parameters.0.contains_key("temperature"));
    let sent = record.upstream_parameters.as_ref().unwrap();
    assert_eq!(sent.0.get("temperature"), Some(&json!(0.6)));
    assert_eq!(sent.0.get("stream"), Some(&json!(false)));
    for scope in [
        limen_storage::PhaseScope::Provider,
        limen_storage::PhaseScope::Model,
    ] {
        let phase = record
            .phases
            .iter()
            .find(|phase| phase.phase == "beforeSend" && phase.scope == scope)
            .unwrap();
        assert_eq!(phase.calls, 1);
        assert_eq!(phase.reported_calls, 1);
        assert_eq!(phase.failed_calls, 0);
        assert!(phase.wall_us.is_some());
        assert_eq!(phase.input_bridge.samples, 1);
    }
    let cleanup = record
        .phases
        .iter()
        .find(|phase| phase.phase == "onFinish")
        .unwrap();
    assert_eq!(cleanup.scope, limen_storage::PhaseScope::Global);
    assert_eq!(cleanup.calls, 1);
    assert!(record.timings.cleanup_us.is_some());
    assert!(record.timings.request_us.is_some());
}

#[tokio::test]
async fn a_single_raw_buffer_keeps_role_heartbeat_tools_reasoning_and_choice_count_facts() {
    let mut source = incoming(true);
    let events = [
        json!({"choices":[{"index":0,"delta":{"role":"assistant"}}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1"}]}}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"search"}}]}}]}),
        json!({"choices":[{"index":0,"delta":{"reasoning":"thinking"}}]}),
        json!({"choices":[{"index":0,"delta":{"content":"answer"},"finish_reason":"tool_calls"},{"index":1,"delta":{"content":"other"},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":8,"total_tokens":11}}),
    ];
    let mut bytes = b": heartbeat\n\n".to_vec();
    for event in events {
        bytes.extend_from_slice(format!("data: {event}\n\n").as_bytes());
    }
    bytes.extend_from_slice(limen_chat::DONE);
    source.body = bytes;
    let (_, record, body) = recorded("", source, true).await;
    assert!(body.starts_with(b": heartbeat"));
    assert_eq!(record.generation.choice_count, 2);
    assert_eq!(record.generation.finish_stop, 1);
    assert_eq!(record.generation.finish_tool, 1);
    assert_eq!(record.generation.upstream_content_observations, 1);
    assert!(record.generation.observed_reasoning);
    assert_eq!(
        record.generation.upstream_first_content_us,
        record.generation.upstream_last_content_us
    );
    assert_eq!(
        record.generation.upstream_first_text_us,
        record.generation.upstream_first_content_us
    );
    assert!(record.generation.downstream_first_text_us.is_some());
    assert!(
        record.timings.downstream_first_event_us <= record.generation.downstream_first_content_us
    );
    let metrics = limen_storage::derive_metrics(&record, true);
    assert_eq!(
        metrics.generation_tps_status,
        limen_storage::MetricStatus::MultipleChoices
    );
}

#[test]
fn acknowledged_sse_success_survives_later_connection_cancellation() {
    let closed = || Err(Failure::new(ErrorCode::Cancelled, "connection"));
    assert!(matches!(
        super::super::terminal_result(Ok(None), closed(), true),
        Ok(None)
    ));
    assert!(matches!(
        super::super::terminal_result(
            Err(Failure::new(ErrorCode::Cancelled, "response")),
            closed(),
            true
        ),
        Ok(None)
    ));
    assert!(matches!(
        super::super::terminal_result(
            Err(Failure::new(ErrorCode::DeadlineExceeded, "response")),
            closed(),
            true
        ),
        Ok(None)
    ));
    let response = super::super::json_response(200, HeaderMap::new(), b"{}".to_vec(), "test");
    assert_eq!(
        super::super::terminal_result(Ok(Some(response)), closed(), true)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        super::super::terminal_result(
            Err(Failure::new(ErrorCode::Cancelled, "response")),
            Ok(()),
            false
        )
        .unwrap_err()
        .code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        super::super::terminal_result(
            Err(Failure::new(ErrorCode::UpstreamProtocol, "response")),
            Ok(()),
            true
        )
        .unwrap_err()
        .code,
        ErrorCode::UpstreamProtocol
    );
}
