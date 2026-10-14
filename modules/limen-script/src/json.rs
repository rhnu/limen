use rquickjs::{Ctx, Error, Function, Object, Persistent, Value};

use crate::{JsonLimits, ScriptError};

struct Bounded {
    remaining: usize,
}

impl std::io::Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("JSON size limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

pub(crate) fn validate_owned(
    value: &serde_json::Value,
    limits: JsonLimits,
) -> Result<(), ScriptError> {
    fn walk(
        v: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
        limits: JsonLimits,
    ) -> Result<(), ScriptError> {
        *nodes += 1;
        if depth > limits.max_depth || *nodes > limits.max_nodes {
            return Err(ScriptError::Json);
        }
        match v {
            serde_json::Value::Number(n) => {
                let f = n.as_f64().ok_or(ScriptError::Json)?;
                if !f.is_finite() || (f.fract() == 0.0 && f.abs() > 9_007_199_254_740_991.0) {
                    return Err(ScriptError::Json);
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
    // Serialize to a counting sink, never a second potentially huge owned buffer.
    serde_json::to_writer(
        Bounded {
            remaining: limits.max_bytes,
        },
        value,
    )
    .map_err(|_error| ScriptError::Json)?;
    Ok(())
}

pub(crate) struct Bridge {
    pub convert: Persistent<Function<'static>>,
    oom: Persistent<Function<'static>>,
}

pub(crate) fn bridge(ctx: &Ctx<'_>, limits: JsonLimits) -> Result<Bridge, ScriptError> {
    let factory: Function = ctx
        .eval(include_str!("json/bridge.js"))
        .map_err(|e| failure(ctx, &e))?;
    let proxy = Function::new(ctx.clone(), |value: Value<'_>| value.is_proxy())
        .map_err(|e| failure(ctx, &e))?;
    let result: Object = factory
        .call((proxy, limits.max_depth, limits.max_nodes, limits.max_bytes))
        .map_err(|e| failure(ctx, &e))?;
    Ok(Bridge {
        convert: Persistent::save(
            ctx,
            result
                .get::<_, Function>("convert")
                .map_err(|e| failure(ctx, &e))?,
        ),
        oom: Persistent::save(
            ctx,
            result
                .get::<_, Function>("oom")
                .map_err(|e| failure(ctx, &e))?,
        ),
    })
}

impl Bridge {
    /// Inspect only an own data descriptor; exception getters must never execute.
    pub fn failure(&self, ctx: &Ctx<'_>, error: &Error) -> ScriptError {
        if matches!(error, Error::Allocation) {
            return ScriptError::MemoryLimit;
        }
        if error.is_exception() {
            let value = ctx.catch();
            let oom = self
                .oom
                .clone()
                .restore(ctx)
                .and_then(|f| f.call::<_, bool>((value,)))
                .unwrap_or(true);
            if oom {
                return ScriptError::MemoryLimit;
            }
        }
        ScriptError::JavaScript
    }
}

/// Used only during pristine intrinsic bootstrap, before any user module runs.
pub(crate) fn failure(ctx: &Ctx<'_>, error: &Error) -> ScriptError {
    if matches!(error, Error::Allocation) {
        return ScriptError::MemoryLimit;
    }
    if error.is_exception() {
        let value = ctx.catch();
        if value.is_null() {
            return ScriptError::MemoryLimit;
        }
    }
    ScriptError::JavaScript
}

#[cfg(test)]
mod tests;
