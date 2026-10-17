use std::{collections::HashSet, time::Instant};

use rquickjs::{Array, Function, Module, Object, Persistent, Promise, Value};
use serde_json::Value as Json;
use tokio::sync::oneshot;

use super::{Hooks, Vm, conversion_error, module_error};
use crate::{
    ScriptError, ScriptErrorKind,
    loader::{self, ModuleLoader, ModuleResolver},
};

pub(super) async fn load(
    vm: &Vm<'_>,
    path: &std::path::Path,
    deadline: Instant,
    ready: &oneshot::Sender<Result<Json, ScriptError>>,
) -> Result<(Hooks, Json), ScriptError> {
    let Vm {
        runtime,
        context,
        bridge,
        budget,
        options,
    } = *vm;
    let path = loader::module_path(path)?;
    let bytes = loader::source(&path, options.max_source_bytes)?;
    let name = path
        .to_str()
        .ok_or(ScriptError::new(ScriptErrorKind::Module))?;
    let diagnostics = bridge.modules.clone();
    diagnostics.register(name);
    runtime
        .set_loader(
            ModuleResolver(diagnostics.clone()),
            ModuleLoader {
                diagnostics: diagnostics.clone(),
                remaining: options.max_source_bytes - bytes.len(),
                max_modules: options.max_modules,
                loaded: HashSet::from([name.to_owned()]),
            },
        )
        .await;
    budget.arm(deadline.min(Instant::now() + options.execution_timeout));
    let evaluated = context
        .with(|ctx| {
            let result = Module::declare(ctx.clone(), name, bytes).and_then(Module::eval);
            match result {
                Ok((module, promise)) => {
                    let namespace = module
                        .namespace()
                        .map_err(|e| module_error(bridge.failure(&ctx, e)))?;
                    Ok((
                        Persistent::save(&ctx, namespace),
                        Persistent::save(&ctx, promise),
                    ))
                },
                Err(error) => Err(module_error(bridge.failure(&ctx, error))),
            }
        })
        .await;
    let evaluated = budget.check_result(evaluated);
    let failure = diagnostics.take();
    let evaluated = evaluated.map_err(|error| {
        if error.kind() == ScriptErrorKind::Module {
            failure.unwrap_or(error)
        } else {
            error
        }
    });
    let (module, evaluation) = evaluated?;
    // A dynamic import may be caught by JS before an unrelated Promise rejects.
    // Only synchronous graph failures can be associated with the loader's I/O cause.
    let waited = wait_for_evaluation(vm, &evaluation, deadline, ready).await;
    let _unassigned = diagnostics.take();
    waited?;
    extract(vm, &module, deadline).await
}

async fn wait_for_evaluation(
    vm: &Vm<'_>,
    evaluation: &Persistent<Promise<'static>>,
    deadline: Instant,
    ready: &oneshot::Sender<Result<Json, ScriptError>>,
) -> Result<(), ScriptError> {
    let Vm {
        runtime,
        context,
        bridge,
        budget,
        options,
    } = *vm;
    let continuous_start = Instant::now();
    loop {
        if Instant::now() >= deadline {
            return Err(ScriptError::new(ScriptErrorKind::Deadline));
        }
        if ready.is_closed() {
            return Err(ScriptError::new(ScriptErrorKind::Cancelled));
        }
        let state = context
            .with(|ctx| {
                let promise = evaluation
                    .clone()
                    .restore(&ctx)
                    .map_err(|e| bridge.failure(&ctx, e))?;
                promise
                    .result::<Value>()
                    .map(|result| {
                        result
                            .map(|_| ())
                            .map_err(|e| module_error(bridge.failure(&ctx, e)))
                    })
                    .transpose()
            })
            .await?;
        if state.is_some() {
            break;
        }
        let mut worked = false;
        for _ in 0..options.job_batch {
            let work_deadline = continuous_start + options.continuous_work_timeout;
            if Instant::now() >= work_deadline {
                return Err(ScriptError::new(ScriptErrorKind::ExecutionBudget));
            }
            budget.arm(
                deadline
                    .min(Instant::now() + options.execution_timeout)
                    .min(work_deadline),
            );
            match runtime.execute_pending_job().await {
                Ok(true) => worked = true,
                Ok(false) => break,
                Err(error) => {
                    let failure = error
                        .0
                        .with(|ctx| bridge.failure(&ctx, rquickjs::Error::Exception))
                        .await;
                    return budget.check_result(Err(module_error(failure)));
                },
            }
            budget.check()?;
            if Instant::now() >= deadline {
                return Err(ScriptError::new(ScriptErrorKind::Deadline));
            }
        }
        if worked {
            tokio::task::yield_now().await;
        } else {
            // No host I/O exists: a stalled top-level Promise can only time out.
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        }
    }
    Ok(())
}

async fn extract(
    vm: &Vm<'_>,
    module: &Persistent<Object<'static>>,
    deadline: Instant,
) -> Result<(Hooks, Json), ScriptError> {
    let Vm {
        context,
        bridge,
        budget,
        options,
        ..
    } = *vm;
    budget.arm(deadline.min(Instant::now() + options.execution_timeout));
    let result = context
        .with(|ctx| {
            let default = module
                .clone()
                .restore(&ctx)
                .and_then(|m| m.get::<_, Value>("default"))
                .map_err(|e| module_error(bridge.failure(&ctx, e)))?;
            let extract = bridge
                .extract
                .clone()
                .restore(&ctx)
                .map_err(|e| bridge.failure(&ctx, e))?;
            let converted: Object = extract
                .call((default,))
                .map_err(|e| conversion_error(bridge.failure(&ctx, e)))?;
            let text: String = converted.get("json").map_err(|e| bridge.failure(&ctx, e))?;
            if text.len() > options.json_limits.max_bytes {
                return Err(ScriptError::new(ScriptErrorKind::Json));
            }
            let config = limen_config::parse_json_str(&text, options.json_limits.max_depth)
                .map_err(|error| ScriptError::with_source(ScriptErrorKind::Json, error))?;
            let functions: Array = converted
                .get("hooks")
                .map_err(|e| bridge.failure(&ctx, e))?;
            let mut hooks = Hooks::new();
            for index in 0..functions.len() {
                let function = functions
                    .get::<Function>(index)
                    .map_err(|e| bridge.failure(&ctx, e))?;
                hooks.push(Persistent::save(&ctx, function));
            }
            Ok((hooks, config))
        })
        .await;
    budget.check_result(result)
}
