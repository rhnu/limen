use std::io::{self, Write};

use serde_json::Value;

use crate::Error;

struct Bounded<W> {
    writer: W,
    length: usize,
    limit: usize,
    exceeded: bool,
}

impl<W: Write> Write for Bounded<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.length.checked_add(bytes.len());
        if next.is_none_or(|next| next > self.limit) {
            self.exceeded = true;
            return Err(io::Error::other("bounded JSON output"));
        }
        self.writer.write_all(bytes)?;
        self.length += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { self.writer.flush() }
}

/// Encode or count JSON without writing beyond the caller's byte allowance.
pub(crate) fn write_json<W: Write>(
    writer: W,
    value: &Value,
    limit: usize,
) -> Result<Option<(W, usize)>, Error> {
    let mut bounded = Bounded {
        writer,
        length: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut bounded, value);
    if bounded.exceeded {
        return Ok(None);
    }
    result.map_err(Error::Json)?;
    Ok(Some((bounded.writer, bounded.length)))
}

/// Counts encoded bytes without allocating an attacker-controlled serialization.
pub(crate) fn json_size(value: &Value, limit: usize) -> Result<Option<usize>, Error> {
    write_json(io::sink(), value, limit).map(|result| result.map(|(_, length)| length))
}
