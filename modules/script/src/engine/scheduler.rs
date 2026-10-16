use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use rquickjs::{AsyncContext, Ctx, Object, Persistent, Promise, Value};
use serde_json::Value as Json;
use tokio::sync::{Notify, mpsc, oneshot};

use super::{Budget, Hooks, Vm, conversion_error, fatal};
use crate::{
    Cancellation, Command, HookId, HookReport, HookTimings, RuntimeOptions, ScriptError,
    ScriptErrorKind, json::Bridge,
};

/// Only settlement authority survives after the input becomes a private VM draft.
struct Completion {
    cancellation: Cancellation,
    deadline: Instant,
    response: oneshot::Sender<HookReport>,
    timings: HookTimings,
    pending_since: Option<Instant>,
}

impl Completion {
    fn invalid(&self) -> Option<ScriptError> {
        if self.cancellation.is_cancelled() || self.response.is_closed() {
            Some(ScriptError::new(ScriptErrorKind::Cancelled))
        } else if Instant::now() >= self.deadline {
            Some(ScriptError::new(ScriptErrorKind::Deadline))
        } else {
            None
        }
    }

    fn send(self, result: Result<Json, ScriptError>) {
        let result = self.invalid().map_or(result, Err);
        self.finish(result);
    }

    fn observe_pending(&mut self) {
        if let Some(started) = self.pending_since.take() {
            self.timings.pending = Some(started.elapsed());
        }
    }

    fn finish(mut self, result: Result<Json, ScriptError>) {
        self.observe_pending();
        let _sent = self.response.send(HookReport::new(result, self.timings));
    }
}

struct Pending {
    completion: Completion,
    draft: Persistent<Value<'static>>,
    promise: Persistent<Promise<'static>>,
}

pub(super) async fn serve(
    vm: &Vm<'_>,
    hooks: &Hooks,
    commands: &mut mpsc::Receiver<Command>,
    stop: Arc<AtomicBool>,
    wake: Arc<Notify>,
) -> Result<(), ScriptError> {
    let mut pending = Vec::new();
    let outcome = run_loop(vm, hooks, commands, &stop, &wake, &mut pending).await;
    let failure = outcome
        .as_ref()
        .err()
        .cloned()
        .unwrap_or_else(|| ScriptError::new(ScriptErrorKind::Stopped));
    for waiting in pending {
        waiting.completion.finish(Err(failure.clone()));
    }
    outcome
}

