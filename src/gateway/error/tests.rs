use std::{error::Error, io};

use super::*;

#[test]
fn public_error_keeps_fixed_message_and_request_id_without_raw_cause() {
    let failure = Failure::new(ErrorCode::InvalidRequest, "request_body")
        .with_source(io::Error::other("secret body token Cookie source"));
    let public = failure.error_json("request-7");
    assert_eq!(public["error"]["code"], "invalid_request");
    assert_eq!(public["error"]["request_id"], "request-7");
    assert!(!public.to_string().contains("secret"));
    assert!(!format!("{failure:?} {failure}").contains("secret"));
    assert!(failure.source().unwrap().is::<io::Error>());
}

#[test]
fn script_classification_preserves_source_and_step_context() {
    for (kind, code) in [
        (ScriptErrorKind::Busy, ErrorCode::Busy),
        (ScriptErrorKind::Deadline, ErrorCode::DeadlineExceeded),
        (ScriptErrorKind::Cancelled, ErrorCode::Cancelled),
        (ScriptErrorKind::Retired, ErrorCode::Unavailable),
        (ScriptErrorKind::Contract, ErrorCode::HookFailed),
    ] {
        let failure = Failure::script(ScriptError::new(kind), "before_send", Some("step-1"));
        assert_eq!(failure.code, code);
        assert_eq!(failure.phase, "before_send");
        assert_eq!(failure.step.as_deref(), Some("step-1"));
        assert!(failure.source().unwrap().is::<ScriptError>());
    }
}

#[test]
fn status_override_cannot_turn_failure_into_success_or_redirect() {
    let mut failure = Failure::new(ErrorCode::UpstreamFailed, "after_receive");
    failure.status = Some(429);
    assert_eq!(failure.http_status(), 429);
    for status in [0, 200, 204, 302, 600] {
        failure.status = Some(status);
        assert_eq!(failure.http_status(), 502);
    }
}

#[test]
fn network_error_is_preserved_without_rendering_endpoint_query() {
    let source = reqwest::Client::new()
        .get("https://fictional.invalid:invalid/?token=secret-query")
        .build()
        .unwrap_err();
    let failure = Failure::upstream(source, "upstream_send");
    assert_eq!(failure.code, ErrorCode::UpstreamFailed);
    assert!(failure.source().unwrap().is::<reqwest::Error>());
    assert!(!format!("{failure:?} {failure}").contains("secret-query"));
}

#[test]
fn body_failure_detects_typed_length_limit_without_reading_error_text() {
    use std::{
        pin::Pin,
        task::{Context, Waker},
    };

    use axum::body::HttpBody;

    let mut body = http_body_util::Limited::new(axum::body::Body::from("abc"), 2);
    let mut context = Context::from_waker(Waker::noop());
    let std::task::Poll::Ready(Some(Err(source))) = Pin::new(&mut body).poll_frame(&mut context)
    else {
        panic!("a synthetic over-limit body must yield a length error immediately");
    };
    let failure = Failure::body(axum::Error::new(source));
    assert_eq!(failure.code, ErrorCode::PayloadTooLarge);
    assert!(failure.source().unwrap().is::<axum::Error>());

    let failure = Failure::body(axum::Error::new(io::Error::other("secret body")));
    assert_eq!(failure.code, ErrorCode::InvalidRequest);
    assert!(!format!("{failure:?}").contains("secret body"));
}

#[test]
fn nested_failures_keep_safe_diagnostics_and_explain_unsupported_transport() {
    let inner = Failure::new(ErrorCode::InvalidRequest, "request_body").with_source(
        io::Error::new(io::ErrorKind::InvalidData, "secret-source-body"),
    );
    let outer = Failure::new(ErrorCode::HookFailed, "beforeSend").with_source(inner);
    let diagnostic = outer.to_string();
    assert!(diagnostic.contains("gateway=invalid_request"));
    assert!(diagnostic.contains("io_kind=InvalidData"));
    assert!(!diagnostic.contains("secret-source-body"));
    let transport = Failure::new(ErrorCode::InvalidRequest, "transport");
    assert_eq!(
        transport.error_json("request-1")["error"]["message"],
        "Streaming is not supported by this gateway"
    );
}
