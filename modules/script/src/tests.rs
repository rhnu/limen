use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{Cancellation, HookCall, RuntimeOptions, RuntimeStatus, ScriptRuntime};

mod context;
mod diagnostics;
mod observations;
mod runtime;
mod timings;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

impl crate::HookReport {
    fn into_context(self) -> Result<Value, crate::ScriptError> {
        self.outcome.map(|output| output.context)
    }
}

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self {
            root: tempfile::tempdir()?,
        })
    }

    fn write(&self, relative: &str, source: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
        let path = self.root.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, source)?;
        Ok(path)
    }

    async fn start(
        &self,
        source: &str,
    ) -> Result<(ScriptRuntime, Value), Box<dyn Error + Send + Sync>> {
        let path = self.write("config.js", source)?;
        let started = ScriptRuntime::start(path, RuntimeOptions::default()).await?;
        ensure(
            matches!(started.0.status(), RuntimeStatus::Ready),
            "successful startup did not publish Ready",
        )?;
        Ok(started)
    }
}

fn hook(config: &Value, name: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
    config[name]
        .as_u64()
        .ok_or_else(|| format!("missing registered {name}").into())
}

// Scheduler tests may use plain owned JSON drafts; phase contract tests supply gateway contexts.
fn call(hook_id: u64, call_id: u64, context: Value) -> HookCall {
    HookCall {
        call_id,
        hook_id,
        context,
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: Cancellation::new(),
    }
}

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

async fn wait_for(runtime: &ScriptRuntime, id: u64, expected: Value) -> TestResult {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if runtime
                .invoke(call(id, 1000, json!({})))
                .await
                .into_context()?
                == expected
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await?
}
