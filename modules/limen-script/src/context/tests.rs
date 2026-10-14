use super::header_name;

#[test]
fn normalizes_names_without_losing_value_boundaries() {
    assert_eq!(
        header_name("X-Example", "a,b", true).as_deref(),
        Some("x-example")
    );
    assert!(header_name("Authorization", "Bearer synthetic", true).is_some());
    assert!(header_name("X-Empty", "", true).is_some());
}

#[test]
fn rejects_invalid_http_names_and_values() {
    for name in ["", "not a name", "x:field", "x\nfield", "非ASCII"] {
        assert!(header_name(name, "value", true).is_none());
    }
    for value in ["line\r\nbreak", "nul\0value", "control\u{7f}"] {
        assert!(header_name("x-example", value, true).is_none());
    }
}

#[test]
fn transport_fields_allow_inspection_and_reject_mutation() {
    for name in [
        "Host",
        "CONTENT-Length",
        "Transfer-Encoding",
        "Connection",
        "Content-Encoding",
        "Content-Type",
        "Proxy-Connection",
        "Keep-Alive",
        "TE",
        "Trailer",
        "Upgrade",
    ] {
        assert!(header_name(name, "value", false).is_some());
        assert!(header_name(name, "value", true).is_none());
    }
}
