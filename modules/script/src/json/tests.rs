use serde_json::json;

use super::validate_owned;
use crate::{JsonLimits, ScriptErrorKind};

#[test]
fn accepts_safe_json_and_rejects_integer_precision_loss() {
    assert!(
        validate_owned(
            &json!({"a":[null, true, 1.25, 9_007_199_254_740_991_u64]}),
            JsonLimits::default()
        )
        .is_ok()
    );
    assert_eq!(
        validate_owned(&json!(9_007_199_254_740_992_u64), JsonLimits::default())
            .unwrap_err()
            .kind(),
        ScriptErrorKind::Json
    );
}

#[test]
fn bounds_depth_nodes_and_encoded_size() {
    let value = json!({"a":[1, 2]});
    for limits in [
        JsonLimits {
            max_depth: 1,
            ..JsonLimits::default()
        },
        JsonLimits {
            max_nodes: 3,
            ..JsonLimits::default()
        },
        JsonLimits {
            max_bytes: 5,
            ..JsonLimits::default()
        },
    ] {
        assert_eq!(
            validate_owned(&value, limits).unwrap_err().kind(),
            ScriptErrorKind::Json
        );
    }
}

#[test]
fn rejects_oversized_strings_without_buffering_serialized_output() {
    let limits = JsonLimits {
        max_bytes: 16,
        ..JsonLimits::default()
    };
    assert_eq!(
        validate_owned(&json!("x".repeat(1024)), limits)
            .unwrap_err()
            .kind(),
        ScriptErrorKind::Json
    );
    assert_eq!(
        validate_owned(&json!("\u{0000}".repeat(3)), limits)
            .unwrap_err()
            .kind(),
        ScriptErrorKind::Json
    );
}
