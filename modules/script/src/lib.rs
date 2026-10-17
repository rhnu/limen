//! Bounded JavaScript configuration and hook execution on one owning thread.

mod bridge;
mod builtins;
mod context;
mod engine;
mod error;
mod host;
mod json;
mod loader;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tracing::Instrument;

pub type HookId = u64;

// QuickJS checks native stack consumption below each rquickjs entry, not the
// owning thread's OS stack limit. Leave the host its own Rust/Tokio/JSON reserve
// so a valid VM budget cannot run through the OS guard before QuickJS rejects it.
const HOST_STACK_RESERVE: usize = 2 * 1024 * 1024;
const MAX_STACK_LIMIT: usize = 8 * 1024 * 1024;

/// Conservative limits; these are bounds, not measured performance promises.
#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub startup_timeout: Duration,
    pub execution_timeout: Duration,
    pub continuous_work_timeout: Duration,
    pub memory_limit: usize,
    /// Native-stack budget for `QuickJS`; the owning thread adds a fixed host reserve.
    pub stack_limit: usize,
    pub queue_capacity: usize,
    pub pending_capacity: usize,
    pub job_batch: usize,
    pub max_modules: usize,
    pub max_source_bytes: usize,
    pub max_output_events: usize,
    pub observation_names: Vec<String>,
    pub max_observations_per_hook: usize,
    pub json_limits: JsonLimits,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(5),
            execution_timeout: Duration::from_millis(100),
            continuous_work_timeout: Duration::from_secs(1),
            memory_limit: 32 * 1024 * 1024,
            stack_limit: 256 * 1024,
            queue_capacity: 64,
            pending_capacity: 64,
            job_batch: 32,
            max_modules: 64,
            max_source_bytes: 1024 * 1024,
            max_output_events: 64,
            observation_names: Vec::new(),
            max_observations_per_hook: 8,
            json_limits: JsonLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JsonLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_bytes: usize,
}

impl Default for JsonLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: 100_000,
            max_bytes: 4 * 1024 * 1024,
        }
    }
}

pub use error::{ScriptError, ScriptErrorKind};

#[derive(Clone, Debug)]
pub enum RuntimeStatus {
    Starting,
    Ready,
    Retired(ScriptError),
    Stopped,
}

#[derive(Debug, Default)]
struct CancelState {
    flag: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<CancelState>);

impl Cancellation {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool { self.0.flag.load(Ordering::Acquire) }

    /// Wait until cancellation, including cancellation signalled before this call.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// The draft remains private until successful completion, validation and deadline checks.
pub struct HookCall {
    pub call_id: u64,
    pub hook_id: HookId,
    pub context: Value,
    pub deadline: Instant,
    pub cancellation: Cancellation,
}

/// Owned draft released only after the hook's completion checks succeed.
#[derive(Debug)]
pub struct HookOutput {
    pub context: Value,
    pub observations: ObservationBatch,
}

/// Registered event IDs collected privately by one successful hook.
#[derive(Debug, Default, Clone)]
pub struct ObservationBatch {
    pub events: Vec<u8>,
    pub dropped: u64,
}

/// Elapsed durations, rather than CPU attribution. Missing phases were not sampled.
#[derive(Clone, Copy, Debug, Default)]
pub struct HookTimings {
    /// Caller entry until the report is ready, including scheduling and bridges.
    pub wall: Duration,
    /// Successful enqueue until the VM begins its admission decision.
    pub queue: Option<Duration>,
    /// Only the initial JavaScript function call, before any Promise jobs.
    pub initial_call: Option<Duration>,
    /// Promise return until settlement or failure is observed, including scheduling.
    pub pending: Option<Duration>,
    /// Caller JSON serialization plus VM JSON parsing and draft preparation.
    pub input_bridge: Duration,
    /// Draft projection and conversion into owned JSON, when attempted.
    pub output_bridge: Option<Duration>,
}

/// Every returned invocation carries its outcome and all available timing samples.
#[derive(Debug)]
pub struct HookReport {
    pub outcome: Result<HookOutput, ScriptError>,
    pub timings: HookTimings,
}

impl HookReport {
    fn new(outcome: Result<HookOutput, ScriptError>, timings: HookTimings) -> Self {
        Self { outcome, timings }
    }

