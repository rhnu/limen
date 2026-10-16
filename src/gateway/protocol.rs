//! Chat Completions wire adaptation, separate from transport facts and business hooks.

use http::{HeaderMap, HeaderValue, header};
use limen_chat::{ChunkTracker, Event, Frame, FrameDecoder};
use limen_config::{ResourceLimits, validate_json};
use serde_json::Value;

use super::{
    error::{ErrorCode, Failure, FailureResponse},
    json, logic,
};

/// Encode only after the routed business pipeline has finished editing its Chat draft.
pub(super) fn encode_request(
    body: &Value,
    headers: &mut HeaderMap,
    limits: &ResourceLimits,
) -> Result<Vec<u8>, Failure> {
    logic::validate_body(body, limits)?;
    if !headers.contains_key(header::ACCEPT) {
        let accept = if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
            "text/event-stream"
        } else {
            "application/json"
        };
        headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    json::encode(
        body,
        limits.max_output_bytes,
        ErrorCode::PayloadTooLarge,
        "upstream_body",
    )
}

/// Native structured errors are private response data, never diagnostic messages.
pub(super) fn interpret_frame(frame: &Frame, limits: &ResourceLimits) -> Result<Event, Failure> {
    frame.event().map_err(|source| {
        let upstream_error = matches!(source, limen_chat::Error::UpstreamError);
        let mut failure =
            Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse").with_source(source);
        if upstream_error
            && let Some(data) = &frame.data
            && data.len() <= limits.max_output_bytes
        {
            let parsed = limen_config::parse_json_str(data, limen_config::MAX_JSON_DEPTH).ok();
            if parsed.as_ref().is_some_and(|value| {
                validate_json(value, limits.max_json_depth, limits.max_json_nodes).is_ok()
            }) {
                failure.response = Some(Box::new(FailureResponse {
                    body: data.as_bytes().to_vec(),
                    content_type: Some(HeaderValue::from_static("application/json")),
                    content_encoding: Vec::new(),
                }));
            }
        }
        failure
    })
}

/// Native control frames have no business rewrite stage to shrink their output.
pub(super) fn control_frame<'a>(
    frame: &'a Frame,
    limits: &ResourceLimits,
) -> Result<&'a [u8], Failure> {
    if frame.raw().len() > limits.max_output_bytes {
        return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse")
            .with_source(limen_chat::Error::EventLimit));
    }
    Ok(frame.raw())
}

/// Transport EOF alone cannot establish a complete Chat generation.
pub(super) fn finish_stream(
    decoder: FrameDecoder,
    tracker: ChunkTracker,
    terminator: Option<Frame>,
) -> Result<Frame, Failure> {
    let failure =
        |source| Failure::new(ErrorCode::UpstreamProtocol, "upstream_sse").with_source(source);
    decoder.finish().map_err(failure)?;
    let terminator = terminator.ok_or_else(|| failure(limen_chat::Error::Truncated))?;
    tracker.finish().map_err(failure)?;
    Ok(terminator)
}

#[cfg(test)]
mod tests;
