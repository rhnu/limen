use super::*;

#[test]
fn client_context_selects_multivalue_headers_and_ignores_connection_fields() {
    let mut headers = HeaderMap::new();
    headers.append("x-profile", HeaderValue::from_static("one"));
    headers.append("x-profile", HeaderValue::from_static("two"));
    headers.insert("x-hop", HeaderValue::from_static("private-hop"));
    headers.insert("connection", HeaderValue::from_static("keep-alive, X-Hop"));
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer synthetic-secret"),
    );
    headers.insert("cookie", HeaderValue::from_static("synthetic-cookie"));
    headers.insert("x-other", HeaderValue::from_static("unselected"));
    let client = client_context(
        &headers,
        &["x-profile".into(), "x-hop".into(), "x-missing".into()],
    )
    .unwrap();
    assert_eq!(
        client.headers,
        vec![
            HeaderField {
                name: "x-profile".into(),
                value: "one".into()
            },
            HeaderField {
                name: "x-profile".into(),
                value: "two".into()
            },
        ]
    );
    assert_eq!(
        client_context(&headers, &[]).unwrap().headers,
        [] as [HeaderField; 0]
    );
}

#[test]
fn selected_client_header_bytes_must_be_utf8_and_bounded() {
    let mut headers = HeaderMap::new();
    headers.insert("x-binary", HeaderValue::from_bytes(&[255]).unwrap());
    assert_eq!(
        client_context(&headers, &["x-binary".into()])
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(client_context(&headers, &[]).is_ok());
    headers.insert(
        "x-large",
        HeaderValue::from_str(&"x".repeat(32 * 1024)).unwrap(),
    );
    assert_eq!(
        client_context(&headers, &["x-large".into()])
            .unwrap_err()
            .code,
        ErrorCode::PayloadTooLarge
    );
    let mut many = HeaderMap::new();
    for _ in 0..101 {
        many.append("x-many", HeaderValue::from_static("v"));
    }
    assert_eq!(
        client_context(&many, &["x-many".into()]).unwrap_err().code,
        ErrorCode::PayloadTooLarge
    );
}

fn config() -> BusinessConfig {
    BusinessConfig::from_value(json!({
        "providers": {"primary": {"endpoint": "https://fictional.invalid/custom/chat?static=1", "auth": {"bearer": "upstream-token"}, "headers": {"X-App": "limen"}, "defaults": {"temperature": 0.8, "nested": {"provider": true}, "only_provider": true}}},
        "models": {"chat": {"provider": "primary", "upstream": "real-model", "defaults": {"temperature": 0.4, "nested": {"model": true}, "only_model": true}}}
    }), &ResourceLimits::default()).unwrap()
}

#[test]
fn request_parser_accepts_the_configured_maximum_json_depth() {
    let limits = ResourceLimits {
        max_json_depth: 128,
        ..ResourceLimits::default()
    };
    let mut nested = Value::Null;
    for _ in 0..127 {
        nested = Value::Array(vec![nested]);
    }
    let mut body = json!({"model":"chat", "messages":[{}], "extension":nested});
    let (parsed, stream) = parse_request(&serde_json::to_vec(&body).unwrap(), &limits).unwrap();
    assert_eq!(parsed, body);
    assert!(!stream);
    body["extension"] = Value::Array(vec![body["extension"].take()]);
    let failure = parse_request(&serde_json::to_vec(&body).unwrap(), &limits).unwrap_err();
    assert_eq!(failure.code, ErrorCode::InvalidRequest);
    assert_eq!(failure.phase, "request_body");
    assert!(
        std::error::Error::source(&failure)
            .unwrap()
            .is::<serde_json::Error>()
    );
}

#[test]
fn request_tree_limits_keep_last_wins_duplicate_key_semantics() {
    let limits = ResourceLimits {
        max_json_depth: 4,
        ..ResourceLimits::default()
    };
    let discarded = format!("{}null{}", "[".repeat(8), "]".repeat(8));
    let bytes = format!(
        "{{\"model\":\"chat\",\"messages\":[{{}}],\"extension\":{discarded},\"extension\":null}}"
    );
    let (body, _) = parse_request(bytes.as_bytes(), &limits).unwrap();
    assert!(body["extension"].is_null());
}

#[test]
fn client_auth_handles_case_and_multiple_values_without_forwarding_credentials() {
    let auth = "client-token";
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("bEaReR  client-token"),
    );
    headers.insert(header::COOKIE, HeaderValue::from_static("secret-cookie"));
    assert!(authorize(auth, &headers).is_ok());
    let outgoing = outbound_headers(&config().providers["primary"]).unwrap();
    assert_eq!(outgoing[header::AUTHORIZATION], "Bearer upstream-token");
    assert!(!outgoing.contains_key(header::ACCEPT));
    assert!(!outgoing.contains_key(header::COOKIE));
    headers.append(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer client-token"),
    );
    assert_eq!(
        authorize(auth, &headers).unwrap_err().code,
        ErrorCode::Unauthorized
    );
}

#[test]
fn rejects_missing_malformed_and_wrong_authentication() {
    let auth = "client-token";
    assert_eq!(
        authorize(auth, &HeaderMap::new())
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
            authorize(auth, &headers).unwrap_err().code,
            ErrorCode::Unauthorized
        );
    }
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
    let draft = vec![
        field("X-Item", "first"),
        field("x-item", "second"),
        field("Authorization", "Bearer changed"),
    ];
    let headers = headers_from_fields(&draft).unwrap();
    assert_eq!(
        headers.get_all("X-ITEM").iter().collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let round_trip = headers_from_fields(&header_fields(&headers).unwrap()).unwrap();
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
        assert!(headers_from_fields(&[field(name, "value")]).is_err());
    }
    assert!(headers_from_fields(&[field("bad header", "x")]).is_err());
    assert!(headers_from_fields(&[field("x-item", "x\r\ny")]).is_err());
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
    let failure = header_fields(&retained).unwrap_err();
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
    let mut config = config();
    config
        .models
        .insert("chat:vertex:flex".to_owned(), config.models["chat"].clone());
    let output = models_json(&config);
    assert_eq!(output["object"], "list");
    assert_eq!(output["data"].as_array().unwrap().len(), 2);
    assert_eq!(output["data"][0]["id"], "chat");
    assert_eq!(output["data"][1]["id"], "chat:vertex:flex");
    assert!(!output.to_string().contains("real-model"));
    assert!(!output.to_string().contains("fictional.invalid"));
}

fn field(name: &str, value: &str) -> HeaderField {
    HeaderField {
        name: name.into(),
        value: value.into(),
    }
}
