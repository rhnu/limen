use serde_json::json;

use super::*;

#[test]
fn bounded_encoding_and_counting_measure_escaped_utf8_bytes() {
    let value = json!({"text":"\n世界"});
    let expected = r#"{"text":"\n世界"}"#.as_bytes();
    assert_eq!(
        encode(&value, expected.len(), ErrorCode::PayloadTooLarge, "test").unwrap(),
        expected
    );
    assert_eq!(
        size(&value, expected.len(), ErrorCode::PayloadTooLarge, "test").unwrap(),
        expected.len()
    );
    for (failure, expected_code) in [
        (
            encode(
                &value,
                expected.len() - 1,
                ErrorCode::PayloadTooLarge,
                "test",
            )
            .unwrap_err(),
            ErrorCode::PayloadTooLarge,
        ),
        (
            size(&value, expected.len() - 1, ErrorCode::HookFailed, "test").unwrap_err(),
            ErrorCode::HookFailed,
        ),
    ] {
        assert_eq!(failure.phase, "test");
        assert_eq!(failure.code, expected_code);
    }
}

#[test]
fn oversized_json_fails_with_the_callers_policy_category() {
    let failure = encode(
        &json!("x".repeat(1000)),
        16,
        ErrorCode::PayloadTooLarge,
        "upstream_body",
    )
    .unwrap_err();
    assert_eq!(failure.code, ErrorCode::PayloadTooLarge);
    assert_eq!(failure.phase, "upstream_body");
    assert_eq!(
        size(&json!(null), 0, ErrorCode::HookFailed, "response")
            .unwrap_err()
            .code,
        ErrorCode::HookFailed
    );
}
