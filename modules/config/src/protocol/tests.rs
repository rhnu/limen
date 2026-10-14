use super::*;

#[test]
fn status_mapping_preserves_success_and_body_semantics() {
    assert!(validate_response_status(200, 201).is_ok());
    for mapped in [101, 204, 205, 302, 400, 500] {
        assert_eq!(
            validate_response_status(200, mapped),
            Err(ErrorCode::HookFailed)
        );
    }
    assert!(validate_response_status(429, 503).is_ok());
    assert!(validate_response_status(302, 502).is_ok());
    for upstream in [302, 400, 500] {
        assert!(validate_response_status(upstream, 200).is_err());
        assert!(validate_response_status(upstream, 302).is_err());
    }
    assert!(validate_response_status(999, 500).is_err());
}

#[test]
fn public_error_categories_map_to_failure_statuses() {
    assert_eq!(ErrorCode::Unauthorized.status(), 401);
    assert_eq!(ErrorCode::Busy.status(), 429);
    assert_eq!(ErrorCode::DeadlineExceeded.status(), 504);
    assert_eq!(ErrorCode::Cancelled.status(), 499);
    assert_eq!(ErrorCode::UpstreamProtocol.status(), 502);
    assert_eq!(ErrorCode::HookFailed.as_str(), "hook_failed");
}
