use serde_json::json;

use super::*;
use crate::validate_json;

fn nested(containers: usize) -> String {
    format!("{}null{}", "[".repeat(containers), "]".repeat(containers))
}

#[test]
fn maximum_configured_depth_accepts_128_containers_and_rejects_129() {
    let text = nested(128);
    let value = parse_json_str(&text, 128).unwrap();
    validate_json(&value, 128, 1000).unwrap();
    assert_eq!(parse_json_slice(text.as_bytes(), 128).unwrap(), value);
    assert!(parse_json_str(&nested(129), 128).unwrap_err().is_data());
}

#[test]
fn configured_depth_counts_the_root_as_zero() {
    for text in ["null", "false", "1", "\"text\"", "[]", "{}"] {
        parse_json_str(text, 0).unwrap();
    }
    for text in ["[null]", "{\"child\":null}"] {
        assert!(parse_json_str(text, 0).unwrap_err().is_data());
        parse_json_str(text, 1).unwrap();
    }
    parse_json_str(&nested(8), 8).unwrap();
    assert!(parse_json_str(&nested(9), 8).unwrap_err().is_data());
}

#[test]
fn hostile_nesting_stops_at_the_supported_depth_even_with_an_unlimited_argument() {
    // No closing brackets: hitting EOF would mean the parser descended too far.
    let text = "[".repeat(10_000);
    let error = parse_json_str(&text, usize::MAX).unwrap_err();
    assert!(error.is_data());
    assert_eq!(error.line(), 1);
    assert!(error.column() <= MAX_JSON_DEPTH + 1);
}

#[test]
fn values_duplicates_and_scalar_kinds_keep_serde_json_semantics() {
    let text = r#"{"null":null,"bool":true,"signed":-4,"unsigned":4,"float":0.5,"text":"escaped\n文字","array":[1,{}],"duplicate":[1,2],"duplicate":3}"#;
    let value = parse_json_str(text, 8).unwrap();
    assert_eq!(value, serde_json::from_str::<Value>(text).unwrap());
    assert_eq!(value["duplicate"], 3);
    assert_eq!(
        parse_json_str("{\"x\":[1,2],\"x\":null}", 2).unwrap(),
        json!({"x":null})
    );
}

#[test]
fn tree_node_and_safe_number_validation_remain_separate() {
    let value = parse_json_str("{\"x\":[1,2],\"x\":null}", 2).unwrap();
    validate_json(&value, 2, 2).unwrap();
    assert!(validate_json(&value, 2, 1).is_err());
    for text in ["9007199254740992", "-9007199254740992", "1e100"] {
        let value = parse_json_str(text, 0).unwrap();
        assert!(validate_json(&value, 0, 1).is_err());
    }
}

#[test]
fn syntax_utf8_and_trailing_input_errors_keep_the_original_json_error() {
    for text in ["[1,]", "{}true", "1e400"] {
        let error: serde_json::Error = parse_json_str(text, 8).unwrap_err();
        assert!(error.is_syntax());
        assert!(error.line() > 0 && error.column() > 0);
    }
    assert!(parse_json_str("{\"x\":", 8).unwrap_err().is_eof());
    assert!(
        parse_json_slice(&[b'"', 0xFF, b'"'], 8)
            .unwrap_err()
            .is_syntax()
    );
}
