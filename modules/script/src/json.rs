use crate::{JsonLimits, ScriptError, ScriptErrorKind};

struct Bounded {
    bytes: Vec<u8>,
    remaining: usize,
}

impl std::io::Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("JSON size limit"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

pub(crate) fn encode_owned(
    value: &serde_json::Value,
    limits: JsonLimits,
) -> Result<Vec<u8>, ScriptError> {
    fn walk(
        v: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
        limits: JsonLimits,
    ) -> Result<(), ScriptError> {
        *nodes += 1;
        if depth > limits.max_depth || *nodes > limits.max_nodes {
            return Err(ScriptError::new(ScriptErrorKind::Json));
        }
        match v {
            serde_json::Value::Number(n) => {
                let f = n.as_f64().ok_or(ScriptError::new(ScriptErrorKind::Json))?;
                if !f.is_finite() || (f.fract() == 0.0 && f.abs() > 9_007_199_254_740_991.0) {
                    return Err(ScriptError::new(ScriptErrorKind::Json));
                }
            },
            serde_json::Value::Array(a) => {
                for item in a {
                    walk(item, depth + 1, nodes, limits)?;
                }
            },
            serde_json::Value::Object(o) => {
                for item in o.values() {
                    walk(item, depth + 1, nodes, limits)?;
                }
            },
            _ => {},
        }
        Ok(())
    }
    walk(value, 0, &mut 0, limits)?;
    // Queue the bounded encoding itself so the VM need not serialize the same tree again.
    let mut bounded = Bounded {
        bytes: Vec::new(),
        remaining: limits.max_bytes,
    };
    serde_json::to_writer(&mut bounded, value)
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Json, error))?;
    Ok(bounded.bytes)
}

#[cfg(test)]
mod tests;
