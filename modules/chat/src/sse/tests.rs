use serde_json::json;

use super::*;

fn frames(input: &[u8], limit: usize) -> Result<Vec<Frame>, Error> {
    let mut decoder = FrameDecoder::new(limit);
    let mut frames = Vec::new();
    for &byte in input {
        if let Some(frame) = decoder.push(byte)? {
            frames.push(frame);
        }
    }
    decoder.finish()?;
    Ok(frames)
}

#[test]
fn data_at_the_shared_depth_limit_can_be_interpreted_and_preserved() {
    let mut extension = Value::Null;
    for _ in 0..127 {
        extension = Value::Array(vec![extension]);
    }
    let value = json!({"choices":[], "extension":extension});
    let input = format!("data: {value}\n\n");
    let frame = frames(input.as_bytes(), 1024).unwrap().remove(0);
    assert_eq!(frame.event().unwrap(), Event::Chunk(value.clone()));
    assert_eq!(frame.encode(&value, 1024).unwrap(), input.as_bytes());
    let input = format!("data: {{\"extension\":[{}]}}\n\n", value["extension"]);
    let frame = frames(input.as_bytes(), 1024).unwrap().remove(0);
    assert!(matches!(frame.event(), Err(Error::Json(_))));
}

#[test]
fn preserves_raw_fields_comments_and_named_chat_events() {
    let input = b": heartbeat\nid: 001\nretry: 00042\nevent: custom\nx-extra: keep\ndata: {\ndata: \"choices\": []}\n\n";
    let output = frames(input, 1024).unwrap();
    assert_eq!(output.len(), 1);
    let frame = &output[0];
    assert_eq!(frame.raw(), input);
    assert_eq!(frame.event, "custom");
    assert_eq!(frame.id.as_deref(), Some("001"));
    assert_eq!(frame.retry.as_deref(), Some("00042"));
    let Event::Chunk(value) = frame.event().unwrap() else {
        panic!("expected Chat object")
    };
    assert_eq!(value, json!({"choices": []}));
    assert_eq!(frame.encode(&value, 1024).unwrap(), input);
    let updated = frame
        .encode(&json!({"choices": [], "model": "alias"}), 1024)
        .unwrap();
    assert_eq!(updated, b": heartbeat\nid: 001\nretry: 00042\nevent: custom\nx-extra: keep\ndata: {\"choices\":[],\"model\":\"alias\"}\n\n");
    assert!(matches!(
        frame.encode(&value, input.len() - 1),
        Err(Error::EventLimit)
    ));
    assert!(matches!(
        frame.encode(&json!({"model":"alias"}), 10),
        Err(Error::EventLimit)
    ));
}

