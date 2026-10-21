use std::{future::poll_fn, pin::Pin, task::Poll};

use axum::body::Bytes;
use http_body_util::BodyExt;
use limen_script::Cancellation;
use tokio::sync::oneshot;

use super::*;

async fn poll_once<F: Future>(future: Pin<&mut F>) -> Option<F::Output> {
    let mut future = future;
    poll_fn(|cx| {
        Poll::Ready(match future.as_mut().poll(cx) {
            Poll::Ready(value) => Some(value),
            Poll::Pending => None,
        })
    })
    .await
}

fn lifecycle() -> Lifecycle { Lifecycle::new("keepalive".into(), Cancellation::new(), 1000) }

fn limits() -> ResourceLimits {
    ResourceLimits {
        sse_keepalive_ms: 100,
        ..ResourceLimits::default()
    }
}

#[tokio::test(start_paused = true)]
async fn idle_read_sends_repeated_comments_without_restarting_upstream_or_generation_metrics() {
    let mut lifecycle = lifecycle();
    let deadline = lifecycle.deadline;
    let limits = limits();
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
    delivery.timings = lifecycle.statistics.timings.clone();
    {
        let (upstream, received) = oneshot::channel();
        let read = read_chunk(
            async { Ok(received.await.ok()) },
            &mut lifecycle,
            Some(&mut delivery),
            &limits,
        );
        tokio::pin!(read);
        assert!(poll_once(read.as_mut()).await.is_none());
        tokio::time::advance(Duration::from_millis(99)).await;
        assert!(poll_once(read.as_mut()).await.is_none());
        assert!(receive.try_recv().is_err());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(poll_once(read.as_mut()).await.is_none());
        let mut body = receive.try_recv().unwrap().into_body();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            ": keepalive\n\n"
        );
        tokio::time::advance(Duration::from_millis(100)).await;
        assert!(poll_once(read.as_mut()).await.is_none());
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            ": keepalive\n\n"
        );
        upstream.send(Bytes::from_static(b"data: partial")).unwrap();
        let result = poll_once(read.as_mut()).await.unwrap().unwrap().unwrap();
        assert_eq!(result.0, "data: partial");
        drop(body);
    }
    assert_eq!(deadline, lifecycle.deadline);
    assert_eq!(lifecycle.output_bytes, 26);
    let (timings, generation, _) = lifecycle.statistics.timings.owned();
    assert!(timings.downstream_first_event_us.is_some());
    assert!(generation.downstream_first_content_us.is_none());
    assert!(generation.downstream_first_text_us.is_none());
}

#[tokio::test(start_paused = true)]
async fn disabled_or_nonstream_reads_remain_silent() {
    for streaming in [false, true] {
        let mut lifecycle = lifecycle();
        let limits = ResourceLimits {
            sse_keepalive_ms: if streaming { 0 } else { 100 },
            ..limits()
        };
        let (reply, mut receive) = oneshot::channel();
        let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
        let (upstream, received) = oneshot::channel();
        let read = read_chunk(
            async { Ok(received.await.ok()) },
            &mut lifecycle,
            streaming.then_some(&mut delivery),
            &limits,
        );
        tokio::pin!(read);
        assert!(poll_once(read.as_mut()).await.is_none());
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(poll_once(read.as_mut()).await.is_none());
        assert!(receive.try_recv().is_err());
        upstream.send(Bytes::from_static(b"payload")).unwrap();
        assert_eq!(
            poll_once(read.as_mut()).await.unwrap().unwrap().unwrap().0,
            "payload"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn each_upstream_chunk_starts_a_new_idle_interval() {
    let mut lifecycle = lifecycle();
    let limits = limits();
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
    {
        let (upstream, received) = oneshot::channel();
        let read = read_chunk(
            async { Ok(received.await.ok()) },
            &mut lifecycle,
            Some(&mut delivery),
            &limits,
        );
        tokio::pin!(read);
        assert!(poll_once(read.as_mut()).await.is_none());
        tokio::time::advance(Duration::from_millis(90)).await;
        upstream.send(Bytes::from_static(b"partial frame")).unwrap();
        assert!(poll_once(read.as_mut()).await.unwrap().is_ok());
    }
    let read = read_chunk(
        std::future::pending(),
        &mut lifecycle,
        Some(&mut delivery),
        &limits,
    );
    tokio::pin!(read);
    assert!(poll_once(read.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(90)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    assert!(receive.try_recv().is_err());
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    assert!(receive.try_recv().is_ok());
}

#[tokio::test(start_paused = true)]
async fn ready_upstream_data_wins_over_a_due_heartbeat() {
    let mut lifecycle = lifecycle();
    let limits = limits();
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
    let (upstream, received) = oneshot::channel();
    let read = read_chunk(
        async { Ok(received.await.ok()) },
        &mut lifecycle,
        Some(&mut delivery),
        &limits,
    );
    tokio::pin!(read);
    assert!(poll_once(read.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(100)).await;
    upstream.send(Bytes::from_static(b"payload")).unwrap();
    assert!(poll_once(read.as_mut()).await.unwrap().is_ok());
    assert!(receive.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn heartbeat_obeys_frame_limit_before_committing_response() {
    let mut lifecycle = lifecycle();
    let limits = ResourceLimits {
        max_output_bytes: 1,
        ..limits()
    };
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
    let read = read_chunk(
        std::future::pending(),
        &mut lifecycle,
        Some(&mut delivery),
        &limits,
    );
    tokio::pin!(read);
    assert!(poll_once(read.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(100)).await;
    let failure = poll_once(read.as_mut()).await.unwrap().unwrap_err();
    assert_eq!(failure.code, ErrorCode::UpstreamProtocol);
    assert_eq!(failure.phase, "downstream_keepalive");
    assert!(receive.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn slow_consumer_keeps_heartbeat_queue_bounded_and_drop_cancels() {
    let mut lifecycle = lifecycle();
    let limits = limits();
    let (reply, mut receive) = oneshot::channel();
    let mut delivery = Delivery::new(reply, lifecycle.cancellation.clone(), "keepalive".into());
    let read = read_chunk(
        std::future::pending(),
        &mut lifecycle,
        Some(&mut delivery),
        &limits,
    );
    tokio::pin!(read);
    assert!(poll_once(read.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(100)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    let mut body = receive.try_recv().unwrap().into_body();
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        ": keepalive\n\n"
    );
    // The second send was blocked until the sole queue slot became available.
    assert!(poll_once(read.as_mut()).await.is_none());
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        ": keepalive\n\n"
    );
    assert!(poll_once(std::pin::pin!(body.frame())).await.is_none());
    drop(body);
    tokio::time::advance(Duration::from_millis(100)).await;
    assert_eq!(
        poll_once(read.as_mut()).await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
}
