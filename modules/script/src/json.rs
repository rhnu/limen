use rquickjs::{Ctx, Error, Function, Object, Persistent, Value};

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

pub(crate) struct Bridge {
    pub convert: Persistent<Function<'static>>,
    pub prepare: Persistent<Function<'static>>,
    pub project: Persistent<Function<'static>>,
    oom: Persistent<Function<'static>>,
}

pub(crate) fn bridge(
    ctx: &Ctx<'_>,
    limits: JsonLimits,
    max_output_events: usize,
) -> Result<Bridge, ScriptError> {
    let factory: Function = ctx
        .eval(include_str!("json/bridge.js"))
        .map_err(|e| failure(ctx, e))?;
    let proxy = Function::new(ctx.clone(), |value: Value<'_>| value.is_proxy())
        .map_err(|e| failure(ctx, e))?;
    let result: Object = factory
        .call((proxy, limits.max_depth, limits.max_nodes, limits.max_bytes))
        .map_err(|e| failure(ctx, e))?;
    let context_factory: Function = ctx
        .eval(include_str!("context.js"))
        .map_err(|e| failure(ctx, e))?;
    let validate_header = Function::new(
        ctx.clone(),
        |name: String, value: String, writable: bool| {
            crate::context::header_name(&name, &value, writable)
        },
    )
    .map_err(|e| failure(ctx, e))?;
    let context: Object = context_factory
        .call((
            validate_header,
            limits.max_nodes,
            limits.max_bytes,
            max_output_events,
        ))
        .map_err(|e| failure(ctx, e))?;
    Ok(Bridge {
        prepare: Persistent::save(
            ctx,
            context
                .get::<_, Function>("prepare")
                .map_err(|e| failure(ctx, e))?,
        ),
        project: Persistent::save(
            ctx,
            context
                .get::<_, Function>("project")
                .map_err(|e| failure(ctx, e))?,
        ),
        convert: Persistent::save(
            ctx,
            result
                .get::<_, Function>("convert")
                .map_err(|e| failure(ctx, e))?,
        ),
        oom: Persistent::save(
            ctx,
            result
                .get::<_, Function>("oom")
                .map_err(|e| failure(ctx, e))?,
        ),
    })
}

impl Bridge {
    /// Inspect only an own data descriptor; exception getters must never execute.
    pub fn failure(&self, ctx: &Ctx<'_>, error: Error) -> ScriptError {
        if matches!(error, Error::Allocation) {
            return ScriptError::with_source(ScriptErrorKind::MemoryLimit, error);
        }
        if error.is_exception() {
            let value = ctx.catch();
            let oom = match self
                .oom
                .clone()
                .restore(ctx)
                .and_then(|f| f.call::<_, bool>((value,)))
            {
                Ok(oom) => oom,
                // An unusable classifier cannot establish VM safety; retain its failure.
                Err(inspection) => {
                    return ScriptError::with_source(ScriptErrorKind::MemoryLimit, inspection);
                },
            };
            if oom {
                return ScriptError::with_source(ScriptErrorKind::MemoryLimit, error);
            }
        }
        ScriptError::with_source(ScriptErrorKind::JavaScript, error)
    }
}

/// Used only during pristine intrinsic bootstrap, before any user module runs.
pub(crate) fn failure(ctx: &Ctx<'_>, error: Error) -> ScriptError {
    if matches!(error, Error::Allocation) {
        return ScriptError::with_source(ScriptErrorKind::MemoryLimit, error);
    }
    if error.is_exception() {
        let value = ctx.catch();
        if value.is_null() {
            return ScriptError::with_source(ScriptErrorKind::MemoryLimit, error);
        }
    }
    ScriptError::with_source(ScriptErrorKind::JavaScript, error)
}

#[cfg(test)]
mod tests;
