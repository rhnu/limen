use super::*;

mod script_context;

fn parse_yaml_config(source: &str) -> Result<ServerConfig, ConfigError> {
    ServerConfig::parse(source, ServerFormat::Yaml)
}

#[test]
fn explicit_none_and_bearer_are_distinct() {
    let config = parse_yaml_config("client_auth: {none: true}").unwrap();
    assert!(config.listen.ip().is_loopback());
    assert!(matches!(config.client_auth, ClientAuth::None));
    let config = parse_yaml_config("client_auth: {bearer: abc.def-123==}").unwrap();
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
            parse_yaml_config(source).is_err(),
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
        let error = parse_yaml_config(source).err().unwrap();
        assert!(error.line.is_some());
        assert!(error.column.is_some());
        let rendered = format!("{error} {error:?} {}", error.diagnostic());
        assert!(!rendered.contains("token-value"));
        assert!(!rendered.contains("secret-value"));
    }
}

#[test]
fn ranges_retain_exact_field_and_value_location() {
    let error = parse_yaml_config("client_auth: {none: true}\nlimits:\n  startup_ms: 0\n")
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
        parse_yaml_config("client_auth: {none: true}\nlimits: {concurrency: 2, hook_ms: 1}")
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
        assert!(parse_yaml_config(source).is_err());
    }
    assert!(parse_yaml_config(&" ".repeat(MAX_SERVER_BYTES + 1)).is_err());
}

#[test]
fn log_schema_is_explicit() {
    let config = parse_yaml_config(
        "client_auth: {none: true}\nlog: {level: warn, format: json, output: stderr}",
    )
    .unwrap();
    assert_eq!(config.log.level, LogLevel::Warn);
    assert_eq!(config.log.format, LogFormat::Json);
    assert_eq!(config.log.output, LogOutput::Stderr);
    assert!(parse_yaml_config("client_auth: {none: true}\nlog: {output: stdout}").is_err());
}

#[test]
fn records_cannot_use_positional_sequence_shapes() {
    for source in [
        "[127.0.0.1:8080, {none: true}]",
        "client_auth: [true]",
        "client_auth: {none: true}\nlog: [info, text, stderr]",
        "client_auth: {none: true}\nlimits: [1, 2, 3]",
    ] {
        assert!(parse_yaml_config(source).is_err());
    }
}

#[test]
fn type_errors_identify_known_schema_fields() {
    let error = parse_yaml_config("client_auth: {none: true}\nlimits: {hook_ms: '1'}")
        .err()
        .unwrap();
    assert_eq!(error.field, "limits.hook_ms");
}

#[test]
fn yaml_error_preserves_original_unknown_field_and_coordinates() {
    use std::error::Error;

    let error = parse_yaml_config("client_auth: {none: true}\nsecret-key: secret-value")
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

    let error = parse_yaml_config("client_auth: {bearer: [secret-token]}")
        .err()
        .unwrap();
    assert!(error.source().unwrap().is::<serde_saphyr::Error>());
    assert_eq!(error.field, "client_auth.bearer");
    assert!(error.line.is_some());
    assert!(!error.diagnostic().to_string().contains("secret-token"));
}

#[test]
fn toml_and_yaml_share_schema_defaults_and_overrides() {
    for (source, format) in [
        (
            "client_auth: {bearer: abc.def-123==}\nlimits: {concurrency: 2, hook_ms: 1}\nlog: {level: warn, format: json}",
            ServerFormat::Yaml,
        ),
        (
            "[client_auth]\nbearer = 'abc.def-123=='\n[limits]\nconcurrency = 2\nhook_ms = 1\n[log]\nlevel = 'warn'\nformat = 'json'",
            ServerFormat::Toml,
        ),
    ] {
        let config = ServerConfig::parse(source, format).unwrap();
        assert_eq!(config.listen, "127.0.0.1:8080".parse().unwrap());
        assert!(
            matches!(config.client_auth, ClientAuth::Bearer(token) if token == "abc.def-123==")
        );
        assert_eq!(config.limits.concurrency, 2);
        assert_eq!(config.limits.hook_ms, 1);
        assert_eq!(config.limits.request_ms, 120_000);
        assert_eq!(config.log.level, LogLevel::Warn);
        assert_eq!(config.log.format, LogFormat::Json);
        assert_eq!(config.log.output, LogOutput::Stderr);
    }
    let config = ServerConfig::parse("client_auth = { none = true }", ServerFormat::Toml).unwrap();
    assert!(matches!(config.client_auth, ClientAuth::None));
}

