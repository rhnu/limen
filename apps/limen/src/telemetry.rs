use limen_config::{LogConfig, LogFormat, LogLevel};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};

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
        .with_target(false)
        .without_time();
    match config.format {
        LogFormat::Text => subscriber.finish().try_init(),
        LogFormat::Json => subscriber.json().finish().try_init(),
    }
}
