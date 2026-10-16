use std::{
    io,
    sync::{Arc, Mutex},
};

use limen_config::{BusinessConfig, ResourceLimits};
use limen_script::{ScriptError, ScriptErrorKind};
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt;

use super::{
    Error, MAX_RECORD_BYTES,
    encoding::{Encoder, Fields},
    expected_rejection,
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

#[test]
fn rejection_and_operational_failure_have_distinct_severity() {
    let source = BusinessConfig::from_value(json!({}), &ResourceLimits::default())
        .err()
        .unwrap();
    assert!(expected_rejection(&Error::Config {
        file: "config.js",
        source: Box::new(source)
    }));
    assert!(expected_rejection(&Error::Script(ScriptError::new(
        ScriptErrorKind::Contract
    ))));
    assert!(!expected_rejection(&Error::Script(ScriptError::new(
        ScriptErrorKind::MemoryLimit
    ))));
    assert!(!expected_rejection(&Error::Executor(
        io::ErrorKind::Other.into()
    )));
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
    tracing::subscriber::with_default(subscriber, || {
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
    });
    drop(guards);
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("secret"));
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["event"], "diagnostic");
    assert_eq!(records[0]["requestId"], "limen-r-test-1");
    assert_eq!(records[1]["event"], "requestFinished");
    assert_ne!(records[0]["logId"], records[1]["logId"]);
    assert_eq!(records[0]["level"], "info");
    assert!(
        chrono::DateTime::parse_from_rfc3339(records[0]["timestamp"].as_str().unwrap()).is_ok()
    );
}
