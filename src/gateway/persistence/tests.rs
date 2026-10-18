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
    fs::write(directory.path().join("gateway.js"),format!("import {{defineGateway}} from 'limen:gateway'; export default defineGateway(c=>{{ const p=c.provider('p',{{endpoint:'https://synthetic.invalid/chat'}}); p.model('chat',{{upstream:'native'}}); {policy} }});")).unwrap();
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
