use super::*;

fn config() -> BusinessConfig {
    BusinessConfig::from_value(json!({
        "providers": {"primary": {"endpoint": "https://fictional.invalid/custom/chat?static=1", "auth": {"bearer": "upstream-token"}, "headers": {"X-App": "limen"}, "defaults": {"temperature": 0.8, "nested": {"provider": true}, "only_provider": true}}},
        "models": {"chat": {"provider": "primary", "upstream": "real-model", "defaults": {"temperature": 0.4, "nested": {"model": true}, "only_model": true}}}
    }), &ResourceLimits::default()).unwrap()
}

#[test]
fn client_auth_handles_case_and_multiple_values_without_forwarding_credentials() {
    let auth = ClientAuth::Bearer("client-token".to_owned());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("bEaReR  client-token"),
    );
    headers.insert(header::COOKIE, HeaderValue::from_static("secret-cookie"));
    assert!(authorize(&auth, &headers).is_ok());
    let outgoing = outbound_headers(&config().providers["primary"]).unwrap();
    assert_eq!(outgoing[header::AUTHORIZATION], "Bearer upstream-token");
    assert_eq!(outgoing[header::ACCEPT], "application/json");
    assert!(!outgoing.contains_key(header::COOKIE));
    headers.append(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer client-token"),
    );
    assert_eq!(
        authorize(&auth, &headers).unwrap_err().code,
        ErrorCode::Unauthorized
    );
}

#[test]
fn rejects_missing_malformed_and_wrong_authentication() {
    let auth = ClientAuth::Bearer("client-token".to_owned());
    assert_eq!(
        authorize(&auth, &HeaderMap::new())
            .unwrap_err()
            .http_status(),
        401
    );
    for value in [
        "Basic client-token",
        "Bearer",
        "Bearer client token",
        "Bearer client-token ",
        "Bearer\tclient-token",
        "Bearer different---",
        "Bearer =",
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        assert_eq!(
            authorize(&auth, &headers).unwrap_err().code,
            ErrorCode::Unauthorized
        );
    }
    assert!(authorize(&ClientAuth::None, &HeaderMap::new()).is_ok());
}

#[test]
fn outbound_headers_preserve_explicit_static_auth_and_accept() {
    let config = BusinessConfig::from_value(
        json!({
            "providers":{"p":{"endpoint":"https://fictional.invalid/chat",
                "headers":{"aUtHoRiZaTiOn":"Custom credential", "Accept":"text/event-stream",
                    "X-Item":"one,two"}}},
            "models":{"chat":{"provider":"p","upstream":"real"}}
        }),
        &ResourceLimits::default(),
    )
    .unwrap();
    let headers = outbound_headers(&config.providers["p"]).unwrap();
    assert_eq!(headers[header::AUTHORIZATION], "Custom credential");
    assert_eq!(headers[header::ACCEPT], "text/event-stream");
    assert_eq!(headers["x-item"], "one,two");
    assert!(!headers.contains_key(header::COOKIE));
}

#[test]
fn shallow_defaults_preserve_null_and_unknown_client_fields() {
    let config = config();
    let client = json!({"model": "chat", "messages": [{"role": "user", "content": "hi"}], "temperature": null, "nested": {"client": true}, "extension": {"unknown": 1}});
    let merged = merge_body(&config.providers["primary"], &config.models["chat"], client).unwrap();
    assert_eq!(merged["model"], "real-model");
    assert!(merged["temperature"].is_null());
    assert_eq!(merged["nested"], json!({"client": true}));
    assert_eq!(merged["extension"], json!({"unknown": 1}));
    assert_eq!(merged["only_provider"], true);
    assert_eq!(merged["only_model"], true);
    let omitted = json!({"model": "chat", "messages": [{}]});
    assert_eq!(
        merge_body(
            &config.providers["primary"],
            &config.models["chat"],
            omitted
        )
        .unwrap()["temperature"],
        0.4
    );
}

#[test]
fn request_preserves_extensions_and_freezes_original_stream_flag() {
    let input = br#"{"model":"chat","messages":[{"custom":"anything"}],"stream":true,"unknown":{"nullable":null}}"#;
    let (mut body, stream) = parse_request(input, &ResourceLimits::default()).unwrap();
    body["stream"] = json!(false);
    assert!(stream);
    assert_eq!(body["unknown"]["nullable"], Value::Null);
    let (_, absent) = parse_request(
        br#"{"model":"chat","messages":[{}]}"#,
        &ResourceLimits::default(),
    )
    .unwrap();
    assert!(!absent);
}