    fn failed(error: ScriptError) -> Self { Self::new(Err(error), HookTimings::default()) }
}

pub(crate) struct Command {
    hook_id: HookId,
    context: Vec<u8>,
    deadline: Instant,
    cancellation: Cancellation,
    enqueued_at: Instant,
    response: oneshot::Sender<HookReport>,
}

struct WakeOnDrop(Arc<Notify>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) { self.0.notify_one(); }
}

#[derive(Clone)]
pub struct ScriptRuntime {
    commands: mpsc::Sender<Command>,
    status: watch::Receiver<RuntimeStatus>,
    stop: Arc<AtomicBool>,
    wake: Arc<Notify>,
    limits: JsonLimits,
}

impl ScriptRuntime {
    /// Evaluate local ESM once and retain its hook functions in the owning VM.
    /// Recognized hook slots in the returned configuration contain numeric IDs.
    ///
    /// # Errors
    /// Rejects invalid limits, failed local modules, invalid JSON and startup budgets.
    pub async fn start(
        path: PathBuf,
        options: RuntimeOptions,
    ) -> Result<(Self, Value), ScriptError> {
        async move {
            if !valid_options(&options) {
                return Err(ScriptError::new(ScriptErrorKind::InvalidOptions));
            }
            let (commands, receiver) = mpsc::channel(options.queue_capacity);
            let (status_tx, status) = watch::channel(RuntimeStatus::Starting);
            let (ready_tx, ready) = oneshot::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let wake = Arc::new(Notify::new());
            let thread_stop = Arc::clone(&stop);
            let thread_wake = Arc::clone(&wake);
            let limits = options.json_limits;
            let runtime_span = tracing::debug_span!("script_runtime_thread").or_current();
            std::thread::Builder::new()
                .name("limen-javascript".into())
                .stack_size(options.stack_limit + HOST_STACK_RESERVE)
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let executor = tokio::runtime::Builder::new_current_thread()
                            .enable_time()
                            .build()
                            .map_err(|error| {
                                ScriptError::with_source(ScriptErrorKind::Thread, error)
                            })?;
                        executor.block_on(
                            engine::run(
                                path,
                                options,
                                receiver,
                                ready_tx,
                                &status_tx,
                                thread_stop,
                                thread_wake,
                            )
                            .instrument(runtime_span),
                        )
                    }));
                    let status = match result {
                        Ok(Ok(())) => RuntimeStatus::Stopped,
                        Ok(Err(e)) => RuntimeStatus::Retired(e),
                        Err(_) => RuntimeStatus::Retired(ScriptError::with_source(
                            ScriptErrorKind::Thread,
                            error::ThreadFailure::Panic,
                        )),
                    };
                    let _sent = status_tx.send(status);
                })
                .map_err(|error| ScriptError::with_source(ScriptErrorKind::Thread, error))?;
            let handle = Self {
                commands,
                status,
                stop,
                wake,
                limits,
            };
            match ready.await {
                Ok(Ok(config)) => Ok((handle, config)),
                Ok(Err(e)) => Err(e),
                Err(source) => {
                    // The sender may drop before the thread publishes its actual failure.
                    let error = match handle.shutdown().await {
                        RuntimeStatus::Retired(error) => error,
                        _ => ScriptError::with_source(ScriptErrorKind::Thread, source),
                    };
                    Err(error)
                },
            }
        }
        .instrument(tracing::debug_span!("script_start"))
        .await
    }

    #[must_use]
    pub fn status(&self) -> RuntimeStatus { self.status.borrow().clone() }

    /// Observe lifecycle changes without polling; terminal states follow VM teardown.
    #[must_use]
    pub fn status_changes(&self) -> watch::Receiver<RuntimeStatus> { self.status.clone() }

    fn retirement_or(&self, fallback: ScriptError) -> ScriptError {
        match self.status() {
            RuntimeStatus::Retired(error) => error,
            _ => fallback,
        }
    }

    /// Execute one hook and report its validated draft or failure with available timings.
    /// Dropping this future prevents reporting; it does not stop pending JavaScript work.
    pub async fn invoke(&self, call: HookCall) -> HookReport {
        let started = Instant::now();
        let span = tracing::debug_span!(
            "script_invoke",
            call_id = call.call_id,
            hook_id = call.hook_id
        );
        let HookCall {
            hook_id,
            context,
            deadline,
            cancellation,
            ..
        } = call;
        let input_started = Instant::now();
        let bytes = json::encode_owned(&context, self.limits);
        let caller_input = input_started.elapsed();
        drop(context);
        let mut report = match bytes {
            Ok(bytes) => {
                self.invoke_encoded(hook_id, bytes, deadline, cancellation)
                    .instrument(span)
                    .await
            },
            Err(error) => HookReport::failed(error),
        };
        report.timings.input_bridge += caller_input;
        report.timings.wall = started.elapsed();
        report
    }

    async fn invoke_encoded(
        &self,
        hook_id: HookId,
        bytes: Vec<u8>,
        deadline: Instant,
        cancellation: Cancellation,
    ) -> HookReport {
        if cancellation.is_cancelled() {
            return HookReport::failed(ScriptError::new(ScriptErrorKind::Cancelled));
        }
        if Instant::now() >= deadline {
            return HookReport::failed(ScriptError::new(ScriptErrorKind::Deadline));
        }
        let _wake_on_drop = WakeOnDrop(Arc::clone(&self.wake));
        let (response, mut receive) = oneshot::channel();
        let sent = self
            .commands
            .try_send(Command {
                hook_id,
                context: bytes,
                deadline,
                cancellation: cancellation.clone(),
                enqueued_at: Instant::now(),
                response,
            })
            .map_err(|error| {
                let kind = match &error {
                    mpsc::error::TrySendError::Full(_) => ScriptErrorKind::Busy,
                    mpsc::error::TrySendError::Closed(_) => ScriptErrorKind::Retired,
                };
                let error = ScriptError::with_source(kind, error);
                if kind == ScriptErrorKind::Retired {
                    self.retirement_or(error)
                } else {
                    error
                }
            });
        if let Err(error) = sent {
            return HookReport::failed(error);
        }
        let mut report = tokio::select! {
            biased;
            () = cancellation.cancelled() => receive.try_recv().unwrap_or_else(|_| HookReport::failed(ScriptError::new(ScriptErrorKind::Cancelled))),
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => receive.try_recv().unwrap_or_else(|_| HookReport::failed(ScriptError::new(ScriptErrorKind::Deadline))),
            result = &mut receive => result.unwrap_or_else(|error| HookReport::failed(self.retirement_or(ScriptError::with_source(ScriptErrorKind::Retired, error)))),
        };
        receive.close();
        if cancellation.is_cancelled() {
            report.outcome = Err(ScriptError::new(ScriptErrorKind::Cancelled));
        } else if Instant::now() >= deadline {
            report.outcome = Err(ScriptError::new(ScriptErrorKind::Deadline));
        }
        report
    }

    /// Completion is reported only after pending refs, hooks, context and VM drop.
    pub async fn shutdown(&self) -> RuntimeStatus {
        self.stop.store(true, Ordering::Release);
        self.wake.notify_one();
        let mut status = self.status.clone();
        loop {
            let current = status.borrow_and_update().clone();
            if matches!(current, RuntimeStatus::Stopped | RuntimeStatus::Retired(_)) {
                return current;
            }
            if let Err(error) = status.changed().await {
                return RuntimeStatus::Retired(ScriptError::with_source(
                    ScriptErrorKind::Thread,
                    error,
                ));
            }
        }
    }
}

