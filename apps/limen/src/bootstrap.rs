use std::{
    fmt,
    path::Path,
    time::{Duration, Instant},
};

use limen_config::{BusinessConfig, ConfigError, ResourceLimits, ServerConfig, load_server};
use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptError, ScriptRuntime};

/// Both commands retain the same runtime and validated configuration until shutdown.
pub struct Bootstrap {
    pub server: ServerConfig,
    pub business: BusinessConfig,
    pub runtime: ScriptRuntime,
}

#[derive(Debug)]
pub enum Error {
    Config {
        file: &'static str,
        error: ConfigError,
    },
    Script(ScriptError),
    StartupTimeout,
    ShutdownTimeout,
    Logging,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { file, error } => write!(formatter, "{file}: {error}"),
            Self::Script(error) => write!(formatter, "config.js: {error}"),
            Self::StartupTimeout => formatter.write_str("configuration startup budget exceeded"),
            Self::ShutdownTimeout => formatter.write_str("JavaScript shutdown budget exceeded"),
            Self::Logging => formatter.write_str("cannot initialize logging"),
        }
    }
}

impl std::error::Error for Error {}

impl Bootstrap {
    /// Resolve the script relative to the selected YAML file, independent of later cwd changes.
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let server = load_server(path).map_err(|error| Error::Config {
            file: "server.yaml",
            error,
        })?;
        crate::telemetry::initialize(&server.log).map_err(|_error| Error::Logging)?;
        let script = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("config.js");
        let budget = Duration::from_millis(server.limits.startup_ms);
        let started = Instant::now();
        let (runtime, exported) = tokio::time::timeout(
            budget,
            ScriptRuntime::start(script, runtime_options(&server.limits)),
        )
        .await
        .map_err(|_error| Error::StartupTimeout)?
        .map_err(Error::Script)?;
        let business =
            BusinessConfig::from_value(exported, &server.limits).map_err(|error| Error::Config {
                file: "config.js",
                error,
            });
        let result = if started.elapsed() >= budget {
            Err(Error::StartupTimeout)
        } else {
            business
        };
        match result {
            Ok(business) => Ok(Self {
                server,
                business,
                runtime,
            }),
            Err(error) => {
                stop(&runtime, server.limits.shutdown_ms).await?;
                Err(error)
            },
        }
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        stop(&self.runtime, self.server.limits.shutdown_ms).await
    }
}

async fn stop(runtime: &ScriptRuntime, timeout_ms: u64) -> Result<(), Error> {
    match tokio::time::timeout(Duration::from_millis(timeout_ms), runtime.shutdown()).await {
        Ok(RuntimeStatus::Stopped) => Ok(()),
        Ok(RuntimeStatus::Retired(error)) => Err(Error::Script(error)),
        Ok(_) | Err(_) => Err(Error::ShutdownTimeout),
    }
}

fn runtime_options(limits: &ResourceLimits) -> RuntimeOptions {
    RuntimeOptions {
        startup_timeout: Duration::from_millis(limits.startup_ms),
        execution_timeout: Duration::from_millis(limits.execution_ms),
        continuous_work_timeout: Duration::from_millis(limits.vm_work_ms),
        memory_limit: limits.memory_bytes,
        stack_limit: limits.stack_bytes,
        queue_capacity: limits.queue_capacity,
        pending_capacity: limits.max_pending,
        job_batch: limits.job_batch,
        max_modules: limits.max_modules,
        max_source_bytes: limits.max_source_bytes,
        json_limits: JsonLimits {
            max_depth: limits.max_json_depth,
            max_nodes: limits.max_json_nodes,
            max_bytes: limits.max_output_bytes,
        },
    }
}
