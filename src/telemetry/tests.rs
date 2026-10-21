use std::{
    io,
    sync::{Arc, Mutex, Once},
};

use serde_json::json;
use tracing_subscriber::layer::SubscriberExt;

use super::{
    MAX_RECORD_BYTES,
    encoding::{Encoder, Fields},
    writer::LogLayer,
};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

pub(crate) fn with_test_subscriber<T>(
    subscriber: impl tracing::Subscriber + Send + Sync + 'static,
    action: impl FnOnce() -> T,
) -> T {
    static INITIALIZE: Once = Once::new();
    INITIALIZE.call_once(|| {
        // Keep a permanent dispatcher so callsite registration considers all scoped subscribers.
        // The single-dispatch fast path can otherwise cache another thread's callsite as disabled.
        tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default())
            .unwrap();
    });
    tracing::subscriber::with_default(subscriber, action)
}

pub(crate) fn capture_logs(action: impl FnOnce()) -> Vec<serde_json::Value> {
    let capture = Capture::default();
    let (layer, guards) = LogLayer::capture(capture.clone());
    with_test_subscriber(tracing_subscriber::registry().with(layer), action);
    drop(guards);
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn capture_keeps_events_first_emitted_without_a_scoped_subscriber() {
    fn emit() {
        tracing::error!(request_id = "capture-request", "request failed");
    }
    let records = capture_logs(|| {
        std::thread::spawn(emit).join().unwrap();
        emit();
    });
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["event"], "requestFailed");
    assert_eq!(records[0]["requestId"], "capture-request");
}

#[test]
fn concurrent_captures_keep_their_own_events() {
    let ready = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let captures: Vec<_> = ["capture-one", "capture-two"]
            .into_iter()
            .map(|request_id| {
                let ready = &ready;
                scope.spawn(move || {
                    let records = capture_logs(|| {
                        ready.wait();
                        tracing::error!(request_id, "request failed");
                    });
                    assert_eq!(records.len(), 1);
                    assert_eq!(records[0]["event"], "requestFailed");
                    assert_eq!(records[0]["requestId"], request_id);
                })
            })
            .collect();
        for capture in captures {
            capture.join().unwrap();
        }
    });
}

#[test]
fn oversized_record_is_complete_correlated_summary() {
    let encoder = Encoder::new("test-run");
    let mut fields = Fields::message("request finished");
    fields.values.insert("requestId".into(), json!("request-1"));
    fields
        .values
        .insert("model".into(), json!("a".repeat(MAX_RECORD_BYTES)));
    let bytes = encoder.encode("INFO", fields);
    assert!(bytes.len() <= MAX_RECORD_BYTES);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["requestId"], "request-1");
    assert_eq!(value["event"], "requestFinished");
    assert_eq!(value["truncated"], true);
    assert!(value.get("model").is_none());
    assert_eq!(bytes.last(), Some(&b'\n'));
}

#[test]
fn whitelist_rejects_body_and_dynamic_messages_before_queueing() {
    let capture = Capture::default();
    let (layer, guards) = LogLayer::capture(capture.clone());
    let subscriber = tracing_subscriber::registry().with(layer);
    with_test_subscriber(subscriber, || {
        let span = tracing::info_span!(
            "request",
            request_id = "limen-r-test-1",
            authorization = "secret-token"
        );
        let _entered = span.enter();
        tracing::info!(
            phase = "onRequest",
            body = "secret-body",
            error = "secret-error",
            "{}",
            "secret-message"
        );
        tracing::info!(phase = "onFinish", "request finished");
        tracing::info!(
            phase = "onContent",
            calls = 3_u64,
            failed_calls = 1_u64,
            interrupted_calls = 0_u64,
            wall_ms = 0.5_f64,
            body = "secret-body",
            "native policy statistics"
        );
    });
    drop(guards);
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("secret"));
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["event"], "diagnostic");
    assert_eq!(records[0]["requestId"], "limen-r-test-1");
    assert_eq!(records[1]["event"], "requestFinished");
    assert_eq!(records[2]["event"], "nativePolicyStatistics");
    assert_eq!(records[2]["calls"], 3);
    assert_eq!(records[2]["failedCalls"], 1);
    assert_eq!(records[2]["interruptedCalls"], 0);
    assert_eq!(records[2]["wallMs"], 0.5);
    assert_ne!(records[0]["logId"], records[1]["logId"]);
    assert_eq!(records[0]["level"], "info");
    assert!(
        chrono::DateTime::parse_from_rfc3339(records[0]["timestamp"].as_str().unwrap()).is_ok()
    );
}

#[test]
fn request_metadata_and_performance_survive_the_shared_encoder() {
    let capture = Capture::default();
    let (layer, guards) = LogLayer::capture(capture.clone());
    let subscriber = tracing_subscriber::registry().with(layer);
    with_test_subscriber(subscriber, || {
        tracing::info!(
            request_id = "limen-r-test-2",
            configured_model = "configured",
            final_model = "sent",
            upstream_media = "sse",
            input_bytes = 42_u64,
            outbound_bytes = 45_u64,
            upstream_bytes = 120_u64,
            output_bytes = 90_u64,
            authorization = "private-credential",
            body = "private-content",
            "request finished"
        );
        tracing::info!(
            request_id = "limen-r-test-2",
            upstream_ttft_ms = 12.25_f64,
            downstream_ttft_ms = 14.5_f64,
            average_output_tps = 30f64,
            generation_tps_status = "insufficient_observations",
            downstream_mode = "native_stream",
            "request performance"
        );
    });
    drop(guards);
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("private-"));
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[0]["configuredUpstreamModel"], "configured");
    assert_eq!(records[0]["finalUpstreamModel"], "sent");
    assert_eq!(records[0]["upstreamMedia"], "sse");
    assert_eq!(records[0]["inputBytes"], 42);
    assert_eq!(records[0]["outboundBytes"], 45);
    assert_eq!(records[0]["upstreamBytes"], 120);
    assert_eq!(records[0]["outputBytes"], 90);
    assert_eq!(records[1]["event"], "requestPerformance");
    assert_eq!(records[1]["upstreamTtftMs"], 12.25);
    assert_eq!(records[1]["downstreamTtftMs"], 14.5);
    assert_eq!(records[1]["averageOutputTps"], 30.0);
    assert_eq!(
        records[1]["generationTpsStatus"],
        "insufficient_observations"
    );
}
