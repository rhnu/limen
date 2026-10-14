//! Bounded JavaScript configuration and hook execution on one owning thread.

mod engine;
mod json;
mod loader;

use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::sync::{Notify, mpsc, oneshot, watch};

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

/// Safe diagnostics deliberately omit JavaScript exception text, source and paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptError {
    InvalidOptions,
    Module,
    JavaScript,
    Json,
    Contract,
    UnknownHook,
    Busy,
    Cancelled,
    Deadline,
    ExecutionBudget,
    MemoryLimit,
    Retired,
    Stopped,
    Thread,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JavaScript runtime: {self:?}")
    }
}

impl std::error::Error for ScriptError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    async fn cancelled(&self) {
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
        if !valid_options(&options) {
            return Err(ScriptError::InvalidOptions);
        }
        let (commands, receiver) = mpsc::channel(options.queue_capacity);
        let (status_tx, status) = watch::channel(RuntimeStatus::Starting);
        let (ready_tx, ready) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let thread_stop = Arc::clone(&stop);
        let thread_wake = Arc::clone(&wake);
        let limits = options.json_limits;
        std::thread::Builder::new()
            .name("limen-javascript".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let executor = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .map_err(|_error| ScriptError::Thread)?;
                    executor.block_on(engine::run(
                        path,
                        options,
                        receiver,
                        ready_tx,
                        &status_tx,
                        thread_stop,
                        thread_wake,
                    ))
                }));
                let status = match result {
                    Ok(Ok(())) => RuntimeStatus::Stopped,
                    Ok(Err(e)) => RuntimeStatus::Retired(e),
                    Err(_) => RuntimeStatus::Retired(ScriptError::Thread),
                };
                let _sent = status_tx.send(status);
            })
            .map_err(|_error| ScriptError::Thread)?;
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
            Err(_) => Err(ScriptError::Thread),
        }
    }

    #[must_use]
    pub fn status(&self) -> RuntimeStatus { *self.status.borrow() }

    /// Execute one hook and return its validated, privately owned draft.
    ///
    /// # Errors
    /// Reports cancellation, deadlines, capacity, hook contracts and VM retirement.
    pub async fn invoke(&self, call: HookCall) -> Result<Value, ScriptError> {
        json::validate_owned(&call.context, self.limits)?;
        if call.request_id.len() > 128 {
            return Err(ScriptError::Contract);
        }
        let cancellation = call.cancellation.clone();
        let deadline = call.deadline;
        if cancellation.is_cancelled() {
            return Err(ScriptError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ScriptError::Deadline);
        }
        let _wake_on_drop = WakeOnDrop(Arc::clone(&self.wake));
        let (response, mut receive) = oneshot::channel();
        self.commands
            .try_send(Command { call, response })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => ScriptError::Busy,
                mpsc::error::TrySendError::Closed(_) => ScriptError::Retired,
            })?;
        self.wake.notify_one();
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ScriptError::Cancelled),
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(ScriptError::Deadline),
            result = &mut receive => result.unwrap_or(Err(ScriptError::Retired)),
        };
        receive.close();
        self.wake.notify_one();
        if cancellation.is_cancelled() {
            return Err(ScriptError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ScriptError::Deadline);
        }
        result
    }

    /// Completion is reported only after pending refs, hooks, context and VM drop.
    pub async fn shutdown(&self) -> RuntimeStatus {
        self.stop.store(true, Ordering::Release);
        self.wake.notify_one();
        let mut status = self.status.clone();
        loop {
            let current = *status.borrow_and_update();
            if matches!(current, RuntimeStatus::Stopped | RuntimeStatus::Retired(_)) {
                return current;
            }
            if status.changed().await.is_err() {
                return RuntimeStatus::Retired(ScriptError::Thread);
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
        && o.json_limits.max_depth > 0
        && o.json_limits.max_depth <= 128
        && o.json_limits.max_nodes > 0
        && o.json_limits.max_bytes > 0
}
