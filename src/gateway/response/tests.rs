use std::{
    pin::Pin,
    task::{Context, Waker},
};

use super::*;

fn body() -> (
    mpsc::Sender<Bytes>,
    StreamBody,
    oneshot::Receiver<()>,
    Cancellation,
) {
    let (sender, receiver) = mpsc::channel(1);
    let (finished, drained) = oneshot::channel();
    let cancellation = Cancellation::new();
    (
        sender,
        StreamBody {
            receiver,
            finished: Some(finished),
            cancellation: cancellation.clone(),
            completed: false,
        },
        drained,
        cancellation,
    )
}

#[test]
fn bounded_body_applies_backpressure_and_acknowledges_only_after_drain() {
    let (sender, mut body, mut drained, cancellation) = body();
    sender.try_send(Bytes::from_static(b"first")).unwrap();
    assert!(matches!(
        sender.try_send(Bytes::from_static(b"second")),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    assert!(matches!(
        drained.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    let mut cx = Context::from_waker(Waker::noop());
    let Poll::Ready(Some(Ok(frame))) = Pin::new(&mut body).poll_frame(&mut cx) else {
        panic!("queued frame must be available");
    };
    assert_eq!(frame.into_data().unwrap(), "first");
    assert!(matches!(
        drained.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(sender);
    assert!(Pin::new(&mut body).poll_frame(&mut cx).is_ready());
    assert!(drained.try_recv().is_ok());
    drop(body);
    assert!(!cancellation.is_cancelled());
}

#[test]
fn dropping_body_cancels_producer_and_does_not_acknowledge_success() {
    let (sender, body, mut drained, cancellation) = body();
    drop(body);
    assert!(cancellation.is_cancelled());
    assert!(sender.is_closed());
    assert!(matches!(
        drained.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));
}

#[test]
fn external_representation_applies_alias_once_and_keeps_absent_model_absent() {
    let mut lifecycle = Lifecycle::new("test".into(), Cancellation::new(), 1000);
    lifecycle.alias = Some("alias".into());
    let mut body = json!({"model":"upstream","vendor":7});
    apply_alias(&mut body, &lifecycle);
    assert_eq!(body["model"], "alias");
    assert_eq!(body["vendor"], 7);
    let mut body = json!({"vendor":7});
    apply_alias(&mut body, &lifecycle);
    assert!(body.get("model").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn json_response_delivers_encoded_bytes_with_rust_owned_media_and_request_id() {
    let mut headers = HeaderMap::new();
    headers.insert("x-draft", HeaderValue::from_static("kept"));
    let bytes = br#"{"message":"hello"}"#.to_vec();
    let response = super::super::json_response(201, headers, bytes.clone(), "request-1");
    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(response.headers()["x-draft"], "kept");
    assert_eq!(response.headers()["x-limen-request-id"], "request-1");
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()[..],
        &bytes
    );
}

#[tokio::test(flavor = "current_thread")]
async fn errors_before_first_frame_use_json_and_after_first_frame_use_error_event() {
    let lifecycle = Lifecycle::new("test".into(), Cancellation::new(), 1000);
    let failure = Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse");
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "test".into());
    delivery.fail(&failure, &lifecycle).await;
    let response = receive.try_recv().unwrap();
    assert_eq!(response.status(), 502);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "upstream_protocol");

    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "test".into());
    delivery.emit(b"data: first\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let ((), bytes) = tokio::join!(
        delivery.fail(&failure, &lifecycle),
        axum::body::to_bytes(response.into_body(), 2048)
    );
    let bytes = bytes.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with("data: first\n\nevent: error\ndata: "));
    assert!(text.contains("upstream_protocol"));
    assert!(!text.contains("[DONE]"));
}

#[tokio::test(flavor = "current_thread")]
async fn success_waits_for_body_drain_and_appends_one_done() {
    let (reply, mut receive) = oneshot::channel();
    let cancellation = Cancellation::new();
    let mut delivery = Delivery::new(reply, cancellation.clone(), "test".into());
    delivery.emit(b"data: first\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    let (result, bytes) = tokio::join!(
        delivery.finish(),
        axum::body::to_bytes(response.into_body(), 2048)
    );
    result.unwrap();
    assert_eq!(&bytes.unwrap()[..], b"data: first\n\ndata: [DONE]\n\n");
    assert!(!cancellation.is_cancelled());
}
