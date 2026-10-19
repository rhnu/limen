use serde_json::json;

use super::*;

#[test]
fn accept_matches_final_upstream_stream_and_preserves_explicit_overrides() {
    for stream in [false, true] {
        let body = json!({"model":"wire","messages":[{"role":"user","content":[]}],"stream":stream,"extension":{"enabled":true}});
        let mut headers = HeaderMap::new();
        let bytes = encode_request(&body, &mut headers, &ResourceLimits::default()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), body);
        assert_eq!(
            headers[header::ACCEPT],
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            }
        );
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/custom"),
        );
        encode_request(&body, &mut headers, &ResourceLimits::default()).unwrap();
        assert_eq!(headers[header::ACCEPT], "application/custom");
    }
}

#[test]
fn wire_encoding_keeps_chat_scope_and_resource_bounds() {
    let limits = ResourceLimits::default();
    assert!(encode_request(&json!({"contents":[]}), &mut HeaderMap::new(), &limits).is_err());
    let body = json!({"model":"wire","messages":[{}]});
    let limits = ResourceLimits {
        max_output_bytes: 1,
        ..limits
    };
    assert_eq!(
        encode_request(&body, &mut HeaderMap::new(), &limits)
            .unwrap_err()
            .code,
        ErrorCode::PayloadTooLarge
    );
}

#[test]
fn native_control_output_is_bounded_even_without_business_hooks() {
    for bytes in [b": heartbeat\n\n".as_slice(), b"id: end\ndata: [DONE]\n\n"] {
        let mut decoder = FrameDecoder::new(1024);
        let mut frame = None;
        for byte in bytes {
            if let Some(value) = decoder.push(*byte).unwrap() {
                frame = Some(value);
            }
        }
        let frame = frame.unwrap();
        let limits = ResourceLimits {
            max_output_bytes: bytes.len() - 1,
            ..ResourceLimits::default()
        };
        assert!(control_frame(&frame, &limits).is_err());
        let limits = ResourceLimits {
            max_output_bytes: bytes.len(),
            ..limits
        };
        assert_eq!(control_frame(&frame, &limits).unwrap(), bytes);
    }
}

#[test]
fn upstream_error_payload_is_preserved_but_only_numeric_status_is_diagnostic() {
    for code in [json!(429), json!("secret-code"), json!(200), json!(70000)] {
        let value = json!({"error":{"code":code,"message":"secret upstream detail",
            "metadata":{"raw":"secret body","authorization":"secret token","id":u64::MAX}}});
        let data = value.to_string();
        for prefix in ["", "event: error\n"] {
            let bytes = format!("{prefix}data: {data}\n\n");
            let mut decoder = FrameDecoder::new(4096);
            let frame = bytes
                .bytes()
                .find_map(|byte| decoder.push(byte).unwrap())
                .unwrap();
            let failure = interpret_frame(&frame, &ResourceLimits::default()).unwrap_err();
            assert_eq!(failure.phase, "upstream_sse");
            assert_eq!(failure.response.as_ref().unwrap().body, data.as_bytes());
            assert_eq!(
                failure.upstream_error_status,
                if code == 429 { Some(429) } else { None }
            );
            let diagnostic = failure.to_string();
            assert!(diagnostic.contains("error event"));
            assert!(!diagnostic.contains("secret"));
            assert!(
                !failure
                    .observer_error("test")
                    .to_string()
                    .contains("secret")
            );
        }
    }
}

#[test]
fn invalid_or_oversized_native_error_payloads_do_not_bypass_bounds() {
    for (data, limits) in [
        ("{broken", ResourceLimits::default()),
        (
            r#"{"error":{"message":"secret"}}"#,
            ResourceLimits {
                max_output_bytes: 8,
                ..ResourceLimits::default()
            },
        ),
        (
            r#"{"error":{"message":"secret"}}"#,
            ResourceLimits {
                max_json_nodes: 2,
                ..ResourceLimits::default()
            },
        ),
    ] {
        let bytes = format!("event: error\ndata: {data}\n\n");
        let mut decoder = FrameDecoder::new(4096);
        let frame = bytes
            .bytes()
            .find_map(|byte| decoder.push(byte).unwrap())
            .unwrap();
        let failure = interpret_frame(&frame, &limits).unwrap_err();
        assert!(failure.response.is_none());
        assert!(!failure.to_string().contains("secret"));
    }
}
