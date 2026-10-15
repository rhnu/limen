use super::*;

fn source(format: ServerFormat, value: &str) -> String {
    match format {
        ServerFormat::Toml => {
            format!("client_auth = {{ none = true }}\n[script_context]\nclient_headers = {value}\n")
        },
        ServerFormat::Yaml => {
            format!("client_auth: {{none: true}}\nscript_context:\n  client_headers: {value}\n")
        },
    }
}

fn parse_headers(format: ServerFormat, names: &[String]) -> Result<ServerConfig, ConfigError> {
    let value = serde_json::to_string(names).unwrap();
    ServerConfig::parse(&source(format, &value), format)
}

#[test]
fn defaults_and_empty_sections_expose_no_client_headers() {
    for (format, source) in [
        (ServerFormat::Toml, "client_auth = { none = true }"),
        (
            ServerFormat::Toml,
            "client_auth = { none = true }\n[script_context]",
        ),
        (ServerFormat::Yaml, "client_auth: {none: true}"),
        (
            ServerFormat::Yaml,
            "client_auth: {none: true}\nscript_context: {}",
        ),
    ] {
        let config = ServerConfig::parse(source, format).unwrap();
        assert_eq!(config.script_context.client_headers, Vec::<String>::new());
    }
    assert_eq!(
        ScriptContextConfig::default().client_headers,
        Vec::<String>::new()
    );
}

#[test]
fn both_formats_normalize_names_and_preserve_allowlist_order() {
    let names = vec!["X-Client-Name".to_owned(), "X-Trace_ID".to_owned()];
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        let config = parse_headers(format, &names).unwrap();
        assert_eq!(
            config.script_context.client_headers,
            ["x-client-name", "x-trace_id"]
        );
        assert_eq!(
            parse_headers(format, &[])
                .unwrap()
                .script_context
                .client_headers,
            Vec::<String>::new()
        );
    }
}

#[test]
fn duplicate_names_are_case_insensitive() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        for names in [
            ["x-client-name", "x-client-name"],
            ["x-client-name", "X-Client-Name"],
        ] {
            let error = parse_headers(format, &names.map(str::to_owned))
                .err()
                .unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Header);
            assert_eq!(error.field, "script_context.client_headers");
        }
    }
}

#[test]
fn sensitive_and_transport_headers_cannot_be_allowlisted() {
    for name in [
        "Authorization",
        "Proxy-Authorization",
        "Cookie",
        "Set-Cookie",
        "Proxy-Authenticate",
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Content-Encoding",
        "Content-Type",
        "Proxy-Connection",
        "Keep-Alive",
        "TE",
        "Trailer",
        "Upgrade",
    ] {
        for format in [ServerFormat::Toml, ServerFormat::Yaml] {
            let error = parse_headers(format, &[name.to_owned()]).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Header);
            assert_eq!(error.field, "script_context.client_headers");
        }
    }
}

#[test]
fn malformed_and_non_ascii_names_are_rejected() {
    use std::error::Error;

    for name in ["", "bad name", "bad:name", "bad\r\nname", "x-客户端"] {
        for format in [ServerFormat::Toml, ServerFormat::Yaml] {
            let error = parse_headers(format, &[name.to_owned()]).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Header);
            assert_eq!(error.field, "script_context.client_headers");
            assert!(
                error
                    .source()
                    .unwrap()
                    .is::<http::header::InvalidHeaderName>()
            );
        }
    }
}

#[test]
fn allowlist_and_name_lengths_are_bounded() {
    let names: Vec<_> = (0..32).map(|index| format!("x-client-{index}")).collect();
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        assert_eq!(
            parse_headers(format, &names)
                .unwrap()
                .script_context
                .client_headers
                .len(),
            32
        );
        let mut too_many = names.clone();
        too_many.push("x-extra".to_owned());
        let error = parse_headers(format, &too_many).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Range);
        assert_eq!(error.field, "script_context.client_headers");
        assert!(parse_headers(format, &["x".repeat(256)]).is_ok());
        let error = parse_headers(format, &["x".repeat(257)]).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Range);
        assert_eq!(error.field, "script_context.client_headers");
    }
}

#[test]
fn header_list_rejects_wrong_types_with_schema_field() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        for value in ["'secret-value'", "123", "true", "[123]", "[true]", "[[]]"] {
            let error = ServerConfig::parse(&source(format, value), format)
                .err()
                .unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Schema);
            assert_eq!(error.field, "script_context.client_headers");
            assert!(error.line.is_some());
            assert!(error.column.is_some());
            let rendered = format!("{error} {error:?} {}", error.diagnostic());
            assert!(!rendered.contains("secret-value"));
        }
    }
    for value in ["null", "[null]", "[{}]", "{secret-key: secret-value}"] {
        assert!(
            ServerConfig::parse(&source(ServerFormat::Yaml, value), ServerFormat::Yaml).is_err()
        );
    }
}

#[test]
fn context_section_rejects_non_objects_unknown_and_duplicate_fields() {
    for (format, source) in [
        (
            ServerFormat::Toml,
            "client_auth = { none = true }\nscript_context = []",
        ),
        (
            ServerFormat::Toml,
            "client_auth = { none = true }\n[script_context]\nsecret-key = 'secret-value'",
        ),
        (
            ServerFormat::Toml,
            "client_auth = { none = true }\n[script_context]\nclient_headers = []\nclient_headers = []",
        ),
        (
            ServerFormat::Yaml,
            "client_auth: {none: true}\nscript_context: []",
        ),
        (
            ServerFormat::Yaml,
            "client_auth: {none: true}\nscript_context: null",
        ),
        (
            ServerFormat::Yaml,
            "client_auth: {none: true}\nscript_context: {secret-key: secret-value}",
        ),
        (
            ServerFormat::Yaml,
            "client_auth: {none: true}\nscript_context: {client_headers: [], client_headers: []}",
        ),
    ] {
        let error = ServerConfig::parse(source, format)
            .err()
            .unwrap_or_else(|| panic!("accepted invalid section: {source}"));
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        let rendered = format!("{error} {error:?} {}", error.diagnostic());
        assert!(!rendered.contains("secret-key"));
        assert!(!rendered.contains("secret-value"));
    }
}

#[test]
fn header_diagnostics_preserve_locations_and_hide_input() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        let error = parse_headers(format, &["secret header-value".to_owned()])
            .err()
            .unwrap();
        assert_eq!(error.field, "script_context.client_headers");
        assert_eq!(error.line, Some(3));
        assert_eq!(
            error.column,
            Some(if format == ServerFormat::Toml { 18 } else { 19 })
        );
        let rendered = format!("{error} {error:?} {}", error.diagnostic());
        assert!(!rendered.contains("secret header-value"));
        assert!(rendered.contains("invalid header name"));
    }
}
