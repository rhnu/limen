use serde_json::json;

use super::*;

fn basic() -> Value {
    json!({"providers": {"primary": {"endpoint": "https://example.invalid/v1/chat/completions?deployment=a"}},
        "models": {"chat": {"provider": "primary", "upstream": "actual-model"}}})
}

fn parse(value: Value) -> Result<BusinessConfig, ConfigError> {
    BusinessConfig::from_value(value, &ResourceLimits::default())
}

#[test]
fn preserves_endpoint_query_defaults_and_hook_order() {
    let mut value = basic();
    value["onRequest"] = json!(0);
    value["providers"]["primary"]["defaults"] = json!({"temperature": null, "extension": [1, "x"]});
    value["providers"]["primary"]["pipeline"] = json!([
        {"name": "first", "beforeSend": 1}, {"name": "second", "onResponse": 2}]);
    let config = parse(value).unwrap();
    let provider = &config.providers["primary"];
    assert!(provider.endpoint.ends_with("?deployment=a"));
    assert_eq!(config.on_request, Some(0));
    assert_eq!(provider.pipeline[0].before_send, Some(1));
    assert_eq!(provider.pipeline[1].on_response, Some(2));
    assert_eq!(provider.defaults["temperature"], Value::Null);
}

#[test]
fn rejects_unsafe_or_repaired_endpoints() {
    for endpoint in [
        "ftp://example.invalid/v1",
        "https://user:secret@example.invalid/v1",
        "https://@example.invalid/v1",
        "https://example.invalid/#fragment",
        "https:example.invalid/v1",
        "https:///example.invalid/v1",
        "https://example.invalid/has space",
        "https://example.invalid\\v1",
        "//example.invalid/v1",
    ] {
        let mut value = basic();
        value["providers"]["primary"]["endpoint"] = json!(endpoint);
        assert!(parse(value).is_err(), "accepted invalid endpoint");
    }
}

#[test]
fn rejects_missing_references_and_unknown_fields_safely() {
    let mut value = basic();
    value["models"]["chat"]["provider"] = json!("secret-name");
    let error = parse(value).err().unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Reference);
    assert!(!error.to_string().contains("secret-name"));
    let mut value = basic();
    value["providers"]["primary"]["secret-key"] = json!("secret-token");
    let error = parse(value).err().unwrap();
    assert!(!format!("{error:?}").contains("secret-"));
}

#[test]
fn validates_case_insensitive_auth_headers_and_reserved_fields() {
    for headers in [
        json!({"aUtHoRiZaTiOn": "Bearer static"}),
        json!({"X-Test": "a", "x-test": "b"}),
        json!({"Content-Length": "2"}),
        json!({"X-Test": "hello\r\ninjected: true"}),
        json!({"bad header": "value"}),
    ] {
        let mut value = basic();
        value["providers"]["primary"]["auth"] = json!({"bearer": "token"});
        value["providers"]["primary"]["headers"] = headers;
        assert!(parse(value).is_err());
    }
    let mut value = basic();
    value["providers"]["primary"]["headers"] = json!({"Authorization": "Custom credential"});
    assert!(parse(value).is_ok());
}

#[test]
fn rejects_defaults_control_fields_on_both_layers() {
    for layer in ["providers", "models"] {
        for key in ["model", "messages", "stream"] {
            let mut value = basic();
            let name = if layer == "providers" {
                "primary"
            } else {
                "chat"
            };
            value[layer][name]["defaults"] = json!({key: null});
            assert!(parse(value).is_err());
        }
    }
}

#[test]
fn steps_require_unique_names_and_known_hooks() {
    for pipeline in [
        json!([{"name": "x"}]),
        json!([{"name": "x", "beforeSend": 1}, {"name": "x", "afterReceive": 2}]),
        json!([{"name": "x", "beforeSend": "function"}]),
        json!([{"name": "x", "unknownStage": 1}]),
    ] {
        let mut value = basic();
        value["models"]["chat"]["pipeline"] = pipeline;
        assert!(parse(value).is_err());
    }
}

#[test]
fn rejects_unsafe_integer_and_deep_json() {
    let mut value = basic();
    value["providers"]["primary"]["defaults"] = json!({"seed": 9_007_199_254_740_992_u64});
    assert_eq!(parse(value).err().unwrap().kind, ConfigErrorKind::Json);
    let mut nested = Value::Null;
    for _ in 0..70 {
        nested = json!([nested]);
    }
    let mut value = basic();
    value["providers"]["primary"]["defaults"] = json!({"extension": nested});
    assert_eq!(parse(value).err().unwrap().kind, ConfigErrorKind::Json);
}

#[test]
fn records_must_be_objects_and_respect_supplied_limits() {
    assert!(parse(json!([{}, {}])).is_err());
    let mut value = basic();
    value["models"]["chat"] = json!(["primary", "model"]);
    assert!(parse(value).is_err());
    let mut value = basic();
    value["providers"]["primary"]["pipeline"] = json!([["step", 1]]);
    assert!(parse(value).is_err());
    let limits = ResourceLimits {
        max_json_nodes: 1,
        ..ResourceLimits::default()
    };
    assert_eq!(
        BusinessConfig::from_value(basic(), &limits)
            .err()
            .unwrap()
            .kind,
        ConfigErrorKind::Json
    );
}
