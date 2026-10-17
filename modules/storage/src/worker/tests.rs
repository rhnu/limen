use std::{
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

use tokio::sync::Semaphore;

use super::{Inner, ReadCommand, State, Storage, WriteCommand};
use crate::{Error, FinalRecord, Health, HealthState, PendingRecord};

fn fake_storage() -> (
    Storage,
    mpsc::Receiver<WriteCommand>,
    mpsc::Receiver<ReadCommand>,
) {
    let (writer, write_commands) = mpsc::sync_channel(1);
    let (reader, reads) = mpsc::sync_channel(1);
    let state = Arc::new(State {
        health: Mutex::new(Health {
            state: HealthState::Ready,
            reason: None,
        }),
        stop: AtomicBool::new(false),
        writer_done: AtomicBool::new(false),
        reader_done: AtomicBool::new(false),
    });
    (
        Storage {
            inner: Arc::new(Inner {
                writer,
                reader: Some(reader),
                state,
            }),
        },
        write_commands,
        reads,
    )
}

fn pending() -> PendingRecord {
    PendingRecord {
        request_id: "one".into(),
        run_id: "run".into(),
        started_at_ms: 1000,
        client_stream: false,
        input_bytes: 2,
    }
}

#[tokio::test]
async fn queue_full_is_temporary_but_dropped_confirmation_latches_failure() {
    let (storage, write_commands, _reads) = fake_storage();
    let writer = storage.writer();
    let reader = storage.reader();
    let first = writer.clone();
    let task = tokio::spawn(async move {
        first
            .begin(pending(), Instant::now() + Duration::from_secs(10))
            .await
    });
    tokio::task::yield_now().await;
    assert!(matches!(
        writer
            .begin(pending(), Instant::now() + Duration::from_secs(10))
            .await,
        Err(Error::Busy)
    ));
    assert_eq!(storage.health().state, HealthState::Ready);
    let command = write_commands.try_recv().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(storage.health().state, HealthState::Failed);
    assert_eq!(storage.health().reason, Some("write_unconfirmed"));
    assert_eq!(writer.health().state, HealthState::Failed);
    assert_eq!(reader.health().reason, Some("write_unconfirmed"));
    assert!(command.reply.send(Ok(())).is_err());
    assert_eq!(storage.health().state, HealthState::Failed);
}

#[tokio::test]
async fn cancelled_read_keeps_its_permit_until_worker_command_is_dropped() {
    let (storage, _writes, reads) = fake_storage();
    let permits = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&permits).acquire_owned().await.unwrap();
    let reader = storage.reader();
    let task = tokio::spawn(async move {
        reader
            .get(
                "one".into(),
                Instant::now() + Duration::from_secs(10),
                permit,
            )
            .await
    });
    tokio::task::yield_now().await;
    let command = reads.try_recv().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(permits.available_permits(), 0);
    assert!(command.cancelled.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(storage.health().state, HealthState::Ready);
    drop(command);
    assert_eq!(permits.available_permits(), 1);
}

#[tokio::test]
async fn owner_shutdown_propagates_to_existing_handles_and_retains_its_deadline() {
    let (storage, _writes, _reads) = fake_storage();
    let reader = storage.reader();
    let writer = storage.writer();
    let reader_clone = reader.clone();
    let writer_clone = writer.clone();
    assert!(matches!(
        storage.shutdown(Instant::now()).await,
        Err(Error::Timeout)
    ));
    assert_eq!(storage.health().state, HealthState::Stopping);
    assert_eq!(reader.health().state, HealthState::Stopping);
    assert_eq!(writer.health().state, HealthState::Stopping);
    assert_eq!(reader_clone.health().state, HealthState::Stopping);
    assert_eq!(writer_clone.health().state, HealthState::Stopping);
    assert!(matches!(
        writer.begin(pending(), Instant::now()).await,
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        writer.finish(FinalRecord::default(), Instant::now()).await,
        Err(Error::Unavailable)
    ));
    let permits = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&permits).acquire_owned().await.unwrap();
    assert!(matches!(
        reader.get("one".into(), Instant::now(), permit).await,
        Err(Error::Unavailable)
    ));
    assert_eq!(permits.available_permits(), 1);
    storage
        .inner
        .state
        .writer_done
        .store(true, std::sync::atomic::Ordering::Release);
    storage
        .inner
        .state
        .reader_done
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(storage.shutdown(Instant::now()).await.is_ok());
}

#[tokio::test]
async fn confirmed_writes_share_health_and_terminal_writes_remain_allowed_after_failure() {
    let (storage, commands, _reads) = fake_storage();
    let reader = storage.reader();
    let writer = storage.writer();
    let first = writer.clone();
    let task = tokio::spawn(async move {
        first
            .begin(pending(), Instant::now() + Duration::from_secs(10))
            .await
    });
    tokio::task::yield_now().await;
    commands.try_recv().unwrap().reply.send(Ok(())).unwrap();
    assert!(task.await.unwrap().is_ok());
    assert_eq!(reader.health().state, HealthState::Ready);
    assert_eq!(writer.health().state, HealthState::Ready);
    storage.inner.state.fail("database_error");
    let terminal = writer.clone();
    let task = tokio::spawn(async move {
        terminal
            .finish(
                FinalRecord::default(),
                Instant::now() + Duration::from_secs(10),
            )
            .await
    });
    tokio::task::yield_now().await;
    commands.try_recv().unwrap().reply.send(Ok(())).unwrap();
    assert!(task.await.unwrap().is_ok());
    assert_eq!(storage.health().state, HealthState::Failed);
    assert_eq!(reader.health().reason, Some("database_error"));
    assert_eq!(writer.health().reason, Some("database_error"));
    assert!(matches!(
        writer.begin(pending(), Instant::now()).await,
        Err(Error::Unavailable)
    ));
}
