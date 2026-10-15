use std::{
    fmt,
    path::Path,
    time::{Duration, Instant},
};

use limen_config::{BusinessConfig, ConfigError, ResourceLimits, ServerConfig, load_server};
use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptError, ScriptRuntime};
use tracing::Instrument;
use tracing_subscriber::util::TryInitError;

/// Both commands retain the same runtime and validated configuration until shutdown.
pub struct Bootstrap {
    pub server: ServerConfig,
    pub business: BusinessConfig,
    pub runtime: ScriptRuntime,
}

#[derive(thiserror::Error)]
pub enum Error {
    #[error("cannot load {file}")]
    Config {
        file: &'static str,
        #[source]
        source: Box<ConfigError>,
    },
    #[error("config.js runtime failed")]
    Script(#[from] ScriptError),
    #[error("configuration startup budget exceeded")]
    StartupTimeout(#[source] tokio::time::error::Elapsed),
    #[error("configuration validation exceeded the startup budget")]
    StartupBudget,
    #[error("JavaScript shutdown budget exceeded")]
    ShutdownTimeout(#[source] tokio::time::error::Elapsed),
    #[error("JavaScript shutdown returned a nonterminal state: {state}")]
    ShutdownState { state: &'static str },
    #[error("cannot initialize logging")]
    Logging(#[source] TryInitError),
    #[error("cannot initialize async executor")]
    Executor(#[source] std::io::Error),
    #[error("HTTP service failed")]
    Http(#[from] crate::server::Error),
    #[error("configuration failed and runtime cleanup also failed")]
    Cleanup {
        #[source]
        primary: Box<Self>,
        cleanup: Box<Self>,
    },
}

/// Render only approved metadata; raw sources remain available for internal inspection.
pub struct Diagnostic<'a>(&'a Error);

impl Error {
    pub fn diagnostic(&self) -> Diagnostic<'_> { Diagnostic(self) }

    pub fn phase(&self) -> &'static str {
        match self {
            Self::Config {
                file: "server configuration",
                ..
            } => "server_config",
            Self::Config { .. } | Self::StartupBudget => "business_config",
            Self::Script(_) | Self::StartupTimeout(_) => "script_startup",
            Self::ShutdownTimeout(_) | Self::ShutdownState { .. } => "shutdown",
            Self::Logging(_) => "logging",
            Self::Executor(_) => "executor",
            Self::Http(source) => source.phase(),
            Self::Cleanup { primary, .. } => primary.phase(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.diagnostic(), formatter)
    }
}

impl fmt::Display for Diagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Error::Config { source, .. } => {
                write!(formatter, "{}: {}", self.0, source.diagnostic())
            },
            Error::Script(source) => write!(formatter, "{}: {}", self.0, source.diagnostic()),
            Error::StartupTimeout(source) | Error::ShutdownTimeout(source) => {
                write!(formatter, "{}: {source}", self.0)
            },
            Error::Executor(source) => write!(formatter, "{}: IO {:?}", self.0, source.kind()),
            Error::Http(source) => write!(formatter, "{}: {}", self.0, source.diagnostic()),
            Error::Logging(_) => write!(formatter, "{}: subscriber installation failed", self.0),
            Error::Cleanup { primary, cleanup } => write!(
                formatter,
                "{}; cleanup also failed: {}",
                primary.diagnostic(),
                cleanup.diagnostic()
            ),
            Error::StartupBudget | Error::ShutdownState { .. } => {
                fmt::Display::fmt(self.0, formatter)
            },
        }
    }
}

impl Bootstrap {
    /// Resolve the script relative to the selected server file, independent of later cwd changes.
    pub async fn load(path: Option<&Path>, command: &'static str) -> Result<Self, Error> {
        let (path, server) = Self::settings(path)?;
        Self::start(&path, server)
            .instrument(tracing::info_span!(
                "configuration",
                command,
                file = "config.js"
            ))
            .await
    }

    pub(crate) fn settings(
        path: Option<&Path>,
    ) -> Result<(std::path::PathBuf, ServerConfig), Error> {
        let (path, server) = load_server(path).map_err(|source| Error::Config {
            file: "server configuration",
            source: Box::new(source),
        })?;
        crate::telemetry::initialize(&server.log).map_err(Error::Logging)?;
        Ok((path, server))
    }

    pub(crate) async fn start(path: &Path, server: ServerConfig) -> Result<Self, Error> {
        tracing::debug!(phase = "script_startup", "evaluating local configuration");
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
        .map_err(Error::StartupTimeout)??;
        let business =
            BusinessConfig::from_value(exported, &server.limits).map_err(|source| Error::Config {
                file: "config.js",
                source: Box::new(source),
            });
        let result = if started.elapsed() >= budget {
            Err(Error::StartupBudget)
        } else {
            business
        };
        match result {
            Ok(business) => {
                tracing::info!(
                    providers = business.providers.len(),
                    models = business.models.len(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "configuration loaded"
                );
                Ok(Self {
                    server,
                    business,
                    runtime,
                })
            },
            Err(primary) => Err(with_cleanup(
                primary,
                stop(&runtime, server.limits.shutdown_ms).await,
            )),
        }
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        async {
            let started = Instant::now();
            stop(&self.runtime, self.server.limits.shutdown_ms).await?;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "configuration runtime stopped"
            );
            Ok(())
        }
        .instrument(tracing::info_span!("runtime_shutdown"))
        .await
    }
}

async fn stop(runtime: &ScriptRuntime, timeout_ms: u64) -> Result<(), Error> {
    let status = tokio::time::timeout(Duration::from_millis(timeout_ms), runtime.shutdown())
        .await
        .map_err(Error::ShutdownTimeout)?;
    shutdown_status(status)
}

fn shutdown_status(status: RuntimeStatus) -> Result<(), Error> {
    match status {
        RuntimeStatus::Stopped => Ok(()),
        RuntimeStatus::Retired(error) => Err(Error::Script(error)),
        RuntimeStatus::Starting => Err(Error::ShutdownState { state: "starting" }),
        RuntimeStatus::Ready => Err(Error::ShutdownState { state: "ready" }),
    }
}

fn with_cleanup(primary: Error, cleanup: Result<(), Error>) -> Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => Error::Cleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        },
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
        max_output_events: limits.max_output_events,
        json_limits: JsonLimits {
            max_depth: limits.max_json_depth,
            max_nodes: limits.max_json_nodes,
            max_bytes: limits.max_output_bytes,
        },
    }
}

#[cfg(test)]
mod tests;
