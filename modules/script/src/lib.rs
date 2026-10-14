//! Bounded JavaScript configuration and hook execution on one owning thread.

mod context;
mod engine;
mod error;
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

/// Conservative limits; these are bounds, not measured performance promises.
#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    pub startup_timeout: Duration,
    pub execution_timeout: Duration,
    pub continuous_work_timeout: Duration,
    pub memory_limit: usize,
    pub stack_limit: usize,
    pub queue_capacity: usize,
    pub pending_capacity: usize,
    pub job_batch: usize,
    pub max_modules: usize,
    pub max_source_bytes: usize,
    pub max_output_events: usize,
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
    pub request_id: String,
    pub call_id: u64,
    pub hook_id: HookId,
    pub context: Value,
    pub deadline: Instant,
    pub cancellation: Cancellation,
}

pub(crate) struct Command {
    call: HookCall,
    response: oneshot::Sender<Result<Value, ScriptError>>,
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

    /// Execute one hook and return its validated, privately owned draft.
    ///
    /// # Errors
    /// Reports cancellation, deadlines, capacity, hook contracts and VM retirement.
    pub async fn invoke(&self, call: HookCall) -> Result<Value, ScriptError> {
        let span = tracing::debug_span!(
            "script_invoke",
            call_id = call.call_id,
            hook_id = call.hook_id
        );
        async move {
        json::validate_owned(&call.context, self.limits)?;
        if call.request_id.len() > 128 {
            return Err(ScriptError::new(ScriptErrorKind::Contract));
        }
        let cancellation = call.cancellation.clone();
        let deadline = call.deadline;
        if cancellation.is_cancelled() {
            return Err(ScriptError::new(ScriptErrorKind::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(ScriptError::new(ScriptErrorKind::Deadline));
        }
        let _wake_on_drop = WakeOnDrop(Arc::clone(&self.wake));
        let (response, mut receive) = oneshot::channel();
        self.commands
            .try_send(Command { call, response })
            .map_err(|error| {
                let kind = match &error {
                    mpsc::error::TrySendError::Full(_) => ScriptErrorKind::Busy,
                    mpsc::error::TrySendError::Closed(_) => ScriptErrorKind::Retired,
                };
                let error = ScriptError::with_source(kind, error);
                if kind == ScriptErrorKind::Retired { self.retirement_or(error) } else { error }
            })?;
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ScriptError::new(ScriptErrorKind::Cancelled)),
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(ScriptError::new(ScriptErrorKind::Deadline)),
            result = &mut receive => result.unwrap_or_else(|error| Err(self.retirement_or(ScriptError::with_source(ScriptErrorKind::Retired, error)))),
        };
        receive.close();
        if cancellation.is_cancelled() {
            return Err(ScriptError::new(ScriptErrorKind::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(ScriptError::new(ScriptErrorKind::Deadline));
        }
        result
        }.instrument(span).await
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
        && o.stack_limit > 0
        && o.queue_capacity > 0
        && o.pending_capacity > 0
        && o.job_batch > 0
        && o.max_modules > 0
        && o.max_source_bytes > 0
        && o.max_output_events > 0
        && o.json_limits.max_depth > 0
        && o.json_limits.max_depth <= 128
        && o.json_limits.max_nodes > 0
        && o.json_limits.max_bytes > 0
}
