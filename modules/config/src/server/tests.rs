use super::*;

#[test]
fn explicit_none_and_bearer_are_distinct() {
    let config = ServerConfig::parse("client_auth: {none: true}").unwrap();
    assert!(config.listen.ip().is_loopback());
    assert!(matches!(config.client_auth, ClientAuth::None));
    let config = ServerConfig::parse("client_auth: {bearer: abc.def-123==}").unwrap();
    assert!(matches!(config.client_auth, ClientAuth::Bearer(token) if token == "abc.def-123=="));
}

#[test]
fn authentication_must_be_explicit_and_unambiguous() {
    for source in [
        "{}",
        "client_auth: {}",
        "client_auth: {none: false}",
        "client_auth: {none: true, bearer: token}",
        "client_auth: {bearer: ''}",
        "client_auth: {bearer: 'contains space'}",
        "client_auth: null",
        "client_auth: {none: true, bearer: null}",
        "client_auth: {none: null, bearer: token}",
    ] {
        assert!(
            ServerConfig::parse(source).is_err(),
            "accepted invalid authentication"
        );
    }
}

#[test]
fn rejects_unknown_duplicate_and_wrong_type_without_secrets() {
    for source in [
        "client_auth: {none: true}\nsecret-value: token-value",
        "client_auth: {bearer: token-value, bearer: secret-value}",
        "client_auth: {bearer: [token-value]}",
        "client_auth: {bearer: 123}",
        "client_auth: {none: 1}",
        "client_auth: {none: 'true'}",
        "client_auth: {none: true}\nlimits: {hook_ms: '1'}",
        "client_auth: {none: true}\nlimits: {startup_ms: token-value}",
        "client_auth: {none: true}\nlog: {level: token-value}",
    ] {
        let error = ServerConfig::parse(source).err().unwrap();
        assert!(error.line.is_some());
        assert!(error.column.is_some());
        let rendered = format!("{error} {error:?} {}", error.diagnostic());
        assert!(!rendered.contains("token-value"));
        assert!(!rendered.contains("secret-value"));
    }
}

#[test]
fn ranges_retain_exact_field_and_value_location() {
    let error = ServerConfig::parse("client_auth: {none: true}\nlimits:\n  startup_ms: 0\n")
        .err()
        .unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Range);
    assert_eq!(error.field, "limits.startup_ms");
    assert_eq!(error.line, Some(3));
    assert_eq!(error.column, Some(15));
}

#[test]
fn resource_override_keeps_other_limits_bounded() {
    let config =
        ServerConfig::parse("client_auth: {none: true}\nlimits: {concurrency: 2, hook_ms: 1}")
            .unwrap();
    assert_eq!(config.limits.concurrency, 2);
    assert_eq!(config.limits.hook_ms, 1);
    assert!(config.limits.queue_capacity > 0);
    assert!(config.limits.max_pending > 0);
    assert!(config.limits.max_body_bytes > 0);
}

#[test]
fn rejects_yaml_expansion_multiple_documents_and_ephemeral_ports() {
    for source in [
        "client_auth: &auth {none: true}",
        "client_auth: {none: true}\n---\nclient_auth: {none: true}",
        "client_auth: {none: true}\nlisten: 127.0.0.1:0",
        "client_auth: {none: true}\nlimits: {<<: {hook_ms: 1}}",
    ] {
        assert!(ServerConfig::parse(source).is_err());
    }
    assert!(ServerConfig::parse(&" ".repeat(MAX_SERVER_BYTES + 1)).is_err());
}

#[test]
fn log_schema_is_explicit() {
    let config = ServerConfig::parse(
        "client_auth: {none: true}\nlog: {level: warn, format: json, output: stderr}",
    )
    .unwrap();
    assert_eq!(config.log.level, LogLevel::Warn);
    assert_eq!(config.log.format, LogFormat::Json);
    assert_eq!(config.log.output, LogOutput::Stderr);
    assert!(ServerConfig::parse("client_auth: {none: true}\nlog: {output: stdout}").is_err());
}

#[test]
fn records_cannot_use_positional_sequence_shapes() {
    for source in [
        "[127.0.0.1:8080, {none: true}]",
        "client_auth: [true]",
        "client_auth: {none: true}\nlog: [info, text, stderr]",
        "client_auth: {none: true}\nlimits: [1, 2, 3]",
    ] {
        assert!(ServerConfig::parse(source).is_err());
    }
}

#[test]
fn type_errors_identify_known_schema_fields() {
    let error = ServerConfig::parse("client_auth: {none: true}\nlimits: {hook_ms: '1'}")
        .err()
        .unwrap();
    assert_eq!(error.field, "limits.hook_ms");
}

#[test]
fn yaml_error_preserves_original_unknown_field_and_coordinates() {
    use std::error::Error;

    let error = ServerConfig::parse("client_auth: {none: true}\nsecret-key: secret-value")
        .err()
        .unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Schema);
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<serde_saphyr::Error>()
        .unwrap();
    assert!(
        matches!(source, serde_saphyr::Error::SerdeUnknownField { field, .. } if field == "secret-key")
    );
    let location = source.location().unwrap();
    assert_eq!(error.line, Some(location.line()));
    assert_eq!(error.column, Some(location.column()));
    assert!(
        error
            .diagnostic()
            .to_string()
            .contains("YAML (unknown field)")
    );
    let rendered = format!("{error} {error:?} {}", error.diagnostic());
    assert!(!rendered.contains("secret-key"));
    assert!(!rendered.contains("secret-value"));
}

#[test]
fn yaml_generic_type_failure_keeps_concrete_safe_source() {
    use std::error::Error;

    let error = ServerConfig::parse("client_auth: {bearer: [secret-token]}")
        .err()
        .unwrap();
    assert!(error.source().unwrap().is::<serde_saphyr::Error>());
    assert_eq!(error.field, "client_auth.bearer");
    assert!(error.line.is_some());
    assert!(!error.diagnostic().to_string().contains("secret-token"));
}