fn valid_options(o: &RuntimeOptions) -> bool {
    let now = Instant::now();
    !o.startup_timeout.is_zero()
        && !o.execution_timeout.is_zero()
        && !o.continuous_work_timeout.is_zero()
        && now.checked_add(o.startup_timeout).is_some()
        && now.checked_add(o.execution_timeout).is_some()
        && now.checked_add(o.continuous_work_timeout).is_some()
        && o.memory_limit > 0
        && (1..=MAX_STACK_LIMIT).contains(&o.stack_limit)
        && o.queue_capacity > 0
        && o.pending_capacity > 0
        && o.job_batch > 0
        && o.max_modules > 0
        && o.max_source_bytes > 0
        && o.max_output_events > 0
        && (1..=64).contains(&o.max_observations_per_hook)
        && valid_observation_names(&o.observation_names)
        && o.json_limits.max_depth > 0
        && o.json_limits.max_depth <= 128
        && o.json_limits.max_nodes > 0
        && o.json_limits.max_bytes > 0
}

fn valid_observation_names(names: &[String]) -> bool {
    names.len() <= 64
        && names.iter().enumerate().all(|(index, name)| {
            (1..=64).contains(&name.len())
                && name.as_bytes()[0].is_ascii_lowercase()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && !names[..index].contains(name)
        })
}

#[cfg(test)]
mod tests;
