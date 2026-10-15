use std::{
    cell::Cell,
    error::Error,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use serde_json::json;
use tokio::sync::{oneshot, watch};

use super::{Budget, conversion_error, fatal, module_error, publish_ready};
use crate::{RuntimeStatus, ScriptError, ScriptErrorKind};

struct ReadyOnWake {
    status: watch::Receiver<RuntimeStatus>,
    woke: AtomicBool,
}

impl Wake for ReadyOnWake {
    fn wake(self: Arc<Self>) { self.wake_by_ref(); }

    fn wake_by_ref(self: &Arc<Self>) {
        assert!(matches!(*self.status.borrow(), RuntimeStatus::Ready));
        self.woke.store(true, Ordering::Release);
    }
}

#[test]
fn startup_notification_observes_ready_before_the_receiver_is_woken() {
    let (status, changes) = watch::channel(RuntimeStatus::Starting);
    let (ready, mut receive) = oneshot::channel();
    let observer = Arc::new(ReadyOnWake {
        status: changes,
        woke: AtomicBool::new(false),
    });
    let waker = Waker::from(Arc::clone(&observer));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        Pin::new(&mut receive).poll(&mut context),
        Poll::Pending
    ));

    assert!(publish_ready(&status, ready, json!({"loaded": true})));

    assert!(observer.woke.load(Ordering::Acquire));
    assert_eq!(
        receive.try_recv().unwrap().unwrap(),
        json!({"loaded": true})
    );
}

#[test]
fn context_reclassification_preserves_owned_causes_and_resource_failures() {
    for (convert, expected) in [
        (
            module_error as fn(ScriptError) -> ScriptError,
            ScriptErrorKind::Module,
        ),
        (conversion_error, ScriptErrorKind::Json),
    ] {
        let error = convert(ScriptError::with_source(
            ScriptErrorKind::JavaScript,
            rquickjs::Error::Exception,
        ));
        assert_eq!(error.kind(), expected);
        assert!(matches!(
            error.source().unwrap().downcast_ref::<rquickjs::Error>(),
            Some(rquickjs::Error::Exception)
        ));
        let error = convert(ScriptError::new(ScriptErrorKind::MemoryLimit));
        assert_eq!(error.kind(), ScriptErrorKind::MemoryLimit);
    }
}

#[test]
fn fatal_classification_uses_policy_kind() {
    for kind in [
        ScriptErrorKind::MemoryLimit,
        ScriptErrorKind::ExecutionBudget,
    ] {
        assert!(fatal(&ScriptError::new(kind)));
    }
    for kind in [
        ScriptErrorKind::JavaScript,
        ScriptErrorKind::Json,
        ScriptErrorKind::Cancelled,
        ScriptErrorKind::Deadline,
    ] {
        assert!(!fatal(&ScriptError::new(kind)));
    }
}

#[test]
fn budget_policy_keeps_the_failed_operation_cause() {
    let budget = Budget {
        deadline: Cell::new(None),
        interrupted: Cell::new(true),
    };
    let error = ScriptError::with_source(
        ScriptErrorKind::JavaScript,
        io::Error::from(io::ErrorKind::Interrupted),
    );
    let error = budget.check_result::<()>(Err(error)).unwrap_err();
    assert_eq!(error.kind(), ScriptErrorKind::ExecutionBudget);
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        budget.check_result(Ok(1)).unwrap_err().kind(),
        ScriptErrorKind::ExecutionBudget
    );
}
