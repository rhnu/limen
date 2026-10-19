use std::io;

use limen_script::{ScriptError, ScriptErrorKind};
use serde_json::Value;

use super::*;
use crate::{gateway::error::FailureResponse, telemetry::tests::capture_logs};

fn record_failure(failure: Failure) -> Value {
    let mut lifecycle = Lifecycle::new("limen-r-diagnostic-1".into(), Cancellation::new(), 1000);
    lifecycle.upstream_status = Some(200);
    lifecycle.upstream_media = Some("sse".into());
    let records = capture_logs(|| lifecycle.record(&Err(failure), &[]));
    records
        .into_iter()
        .find(|record| {
            matches!(
                record["event"].as_str(),
                Some("requestFailed" | "requestRejected")
            )
        })
        .expect("a failure must produce its own correlated diagnostic event")
}

#[test]
fn truncated_sse_diagnostic_survives_encoding_without_request_span() {
    let record = record_failure(
        Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse")
            .with_source(limen_chat::Error::Truncated),
    );
    assert_eq!(record["requestId"], "limen-r-diagnostic-1");
    assert_eq!(record["code"], "upstream_protocol");
    assert_eq!(record["status"], 502);
    assert_eq!(record["phase"], "upstream_sse");
    assert_eq!(record["upstreamStatus"], 200);
    assert_eq!(record["upstreamMedia"], "sse");
    assert!(
        record["errorDiagnostic"]
            .as_str()
            .unwrap()
            .contains("without DONE")
    );
}

#[test]
fn invalid_sse_json_keeps_category_and_location_without_upstream_text() {
    let source = serde_json::from_str::<Value>("{\"private-upstream-body\": invalid}").unwrap_err();
    let location = format!("line={}, column={}", source.line(), source.column());
    let record = record_failure(
        Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse")
            .with_source(limen_chat::Error::Json(source)),
    );
    let diagnostic = record["errorDiagnostic"].as_str().unwrap();
    assert!(diagnostic.contains("invalid Chat Completions event JSON"));
    assert!(diagnostic.contains("json=Syntax"));
    assert!(diagnostic.contains(&location));
    assert!(!record.to_string().contains("private-upstream-body"));
}

#[test]
fn upstream_error_diagnostic_keeps_declared_status_and_omits_private_response() {
    let mut failure = Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse")
        .with_source(limen_chat::Error::UpstreamError);
    failure.upstream_error_status = Some(429);
    failure.response = Some(Box::new(FailureResponse {
        body: br#"{"error":{"code":429,"message":"private-upstream-body Bearer private-token"}}"#
            .to_vec(),
        content_type: Some(http::HeaderValue::from_static("private-media")),
        content_encoding: vec![http::HeaderValue::from_static("private-encoding")],
    }));
    let record = record_failure(failure);
    assert_eq!(record["upstreamErrorStatus"], 429);
    assert!(
        record["errorDiagnostic"]
            .as_str()
            .unwrap()
            .contains("upstream returned a Chat Completions error event")
    );
    assert!(!record.to_string().contains("private-"));
}

#[test]
fn transport_and_script_diagnostics_keep_typed_causes_without_sensitive_messages() {
    for (failure, expected) in [
        (
            Failure::new(ErrorCode::UpstreamFailed, "upstream_sse").with_source(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "private-upstream-body Cookie private-token",
            )),
            "io_kind=ConnectionReset",
        ),
        (
            Failure::script(
                ScriptError::with_source(
                    ScriptErrorKind::JavaScript,
                    io::Error::other("private-script-source Authorization private-token"),
                ),
                "beforeSend",
                Some("policy"),
            ),
            "script=JavaScript",
        ),
        (
            Failure::upstream(
                reqwest::Client::new()
                    .get("https://fictional.invalid:invalid/?token=private-token")
                    .build()
                    .unwrap_err(),
                "upstream_send",
            ),
            "upstream_builder=true",
        ),
    ] {
        let record = record_failure(failure);
        assert_eq!(record["requestId"], "limen-r-diagnostic-1");
        assert!(
            record["errorDiagnostic"]
                .as_str()
                .unwrap()
                .contains(expected)
        );
        assert!(!record.to_string().contains("private-"));
        assert!(!record.to_string().contains("fictional.invalid"));
    }
}

#[test]
fn rejected_request_retains_safe_diagnostic_and_own_request_id() {
    let record = record_failure(
        Failure::new(ErrorCode::InvalidRequest, "request_body").with_source(io::Error::new(
            io::ErrorKind::InvalidData,
            "private-client-body",
        )),
    );
    assert_eq!(record["event"], "requestRejected");
    assert_eq!(record["requestId"], "limen-r-diagnostic-1");
    assert_eq!(record["code"], "invalid_request");
    assert_eq!(record["status"], 400);
    assert!(
        record["errorDiagnostic"]
            .as_str()
            .unwrap()
            .contains("io_kind=InvalidData")
    );
    assert!(!record.to_string().contains("private-client-body"));
}
