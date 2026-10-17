use std::{
    fmt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use limen_config::{BusinessConfig, ConfigError, ServerConfig, load_server};
use limen_script::{JsonLimits, RuntimeOptions, RuntimeStatus, ScriptError, ScriptRuntime};
use limen_storage::{Storage, StorageOptions};
use tracing::Instrument;

/// Both commands retain the same runtime and validated configuration until shutdown.
pub struct Bootstrap {
    pub server: ServerConfig,
    pub business: BusinessConfig,
    pub(crate) native_content: Arc<crate::gateway::NativeContentTemplates>,
    pub runtime: ScriptRuntime,
    pub run_id: String,
    pub started_at: String,
    pub storage: Option<Storage>,
    pub logging: Option<crate::telemetry::Logging>,
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
    #[error("native policy configuration failed")]
    NativeContent(#[from] limen_chat::NativeContentError),
    #[error("configuration startup budget exceeded")]
    StartupTimeout(#[source] tokio::time::error::Elapsed),
    #[error("configuration validation exceeded the startup budget")]
    StartupBudget,
    #[error("JavaScript shutdown budget exceeded")]
    ShutdownTimeout(#[source] tokio::time::error::Elapsed),
    #[error("JavaScript shutdown returned a nonterminal state: {state}")]
    ShutdownState { state: &'static str },
    #[error("cannot initialize logging")]
    Logging(#[source] crate::telemetry::InitError),
    #[error("request storage failed")]
    Storage(#[from] limen_storage::Error),
    #[error("startup worker failed")]
    StartupWorker(#[from] tokio::task::JoinError),
    #[error("log shutdown budget exceeded")]
    LogShutdown,
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
            Self::Config { .. } | Self::StartupBudget | Self::NativeContent(_) => "business_config",
            Self::Script(_) | Self::StartupTimeout(_) => "script_startup",
            Self::ShutdownTimeout(_) | Self::ShutdownState { .. } | Self::LogShutdown => "shutdown",
            Self::Logging(_) => "logging",
            Self::Storage(_) => "storage",
            Self::StartupWorker(_) => "startup",
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
            Error::NativeContent(source) => write!(formatter, "{}: {source}", self.0),
            Error::StartupTimeout(source) | Error::ShutdownTimeout(source) => {
                write!(formatter, "{}: {source}", self.0)
            },
            Error::Executor(source) | Error::Logging(crate::telemetry::InitError::Io(source)) => {
                write!(formatter, "{}: IO {:?}", self.0, source.kind())
            },
            Error::Http(source) => write!(formatter, "{}: {}", self.0, source.diagnostic()),
            Error::Logging(crate::telemetry::InitError::Subscriber(_)) => {
                write!(formatter, "{}: subscriber installation failed", self.0)
            },
            Error::Storage(source) => write!(formatter, "{}: {}", self.0, source.code()),
            Error::Cleanup { primary, cleanup } => write!(
                formatter,
                "{}; cleanup also failed: {}",
                primary.diagnostic(),
                cleanup.diagnostic()
            ),
            Error::StartupBudget
            | Error::ShutdownState { .. }
            | Error::StartupWorker(_)
            | Error::LogShutdown => fmt::Display::fmt(self.0, formatter),
        }
    }
}

impl Bootstrap {
    /// Resolve the script relative to the selected server file, independent of later cwd changes.
    pub async fn load(path: Option<&Path>, command: &'static str) -> Result<Self, Error> {
        let (path, server) = Self::settings(path)?;
        let deadline = Instant::now() + Duration::from_millis(server.limits.startup_ms);
        let run_id = uuid::Uuid::new_v4().to_string();
        let resources = if command == "serve" {
            let options = StorageOptions {
                dir: server.storage.dir.clone(),
                run_id: run_id.clone(),
                retention_days: server.storage.retention_days,
                max_records: server.storage.max_records,
                max_db_bytes: server.storage.max_db_bytes,
                generation_concurrency: server.limits.concurrency,
                management_concurrency: server
                    .auth
                    .manager_key
                    .as_ref()
                    .map(|_| server.management.concurrency),
            };
            let log_config = server.log.clone();
            let log_run = run_id.clone();
            let storage = tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                tokio::task::spawn_blocking(move || Storage::open(options, deadline)),
            )
            .await
            .map_err(Error::StartupTimeout)???;
            let logging = match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                tokio::task::spawn_blocking(move || {
                    crate::telemetry::initialize(&log_config, &log_run)
                }),
            )
            .await
            .map_err(Error::StartupTimeout)
            .and_then(|result| result.map_err(Error::from))
            .and_then(|result| result.map_err(Error::Logging))
            {
                Ok(logging) => logging,
                Err(error) => {
                    let stopped = storage
                        .shutdown(Instant::now() + Duration::from_millis(server.limits.shutdown_ms))
                        .await
                        .map_err(Error::Storage);
                    return Err(with_cleanup(error, stopped));
                },
            };
            Some((storage, logging))
        } else {
            None
        };
        let shutdown_ms = server.limits.shutdown_ms;
        let result = Self::start_until(&path, server, deadline, run_id)
            .instrument(tracing::info_span!(
                "configuration",
                command,
                file = "config.js"
            ))
            .await;
        match result {
            Ok(mut loaded) => {
                if let Some((storage, logging)) = resources {
                    loaded.storage = Some(storage);
                    loaded.logging = Some(logging);
                }
                Ok(loaded)
            },
            Err(error) => {
                if let Some((storage, logging)) = resources {
                    let stopped = storage
                        .shutdown(Instant::now() + Duration::from_millis(shutdown_ms))
                        .await
                        .map_err(Error::Storage);
                    let error = with_cleanup(error, stopped);
                    let flushed = if logging.shutdown() {
                        Ok(())
                    } else {
                        Err(Error::LogShutdown)
                    };
                    return Err(with_cleanup(error, flushed));
                }
                Err(error)
            },
        }
    }

    pub(crate) fn settings(
        path: Option<&Path>,
    ) -> Result<(std::path::PathBuf, ServerConfig), Error> {
        let (path, server) = load_server(path).map_err(|source| Error::Config {
            file: "server configuration",
            source: Box::new(source),
        })?;
        Ok((path, server))
    }

    pub(crate) async fn start(path: &Path, server: ServerConfig) -> Result<Self, Error> {
        let deadline = Instant::now() + Duration::from_millis(server.limits.startup_ms);
        Self::start_until(path, server, deadline, uuid::Uuid::new_v4().to_string()).await
    }

    async fn start_until(
        path: &Path,
        server: ServerConfig,
        deadline: Instant,
        run_id: String,
    ) -> Result<Self, Error> {
        tracing::debug!(phase = "script_startup", "evaluating local configuration");
        let script = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("config.js");
        let started = Instant::now();
        let (runtime, exported) = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            ScriptRuntime::start(script, runtime_options(&server)),
        )
        .await
        .map_err(Error::StartupTimeout)??;
        let business =
            BusinessConfig::from_value(exported, &server.limits).map_err(|source| Error::Config {
                file: "config.js",
                source: Box::new(source),
            });
        let result = if Instant::now() >= deadline {
            Err(Error::StartupBudget)
        } else {
            business
        };
        match result {
            Ok(business) => {
                let native_content = match crate::gateway::NativeContentTemplates::compile(
                    &business,
                    &server.limits,
                    deadline,
                ) {
                    Ok(templates) => Ok(Arc::new(templates)),
                    Err(limen_chat::NativeContentError::Interrupted) => Err(Error::StartupBudget),
                    Err(source) => Err(Error::NativeContent(source)),
                };
                let native_content = match native_content {
                    Ok(templates) => templates,
                    Err(primary) => {
                        return Err(with_cleanup(
                            primary,
                            stop(&runtime, server.limits.shutdown_ms).await,
                        ));
                    },
                };
                tracing::info!(
                    providers = business.providers.len(),
                    models = business.models.len(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "configuration loaded"
                );
                Ok(Self {
                    server,
                    business,
                    native_content,
                    runtime,
                    run_id,
                    started_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    storage: None,
                    logging: None,
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
            let storage = if let Some(storage) = &self.storage {
                storage
                    .shutdown(
                        Instant::now() + Duration::from_millis(self.server.limits.shutdown_ms),
                    )
                    .await
                    .map_err(Error::Storage)
            } else {
                Ok(())
            };
            let vm = stop(&self.runtime, self.server.limits.shutdown_ms).await;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "configuration runtime stopped"
            );
            if let Err(error) = storage {
                return Err(with_cleanup(error, vm));
            }
            vm?;
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

fn runtime_options(server: &ServerConfig) -> RuntimeOptions {
    let limits = &server.limits;
    RuntimeOptions {
        observation_names: server.observability.events.clone(),
        max_observations_per_hook: limits.max_observations_per_hook,
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
