use std::{
    pin::Pin,
    task::{Context, Waker},
};

use super::*;

fn body() -> (
    mpsc::Sender<QueuedFrame>,
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
            timings: Timings::default(),
            started: Instant::now(),
        },
        drained,
        cancellation,
    )
}

#[test]
fn bounded_body_applies_backpressure_and_acknowledges_only_after_drain() {
    let (sender, mut body, mut drained, cancellation) = body();
    sender
        .try_send(QueuedFrame {
            bytes: Bytes::from_static(b"first"),
            content: limen_chat::GeneratedContent::default(),
        })
        .unwrap();
    assert!(matches!(
        sender.try_send(QueuedFrame {
            bytes: Bytes::from_static(b"second"),
            content: limen_chat::GeneratedContent::default()
        }),
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
async fn streaming_deadline_errors_keep_their_message_and_never_append_done() {
    let lifecycle = Lifecycle::new("test".into(), Cancellation::new(), 1000);
    for (failure, message) in [
        (
            Failure::new(ErrorCode::DeadlineExceeded, "upstream_receive"),
            "The request deadline was exceeded",
        ),
        (
            Failure::hook_timeout("onStreamEvent"),
            "The hook deadline was exceeded",
        ),
    ] {
        let (reply, mut receive) = oneshot::channel();
        let mut delivery = Delivery::new(reply, Cancellation::new(), "test".into());
        delivery.emit(b": alive\n\n".to_vec()).await.unwrap();
        let response = receive.try_recv().unwrap();
        assert_eq!(response.status(), 200);
        let ((), bytes) = tokio::join!(
            delivery.fail(&failure, &lifecycle),
            axum::body::to_bytes(response.into_body(), 2048)
        );
        let bytes = bytes.unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("event: error\n"));
        assert!(text.contains("deadline_exceeded"));
        assert!(text.contains(message));
        assert!(!text.contains("[DONE]"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn success_waits_for_body_drain_and_appends_one_done() {
    let (reply, mut receive) = oneshot::channel();
    let cancellation = Cancellation::new();
    let mut delivery = Delivery::new(reply, cancellation.clone(), "test".into());
    delivery.emit(b"data: first\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    let (result, bytes) = tokio::join!(
        delivery.finish(None),
        axum::body::to_bytes(response.into_body(), 2048)
    );
    result.unwrap();
    assert_eq!(&bytes.unwrap()[..], b"data: first\n\ndata: [DONE]\n\n");
    assert!(!cancellation.is_cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn native_stream_errors_remain_response_data_after_heartbeat_commits_status() {
    let lifecycle = Lifecycle::new("native".into(), Cancellation::new(), 1000);
    let mut failure = Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse");
    failure.response = Some(Box::new(super::super::error::FailureResponse {
        body: br#"{"error":{"code":"capacity","message":"synthetic-private-detail"}}"#.to_vec(),
        content_type: Some(HeaderValue::from_static("application/json")),
        content_encoding: Vec::new(),
    }));
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "native".into());
    delivery.emit(b": alive\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    assert_eq!(response.status(), 200);
    let ((), bytes) = tokio::join!(
        delivery.fail(&failure, &lifecycle),
        axum::body::to_bytes(response.into_body(), 2048)
    );
    let bytes = bytes.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with(": alive\n\nevent: error\ndata: "));
    assert!(text.contains("synthetic-private-detail"));
    assert!(!text.contains("[DONE]"));
    assert!(!failure.to_string().contains("synthetic-private-detail"));
}

#[tokio::test(flavor = "current_thread")]
async fn native_error_stream_preserves_compact_numbers_and_multiline_data() {
    let numbers = ["1e9"; 300].join(",");
    let data = format!(
        "{{\r\n\"error\": {{\"message\":\"private-native-error\",\"metadata\":[{numbers}]}}\r\n}}"
    );
    let lifecycle = Lifecycle::new("native".into(), Cancellation::new(), 1000);
    let value: Value = serde_json::from_str(&data).unwrap();
    let failure = super::super::protocol::native_error(data.into_bytes(), &value, "upstream_sse");
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "native".into());
    delivery.emit(b": alive\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    let ((), bytes) = tokio::join!(
        delivery.fail(&failure, &lifecycle),
        axum::body::to_bytes(response.into_body(), 8192)
    );
    let bytes = bytes.unwrap();
    let mut decoder = limen_chat::FrameDecoder::new(8192);
    let frames: Vec<_> = bytes
        .iter()
        .filter_map(|byte| decoder.push(*byte).unwrap())
        .collect();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].event, "error");
    assert_eq!(
        serde_json::from_str::<Value>(frames[1].data.as_ref().unwrap()).unwrap(),
        value
    );
    assert!(frames[1].data.as_ref().unwrap().contains("1e9"));
    assert!(!String::from_utf8_lossy(&bytes).contains("[DONE]"));
}

#[tokio::test(flavor = "current_thread")]
async fn success_preserves_native_terminator_metadata_after_validation() {
    let terminal = b": final-note\nid: end\nretry: 1000\ndata: [DONE]\n\n";
    let mut decoder = limen_chat::FrameDecoder::new(1024);
    let mut frame = None;
    for byte in terminal {
        if let Some(value) = decoder.push(*byte).unwrap() {
            frame = Some(value);
        }
    }
    let frame = frame.unwrap();
    assert_eq!(frame.event().unwrap(), limen_chat::Event::Done);
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "terminal".into());
    delivery.emit(b": first\n\n".to_vec()).await.unwrap();
    let response = receive.try_recv().unwrap();
    let (result, bytes) = tokio::join!(
        delivery.finish(Some(&frame)),
        axum::body::to_bytes(response.into_body(), 2048)
    );
    result.unwrap();
    assert_eq!(
        bytes.unwrap().as_ref(),
        [b": first\n\n".as_slice(), terminal].concat()
    );
}

#[test]
fn downstream_content_is_observed_only_when_the_body_yields_the_final_event() {
    let (sender, mut body, _drained, _cancellation) = body();
    let facts = limen_chat::GeneratedContent {
        generation: true,
        text: false,
        ..limen_chat::GeneratedContent::default()
    };
    sender
        .try_send(QueuedFrame {
            bytes: Bytes::from_static(b"encoded tool"),
            content: facts,
        })
        .unwrap();
    assert_eq!(body.timings.owned().1.downstream_first_content_us, None);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(Pin::new(&mut body).poll_frame(&mut cx).is_ready());
    let observed = body.timings.owned().1;
    assert!(observed.downstream_first_content_us.is_some());
    assert!(observed.downstream_first_text_us.is_none());
    sender
        .try_send(QueuedFrame {
            bytes: Bytes::from_static(b"encoded text"),
            content: limen_chat::GeneratedContent {
                text: true,
                ..facts
            },
        })
        .unwrap();
    assert!(Pin::new(&mut body).poll_frame(&mut cx).is_ready());
    let later = body.timings.owned().1;
    assert_eq!(
        later.downstream_first_content_us,
        observed.downstream_first_content_us
    );
    assert!(later.downstream_first_text_us.is_some());
}

#[test]
fn stream_completion_requires_done_and_a_body_eof_observed_within_deadline() {
    let start = Instant::now();
    let (reply, _receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, Cancellation::new(), "test".into());
    delivery
        .timings
        .drained(start + std::time::Duration::from_micros(15));
    assert!(!delivery.completed(start + std::time::Duration::from_micros(20)));
    delivery.done_sent = true;
    assert!(delivery.completed(start + std::time::Duration::from_micros(20)));
    assert!(!delivery.completed(start + std::time::Duration::from_micros(10)));
    let (reply, _receive) = oneshot::channel();
    let mut incomplete = Delivery::new(reply, Cancellation::new(), "test".into());
    incomplete.done_sent = true;
    assert!(!incomplete.completed(start + std::time::Duration::from_micros(20)));
}
