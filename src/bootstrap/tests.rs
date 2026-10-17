use std::{error::Error as _, io};

use limen_config::{BusinessConfig, ResourceLimits};
use limen_script::{RuntimeStatus, ScriptError, ScriptErrorKind};
use serde_json::json;

use super::{Error, shutdown_status, with_cleanup};

#[test]
fn native_configuration_failures_keep_typed_cause_and_safe_diagnostic() {
    let error = Error::NativeContent(limen_chat::NativeContentError::Limit);
    assert_eq!(error.phase(), "business_config");
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<limen_chat::NativeContentError>(),
        Some(&limen_chat::NativeContentError::Limit)
    );
    assert!(error.diagnostic().to_string().contains("resource limit"));
}

#[test]
fn server_errors_keep_their_phase_for_both_formats() {
    for format in [
        limen_config::ServerFormat::Toml,
        limen_config::ServerFormat::Yaml,
    ] {
        let source = limen_config::ServerConfig::parse("", format).err().unwrap();
        let error = Error::Config {
            file: "server configuration",
            source: Box::new(source),
        };
        assert_eq!(error.phase(), "server_config");
    }
}

#[test]
fn config_context_preserves_the_original_failure() {
    let source = BusinessConfig::from_value(json!({}), &ResourceLimits::default())
        .err()
        .unwrap();
    let error = Error::Config {
        file: "config.js",
        source: Box::new(source),
    };
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<Box<limen_config::ConfigError>>()
        .unwrap();
    assert_eq!(error.phase(), "business_config");
    assert_eq!(source.kind, limen_config::ConfigErrorKind::Schema);
    assert!(source.source().is_some());
}

#[test]
fn cleanup_failure_keeps_the_primary_and_secondary_results() {
    let primary = Error::Script(ScriptError::new(ScriptErrorKind::Json));
    let cleanup = Error::Script(ScriptError::new(ScriptErrorKind::MemoryLimit));
    let error = with_cleanup(primary, Err(cleanup));
    let primary = error
        .source()
        .unwrap()
        .downcast_ref::<Box<Error>>()
        .unwrap();
    assert!(
        matches!(primary.as_ref(), Error::Script(source) if source.kind() == ScriptErrorKind::Json)
    );
    match &error {
        Error::Cleanup { cleanup, .. } => {
            assert!(
                matches!(cleanup.as_ref(), Error::Script(source) if source.kind() == ScriptErrorKind::MemoryLimit)
            );
        },
        _ => panic!("both failures must remain available"),
    }
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("Json"));
    assert!(diagnostic.contains("MemoryLimit"));
}

#[test]
fn successful_cleanup_does_not_change_the_primary_failure() {
    let error = with_cleanup(Error::StartupBudget, Ok(()));
    assert!(matches!(error, Error::StartupBudget));
}

#[test]
fn shutdown_distinguishes_retirement_from_nonterminal_states() {
    assert!(shutdown_status(RuntimeStatus::Stopped).is_ok());
    let error = shutdown_status(RuntimeStatus::Retired(ScriptError::new(
        ScriptErrorKind::ExecutionBudget,
    )))
    .unwrap_err();
    assert!(
        matches!(error, Error::Script(source) if source.kind() == ScriptErrorKind::ExecutionBudget)
    );
    for status in [RuntimeStatus::Starting, RuntimeStatus::Ready] {
        assert!(matches!(
            shutdown_status(status),
            Err(Error::ShutdownState { .. })
        ));
    }
}

#[test]
fn executor_source_remains_typed_while_reports_hide_its_payload() {
    let error = Error::Executor(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "secret-executor-payload",
    ));
    let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "secret-executor-payload");
    assert!(error.diagnostic().to_string().contains("PermissionDenied"));
    assert!(
        !error
            .diagnostic()
            .to_string()
            .contains("secret-executor-payload")
    );
    assert!(!format!("{error:?}").contains("secret-executor-payload"));
}
