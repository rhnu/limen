use super::*;

fn assert_redacted(error: &ConfigError, secrets: &[&str]) {
    let rendered = format!("{error} {error:?} {}", error.diagnostic());
    for secret in secrets {
        assert!(
            !rendered.contains(secret),
            "diagnostic exposed sensitive input"
        );
    }
}

#[test]
fn io_source_remains_typed_while_custom_text_is_hidden() {
    let error = ConfigError::new(
        ConfigErrorKind::Io,
        "server",
        "cannot read configuration file",
    )
    .with_source(
        "read configuration bytes",
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "secret-path-and-token",
        ),
    );
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "secret-path-and-token");
    assert_eq!(error.field, "server");
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("read configuration bytes"));
    assert!(diagnostic.contains("PermissionDenied"));
    assert_redacted(&error, &["secret-path-and-token"]);
}

#[test]
fn opaque_nested_cause_is_retained_without_rendering_its_text() {
    #[derive(Debug, thiserror::Error)]
    #[error("private implementation secret")]
    struct PrivateCause {
        #[source]
        source: serde_json::Error,
    }
    let source = PrivateCause {
        source: serde_json::from_str::<bool>("\"secret-value\"").unwrap_err(),
    };
    let error = ConfigError::new(
        ConfigErrorKind::Io,
        "server",
        "cannot read configuration file",
    )
    .with_source("read configuration bytes", source);
    let nested = error.source().unwrap().source().unwrap();
    assert!(nested.is::<serde_json::Error>());
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("opaque dependency error"));
    assert!(diagnostic.contains("JSON (Data, line 1"));
    assert_redacted(&error, &["private implementation secret", "secret-value"]);
}

#[test]
fn validation_error_has_context_without_a_fabricated_source() {
    let error = crate::ServerConfig::parse(
        "client_auth: {none: true}\nlimits:\n  hook_ms: 0\n",
        crate::ServerFormat::Yaml,
    )
    .err()
    .unwrap();
    assert!(error.source().is_none());
    assert_eq!(error.line, Some(3));
    assert_eq!(error.column, Some(12));
    assert_eq!(error.diagnostic().to_string(), error.to_string());
}
