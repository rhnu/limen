use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::{CheckedWriter, IoHealth};

struct FailWriter(Arc<AtomicUsize>);

impl Write for FailWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[test]
fn file_failure_latches_without_reopening_or_retrying() {
    let calls = Arc::new(AtomicUsize::new(0));
    let health = Arc::new(IoHealth::default());
    let mut writer = CheckedWriter {
        inner: FailWriter(Arc::clone(&calls)),
        health: Arc::clone(&health),
        latch: true,
        failure_report: None,
    };
    assert!(writer.write_all(b"first\n").is_err());
    writer.write_all(b"second\n").unwrap();
    writer.flush().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(health.errors.load(Ordering::Relaxed), 1);
    assert_eq!(health.dropped.load(Ordering::Relaxed), 2);
    assert!(health.failed.load(Ordering::Acquire));
}

#[test]
fn stderr_counts_failures_independently() {
    let health = Arc::new(IoHealth::default());
    let mut writer = CheckedWriter {
        inner: FailWriter(Arc::new(AtomicUsize::new(0))),
        health: Arc::clone(&health),
        latch: false,
        failure_report: None,
    };
    assert!(writer.write_all(b"first\n").is_err());
    assert!(writer.write_all(b"second\n").is_err());
    assert_eq!(health.errors.load(Ordering::Relaxed), 2);
}
