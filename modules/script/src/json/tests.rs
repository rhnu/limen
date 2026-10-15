use serde_json::json;

use super::encode_owned;
use crate::{JsonLimits, ScriptErrorKind};

#[test]
fn accepts_safe_json_and_rejects_integer_precision_loss() {
    assert!(
        encode_owned(
            &json!({"a":[null, true, 1.25, 9_007_199_254_740_991_u64]}),
            JsonLimits::default()
        )
        .is_ok()
    );
    assert_eq!(
        encode_owned(&json!(9_007_199_254_740_992_u64), JsonLimits::default())
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
            encode_owned(&value, limits).unwrap_err().kind(),
            ScriptErrorKind::Json
        );
    }
}

#[test]
fn rejects_strings_before_encoded_output_exceeds_its_budget() {
    let limits = JsonLimits {
        max_bytes: 16,
        ..JsonLimits::default()
    };
    assert_eq!(
        encode_owned(&json!("x".repeat(1024)), limits)
            .unwrap_err()
            .kind(),
        ScriptErrorKind::Json
    );
    assert_eq!(
        encode_owned(&json!("\u{0000}".repeat(3)), limits)
            .unwrap_err()
            .kind(),
        ScriptErrorKind::Json
    );
}

#[test]
fn encoding_preserves_utf8_and_escapes_at_the_exact_byte_limit() {
    let expected = "\"世\\n\\\"\"".as_bytes();
    let value = json!("世\n\"");
    let limits = JsonLimits {
        max_bytes: expected.len(),
        ..JsonLimits::default()
    };
    assert_eq!(encode_owned(&value, limits).unwrap(), expected);
    assert_eq!(
        encode_owned(
            &value,
            JsonLimits {
                max_bytes: expected.len() - 1,
                ..limits
            }
        )
        .unwrap_err()
        .kind(),
        ScriptErrorKind::Json
    );
}
