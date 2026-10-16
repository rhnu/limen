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