#[test]
fn unicode_bom_multiline_and_all_endings_preserve_raw_at_every_network_split() {
    for ending in ["\n", "\r\n", "\r"] {
        let input = format!(
            "\u{feff}: 心跳{ending}{ending}id: 1{ending}data: {{\"choices\": [],{ending}data: \"text\": \"世界🌍\"}}{ending}{ending}"
        );
        let expected = frames(input.as_bytes(), 1024).unwrap();
        assert_eq!(
            expected
                .iter()
                .flat_map(Frame::raw)
                .copied()
                .collect::<Vec<_>>(),
            input.as_bytes()
        );
        let data = expected.iter().find(|frame| frame.data.is_some()).unwrap();
        assert_eq!(
            data.event().unwrap(),
            Event::Chunk(json!({"choices": [], "text":"世界🌍"}))
        );
        for split in 0..=input.len() {
            let mut decoder = FrameDecoder::new(1024);
            let mut actual = Vec::new();
            for block in [&input.as_bytes()[..split], &input.as_bytes()[split..]] {
                for &byte in block {
                    if let Some(frame) = decoder.push(byte).unwrap() {
                        actual.push(frame);
                    }
                }
            }
            decoder.finish().unwrap();
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn framing_completion_is_independent_of_chat_done_and_errors() {
    assert!(frames(b"", 10).is_ok());
    assert!(frames(b"data: {}\n\n", 100).is_ok());
    assert!(matches!(
        frames(b"data: [DONE]\n\npartial", 100),
        Err(Error::Truncated)
    ));
    assert!(matches!(
        frames(b"data: [DONE]\n", 100),
        Err(Error::Truncated)
    ));
    assert!(matches!(
        frames(b"data: \xff\n\n", 100),
        Err(Error::Utf8(_))
    ));
    for bytes in [
        b"event: error\n\n".as_slice(),
        b"event: error\ndata: [DONE]\n\n",
        b"data: {\"error\": {}}\n\n",
    ] {
        let frame = frames(bytes, 100).unwrap().remove(0);
        assert!(matches!(frame.event(), Err(Error::UpstreamError)));
    }
    let data_free = frames(b": heartbeat\nid: 1\nretry: 12\n\n", 100).unwrap();
    assert_eq!(data_free[0].event().unwrap(), Event::KeepAlive);
}

#[test]
fn raw_bounds_include_comments_and_unterminated_lines() {
    let input = b": a\n: b\n\n";
    assert_eq!(frames(input, input.len()).unwrap()[0].raw(), input);
    assert!(matches!(
        frames(input, input.len() - 1),
        Err(Error::EventLimit)
    ));
    assert!(matches!(
        frames(b"data: never-ending", 10),
        Err(Error::EventLimit)
    ));
    let mut decoder = FrameDecoder::new(1);
    decoder.push(b'a').unwrap();
    assert!(matches!(decoder.push(b'b'), Err(Error::EventLimit)));
    assert!(matches!(decoder.push(b'\n'), Err(Error::InvalidChunk)));
    assert!(decoder.finish().is_err());
}

#[test]
fn data_reencoding_keeps_crlf_outer_fields_and_stays_bounded() {
    for ending in ["\r", "\r\n", "\n"] {
        let input = format!(
            "\u{feff}data: {{\"model\": \"old\"}}{ending}: keep{ending}event: named{ending}{ending}"
        );
        let original = frames(input.as_bytes(), 1024).unwrap();
        let mut changed = Vec::new();
        for frame in original {
            if frame.data.is_some() {
                changed.extend(frame.encode(&json!({"model":"new"}), 1024).unwrap());
            } else {
                changed.extend_from_slice(frame.raw());
            }
        }
        let parsed = frames(&changed, 1024).unwrap();
        let frame = parsed.iter().find(|frame| frame.data.is_some()).unwrap();
        assert_eq!(frame.event, "named");
        assert_eq!(frame.event().unwrap(), Event::Chunk(json!({"model":"new"})));
        assert!(String::from_utf8(changed).unwrap().contains(": keep"));
    }
}

#[test]
fn transport_defaults_and_repeated_fields_follow_sse_rules() {
    let output = frames(
        b"event: named\nevent:\nid: first\nid: invalid\0id\nretry: +not-a-delay\ndata: {}\n\n",
        1024,
    )
    .unwrap();
    let frame = &output[0];
    assert_eq!(frame.event, "message");
    assert_eq!(frame.id.as_deref(), Some("first"));
    assert_eq!(frame.retry.as_deref(), Some("+not-a-delay"));
    assert_eq!(frame.event().unwrap(), Event::Chunk(json!({})));
}

#[test]
fn rewritten_data_counts_bom_unicode_escapes_and_outer_framing() {
    let frame = frames(b"\xef\xbb\xbfdata: {}\n: keep\n\n", 1024)
        .unwrap()
        .remove(0);
    let value = json!({"model":"世\n🌍"});
    let output = frame.encode(&value, 1024).unwrap();
    assert!(output.starts_with(b"\xef\xbb\xbfdata: "));
    assert_eq!(frame.encode(&value, output.len()).unwrap(), output);
    assert!(matches!(
        frame.encode(&value, output.len() - 1),
        Err(Error::EventLimit)
    ));
}

#[test]
fn only_the_stream_initial_bom_is_transport_syntax_during_reencoding() {
    let input = ": first\n\n\u{feff}data: an-unknown-field\ndata: {}\n\n";
    let original = frames(input.as_bytes(), 1024).unwrap();
    let output = original[1].encode(&json!({"model":"alias"}), 1024).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\u{feff}data: an-unknown-field\ndata: {\"model\":\"alias\"}\n\n"
    );
}