async fn run_loop(
    vm: &Vm<'_>,
    hooks: &Hooks,
    commands: &mut mpsc::Receiver<Command>,
    stop: &AtomicBool,
    wake: &Notify,
    pending: &mut Vec<Pending>,
) -> Result<(), ScriptError> {
    let Vm {
        context,
        bridge,
        budget,
        options,
        ..
    } = *vm;
    let mut continuous_start = None;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        poll_pending(context, bridge, budget, options, pending).await?;
        for _ in 0..options.job_batch {
            match commands.try_recv() {
                Ok(command) => admit(vm, hooks, command, pending).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if pump(vm, &mut continuous_start).await? {
            tokio::task::yield_now().await;
            continue;
        }
        continuous_start = None;
        // Inspect completion before sleeping; no native jobs means no polling.
        poll_pending(context, bridge, budget, options, pending).await?;
        let deadline = pending.iter().map(|p| p.completion.deadline).min();
        match wait(commands, wake, deadline).await {
            Wake::Command(command) => admit(vm, hooks, command, pending).await?,
            Wake::Notified => {},
            Wake::Closed => return Ok(()),
        }
    }
}

async fn admit(
    vm: &Vm<'_>,
    hooks: &Hooks,
    command: Command,
    pending: &mut Vec<Pending>,
) -> Result<(), ScriptError> {
    if pending.len() >= vm.options.pending_capacity {
        let timings = HookTimings {
            queue: Some(command.enqueued_at.elapsed()),
            ..HookTimings::default()
        };
        let _sent = command.response.send(HookReport::new(
            Err(ScriptError::new(ScriptErrorKind::Busy)),
            timings,
        ));
        return Ok(());
    }
    vm.budget.arm(Instant::now() + vm.options.execution_timeout);
    if let Some(waiting) =
        begin(vm.context, vm.bridge, vm.budget, hooks, command, vm.options).await?
    {
        pending.push(waiting);
    }
    vm.budget.check()
}

async fn pump(vm: &Vm<'_>, continuous_start: &mut Option<Instant>) -> Result<bool, ScriptError> {
    let Vm {
        runtime,
        bridge,
        budget,
        options,
        ..
    } = *vm;
    for _ in 0..options.job_batch {
        let now = Instant::now();
        let since = *continuous_start.get_or_insert(now);
        if now.duration_since(since) >= options.continuous_work_timeout {
            break;
        }
        budget.arm((now + options.execution_timeout).min(since + options.continuous_work_timeout));
        match runtime.execute_pending_job().await {
            Ok(true) => {},
            Ok(false) => {
                *continuous_start = None;
                break;
            },
            Err(error) => {
                let failure = error
                    .0
                    .with(|ctx| bridge.failure(&ctx, rquickjs::Error::Exception))
                    .await;
                // A detached-job exception cannot be assigned to one request.
                return budget.check_result(Err(failure));
            },
        }
        budget.check()?;
    }
    let pending = runtime.is_job_pending().await;
    if pending
        && continuous_start.is_some_and(|start| start.elapsed() >= options.continuous_work_timeout)
    {
        return Err(ScriptError::new(ScriptErrorKind::ExecutionBudget));
    }
    Ok(pending)
}

enum Wake {
    Command(Command),
    Notified,
    Closed,
}

async fn wait(
    commands: &mut mpsc::Receiver<Command>,
    wake: &Notify,
    deadline: Option<Instant>,
) -> Wake {
    // recv also wakes on final handle drop; an idle VM never leaks its thread.
    tokio::select! {
        () = wake.notified() => Wake::Notified,
        () = async {
            if let Some(deadline) = deadline { tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await; }
            else { std::future::pending::<()>().await; }
        } => Wake::Notified,
        command = commands.recv() => command.map_or(Wake::Closed, Wake::Command),
    }
}

async fn begin(
    context: &AsyncContext,
    bridge: &Bridge,
    budget: &Budget,
    hooks: &Hooks,
    command: Command,
    options: &RuntimeOptions,
) -> Result<Option<Pending>, ScriptError> {
    let Command {
        hook_id,
        context: input,
        deadline,
        cancellation,
        enqueued_at,
        response,
    } = command;
    let mut completion = Completion {
        cancellation,
        deadline,
        response,
        timings: HookTimings {
            queue: Some(enqueued_at.elapsed()),
            ..HookTimings::default()
        },
        pending_since: None,
    };
    if let Some(error) = completion.invalid() {
        completion.finish(Err(error));
        return Ok(None);
    }
    let Some(hook) = lookup_hook(hooks, hook_id) else {
        completion.finish(Err(ScriptError::new(ScriptErrorKind::UnknownHook)));
        return Ok(None);
    };
    let result = context
        .with(|ctx| {
            let hook = hook
                .clone()
                .restore(&ctx)
                .map_err(|e| bridge.failure(&ctx, e))?;
            let input_started = Instant::now();
            let draft = (|| {
                let draft = ctx.json_parse(input).map_err(|e| bridge.failure(&ctx, e))?;
                let prepare = bridge
                    .prepare
                    .clone()
                    .restore(&ctx)
                    .map_err(|e| bridge.failure(&ctx, e))?;
                prepare
                    .call::<_, Value>((draft,))
                    .map_err(|e| bridge.failure(&ctx, e))
            })();
            completion.timings.input_bridge = input_started.elapsed();
            let draft = draft?;
            let argument = draft.clone();
            let call_started = Instant::now();
            let result = hook.call::<_, Value>((argument,));
            completion.timings.initial_call = Some(call_started.elapsed());
            let returned_at = Instant::now();
            let result = result.map_err(|e| bridge.failure(&ctx, e))?;
            if result.is_undefined() {
                let json = timed_output(&ctx, bridge, draft, options, &mut completion.timings)?;
                Ok(Ok(json))
            } else if let Some(promise) = result.into_promise() {
                completion.pending_since = Some(returned_at);
                Ok(Err((
                    Persistent::save(&ctx, draft),
                    Persistent::save(&ctx, promise),
                )))
            } else {
                Err(ScriptError::new(ScriptErrorKind::Contract))
            }
        })
        .await;
    let result = budget.check_result(result);
    match result {
        Ok(Ok(value)) => {
            completion.send(Ok(value));
            Ok(None)
        },
        Ok(Err((draft, promise))) => Ok(Some(Pending {
            completion,
            draft,
            promise,
        })),
        Err(error) => {
            completion.finish(Err(error.clone()));
            if fatal(&error) { Err(error) } else { Ok(None) }
        },
    }
}

/// Captured hooks receive contiguous one-based IDs in the bridge's array order.
fn lookup_hook<T>(hooks: &[T], hook_id: HookId) -> Option<&T> {
    let index = usize::try_from(hook_id.checked_sub(1)?).ok()?;
    hooks.get(index)
}

fn timed_output<'js>(
    ctx: &Ctx<'js>,
    bridge: &Bridge,
    draft: Value<'js>,
    options: &RuntimeOptions,
    timings: &mut HookTimings,
) -> Result<Json, ScriptError> {
    let started = Instant::now();
    let result = output(ctx, bridge, draft, options);
    timings.output_bridge = Some(started.elapsed());
    result
}

