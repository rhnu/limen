use std::sync::atomic::{AtomicBool, Ordering};

use limen_config::{ConfigErrorKind, LogConfig, LogFormat, LogLevel};
use limen_script::ScriptErrorKind;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};

use crate::bootstrap::Error;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Panic payloads can contain user configuration; the process boundary reports location only.
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

/// Install only application metadata logging; arbitrary script text is never logged.
pub fn initialize(config: &LogConfig) -> Result<(), TryInitError> {
    let level = match config.level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .without_time();
    match config.format {
        LogFormat::Text => subscriber.finish().try_init(),
        LogFormat::Json => subscriber.json().finish().try_init(),
    }?;
    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// The application reports each failure once, after all cleanup results are available.
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
        tracing::warn!(command, phase, error = %error.diagnostic(), "configuration rejected");
    } else {
        tracing::error!(command, phase, error = %error.diagnostic(), "configuration operation failed");
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