#[test]
fn toml_rejects_invalid_schema_and_redacts_parser_causes() {
    use std::error::Error;

    for source in [
        "secret-key = 'secret-value'",
        "client_auth = { none = true }\nsecret-key = 'secret-value'",
        "client_auth = { bearer = 'secret-value', none = true }",
        "client_auth = { bearer = 'secret-value', bearer = 'secret-value' }",
        "client_auth = { bearer = ['secret-value'] }",
        "client_auth = { bearer = 123 }",
        "client_auth = { bearer = '' }",
        "client_auth = { bearer = 'secret-value with spaces' }",
        "client_auth = { none = false }",
        "client_auth = { none = 'true' }",
        "client_auth = {}",
        "client_auth = []",
        "client_auth = { none = true }\nlog = ['info']",
        "client_auth = { none = true }\nlog = { output = 'stdout' }",
        "client_auth = { none = true }\nlog = { level = 'secret-value' }",
        "client_auth = { none = true }\nlimits = { hook_ms = 'secret-value' }",
        "client_auth = { none = true }\nlimits = { hook_ms = -1 }",
        "client_auth = { none = true }\nlimits = { hook_ms = 1.0 }",
        "client_auth = { none = true }\nlimits = { hook_ms = true }",
        "client_auth = { none = true }\nlimits = { hook_ms = 0 }",
        "client_auth = { none = true }\nlisten = '127.0.0.1:0'",
        "client_auth = { bearer = 'secret-value'",
    ] {
        let error = ServerConfig::parse(source, ServerFormat::Toml)
            .err()
            .unwrap();
        assert!(error.line.is_some());
        let rendered = format!("{error} {error:?} {}", error.diagnostic());
        assert!(!rendered.contains("secret-key"));
        assert!(!rendered.contains("secret-value"));
        if let Some(cause) = error.source() {
            assert!(cause.is::<toml::de::Error>());
            assert!(rendered.contains("TOML"));
        }
    }
    assert!(ServerConfig::parse(&" ".repeat(MAX_SERVER_BYTES + 1), ServerFormat::Toml).is_err());
    let nested = format!("client_auth = {}true{}", "[".repeat(150), "]".repeat(150));
    assert!(ServerConfig::parse(&nested, ServerFormat::Toml).is_err());
}

#[test]
fn toml_retains_known_fields_and_exact_range_locations() {
    let error = ServerConfig::parse(
        "client_auth = { none = true }\n[limits]\nstartup_ms = 0\n",
        ServerFormat::Toml,
    )
    .err()
    .unwrap();
    assert_eq!(error.kind, ConfigErrorKind::Range);
    assert_eq!(error.field, "limits.startup_ms");
    assert_eq!(error.line, Some(3));
    assert_eq!(error.column, Some(14));
    let error = ServerConfig::parse(
        "client_auth = { none = true }\n[limits]\nhook_ms = '1'",
        ServerFormat::Toml,
    )
    .err()
    .unwrap();
    assert_eq!(error.field, "limits.hook_ms");
    assert_eq!(error.line, Some(3));
}

#[test]
fn default_toml_success_does_not_open_or_merge_yaml() {
    let mut opened = Vec::new();
    let (path, config) = load_server_with(None, |path| {
        opened.push(path.to_path_buf());
        Ok(io::Cursor::new(b"client_auth = { none = true }"))
    })
    .unwrap();
    assert_eq!(path, Path::new("server.toml"));
    assert_eq!(opened, [Path::new("server.toml")]);
    assert!(matches!(config.client_auth, ClientAuth::None));
    assert_eq!(config.log.format, LogFormat::Text);
}

#[test]
fn missing_default_toml_loads_yaml() {
    let mut opened = Vec::new();
    let (path, config) = load_server_with(None, |path| {
        opened.push(path.to_path_buf());
        if path == Path::new("server.toml") {
            Err(io::Error::from(io::ErrorKind::NotFound))
        } else {
            Ok(io::Cursor::new(
                b"client_auth: {none: true}\nlog: {format: json}",
            ))
        }
    })
    .unwrap();
    assert_eq!(path, Path::new("server.yaml"));
    assert_eq!(opened, [Path::new("server.toml"), Path::new("server.yaml")]);
    assert_eq!(config.log.format, LogFormat::Json);
}

#[test]
fn default_open_errors_stop_without_yaml_fallback() {
    use std::error::Error;

    for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidInput] {
        let mut opens = 0;
        let error = load_server_with::<io::Empty>(None, |_| {
            opens += 1;
            Err(io::Error::from(kind))
        })
        .err()
        .unwrap();
        assert_eq!(opens, 1);
        assert_eq!(error.kind, ConfigErrorKind::Io);
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            kind
        );
    }
}

#[test]
fn default_content_errors_stop_without_yaml_fallback() {
    struct FailedRead;
    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
    }

    for bytes in [
        b"client_auth = { none = true".to_vec(),
        b"client_auth = { none = false }".to_vec(),
        b"client_auth = { none = true }\nlimits = { hook_ms = 0 }".to_vec(),
        vec![0xFF],
        vec![b' '; MAX_SERVER_BYTES + 1],
    ] {
        let mut opens = 0;
        assert!(
            load_server_with(None, |_| {
                opens += 1;
                Ok(io::Cursor::new(&bytes))
            })
            .is_err()
        );
        assert_eq!(opens, 1);
    }
    let mut opens = 0;
    assert!(
        load_server_with(None, |_| {
            opens += 1;
            Ok(FailedRead)
        })
        .is_err()
    );
    assert_eq!(opens, 1);
}

#[test]
fn explicit_paths_use_only_the_selected_format_and_directory() {
    for (path, source) in [
        ("another/server.toml", "client_auth = { none = true }"),
        ("another/server.yaml", "client_auth: {none: true}"),
        ("another/server.yml", "client_auth: {none: true}"),
    ] {
        let (selected, config) = load_server_with(Some(Path::new(path)), |requested| {
            assert_eq!(requested, Path::new(path));
            Ok(io::Cursor::new(source))
        })
        .unwrap();
        assert_eq!(selected, Path::new(path));
        assert!(matches!(config.client_auth, ClientAuth::None));
    }
    for explicit in [None, Some(Path::new("server.toml"))] {
        let mut opens = 0;
        let error = load_server_with::<io::Empty>(explicit, |_| {
            opens += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        })
        .err()
        .unwrap();
        assert_eq!(opens, if explicit.is_none() { 2 } else { 1 });
        assert_eq!(error.kind, ConfigErrorKind::Io);
    }
    assert!(
        load_server_with::<io::Empty>(Some(Path::new("server.json")), |_| {
            panic!("unsupported extensions must be rejected before opening")
        })
        .is_err()
    );
}
