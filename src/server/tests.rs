use super::*;

#[test]
fn runtime_requires_ready_state() {
    assert!(runtime_ready(&RuntimeStatus::Ready).is_ok());
    for status in [RuntimeStatus::Starting, RuntimeStatus::Stopped] {
        assert!(matches!(runtime_ready(&status), Err(Error::RuntimeStopped)));
    }
}

#[test]
fn cleanup_keeps_both_failures() {
    let primary = Error::Accept(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
    let cleanup = Error::RuntimeStopped;
    let error = with_cleanup(Err(primary), Err(cleanup)).unwrap_err();
    let Error::Cleanup { primary, cleanup } = error else {
        panic!("cleanup wrapper expected")
    };
    assert!(matches!(*primary, Error::Accept(_)));
    assert!(matches!(*cleanup, Error::RuntimeStopped));
}

#[test]
fn driver_drop_cancels_inflight_request() {
    let cancellation = Cancellation::new();
    let guard = CancelOnDrop(cancellation.clone());
    assert!(!cancellation.is_cancelled());
    drop(guard);
    assert!(cancellation.is_cancelled());
}

#[test]
fn transport_diagnostics_hide_raw_source() {
    let error = Error::Accept(std::io::Error::other("secret token query and body"));
    assert!(!error.diagnostic().to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    assert_eq!(error.phase(), "http_transport");
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
}
