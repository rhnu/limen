use serde_json::Value;
use sse_parse::SseParser;

use crate::{Error, validate_chunk};

pub const DONE: &[u8] = b"data: [DONE]\n\n";

pub enum Event {
    Chunk(Value),
    Done,
}

/// Byte-driven decoding prevents a large network block from queuing unbounded events.
pub struct Decoder {
    parser: SseParser,
    done: bool,
    failed: bool,
    limit: usize,
    block_bytes: usize,
    line_bytes: usize,
    previous_cr: bool,
}

impl Decoder {
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            parser: SseParser::new().max_buffer(max_event_bytes),
            done: false,
            failed: false,
            limit: max_event_bytes,
            block_bytes: 0,
            line_bytes: 0,
            previous_cr: false,
        }
    }

    /// Advances one byte without queuing a batch of events.
    ///
    /// # Errors
    /// Rejects oversized events, malformed Chat data, upstream error frames or data after DONE.
    pub fn push(&mut self, byte: u8) -> Result<Option<Event>, Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        let result = self.push_inner(byte);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, byte: u8) -> Result<Option<Event>, Error> {
        if byte == b'\n' && self.previous_cr {
            self.previous_cr = false;
            self.parser.feed(&[byte]);
            return Ok(None);
        }
        self.previous_cr = byte == b'\r';
        self.block_bytes = self.block_bytes.checked_add(1).ok_or(Error::EventLimit)?;
        if self.block_bytes > self.limit {
            return Err(Error::EventLimit);
        }
        if matches!(byte, b'\n' | b'\r') {
            if self.line_bytes == 0 {
                self.block_bytes = 0;
            }
            self.line_bytes = 0;
        } else {
            self.line_bytes += 1;
        }
        self.parser.feed(&[byte]);
        if self.parser.overflows() != 0 {
            return Err(Error::EventLimit);
        }
        let Some(event) = self.parser.next_event() else {
            return Ok(None);
        };
        if self.done {
            return Err(Error::AfterDone);
        }
        if event.event == "error" {
            return Err(Error::UpstreamError);
        }
        if event.event != "message" {
            return Err(Error::InvalidChunk);
        }
        if event.data == "[DONE]" {
            self.done = true;
            return Ok(Some(Event::Done));
        }
        let value = serde_json::from_str(&event.data).map_err(Error::Json)?;
        validate_chunk(&value)?;
        Ok(Some(Event::Chunk(value)))
    }

    /// Discards an unterminated SSE block and verifies the explicit Chat terminator.
    ///
    /// # Errors
    /// Returns [`Error::Truncated`] without DONE, or rejects an already failed decoder.
    pub fn finish(mut self) -> Result<(), Error> {
        self.parser.finish();
        if self.failed {
            Err(Error::InvalidChunk)
        } else if self.done {
            Ok(())
        } else {
            Err(Error::Truncated)
        }
    }
}

/// Encodes a validated chunk as one data event.
///
/// # Errors
/// Rejects malformed chunks or encoded output exceeding the event limit.
pub fn encode_chunk(value: &Value, max_event_bytes: usize) -> Result<Vec<u8>, Error> {
    validate_chunk(value)?;
    encode(value, b"data: ", max_event_bytes)
}

/// Counts framing and JSON size for chunks already established by the converter.
pub(crate) fn chunk_size(value: &Value, max_event_bytes: usize) -> Result<usize, Error> {
    encoded_size(value, b"data: ", max_event_bytes)
}

/// Encodes the caller's sanitized error envelope without a success terminator.
///
/// # Errors
/// Rejects encoded output exceeding the event limit or JSON serialization failures.
pub fn encode_error(value: &Value, max_event_bytes: usize) -> Result<Vec<u8>, Error> {
    encode(value, b"event: error\ndata: ", max_event_bytes)
}

fn encoded_size(value: &Value, prefix: &[u8], limit: usize) -> Result<usize, Error> {
    let json_limit = limit
        .checked_sub(prefix.len() + 2)
        .ok_or(Error::EventLimit)?;
    let json_size = crate::bounded::json_size(value, json_limit)?.ok_or(Error::EventLimit)?;
    Ok(prefix.len() + json_size + 2)
}

fn encode(value: &Value, prefix: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let json_limit = limit
        .checked_sub(prefix.len() + 2)
        .ok_or(Error::EventLimit)?;
    let (mut bytes, _) =
        crate::bounded::write_json(prefix.to_vec(), value, json_limit)?.ok_or(Error::EventLimit)?;
    bytes.extend_from_slice(b"\n\n");
    Ok(bytes)
}
