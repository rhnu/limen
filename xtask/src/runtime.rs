use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use limen_script::{
    Cancellation, ClientContext, HookCall, HookContext, HookInput, HookMeta, RuntimeOptions,
    RuntimeStatus, ScriptErrorKind, ScriptRuntime,
};
use serde_json::{Value, json};

pub(crate) mod environment;
mod json_depth;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

/// The external timeout driver owns and removes the temporary root.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(root: PathBuf) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if !root.is_dir() || fs::read_dir(&root)?.next().is_some() {
            return Err("fixture directory must exist and be empty".into());
        }
        Ok(Self { root })
    }

    fn write(&self, relative: &str, source: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, source)?;
        Ok(path)
    }
}

pub const CASES: &[&str] = &[
    #[cfg(windows)]
    "process-tree",
    "environment",
    "sync-loop",
    "microtask-loop",
    "memory-limit",
    "startup-loop",
    "json-depth-startup",
    "json-depth-hook",
    "stack-limit",
];

pub async fn run(case: &str, root: PathBuf) -> TestResult {
    let fixture = Fixture::new(root)?;
    match case {
        #[cfg(windows)]
        "process-tree" => crate::process::experiments::run(&fixture.root)
            .await
            .map_err(Into::into),
        "environment" => environment::run(&fixture).await,
        "sync-loop" | "microtask-loop" | "memory-limit" => retirement(&fixture, case).await,
        "startup-loop" => startup_loop(&fixture).await,
        "json-depth-startup" => json_depth::startup(&fixture).await,
        "json-depth-hook" => json_depth::hooks(&fixture).await,
        "stack-limit" => json_depth::stack_limit(&fixture).await,
        _ => Err("unknown experiment".into()),
    }
}

fn hook(config: &Value, name: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
    config[name]
        .as_u64()
        .ok_or_else(|| format!("missing registered {name}").into())
}

fn call(hook_id: u64, call_id: u64, context: HookContext) -> HookCall {
    HookCall {
        call_id,
        hook_id,
        context,
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: Cancellation::new(),
    }
}

fn context(input: HookInput, state: Value) -> HookContext {
    let Value::Object(state) = state else {
        panic!("probe state fixtures are objects");
    };
    HookContext {
        meta: HookMeta {
            request_id: "isolated-probe".into(),
            ..HookMeta::default()
        },
        client: ClientContext::default(),
        state,
        input,
    }
}

fn request(state: Value) -> HookContext { context(HookInput::OnRequest { body: json!({}) }, state) }

fn ensure(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

async fn stop(runtime: &ScriptRuntime) -> TestResult {
    ensure(
        matches!(runtime.shutdown().await, RuntimeStatus::Stopped),
        "ordinary VM shutdown failed",
    )
}

async fn retirement(fixture: &Fixture, case: &str) -> TestResult {
    let (sources, expected): (&[&str], _) = match case {
        "sync-loop" => (
            &[include_str!("../fixtures/runtime/sync-loop-1.js")],
            ScriptErrorKind::ExecutionBudget,
        ),
        "microtask-loop" => (
            &[include_str!("../fixtures/runtime/microtask-loop-1.js")],
            ScriptErrorKind::ExecutionBudget,
        ),
        "memory-limit" => (
            &[
                include_str!("../fixtures/runtime/memory-limit-1.js"),
                include_str!("../fixtures/runtime/memory-limit-2.js"),
                include_str!("../fixtures/runtime/memory-limit-3.js"),
                include_str!("../fixtures/runtime/memory-limit-4.js"),
                include_str!("../fixtures/runtime/memory-limit-5.js"),
            ],
            ScriptErrorKind::MemoryLimit,
        ),
        _ => return Err("invalid retirement experiment".into()),
    };
    let mut options = RuntimeOptions {
        execution_timeout: Duration::from_millis(40),
        continuous_work_timeout: Duration::from_millis(160),
        ..RuntimeOptions::default()
    };
    if case == "memory-limit" {
        options.memory_limit = 8 * 1024 * 1024;
        options.execution_timeout = Duration::from_secs(2);
        options.continuous_work_timeout = Duration::from_secs(3);
    }
    for source in sources {
        let path = fixture.write("gateway.js", source)?;
        let (runtime, config) = ScriptRuntime::start(path, options.clone()).await?;
        let mut changes = runtime.status_changes();
        changes.borrow_and_update();
        let result = runtime
            .invoke(call(hook(&config, "onRequest")?, 1, request(json!({}))))
            .await;
        ensure(
            result.outcome.is_err_and(|error| error.kind() == expected),
            "resource failure or conservative OOM signal had the wrong classification",
        )?;
        // Retiring the VM must wake an HTTP server even when there is no next request.
        while !matches!(*changes.borrow_and_update(), RuntimeStatus::Retired(_)) {
            changes.changed().await?;
        }
        let status = runtime.shutdown().await;
        ensure(
            matches!(status, RuntimeStatus::Retired(error) if error.kind() == expected),
            "resource failure did not retire the VM",
        )?;
        let later = runtime
            .invoke(call(hook(&config, "onRequest")?, 2, request(json!({}))))
            .await;
        ensure(later.outcome.is_err(), "retired VM accepted a new call")?;
    }
    Ok(())
}

async fn startup_loop(fixture: &Fixture) -> TestResult {
    for source in [
        include_str!("../fixtures/runtime/startup-sync.js"),
        include_str!("../fixtures/runtime/startup-pending.js"),
    ] {
        let path = fixture.write("gateway.js", source)?;
        let options = RuntimeOptions {
            startup_timeout: Duration::from_millis(200),
            execution_timeout: Duration::from_millis(40),
            ..RuntimeOptions::default()
        };
        match ScriptRuntime::start(path, options).await {
            Err(error)
                if matches!(
                    error.kind(),
                    ScriptErrorKind::ExecutionBudget | ScriptErrorKind::Deadline
                ) => {},
            Err(error) => {
                return Err(format!("startup limit used unexpected error: {error}").into());
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("unbounded startup was accepted".into());
            },
        }
    }
    Ok(())
}
