use std::{
    fs::OpenOptions,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use rusqlite::{Connection, OpenFlags};
use tokio::sync::{OwnedSemaphorePermit, oneshot};

use crate::{
    Error, FinalRecord, Health, HealthState, PendingRecord, RequestDetail, RequestPage,
    RequestQuery, Result, StorageOptions, UsageQuery, UsageSummary, database, query,
};

/// Owns startup and bounded shutdown of the shared storage workers.
pub struct Storage {
    inner: Arc<Inner>,
}

/// Read operations and health, without write or lifecycle authority.
#[derive(Clone)]
pub struct StorageReader {
    inner: Arc<Inner>,
}

/// Confirmed record writes and health, without query or lifecycle authority.
#[derive(Clone)]
pub struct StorageWriter {
    inner: Arc<Inner>,
}

struct Inner {
    writer: mpsc::SyncSender<WriteCommand>,
    reader: Option<mpsc::SyncSender<ReadCommand>>,
    state: Arc<State>,
}

struct State {
    health: Mutex<Health>,
    stop: AtomicBool,
    writer_done: AtomicBool,
    reader_done: AtomicBool,
}

enum WriteOperation {
    Begin(PendingRecord),
    Finish(Box<FinalRecord>),
}

struct WriteCommand {
    operation: WriteOperation,
    deadline: Instant,
    reply: oneshot::Sender<Result<()>>,
}

enum ReadOperation {
    List(RequestQuery),
    Get(String),
    Usage(UsageQuery),
}

enum ReadResult {
    List(RequestPage),
    Get(Box<Option<RequestDetail>>),
    Usage(Box<UsageSummary>),
}

struct ReadCommand {
    operation: ReadOperation,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    _permit: OwnedSemaphorePermit,
    reply: oneshot::Sender<Result<ReadResult>>,
}

struct CancelOnDrop(Arc<AtomicBool>);

struct WriteConfirmation {
    state: Arc<State>,
    confirmed: bool,
}

impl Drop for WriteConfirmation {
    fn drop(&mut self) {
        if !self.confirmed {
            self.state.fail("write_unconfirmed");
        }
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}

struct WorkerCompletion {
    state: Arc<State>,
    writer: bool,
}

impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        if thread::panicking() {
            self.state.fail("worker_stopped");
        }
        let done = if self.writer {
            &self.state.writer_done
        } else {
            &self.state.reader_done
        };
        done.store(true, Ordering::Release);
    }
}

