//! Real SQLite files and workers only run in externally supervised fixture processes.

use std::{
    fs::OpenOptions,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail, ensure};
use limen_storage::{
    Error, FinalRecord, HealthState, Outcome, PendingRecord, RecordState, Storage, StorageOptions,
};
use tokio::sync::Semaphore;

pub const CASES: &[&str] = &[
    "recovery-lock",
    "retention-capacity",
    "failure-deadline",
    "read-cancellation",
];

pub async fn run(case: &str, directory: &Path) -> Result<()> {
    match case {
        "recovery-lock" => recovery_lock(directory).await,
        "retention-capacity" => retention_capacity(directory).await,
        "failure-deadline" => failure_deadline(directory).await,
        "read-cancellation" => read_cancellation(directory).await,
        _ => bail!("unknown storage experiment: {case}"),
    }
}

fn options(directory: &Path) -> StorageOptions {
    StorageOptions {
        dir: directory.into(),
        run_id: "isolated-run".into(),
        retention_days: 90,
        max_records: 1000,
        max_db_bytes: 64 * 1024 * 1024,
        generation_concurrency: 2,
        management_concurrency: Some(1),
    }
}

fn deadline() -> Instant { Instant::now() + Duration::from_secs(3) }

fn now() -> Result<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

fn pending(id: &str, started_at_ms: i64) -> PendingRecord {
    PendingRecord {
        request_id: id.into(),
        run_id: "isolated-run".into(),
        started_at_ms,
        client_stream: true,
        input_bytes: 7,
    }
}

fn terminal(id: &str) -> Result<FinalRecord> {
    Ok(FinalRecord {
        request_id: id.into(),
        finished_at_ms: now()?,
        outcome: Some(Outcome::Success),
        input_tokens: Some(3),
        ..FinalRecord::default()
    })
}

async fn open(directory: &Path) -> Result<Storage> {
    let options = options(directory);
    Ok(tokio::task::spawn_blocking(move || Storage::open(options, deadline())).await??)
}

async fn get(storage: &Storage, id: &str) -> Result<Option<limen_storage::RequestRecord>> {
    Ok(storage
        .get(
            id.into(),
            deadline(),
            Arc::new(Semaphore::new(1)).acquire_owned().await?,
        )
        .await?)
}

async fn recovery_lock(directory: &Path) -> Result<()> {
    let storage = open(directory).await?;
    let second = options(directory);
    let blocked = tokio::task::spawn_blocking(move || Storage::open(second, deadline())).await?;
    ensure!(
        matches!(blocked, Err(Error::Locked)),
        "second instance acquired the lock"
    );
    storage
        .begin(pending("pending", now()?), deadline())
        .await?;
    storage.begin(pending("done", now()?), deadline()).await?;
    storage.finish(terminal("done")?, deadline()).await?;
    storage.shutdown(deadline()).await?;
    let reopened = open(directory).await?;
    let pending = get(&reopened, "pending")
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing recovered request"))?;
    ensure!(
        pending.state == RecordState::Interrupted
            && pending.outcome.is_none()
            && pending.finished_at_ms.is_none()
            && pending.input_tokens.is_none(),
        "recovery fabricated a terminal fact"
    );
    let done = get(&reopened, "done")
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing finalized request"))?;
    ensure!(
        done.state == RecordState::Finalized && done.input_tokens == Some(3),
        "finalized request changed after reopen"
    );
    reopened.shutdown(deadline()).await?;
    Ok(())
}

async fn retention_capacity(directory: &Path) -> Result<()> {
    let storage = open(directory).await?;
    storage
        .begin(pending("old", now()? - 91 * 86_400_000), deadline())
        .await?;
    storage.finish(terminal("old")?, deadline()).await?;
    storage.begin(pending("recent", now()?), deadline()).await?;
    storage.shutdown(deadline()).await?;
    let reopened = open(directory).await?;
    ensure!(
        get(&reopened, "old").await?.is_none(),
        "startup age cleanup retained expired row"
    );
    ensure!(
        get(&reopened, "recent").await?.is_some(),
        "startup cleanup lost recent request"
    );
    reopened.shutdown(deadline()).await?;
    let database = directory.join("limen.sqlite3");
    OpenOptions::new()
        .write(true)
        .open(&database)?
        .set_len(65 * 1024 * 1024)?;
    let options = options(directory);
    let oversized = tokio::task::spawn_blocking(move || Storage::open(options, deadline())).await?;
    ensure!(
        matches!(oversized, Err(Error::Capacity)),
        "oversized existing database was accepted"
    );
    ensure!(
        std::fs::metadata(database)?.len() == 65 * 1024 * 1024,
        "startup failure recreated database"
    );
    Ok(())
}

async fn failure_deadline(directory: &Path) -> Result<()> {
    let storage = open(directory).await?;
    storage.begin(pending("one", now()?), deadline()).await?;
    ensure!(
        storage
            .begin(pending("one", now()?), deadline())
            .await
            .is_err(),
        "duplicate begin overwrote a row"
    );
    ensure!(
        storage.health().state == HealthState::Failed,
        "failed write did not latch health"
    );
    ensure!(
        matches!(
            storage.begin(pending("denied", now()?), deadline()).await,
            Err(Error::Unavailable)
        ),
        "failed storage admitted a new generation"
    );
    storage.finish(terminal("one")?, deadline()).await?;
    ensure!(
        storage.health().state == HealthState::Failed,
        "successful terminal write reset failed health"
    );
    storage.shutdown(deadline()).await?;
    let reopened = open(directory).await?;
    ensure!(
        reopened.health().state == HealthState::Ready,
        "explicit restart did not reset health"
    );
    ensure!(
        matches!(
            reopened
                .begin(pending("expired", now()?), Instant::now())
                .await,
            Err(Error::Timeout)
        ),
        "expired write was acknowledged"
    );
    ensure!(
        reopened.health().state == HealthState::Failed,
        "write timeout did not latch health"
    );
    let started = Instant::now();
    let result = reopened.shutdown(Instant::now()).await;
    ensure!(
        result.is_ok() || matches!(result, Err(Error::Timeout)),
        "unexpected shutdown result"
    );
    ensure!(
        started.elapsed() < Duration::from_millis(100),
        "shutdown ignored its budget"
    );
    reopened.shutdown(deadline()).await?;
    Ok(())
}

async fn read_cancellation(directory: &Path) -> Result<()> {
    let storage = open(directory).await?;
    storage.begin(pending("one", now()?), deadline()).await?;
    let semaphore = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&semaphore).acquire_owned().await?;
    let result = storage.get("one".into(), Instant::now(), permit).await;
    ensure!(
        matches!(result, Err(Error::Timeout)),
        "expired read succeeded"
    );
    let permit = tokio::time::timeout(
        Duration::from_secs(1),
        Arc::clone(&semaphore).acquire_owned(),
    )
    .await??;
    ensure!(
        storage.health().state == HealthState::Ready,
        "read timeout poisoned write health"
    );
    let record = storage.get("one".into(), deadline(), permit).await?;
    ensure!(record.is_some(), "query after read cancellation failed");
    storage.shutdown(deadline()).await?;
    Ok(())
}
