use std::io::{self, Write};

use serde_json::Value;

use crate::Error;

struct Counter {
    length: usize,
    limit: usize,
    exceeded: bool,
}

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.length.checked_add(bytes.len());
        if next.is_none_or(|next| next > self.limit) {
            self.exceeded = true;
            return Err(io::Error::other("bounded JSON output"));
        }
        self.length += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

/// Counts encoded bytes without allocating an attacker-controlled serialization.
pub(crate) fn json_size(value: &Value, limit: usize) -> Result<Option<usize>, Error> {
    let mut counter = Counter {
        length: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut counter, value);
    if counter.exceeded {
        return Ok(None);
    }
    result.map_err(Error::Json)?;
    Ok(Some(counter.length))
}
