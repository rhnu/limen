use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};

use limen_config::{LogConfig, LogFormat};
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use serde_json::{Value, json};
use tracing::{
    Event, Subscriber,
    span::{Attributes, Id, Record},
};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use super::{
    FILE_NAME,
    encoding::{self, Encoder, Fields},
    reader,
};

#[derive(Default)]
struct IoHealth {
    failed: AtomicBool,
    errors: AtomicU64,
    dropped: AtomicU64,
    kind: AtomicU8,
    stopped: Mutex<bool>,
    stopped_signal: Condvar,
}

impl IoHealth {
    fn failure(&self, error: &io::Error) {
        self.failed.store(true, Ordering::Release);
        self.errors.fetch_add(1, Ordering::Relaxed);
        let kind = match error.kind() {
            io::ErrorKind::PermissionDenied => 1,
            io::ErrorKind::StorageFull => 2,
            io::ErrorKind::NotFound => 3,
            _ => 4,
        };
        self.kind.store(kind, Ordering::Release);
    }

    fn status(&self, queue: &ErrorCounter) -> Value {
        let dropped = self
            .dropped
            .load(Ordering::Relaxed)
            .saturating_add(queue.dropped_lines() as u64);
        json!({"state":if self.failed.load(Ordering::Acquire) {"failed"} else {"ready"},
            "dropped":dropped.to_string(), "ioErrors":self.errors.load(Ordering::Relaxed).to_string(),
            "lastError":match self.kind.load(Ordering::Acquire) {0 => None, 1 => Some("permission_denied"),
                2 => Some("storage_full"), 3 => Some("not_found"), _ => Some("io")}})
    }
}

#[derive(Clone)]
pub struct LoggingHealth {
    file: Arc<IoHealth>,
    stderr: Arc<IoHealth>,
    file_queue: ErrorCounter,
    stderr_queue: ErrorCounter,
}

impl LoggingHealth {
    pub(super) fn wait_stopped(&self, deadline: std::time::Instant) -> bool {
        for sink in [&self.file, &self.stderr] {
            let stopped = sink
                .stopped
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (stopped, _) = sink
                .stopped_signal
                .wait_timeout_while(
                    stopped,
                    deadline.saturating_duration_since(std::time::Instant::now()),
                    |stopped| !*stopped,
                )
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !*stopped {
                return false;
            }
        }
        true
    }

    pub fn status(&self) -> Value {
        json!({"file":self.file.status(&self.file_queue),"stderr":self.stderr.status(&self.stderr_queue)})
    }
}

struct CheckedWriter<W> {
    inner: W,
    health: Arc<IoHealth>,
    latch: bool,
    failure_report: Option<(NonBlocking, Encoder, LogFormat)>,
}

impl<W> Drop for CheckedWriter<W> {
    fn drop(&mut self) {
        *self
            .health
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.health.stopped_signal.notify_all();
    }
}

impl<W: Write> CheckedWriter<W> {
    fn fail(&mut self, error: &io::Error) {
        self.health.failure(error);
        if let Some((mut stderr, encoder, format)) = self.failure_report.take() {
            let bytes = encoder.encode(
                "error",
                Fields::message("file logging disabled after I/O failure"),
            );
            let bytes = match format {
                LogFormat::Json => bytes,
                LogFormat::Text => encoding::text(&bytes),
            };
            // This bypasses the failed file sink and is emitted once per run, not recursively.
            if stderr.write_all(&bytes).is_err() { /* The stderr wrapper accounts for its I/O errors. */
            }
        }
    }
}

impl<W: Write> Write for CheckedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.latch && self.health.failed.load(Ordering::Acquire) {
            self.health.dropped.fetch_add(1, Ordering::Relaxed);
            return Ok(bytes.len());
        }
        match self.inner.write_all(bytes) {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.health.dropped.fetch_add(1, Ordering::Relaxed);
                self.fail(&error);
                Err(error)
            },
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.latch && self.health.failed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.inner.flush().inspect_err(|error| self.fail(error))
    }
}

/// Invoke the library's fallible rollover explicitly: its automatic condition swallows errors.
struct SizedFile {
    inner: BasicRollingFileAppender,
    size: u64,
    max_size: u64,
    directory: PathBuf,
    max_files: usize,
}