#[test]
fn request_rejects_invalid_shape_numbers_depth_and_size() {
    let limits = ResourceLimits::default();
    for body in [
        json!([]),
        json!({"model":"","messages":[{}]}),
        json!({"model":"a\n","messages":[{}]}),
        json!({"model":"chat","messages":[]}),
        json!({"model":"chat","messages":[null]}),
        json!({"model":"chat","messages":[{}],"stream":null}),
        json!({"model":"chat","messages":[{}],"x":9_007_199_254_740_992_u64}),
    ] {
        assert_eq!(
            validate_body(&body, &limits).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let body = json!({"model":"chat","messages":[{}]});
    let tiny = ResourceLimits {
        max_body_bytes: 2,
        ..limits
    };
    assert_eq!(
        parse_request(b"{} ", &tiny).unwrap_err().code,
        ErrorCode::PayloadTooLarge
    );
    let shallow = ResourceLimits {
        max_json_depth: 1,
        ..limits
    };
    assert!(validate_body(&body, &shallow).is_err());
    let few_nodes = ResourceLimits {
        max_json_nodes: 3,
        ..limits
    };
    assert!(validate_body(&body, &few_nodes).is_err());
}

#[test]
fn header_draft_preserves_case_insensitive_multiple_values() {
    let draft = json!([{"name":"X-Item","value":"first"},{"name":"x-item","value":"second"},{"name":"Authorization","value":"Bearer changed"}]);
    let headers = headers_from_json(&draft).unwrap();
    assert_eq!(
        headers.get_all("X-ITEM").iter().collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let round_trip = headers_from_json(&headers_to_json(&headers).unwrap()).unwrap();
    assert_eq!(round_trip, headers);
    for name in [
        "Host",
        "Content-Type",
        "Content-Length",
        "Connection",
        "Transfer-Encoding",
        "Content-Encoding",
        "Upgrade",
        "Trailer",
        "TE",
    ] {
        assert!(headers_from_json(&json!([{"name":name,"value":"value"}])).is_err());
    }
    assert!(headers_from_json(&json!([{"name":"bad header","value":"x"}])).is_err());
    assert!(headers_from_json(&json!([{"name":"x-item","value":"x\r\ny"}])).is_err());
}

#[test]
fn response_header_policy_keeps_metadata_and_drops_sensitive_transport_fields() {
    let mut upstream = HeaderMap::new();
    for (name, value) in [
        ("content-language", "en"),
        ("x-request-id", "upstream-id"),
        ("retry-after", "2"),
        ("set-cookie", "secret-cookie"),
        ("authorization", "secret-token"),
        ("content-type", "application/json"),
        ("content-length", "999"),
        ("connection", "x-private"),
        ("x-private", "internal"),
    ] {
        upstream.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static(value),
        );
    }
    upstream.append("content-language", HeaderValue::from_static("zh"));
    let headers = response_headers(&upstream);
    assert_eq!(headers.len(), 4);
    assert_eq!(
        headers
            .get_all("content-language")
            .iter()
            .collect::<Vec<_>>(),
        vec!["en", "zh"]
    );
    assert_eq!(headers["retry-after"], "2");
}

#[test]
fn non_utf8_response_metadata_fails_instead_of_changing_header_bytes() {
    let mut upstream = HeaderMap::new();
    upstream.insert("x-request-id", HeaderValue::from_bytes(&[0xFF]).unwrap());
    let retained = response_headers(&upstream);
    let failure = headers_to_json(&retained).unwrap_err();
    assert_eq!(failure.code, ErrorCode::UpstreamProtocol);
    assert!(
        std::error::Error::source(&failure)
            .unwrap()
            .is::<std::str::Utf8Error>()
    );
}

#[test]
fn connection_nominated_metadata_is_hop_by_hop_and_is_not_forwarded() {
    let mut upstream = HeaderMap::new();
    upstream.insert("x-request-id", HeaderValue::from_static("internal-id"));
    upstream.insert("retry-after", HeaderValue::from_static("2"));
    upstream.append(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    upstream.append(
        header::CONNECTION,
        HeaderValue::from_static(" X-Request-ID , Retry-After "),
    );
    assert!(response_headers(&upstream).is_empty());
}

#[test]
fn endpoint_uses_complete_url_and_diagnostics_remove_secrets() {
    let valid = "https://fictional.invalid:8443/custom/chat?token=hidden";
    assert!(validate_url(valid).is_ok());
    assert_eq!(safe_target(valid), "https://fictional.invalid:8443");
    assert_eq!(
        safe_target("https://user:secret@fictional.invalid/secret-path?token=hidden#secret"),
        "https://fictional.invalid"
    );
    assert_eq!(safe_target("not a URL secret"), "invalid_endpoint");
    for invalid in [
        "ftp://fictional.invalid/chat",
        "https://secret@fictional.invalid/chat",
        "https://fictional.invalid/#secret",
        "https:\\fictional.invalid/chat",
    ] {
        let failure = validate_url(invalid).unwrap_err();
        assert_eq!(failure.code, ErrorCode::HookFailed);
        assert!(!format!("{failure:?}").contains("secret"));
    }
}

#[test]
fn model_list_contains_only_explicit_aliases() {
    let output = models_json(&config());
    assert_eq!(output["object"], "list");
    assert_eq!(output["data"].as_array().unwrap().len(), 1);
    assert_eq!(output["data"][0]["id"], "chat");
    assert!(!output.to_string().contains("real-model"));
    assert!(!output.to_string().contains("fictional.invalid"));
}
