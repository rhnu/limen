use std::{
    io,
    sync::{Arc, Mutex},
};

use limen_config::{BusinessConfig, ResourceLimits};
use limen_script::{ScriptError, ScriptErrorKind};
use serde_json::json;

use super::{Error, expected_rejection, record_failure};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

#[test]
fn rejection_and_operational_failure_have_distinct_severity() {
    let source = BusinessConfig::from_value(json!({}), &ResourceLimits::default())
        .err()
        .unwrap();
    assert!(expected_rejection(&Error::Config {
        file: "config.js",
        source: Box::new(source)
    }));
    assert!(expected_rejection(&Error::Script(ScriptError::new(
        ScriptErrorKind::Contract
    ))));
    assert!(!expected_rejection(&Error::Script(ScriptError::new(
        ScriptErrorKind::MemoryLimit
    ))));
    assert!(!expected_rejection(&Error::Executor(
        io::ErrorKind::Other.into()
    )));
    assert!(!expected_rejection(&Error::Cleanup {
        primary: Box::new(Error::StartupBudget),
        cleanup: Box::new(Error::ShutdownState { state: "ready" }),
    }));
}

#[test]
fn json_log_retains_metadata_and_safe_diagnostics() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_writer(capture.clone())
        .finish();
    let error = Error::Executor(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "secret-log-payload",
    ));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("configuration", file = "server.yaml");
        let _guard = span.enter();
        record_failure(&error, "check", "executor");
    });
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    let event: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(event["level"], "ERROR");
    assert_eq!(event["fields"]["command"], "check");
    assert_eq!(event["fields"]["phase"], "executor");
    assert_eq!(event["span"]["file"], "server.yaml");
    assert!(
        event["fields"]["error"]
            .as_str()
            .unwrap()
            .contains("PermissionDenied")
    );
    assert!(!output.contains("secret-log-payload"));
}
