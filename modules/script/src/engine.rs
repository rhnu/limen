mod scheduler;
mod startup;

use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

use rquickjs::{AsyncContext, AsyncRuntime, Function, Persistent};
use serde_json::Value as Json;
use tokio::sync::{Notify, mpsc, oneshot, watch};

use crate::{
    Command, RuntimeOptions, RuntimeStatus, ScriptError, ScriptErrorKind, host,
    json::{self, Bridge},
};

type Hooks = Vec<Persistent<Function<'static>>>;

#[derive(Clone, Copy)]
struct Vm<'a> {
    runtime: &'a AsyncRuntime,
    context: &'a AsyncContext,
    bridge: &'a Bridge,
    budget: &'a Budget,
    options: &'a RuntimeOptions,
}

struct Budget {
    deadline: Cell<Option<Instant>>,
    interrupted: Cell<bool>,
}

impl Budget {
    fn arm(&self, deadline: Instant) { self.deadline.set(Some(deadline)); }

    fn check(&self) -> Result<(), ScriptError> {
        if self.interrupted.get()
            || self
                .deadline
                .get()
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.interrupted.set(true);
            Err(ScriptError::new(ScriptErrorKind::ExecutionBudget))
        } else {
            Ok(())
        }
    }

    /// Resource policy wins, while the failed VM operation keeps its owned cause.
    fn check_result<T>(&self, result: Result<T, ScriptError>) -> Result<T, ScriptError> {
        match (self.check(), result) {
            (Err(budget), Err(error)) => Err(error.reclassify(budget.kind())),
            (Err(budget), Ok(_)) => Err(budget),
            (Ok(()), result) => result,
        }
    }
}

/// Every JS handle is created and destroyed below this OS-thread boundary.
pub(crate) async fn run(
    path: PathBuf,
    options: RuntimeOptions,
    mut commands: mpsc::Receiver<Command>,
    ready: oneshot::Sender<Result<Json, ScriptError>>,
    status: &watch::Sender<RuntimeStatus>,
    stop: Arc<AtomicBool>,
    wake: Arc<Notify>,
) -> Result<(), ScriptError> {
    let startup_deadline = Instant::now() + options.startup_timeout;
    let runtime = match AsyncRuntime::new() {
        Ok(runtime) => runtime,
        Err(source) => {
            let error = ScriptError::with_source(ScriptErrorKind::MemoryLimit, source);
            let _sent = ready.send(Err(error.clone()));
            return Err(error);
        },
    };
    runtime.set_memory_limit(options.memory_limit).await;
    runtime.set_max_stack_size(options.stack_limit).await;
    let budget = Rc::new(Budget {
        deadline: Cell::new(None),
        interrupted: Cell::new(false),
    });
    let interrupt_budget = Rc::clone(&budget);
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            let interrupt = interrupt_budget
                .deadline
                .get()
                .is_some_and(|deadline| Instant::now() >= deadline);
            if interrupt {
                interrupt_budget.interrupted.set(true);
            }
            interrupt
        })))
        .await;
    let context = match AsyncContext::full(&runtime).await {
        Ok(context) => context,
        Err(source) => {
            let error = ScriptError::with_source(ScriptErrorKind::MemoryLimit, source);
            let _sent = ready.send(Err(error.clone()));
            return Err(error);
        },
    };
    budget.arm(startup_deadline.min(Instant::now() + options.execution_timeout));
    let bridge = match context
        .with(|ctx| {
            host::install(&ctx, options.json_limits.max_bytes)
                .map_err(|error| json::failure(&ctx, error))?;
            json::bridge(&ctx, options.json_limits, options.max_output_events)
        })
        .await
    {
        Ok(value) => value,
        Err(error) => {
            let _sent = ready.send(Err(error.clone()));
            return Err(error);
        },
    };
    let vm = Vm {
        runtime: &runtime,
        context: &context,
        bridge: &bridge,
        budget: &budget,
        options: &options,
    };
    let loaded = startup::load(&vm, &path, startup_deadline, &ready).await;
    let (hooks, config) = match loaded {
        Ok(value) => value,
        Err(error) => {
            let _sent = ready.send(Err(error.clone()));
            return Err(error);
        },
    };
    budget.check()?;
    if Instant::now() >= startup_deadline {
        let _sent = ready.send(Err(ScriptError::new(ScriptErrorKind::Deadline)));
        return Err(ScriptError::new(ScriptErrorKind::Deadline));
    }
    let outcome = if publish_ready(status, ready, config) {
        scheduler::serve(&vm, &hooks, &mut commands, stop, wake).await
    } else {
        Ok(())
    };
    commands.close();
    while let Some(command) = commands.recv().await {
        let error = outcome
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| ScriptError::new(ScriptErrorKind::Stopped));
        let _sent = command.response.send(crate::HookReport::failed(error));
    }
    // Pending refs drop in serve, then hook/bridge refs, then context, then VM.
    drop(hooks);
    drop(bridge);
    drop(context);
    drop(runtime);
    outcome
}

/// Publish readiness before waking the starter, which can inspect status immediately.
fn publish_ready(
    status: &watch::Sender<RuntimeStatus>,
    ready: oneshot::Sender<Result<Json, ScriptError>>,
    config: Json,
) -> bool {
    let _sent = status.send(RuntimeStatus::Ready);
    ready.send(Ok(config)).is_ok()
}

fn module_error(error: ScriptError) -> ScriptError {
    if error.kind() == ScriptErrorKind::JavaScript {
        error.reclassify(ScriptErrorKind::Module)
    } else {
        error
    }
}

fn conversion_error(error: ScriptError) -> ScriptError {
    if error.kind() == ScriptErrorKind::JavaScript {
        error.reclassify(ScriptErrorKind::Json)
    } else {
        error
    }
}

fn fatal(error: &ScriptError) -> bool {
    matches!(
        error.kind(),
        ScriptErrorKind::MemoryLimit | ScriptErrorKind::ExecutionBudget
    )
}

#[cfg(test)]
mod tests;
