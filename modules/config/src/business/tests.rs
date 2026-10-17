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
fn model_aliases_accept_colons_and_preserve_distinct_mappings() {
    for alias in [
        "Vendor/model_v1.2:vertex:flex".to_owned(),
        format!("{}:", "a".repeat(127)),
    ] {
        let mut value = basic();
        value["models"][&alias] = json!({"provider": "primary", "upstream": "variant-model"});
        let config = parse(value).unwrap();
        assert_eq!(config.models[&alias].upstream, "variant-model");
        assert_eq!(config.models["chat"].upstream, "actual-model");
    }
}

#[test]
fn model_aliases_still_reject_invalid_characters_and_lengths() {
    for alias in [
        String::new(),
        "chat:has space".to_owned(),
        "chat:flex\n".to_owned(),
        "chat:\0flex".to_owned(),
        "chat:模型".to_owned(),
        "chat@flex".to_owned(),
        format!("{}:", "a".repeat(128)),
    ] {
        let mut value = basic();
        value["models"][&alias] = json!({"provider": "primary", "upstream": "variant-model"});
        let error = parse(value).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        assert_eq!(error.field, "models.*");
    }
}

#[test]
fn colons_remain_invalid_in_provider_and_step_names() {
    let mut value = basic();
    let provider = value["providers"]["primary"].clone();
    value["providers"] = json!({"primary:region": provider});
    value["models"]["chat"]["provider"] = json!("primary:region");
    let error = parse(value).err().unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Schema);
    assert_eq!(error.field, "providers.*");

    for (layer, name) in [("providers", "primary"), ("models", "chat")] {
        let mut value = basic();
        value[layer][name]["pipeline"] = json!([{"name": "step:flex", "beforeSend": 1}]);
        let error = parse(value).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        assert_eq!(error.field, format!("{layer}.*.pipeline"));
    }
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
fn error_response_hooks_register_globally_and_as_the_only_pipeline_phase() {
    let mut value = basic();
    value["onErrorResponse"] = json!(1);
    value["providers"]["primary"]["pipeline"] =
        json!([{"name": "provider-error", "onErrorResponse": 2}]);
    value["models"]["chat"]["pipeline"] = json!([{"name": "model-error", "onErrorResponse": 3}]);
    let config = parse(value).unwrap();
    assert_eq!(config.on_error_response, Some(1));
    assert_eq!(
        config.providers["primary"].pipeline[0].on_error_response,
        Some(2)
    );
    assert_eq!(config.models["chat"].pipeline[0].on_error_response, Some(3));
}

#[test]
fn error_response_hook_types_report_safe_schema_paths() {
    for invalid in [Value::Null, json!("private-function"), json!({}), json!(-1)] {
        let mut value = basic();
        value["onErrorResponse"] = invalid.clone();
        let error = parse(value).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        assert_eq!(error.field, "onErrorResponse");
        for (layer, name) in [("providers", "primary"), ("models", "chat")] {
            let mut value = basic();
            value[layer][name]["pipeline"] =
                json!([{"name": "private-step", "onErrorResponse": invalid}]);
            let error = parse(value).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Schema);
            assert_eq!(error.field, format!("{layer}.*.pipeline.*.onErrorResponse"));
            assert!(!format!("{error:?}").contains("private-"));
        }
    }
}

#[test]
fn content_hooks_register_as_the_only_pipeline_phase_and_coexist_with_transport_hooks() {
    let mut value = basic();
    value["providers"]["primary"]["pipeline"] = json!([{"name": "content", "onContent": 1}]);
    value["models"]["chat"]["pipeline"] = json!([{
        "name": "content", "onContent": 2, "onResponse": 3,
        "onUpstreamEvent": 4, "onStreamEvent": 5
    }]);
    let config = parse(value).unwrap();
    assert_eq!(config.providers["primary"].pipeline[0].on_content, Some(1));
    let model = &config.models["chat"].pipeline[0];
    assert_eq!(model.on_content, Some(2));
    assert_eq!(model.on_response, Some(3));
    assert_eq!(model.on_upstream_event, Some(4));
    assert_eq!(model.on_stream_event, Some(5));
}

#[test]
fn content_hook_types_report_safe_schema_paths() {
    for invalid in [Value::Null, json!("private-function"), json!({}), json!(-1)] {
        for (layer, name) in [("providers", "primary"), ("models", "chat")] {
            let mut value = basic();
            value[layer][name]["pipeline"] =
                json!([{"name": "private-step", "onContent": invalid}]);
            let error = parse(value).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Schema);
            assert_eq!(error.field, format!("{layer}.*.pipeline.*.onContent"));
            assert!(!format!("{error:?}").contains("private-"));
        }
    }
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

#[test]
fn business_schema_error_retains_json_source_with_redacted_path() {
    use std::error::Error;

    let mut value = basic();
    value["providers"]["primary"]["secret-key"] = json!("secret-value");
    let error = parse(value).err().unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Schema);
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<serde_json::Error>()
        .unwrap();
    assert!(source.is_data());
    assert!(source.to_string().contains("secret-key"));
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("deserialize business configuration"));
    assert!(diagnostic.contains("JSON (Data"));
    let rendered = format!("{error} {error:?} {diagnostic}");
    assert!(!rendered.contains("secret-key"));
    assert!(!rendered.contains("secret-value"));
    assert!(!rendered.contains("primary"));
}

#[test]
fn endpoint_parser_error_retains_source_without_query() {
    use std::error::Error;

    let mut value = basic();
    value["providers"]["primary"]["endpoint"] = json!("https://[invalid]/?token=secret-query");
    let error = parse(value).err().unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Endpoint);
    assert_eq!(error.field, "providers.*.endpoint");
    assert_eq!(
        error.source().unwrap().downcast_ref::<url::ParseError>(),
        Some(&url::ParseError::InvalidIpv6Address)
    );
    assert!(error.diagnostic().to_string().contains("invalid URL"));
    assert!(!format!("{error} {error:?} {}", error.diagnostic()).contains("secret-query"));
}

#[test]
fn header_parser_errors_keep_name_and_value_sources_separate() {
    use std::error::Error;

    let mut value = basic();
    value["providers"]["primary"]["headers"] = json!({"secret header": "secret-token"});
    let error = parse(value).err().unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Header);
    assert!(
        error
            .source()
            .unwrap()
            .is::<http::header::InvalidHeaderName>()
    );
    assert!(
        error
            .diagnostic()
            .to_string()
            .contains("invalid header name")
    );
    assert!(!format!("{error:?}").contains("secret"));

    let mut value = basic();
    value["providers"]["primary"]["headers"] =
        json!({"x-secret": "secret-token\r\nCookie: private-cookie"});
    let error = parse(value).err().unwrap();
    assert!(
        error
            .source()
            .unwrap()
            .is::<http::header::InvalidHeaderValue>()
    );
    assert!(
        error
            .diagnostic()
            .to_string()
            .contains("invalid header value")
    );
    let rendered = format!("{error} {error:?} {}", error.diagnostic());
    assert!(!rendered.contains("x-secret"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("private-cookie"));
}
