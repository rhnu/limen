use serde_json::Value;

use crate::{Error, validate_chunk};

pub const DONE: &[u8] = b"data: [DONE]\n\n";

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Chunk(Value),
    Done,
    KeepAlive,
}

/// One bounded SSE block, retaining transport fields and the original bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub event: String,
    pub id: Option<String>,
    pub retry: Option<String>,
    pub data: Option<String>,
    raw: Vec<u8>,
    leading_bom: bool,
}

impl Frame {
    pub fn raw(&self) -> &[u8] { &self.raw }

    /// A source BOM is repeated only when its original frame is expanded.
    pub fn has_leading_bom(&self) -> bool { self.leading_bom }

    /// Interprets Chat data independently of framing or transfer completion.
    ///
    /// # Errors
    /// Rejects invalid JSON, malformed Chat objects and upstream error events.
    pub fn event(&self) -> Result<Event, Error> {
        if self.event == "error" {
            return Err(Error::UpstreamError);
        }
        let Some(data) = &self.data else {
            return Ok(Event::KeepAlive);
        };
        if data == "[DONE]" {
            return Ok(Event::Done);
        }
        let value = serde_json::from_str(data).map_err(Error::Json)?;
        validate_chunk(&value)?;
        Ok(Event::Chunk(value))
    }

    /// Replaces only data lines, preserving comments and other transport fields.
    ///
    /// # Errors
    /// Rejects frames without data, malformed Chat values or oversized output.
    pub fn encode(&self, value: &Value, max_bytes: usize) -> Result<Vec<u8>, Error> {
        validate_chunk(value)?;
        let data = self.data.as_ref().ok_or(Error::InvalidChunk)?;
        if serde_json::from_str::<Value>(data).is_ok_and(|source| source == *value) {
            if self.raw.len() > max_bytes {
                return Err(Error::EventLimit);
            }
            return Ok(self.raw.clone());
        }
        let lines = raw_lines(&self.raw);
        let removed = lines
            .iter()
            .filter(|line| is_data(&self.raw, line, self.leading_bom))
            .map(|line| line.end - line.start)
            .sum::<usize>();
        let first = lines
            .iter()
            .find(|line| is_data(&self.raw, line, self.leading_bom))
            .ok_or(Error::InvalidChunk)?;
        let ending = &self.raw[first.content_end..first.end];
        let bom_bytes = usize::from(first.start == 0 && self.leading_bom) * 3;
        let overhead = self.raw.len() - removed + bom_bytes + b"data: ".len() + ending.len();
        let json_limit = max_bytes.checked_sub(overhead).ok_or(Error::EventLimit)?;
        let (json, _) =
            crate::bounded::write_json(Vec::new(), value, json_limit)?.ok_or(Error::EventLimit)?;
        let mut output = Vec::new();
        let mut inserted = false;
        for line in lines {
            if is_data(&self.raw, &line, self.leading_bom) {
                if !inserted {
                    // A stream-leading BOM belongs to transport, not to the replaced data.
                    if line.start == 0 && self.leading_bom {
                        output.extend_from_slice(b"\xef\xbb\xbf");
                    }
                    output.extend_from_slice(b"data: ");
                    output.extend_from_slice(&json);
                    output.extend_from_slice(ending);
                    inserted = true;
                }
            } else {
                output.extend_from_slice(&self.raw[line.start..line.end]);
            }
        }
        if output.len() > max_bytes {
            return Err(Error::EventLimit);
        }
        Ok(output)
    }
}

struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}

fn raw_lines(raw: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    while cursor < raw.len() {
        if matches!(raw[cursor], b'\r' | b'\n') {
            let content_end = cursor;
            cursor += 1;
            if raw[content_end] == b'\r' && raw.get(cursor) == Some(&b'\n') {
                cursor += 1;
            }
            lines.push(Line {
                start,
                content_end,
                end: cursor,
            });
            start = cursor;
        } else {
            cursor += 1;
        }
    }
    lines
}

