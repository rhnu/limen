use std::{
    io,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use limen_config::{ConfigErrorKind, LogConfig, LogLevel};
use limen_script::ScriptErrorKind;
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::bootstrap::Error;

mod encoding;
pub mod reader;
mod writer;

pub use writer::LoggingHealth;

static INITIALIZED: AtomicBool = AtomicBool::new(false);
pub const MAX_RECORD_BYTES: usize = 16 * 1024;
const FILE_NAME: &str = "limen.jsonl";

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("cannot initialize log files")]
    Io(#[from] io::Error),
    #[error("cannot install logging subscriber")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Owns both workers until the application finishes VM and storage shutdown.
pub struct Logging {
    guards: Mutex<Option<(WorkerGuard, WorkerGuard)>>,
    health: LoggingHealth,
    reader: reader::LogReader,
}

impl Logging {
    pub fn health(&self) -> LoggingHealth { self.health.clone() }

    pub fn reader(&self) -> reader::LogReader { self.reader.clone() }

    /// Each appender guard already has bounded send/flush waits; never join it again.
    pub fn shutdown(&self) -> bool {
        let started = Instant::now();
        let guards = self
            .guards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(guards);
        INITIALIZED.store(false, Ordering::Release);
        self.health.wait_stopped(started + Duration::from_secs(3))
            && started.elapsed() <= Duration::from_secs(3)
    }
}

/// Install only the application's approved metadata, encoded before lossy queues.
pub fn initialize(config: &LogConfig, run_id: &str) -> Result<Logging, InitError> {
    let level = match config.level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    };
    let encoder = encoding::Encoder::new(run_id);
    let (layer, guards, health) = writer::create(config, encoder)?;
    tracing_subscriber::registry()
        .with(level)
        .with(layer)
        .try_init()?;
    INITIALIZED.store(true, Ordering::Release);
    Ok(Logging {
        guards: Mutex::new(Some(guards)),
        health,
        reader: reader::LogReader::new(config.dir.clone(), config.max_files, run_id.to_owned()),
    })
}

/// Panic payloads can contain user configuration; report location only.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let line = info.location().map(std::panic::Location::line);
        let column = info.location().map(std::panic::Location::column);
        if INITIALIZED.load(Ordering::Acquire) && tracing::enabled!(tracing::Level::ERROR) {
            tracing::error!(
                phase = "panic",
                line,
                column,
                "process thread panicked; payload omitted"
            );
        } else {
            eprintln!(
                "limen: process thread panicked at line {line:?}, column {column:?}; payload omitted"
            );
        }
    }));
}

/// Report each failure once, after all cleanup results are available.
pub fn report_failure(error: &Error, command: &'static str, phase: &'static str) {
    let enabled = if expected_rejection(error) {
        tracing::enabled!(tracing::Level::WARN)
    } else {
        tracing::enabled!(tracing::Level::ERROR)
    };
    if INITIALIZED.load(Ordering::Acquire) && enabled {
        record_failure(error, command, phase);
    } else {
        eprintln!("limen: {}", error.diagnostic());
    }
}

fn record_failure(error: &Error, command: &'static str, phase: &'static str) {
    if expected_rejection(error) {
        tracing::warn!(command, phase, "configuration rejected");
    } else {
        tracing::error!(command, phase, "configuration operation failed");
    }
}

fn expected_rejection(error: &Error) -> bool {
    match error {
        Error::Config { source, .. } => source.kind != ConfigErrorKind::Io,
        Error::Script(source) => matches!(
            source.kind(),
            ScriptErrorKind::InvalidOptions
                | ScriptErrorKind::Module
                | ScriptErrorKind::JavaScript
                | ScriptErrorKind::Json
                | ScriptErrorKind::Contract
                | ScriptErrorKind::UnknownHook
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