impl State {
    fn fail(&self, reason: &'static str) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if health.state != HealthState::Failed {
            *health = Health {
                state: HealthState::Failed,
                reason: Some(reason),
            };
        }
    }

    fn health(&self) -> Health {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Storage {
    /// Opens the instance within a caller-owned startup budget. Call from a blocking thread.
    ///
    /// # Errors
    /// Rejects invalid options, held locks, mismatched schemas, I/O failures and expired startup.
    pub fn open(options: StorageOptions, deadline: Instant) -> Result<Self> {
        validate_options(&options)?;
        database::check_deadline(deadline)?;
        std::fs::create_dir_all(&options.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(options.dir.join("limen.lock"))?;
        match lock.try_lock() {
            Ok(()) => {},
            Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Locked),
            Err(std::fs::TryLockError::Error(error)) => return Err(Error::Io(error)),
        }
        let lock = Arc::new(lock);
        let state = Arc::new(State {
            health: Mutex::new(Health {
                state: HealthState::Ready,
                reason: None,
            }),
            stop: AtomicBool::new(false),
            writer_done: AtomicBool::new(false),
            reader_done: AtomicBool::new(options.management_concurrency.is_none()),
        });
        let options = Arc::new(options);
        let (writer, writer_rx) = mpsc::sync_channel(options.generation_concurrency + 1);
        let (ready, ready_rx) = mpsc::sync_channel(1);
        let worker_state = Arc::clone(&state);
        let worker_options = Arc::clone(&options);
        let worker_lock = Arc::clone(&lock);
        thread::Builder::new()
            .name("limen-storage-write".into())
            .spawn(move || {
                let _completion = WorkerCompletion {
                    state: Arc::clone(&worker_state),
                    writer: true,
                };
                let _lock = worker_lock;
                writer_start(&worker_options, &worker_state, &writer_rx, &ready, deadline);
            })?;
        if let Err(error) = wait_ready(&ready_rx, deadline) {
            state.stop.store(true, Ordering::Release);
            return Err(error);
        }
        let reader = if let Some(capacity) = options.management_concurrency {
            let (sender, receiver) = mpsc::sync_channel(capacity);
            let (ready, ready_rx) = mpsc::sync_channel(1);
            let worker_state = Arc::clone(&state);
            let worker_options = Arc::clone(&options);
            let worker_lock = Arc::clone(&lock);
            let spawned = thread::Builder::new()
                .name("limen-storage-read".into())
                .spawn(move || {
                    let _completion = WorkerCompletion {
                        state: Arc::clone(&worker_state),
                        writer: false,
                    };
                    let _lock = worker_lock;
                    reader_start(&worker_options, &worker_state, &receiver, &ready, deadline);
                });
            if let Err(error) = spawned {
                state.stop.store(true, Ordering::Release);
                return Err(Error::Io(error));
            }
            if let Err(error) = wait_ready(&ready_rx, deadline) {
                state.stop.store(true, Ordering::Release);
                return Err(error);
            }
            Some(sender)
        } else {
            None
        };
        Ok(Self {
            inner: Arc::new(Inner {
                writer,
                reader,
                state,
            }),
        })
    }

    pub fn health(&self) -> Health { self.inner.state.health() }

    /// Shares only read operations; a disabled reader rejects queries as unavailable.
    pub fn reader(&self) -> StorageReader {
        StorageReader {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Shares only record write operations.
    pub fn writer(&self) -> StorageWriter {
        StorageWriter {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Stops admission, drains queued commands and waits only until the supplied deadline.
    ///
    /// # Errors
    /// Returns timeout while a worker still owns its connection and the shared instance lock.
    pub async fn shutdown(&self, deadline: Instant) -> Result<()> {
        self.inner.state.stop.store(true, Ordering::Release);
        {
            let mut health = self
                .inner
                .state
                .health
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            health.state = HealthState::Stopping;
        }
        loop {
            if self.inner.state.writer_done.load(Ordering::Acquire)
                && self.inner.state.reader_done.load(Ordering::Acquire)
            {
                return Ok(());
            }
            database::check_deadline(deadline)?;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl StorageWriter {
    pub fn health(&self) -> Health { self.inner.state.health() }

    /// Confirms a pending row before generation starts.
    ///
    /// # Errors
    /// Returns busy without retry, or a latched failure when the write cannot be confirmed.
    pub async fn begin(&self, record: PendingRecord, deadline: Instant) -> Result<()> {
        if self.health().state != HealthState::Ready {
            return Err(Error::Unavailable);
        }
        self.write(WriteOperation::Begin(record), deadline).await
    }

    /// Attempts the single terminal transition, including after storage health has failed.
    ///
    /// # Errors
    /// Returns queue, deadline, record-transition or database errors without changing network results.
    pub async fn finish(&self, record: FinalRecord, deadline: Instant) -> Result<()> {
        self.write(WriteOperation::Finish(Box::new(record)), deadline)
            .await
    }

    async fn write(&self, operation: WriteOperation, deadline: Instant) -> Result<()> {
        if self.inner.state.stop.load(Ordering::Acquire) {
            return Err(Error::Unavailable);
        }
        let (reply, receiver) = oneshot::channel();
        if let Err(error) = send(
            &self.inner.writer,
            WriteCommand {
                operation,
                deadline,
                reply,
            },
        ) {
            if !matches!(error, Error::Busy) {
                self.inner.state.fail(error.code());
            }
            return Err(error);
        }
        let mut confirmation = WriteConfirmation {
            state: Arc::clone(&self.inner.state),
            confirmed: false,
        };
        let result = match tokio::time::timeout_at(deadline.into(), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Worker),
            Err(_) => Err(Error::Timeout),
        };
        if let Err(error) = &result {
            self.inner.state.fail(error.code());
        }
        confirmation.confirmed = true;
        result
    }
}

impl StorageReader {
    pub fn health(&self) -> Health { self.inner.state.health() }

    /// Reads a bounded page, retaining the permit until the database work really completes.
    ///
    /// # Errors
    /// Returns invalid filters, busy, timeout or database errors.
    pub async fn list(
        &self,
        query: RequestQuery,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Result<RequestPage> {
        match self
            .read(ReadOperation::List(query), deadline, permit)
            .await?
        {
            ReadResult::List(value) => Ok(value),
            _ => Err(Error::Worker),
        }
    }

    /// Looks up one persisted request without applying a time window.
    ///
    /// # Errors
    /// Returns invalid identifiers, busy, timeout or database errors.
    pub async fn get(
        &self,
        request_id: String,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Result<Option<RequestDetail>> {
        match self
            .read(ReadOperation::Get(request_id), deadline, permit)
            .await?
        {
            ReadResult::Get(value) => Ok(*value),
            _ => Err(Error::Worker),
        }
    }

    /// Aggregates known counts in bounded memory within one read transaction.
    ///
    /// # Errors
    /// Returns invalid or overly broad queries, busy, timeout or database errors.
    pub async fn usage(
        &self,
        query: UsageQuery,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Result<UsageSummary> {
        match self
            .read(ReadOperation::Usage(query), deadline, permit)
            .await?
        {
            ReadResult::Usage(value) => Ok(*value),
            _ => Err(Error::Worker),
        }
    }

    async fn read(
        &self,
        operation: ReadOperation,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Result<ReadResult> {
        if self.inner.state.stop.load(Ordering::Acquire) {
            return Err(Error::Unavailable);
        }
        let sender = self.inner.reader.as_ref().ok_or(Error::Unavailable)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(Arc::clone(&cancelled));
        let (reply, receiver) = oneshot::channel();
        send(
            sender,
            ReadCommand {
                operation,
                deadline,
                cancelled,
                _permit: permit,
                reply,
            },
        )?;
        match tokio::time::timeout_at(deadline.into(), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Worker),
            Err(_) => Err(Error::Timeout),
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) { self.state.stop.store(true, Ordering::Release); }
}

fn send<T>(sender: &mpsc::SyncSender<T>, value: T) -> Result<()> {
    sender.try_send(value).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => Error::Busy,
        mpsc::TrySendError::Disconnected(_) => Error::Worker,
    })
}

fn wait_ready(receiver: &mpsc::Receiver<Result<()>>, deadline: Instant) -> Result<()> {
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => Error::Timeout,
            mpsc::RecvTimeoutError::Disconnected => Error::Worker,
        })?
}

fn validate_options(options: &StorageOptions) -> Result<()> {
    if options.dir.as_os_str().is_empty()
        || options.run_id.is_empty()
        || options.run_id.len() > 64
        || !(1..=3650).contains(&options.retention_days)
        || !(1000..=10_000_000).contains(&options.max_records)
        || !(64 * 1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&options.max_db_bytes)
        || !options.max_db_bytes.is_multiple_of(4096)
        || options.generation_concurrency == 0
        || options.generation_concurrency as u64 >= options.max_records
        || options
            .management_concurrency
            .is_some_and(|value| !(1..=16).contains(&value))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn writer_start(
    options: &StorageOptions,
    state: &State,
    receiver: &mpsc::Receiver<WriteCommand>,
    ready: &mpsc::SyncSender<Result<()>>,
    deadline: Instant,
) {
    let initialize = || -> Result<Connection> {
        let path = options.dir.join("limen.sqlite3");
        let fresh = match std::fs::metadata(&path) {
            Ok(metadata) => {
                if metadata.len() > options.max_db_bytes {
                    return Err(Error::Capacity);
                }
                false
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error.into()),
        };
        let mut connection = Connection::open(path)?;
        connection.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
        database::initialize(&mut connection, fresh, options, deadline)?;
        connection.progress_handler(0, None::<fn() -> bool>)?;
        Ok(connection)
    };
    let mut connection = match initialize() {
        Ok(connection) => connection,
        Err(error) => {
            state.fail(error.code());
            let _receiver_closed = ready.send(Err(error));
            return;
        },
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let mut next_maintenance = Instant::now() + Duration::from_secs(60);
    loop {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(command) => {
                let result = database::check_deadline(command.deadline)
                    .and_then(|()| match command.operation {
                        WriteOperation::Begin(record)
                            if state.health().state == HealthState::Ready =>
                        {
                            database::begin(&mut connection, &record, options)
                        },
                        WriteOperation::Begin(_) => Err(Error::Unavailable),
                        WriteOperation::Finish(record) => database::finish(&connection, &record),
                    })
                    .and_then(|()| check_wal(options));
                if let Err(error) = &result {
                    state.fail(error.code());
                }
                let _receiver_closed = command.reply.send(result);
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) if state.stop.load(Ordering::Acquire) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {},
        }
        if !state.stop.load(Ordering::Acquire)
            && state.health().state == HealthState::Ready
            && Instant::now() >= next_maintenance
        {
            match database::now_ms()
                .and_then(|now| database::prune_age(&connection, options, now))
                .and_then(|count| check_wal(options).map(|()| count))
            {
                Ok(500) => {},
                Ok(_) => next_maintenance = Instant::now() + Duration::from_secs(60),
                Err(error) => state.fail(error.code()),
            }
        }
    }
}

fn check_wal(options: &StorageOptions) -> Result<()> {
    match std::fs::metadata(options.dir.join("limen.sqlite3-wal")) {
        Ok(metadata) if metadata.len() > 64 * 1024 * 1024 => Err(Error::Capacity),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reader_start(
    options: &StorageOptions,
    state: &State,
    receiver: &mpsc::Receiver<ReadCommand>,
    ready: &mpsc::SyncSender<Result<()>>,
    deadline: Instant,
) {
    let initialize = || -> Result<Connection> {
        database::check_deadline(deadline)?;
        let connection = Connection::open_with_flags(
            options.dir.join("limen.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        database::configure(&connection)?;
        Ok(connection)
    };
    let mut connection = match initialize() {
        Ok(connection) => connection,
        Err(error) => {
            state.fail(error.code());
            let _receiver_closed = ready.send(Err(error));
            return;
        },
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) if state.stop.load(Ordering::Acquire) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let cancelled = Arc::clone(&command.cancelled);
        let deadline = command.deadline;
        let mut result = (|| -> Result<ReadResult> {
            connection.progress_handler(
                1000,
                Some(move || Instant::now() >= deadline || cancelled.load(Ordering::Acquire)),
            )?;
            database::check_deadline(deadline)?;
            if command.cancelled.load(Ordering::Acquire) {
                return Err(Error::Timeout);
            }
            match command.operation {
                ReadOperation::List(query) => {
                    query::list(&mut connection, &query, deadline).map(ReadResult::List)
                },
                ReadOperation::Get(id) => {
                    query::get(&connection, &id).map(|record| ReadResult::Get(Box::new(record)))
                },
                ReadOperation::Usage(query) => {
                    query::usage(&mut connection, &query, options, deadline)
                        .map(|value| ReadResult::Usage(Box::new(value)))
                },
            }
        })();
        if let Err(error) = connection.progress_handler(0, None::<fn() -> bool>) {
            state.fail("database_error");
            if result.is_ok() {
                result = Err(error.into());
            }
        }
        if let Err(Error::Database(rusqlite::Error::SqliteFailure(error, _))) = &result {
            if error.code == rusqlite::ErrorCode::OperationInterrupted {
                result = Err(Error::Timeout);
            } else if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseCorrupt
                    | rusqlite::ErrorCode::SystemIoFailure
                    | rusqlite::ErrorCode::NotADatabase
            ) {
                state.fail("database_error");
            }
        }
        let _receiver_closed = command.reply.send(result);
    }
}

#[cfg(test)]
mod tests;
