mod scheduler;
mod startup;

use std::{
    cell::Cell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

use rquickjs::{AsyncContext, AsyncRuntime, Function, Persistent};
use serde_json::Value as Json;
use tokio::sync::{Notify, mpsc, oneshot, watch};

use crate::{
    Command, HookId, RuntimeOptions, RuntimeStatus, ScriptError,
    json::{self, Bridge},
};

type Hooks = HashMap<HookId, Persistent<Function<'static>>>;

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
            Err(ScriptError::ExecutionBudget)
        } else {
            Ok(())
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
    let Ok(runtime) = AsyncRuntime::new() else {
        let _sent = ready.send(Err(ScriptError::MemoryLimit));
        return Err(ScriptError::MemoryLimit);
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
    let Ok(context) = AsyncContext::full(&runtime).await else {
        let _sent = ready.send(Err(ScriptError::MemoryLimit));
        return Err(ScriptError::MemoryLimit);
    };
    budget.arm(startup_deadline.min(Instant::now() + options.execution_timeout));
    let bridge = match context
        .with(|ctx| json::bridge(&ctx, options.json_limits))
        .await
    {
        Ok(value) => value,
        Err(error) => {
            let _sent = ready.send(Err(error));
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
            let _sent = ready.send(Err(error));
            return Err(error);
        },
    };
    budget.check()?;
    if Instant::now() >= startup_deadline {
        let _sent = ready.send(Err(ScriptError::Deadline));
        return Err(ScriptError::Deadline);
    }
    let outcome = if ready.send(Ok(config)).is_err() {
        Ok(())
    } else {
        let _sent = status.send(RuntimeStatus::Ready);
        scheduler::serve(&vm, &hooks, &mut commands, stop, wake).await
    };
    commands.close();
    while let Some(command) = commands.recv().await {
        let _sent = command.response.send(Err(ScriptError::Retired));
    }
    // Pending refs drop in serve, then hook/bridge refs, then context, then VM.
    drop(hooks);
    drop(bridge);
    drop(context);
    drop(runtime);
    outcome
}

fn module_error(error: ScriptError) -> ScriptError {
    if error == ScriptError::JavaScript {
        ScriptError::Module
    } else {
        error
    }
}

fn conversion_error(error: ScriptError) -> ScriptError {
    if error == ScriptError::JavaScript {
        ScriptError::Json
    } else {
        error
    }
}

fn fatal(error: ScriptError) -> bool {
    matches!(
        error,
        ScriptError::MemoryLimit | ScriptError::ExecutionBudget
    )
}