fn output<'js>(
    ctx: &Ctx<'js>,
    bridge: &Bridge,
    draft: Value<'js>,
    options: &RuntimeOptions,
) -> Result<Json, ScriptError> {
    let project = bridge
        .project
        .clone()
        .restore(ctx)
        .map_err(|e| bridge.failure(ctx, e))?;
    let draft: Value = project.call((draft,)).map_err(|e| bridge.failure(ctx, e))?;
    let function = bridge
        .convert
        .clone()
        .restore(ctx)
        .map_err(|e| bridge.failure(ctx, e))?;
    let converted: Object = function
        .call((draft, false))
        .map_err(|e| conversion_error(bridge.failure(ctx, e)))?;
    let text: String = converted.get("json").map_err(|e| bridge.failure(ctx, e))?;
    if text.len() > options.json_limits.max_bytes {
        return Err(ScriptError::new(ScriptErrorKind::Json));
    }
    serde_json::from_str(&text)
        .map_err(|error| ScriptError::with_source(ScriptErrorKind::Json, error))
}

async fn poll_pending(
    context: &AsyncContext,
    bridge: &Bridge,
    budget: &Budget,
    options: &RuntimeOptions,
    pending: &mut Vec<Pending>,
) -> Result<(), ScriptError> {
    let mut index = 0;
    while index < pending.len() {
        if let Some(error) = pending[index].completion.invalid() {
            let waiting = pending.swap_remove(index);
            waiting.completion.finish(Err(error));
            continue;
        }
        budget.arm(Instant::now() + options.execution_timeout);
        let result = context
            .with(|ctx| {
                let waiting = &mut pending[index];
                let promise = waiting
                    .promise
                    .clone()
                    .restore(&ctx)
                    .map_err(|e| bridge.failure(&ctx, e))?;
                let Some(result) = promise.result::<Value>() else {
                    return Ok(None);
                };
                waiting.completion.observe_pending();
                let value = result.map_err(|e| bridge.failure(&ctx, e))?;
                if !value.is_undefined() {
                    return Err(ScriptError::new(ScriptErrorKind::Contract));
                }
                let draft = waiting
                    .draft
                    .clone()
                    .restore(&ctx)
                    .map_err(|e| bridge.failure(&ctx, e))?;
                timed_output(
                    &ctx,
                    bridge,
                    draft,
                    options,
                    &mut waiting.completion.timings,
                )
                .map(Some)
            })
            .await;
        match budget.check_result(result) {
            Ok(None) => index += 1,
            Ok(Some(value)) => pending.swap_remove(index).completion.send(Ok(value)),
            Err(error) => {
                pending
                    .swap_remove(index)
                    .completion
                    .send(Err(error.clone()));
                if fatal(&error) {
                    return Err(error);
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