fn line_content<'a>(raw: &'a [u8], line: &Line, leading_bom: bool) -> &'a [u8] {
    let content = &raw[line.start..line.content_end];
    if line.start == 0 && leading_bom {
        content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content)
    } else {
        content
    }
}

fn is_data(raw: &[u8], line: &Line, leading_bom: bool) -> bool {
    let content = line_content(raw, line, leading_bom);
    content == b"data" || content.starts_with(b"data:")
}

/// Byte-driven framing bounds memory independently of incoming network blocks.
/// A CR dispatches immediately; its optional following LF is a data-free frame.
/// Concatenating raw frames therefore preserves CRLF exactly, including at EOF.
pub struct FrameDecoder {
    raw: Vec<u8>,
    line_bytes: usize,
    previous_cr: bool,
    first: bool,
    failed: bool,
    limit: usize,
}

impl FrameDecoder {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            raw: Vec::new(),
            line_bytes: 0,
            previous_cr: false,
            first: true,
            failed: false,
            limit: max_bytes,
        }
    }

    /// Advances one byte without queuing a batch of frames.
    ///
    /// # Errors
    /// Rejects oversized blocks, invalid UTF-8 and reuse after framing failure.
    pub fn push(&mut self, byte: u8) -> Result<Option<Frame>, Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        let result = self.push_inner(byte);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, byte: u8) -> Result<Option<Frame>, Error> {
        if self.previous_cr && byte == b'\n' {
            self.previous_cr = false;
            if self.raw.is_empty() {
                if self.limit == 0 {
                    return Err(Error::EventLimit);
                }
                return Ok(Some(Frame {
                    event: "message".into(),
                    id: None,
                    retry: None,
                    data: None,
                    raw: vec![byte],
                    leading_bom: false,
                }));
            }
            self.append(byte)?;
            return Ok(None);
        }
        self.previous_cr = byte == b'\r';
        self.append(byte)?;
        if matches!(byte, b'\r' | b'\n') {
            let blank = self.line_bytes == 0
                || self.first && self.raw == b"\xef\xbb\xbf\n"
                || self.first && self.raw == b"\xef\xbb\xbf\r";
            self.line_bytes = 0;
            if blank {
                return self.take_frame().map(Some);
            }
        } else {
            self.line_bytes += 1;
        }
        Ok(None)
    }

    fn append(&mut self, byte: u8) -> Result<(), Error> {
        if self.raw.len() >= self.limit {
            return Err(Error::EventLimit);
        }
        self.raw.push(byte);
        Ok(())
    }

    fn take_frame(&mut self) -> Result<Frame, Error> {
        let text = std::str::from_utf8(&self.raw).map_err(Error::Utf8)?;
        let text = if self.first {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        let mut frame = Frame {
            event: "message".into(),
            id: None,
            retry: None,
            data: None,
            raw: Vec::new(),
            leading_bom: self.first && self.raw.starts_with(b"\xef\xbb\xbf"),
        };
        for line in text.split(['\r', '\n']).filter(|line| !line.is_empty()) {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => {
                    frame.event = if value.is_empty() {
                        "message".into()
                    } else {
                        value.into()
                    }
                },
                "id" if !value.contains('\0') => frame.id = Some(value.into()),
                "retry" => frame.retry = Some(value.into()),
                "data" => {
                    if let Some(data) = &mut frame.data {
                        data.push('\n');
                        data.push_str(value);
                    } else {
                        frame.data = Some(value.into());
                    }
                },
                _ => {},
            }
        }
        frame.raw = std::mem::take(&mut self.raw);
        self.first = false;
        Ok(frame)
    }

    /// Verifies that EOF leaves no partial block; it never synthesizes an event.
    ///
    /// # Errors
    /// Rejects a previous failure or any unterminated block, independently of DONE.
    pub fn finish(self) -> Result<(), Error> {
        if self.failed {
            Err(Error::InvalidChunk)
        } else if !self.raw.is_empty() {
            Err(Error::Truncated)
        } else {
            Ok(())
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

#[cfg(test)]
mod tests;
