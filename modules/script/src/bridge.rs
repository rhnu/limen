use rquickjs::{Ctx, Error, Function, Object, Persistent, Value};

use crate::{
    RuntimeOptions, ScriptError, ScriptErrorKind,
    error::exception::{ExceptionDiagnostic, ExceptionKind},
    loader::ModuleDiagnostics,
};

pub(crate) struct Bridge {
    pub encode: Persistent<Function<'static>>,
    pub extract: Persistent<Function<'static>>,
    pub prepare: Persistent<Function<'static>>,
    pub project: Persistent<Function<'static>>,
    oom: Persistent<Function<'static>>,
    exception: Persistent<Function<'static>>,
    pub modules: ModuleDiagnostics,
}

/// Compose private capabilities before evaluating any business or public builtin module.
pub(crate) fn install(ctx: &Ctx<'_>, options: &RuntimeOptions) -> Result<Bridge, ScriptError> {
    let limits = options.json_limits;
    let result = (|| -> rquickjs::Result<Bridge> {
        let proxy = Function::new(ctx.clone(), |value: Value<'_>| value.is_proxy())?;
        let factory: Function = ctx.eval(include_str!("internal/intrinsics.js"))?;
        let intrinsics: Object = factory.call((proxy,))?;
        let codec: Function = ctx.eval(include_str!("json/codec.js"))?;
        let encode: Function = codec.call((
            intrinsics.clone(),
            limits.max_depth,
            limits.max_nodes,
            limits.max_bytes,
        ))?;
        let config: Function = ctx.eval(include_str!("json/config.js"))?;
        let extract: Function =
            config.call((intrinsics.clone(), limits.max_nodes, encode.clone()))?;
        let diagnostics: Function = ctx.eval(include_str!("error/exception.js"))?;
        let diagnostics: Object = diagnostics.call((intrinsics.clone(),))?;
        let capabilities = Object::new(ctx.clone())?;
        let headers: Function = ctx.eval(include_str!("context/headers.js"))?;
        let validate_header = Function::new(
            ctx.clone(),
            |name: String, value: String, writable: bool| {
                crate::context::header_name(&name, &value, writable)
            },
        )?;
        capabilities.set(
            "headers",
            headers.call::<_, Function>((
                intrinsics.clone(),
                validate_header,
                limits.max_nodes,
                limits.max_bytes,
            ))?,
        )?;
        let observations: Function = ctx.eval(include_str!("context/observations.js"))?;
        capabilities.set(
            "observations",
            observations.call::<_, Function>((
                intrinsics.clone(),
                options.observation_names.clone(),
                options.max_observations_per_hook,
            ))?,
        )?;
        let rejection: Function = ctx.eval(include_str!("context/rejection.js"))?;
        capabilities.set(
            "rejection",
            rejection.call::<_, Function>((intrinsics.clone(),))?,
        )?;
        let events: Function = ctx.eval(include_str!("context/events.js"))?;
        capabilities.set(
            "events",
            events.call::<_, Function>((intrinsics.clone(), options.max_output_events))?,
        )?;
        let phase: Function = ctx.eval(include_str!("context/phase.js"))?;
        capabilities.set("phase", phase.call::<_, Function>((intrinsics.clone(),))?)?;
        let context: Function = ctx.eval(include_str!("context.js"))?;
        let context: Object = context.call((intrinsics, capabilities))?;
        Ok(Bridge {
            encode: Persistent::save(ctx, encode),
            extract: Persistent::save(ctx, extract),
            prepare: Persistent::save(ctx, context.get::<_, Function>("prepare")?),
            project: Persistent::save(ctx, context.get::<_, Function>("project")?),
            oom: Persistent::save(ctx, diagnostics.get::<_, Function>("oom")?),
            exception: Persistent::save(ctx, diagnostics.get::<_, Function>("exception")?),
            modules: ModuleDiagnostics::default(),
        })
    })();
    result.map_err(|error| failure(ctx, error))
}

impl Bridge {
    /// Captured intrinsics inspect metadata without executing script accessors or traps.
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
                .and_then(|f| f.call::<_, bool>((value.clone(),)))
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
            let diagnostic = self.exception.clone().restore(ctx).and_then(|function| {
                let info: Object = function.call((value,))?;
                let name: String = info.get("kind")?;
                let stack: Option<String> = info.get("stack")?;
                Ok(ExceptionDiagnostic {
                    kind: ExceptionKind::from_name(&name),
                    location: stack
                        .as_deref()
                        .and_then(|stack| self.modules.locate(stack)),
                })
            });
            return match diagnostic {
                Ok(diagnostic) => ScriptError::with_source(ScriptErrorKind::JavaScript, error)
                    .with_exception(diagnostic),
                Err(inspection) => {
                    ScriptError::with_source(ScriptErrorKind::MemoryLimit, inspection)
                },
            };
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
