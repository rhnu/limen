use std::io::{self, Write};

use serde_json::Value;

use super::error::{ErrorCode, Failure};

struct Bounded<W> {
    writer: W,
    length: usize,
    limit: usize,
    exceeded: bool,
}

impl<W: Write> Write for Bounded<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.length) {
            self.exceeded = true;
            return Err(io::Error::other("JSON output limit"));
        }
        self.writer.write_all(bytes)?;
        self.length += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { self.writer.flush() }
}

fn write<W: Write>(
    writer: W,
    value: &Value,
    limit: usize,
    code: ErrorCode,
    phase: &'static str,
) -> Result<(W, usize), Failure> {
    let mut bounded = Bounded {
        writer,
        length: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut bounded, value);
    if bounded.exceeded {
        return Err(Failure::new(code, phase));
    }
    result.map_err(|source| Failure::new(ErrorCode::Internal, phase).with_source(source))?;
    Ok((bounded.writer, bounded.length))
}

/// Encode once, rejecting overflow before growing the outgoing buffer beyond its budget.
pub(super) fn encode(
    value: &Value,
    limit: usize,
    code: ErrorCode,
    phase: &'static str,
) -> Result<Vec<u8>, Failure> {
    write(Vec::new(), value, limit, code, phase).map(|(bytes, _)| bytes)
}

/// Count conversion or expansion work without allocating an unused JSON representation.
pub(super) fn size(
    value: &Value,
    limit: usize,
    code: ErrorCode,
    phase: &'static str,
) -> Result<usize, Failure> {
    write(io::sink(), value, limit, code, phase).map(|(_, length)| length)
}

#[cfg(test)]
mod tests;