impl Write for SizedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.size >= self.max_size {
            validate_files(&self.directory, self.max_files)?;
            self.inner.rollover()?;
            self.size = 0;
        }
        self.inner.write_all(bytes)?;
        self.size = self.size.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { self.inner.flush() }
}

fn validate_files(directory: &std::path::Path, max_files: usize) -> io::Result<()> {
    reader::regular_directory(directory)?;
    for index in 0..=max_files {
        let path = reader::file_path(directory, index);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if reader::is_regular(&metadata) => {},
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "log path is not a regular file",
                ));
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) struct LogLayer {
    file: NonBlocking,
    stderr: NonBlocking,
    encoder: Encoder,
    format: LogFormat,
    file_health: Arc<IoHealth>,
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        if let Some(span) = context.span(id) {
            let mut fields = Fields::default();
            attributes.record(&mut fields);
            span.extensions_mut().insert(fields);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        if let Some(span) = context.span(id)
            && let Some(fields) = span.extensions_mut().get_mut::<Fields>()
        {
            values.record(fields);
        }
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        if !event.metadata().target().starts_with("limen") {
            return;
        }
        let mut fields = Fields::default();
        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                if let Some(span_fields) = span.extensions().get::<Fields>() {
                    fields.values.extend(span_fields.values.clone());
                }
            }
        }
        event.record(&mut fields);
        let bytes = self
            .encoder
            .encode(event.metadata().level().as_str(), fields);
        if self.file_health.failed.load(Ordering::Acquire) {
            self.file_health.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            // Lossy NonBlocking returns success on queue overflow; its counter owns that fact.
            let _queued = self.file.clone().write_all(&bytes);
        }
        let bytes = match self.format {
            LogFormat::Json => bytes,
            LogFormat::Text => encoding::text(&bytes),
        };
        let _queued = self.stderr.clone().write_all(&bytes);
    }
}

pub(super) fn create(
    config: &LogConfig,
    encoder: Encoder,
) -> io::Result<(LogLayer, (WorkerGuard, WorkerGuard), LoggingHealth)> {
    fs::create_dir_all(&config.dir)?;
    let max_files = config.max_files;
    validate_files(&config.dir, max_files)?;
    let path = config.dir.join(FILE_NAME);
    let nonempty = match fs::metadata(&path) {
        Ok(metadata) => metadata.len() != 0,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let mut file = BasicRollingFileAppender::new_with_buffer_capacity(
        &path,
        RollingConditionBasic::new(),
        max_files,
        0,
    )?;
    if nonempty {
        file.rollover()?;
    }
    let stderr_health = Arc::new(IoHealth::default());
    let file_health = Arc::new(IoHealth::default());
    let (stderr, stderr_guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .lossy(true)
        .thread_name("limen-stderr")
        .finish(CheckedWriter {
            inner: io::stderr(),
            health: Arc::clone(&stderr_health),
            latch: false,
            failure_report: None,
        });
    let (file, file_guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .lossy(true)
        .thread_name("limen-log-file")
        .finish(CheckedWriter {
            inner: SizedFile {
                inner: file,
                size: 0,
                max_size: config.max_file_bytes,
                directory: config.dir.clone(),
                max_files,
            },
            health: Arc::clone(&file_health),
            latch: true,
            failure_report: Some((stderr.clone(), encoder.clone(), config.format)),
        });
    let health = LoggingHealth {
        file: Arc::clone(&file_health),
        stderr: stderr_health,
        file_queue: file.error_counter(),
        stderr_queue: stderr.error_counter(),
    };
    let layer = LogLayer {
        file,
        stderr,
        encoder,
        format: config.format,
        file_health,
    };
    // Drop file first so its final diagnostic can still reach the stderr worker.
    Ok((layer, (file_guard, stderr_guard), health))
}

#[cfg(test)]
impl LogLayer {
    pub(super) fn capture<W: Write + Send + 'static>(
        writer: W,
    ) -> (Self, (WorkerGuard, WorkerGuard)) {
        let (file, file_guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(1024)
            .lossy(true)
            .finish(writer);
        let (stderr, stderr_guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(1024)
            .lossy(true)
            .finish(io::sink());
        (
            Self {
                file,
                stderr,
                encoder: Encoder::new("test-run"),
                format: LogFormat::Json,
                file_health: Arc::new(IoHealth::default()),
            },
            (file_guard, stderr_guard),
        )
    }
}

#[cfg(test)]
mod tests;
