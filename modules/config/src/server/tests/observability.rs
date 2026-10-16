use super::*;

fn source(format: ServerFormat, section: &str, key: &str, value: &str) -> String {
    match format {
        ServerFormat::Toml => {
            format!(
                "auth = {{ api_key = 'test-api-key-01234567890123456789' }}\n[{section}]\n{key} = {value}\n"
            )
        },
        ServerFormat::Yaml => {
            format!(
                "auth: {{api_key: test-api-key-01234567890123456789}}\n{section}:\n  {key}: {value}\n"
            )
        },
    }
}

fn parse_events(format: ServerFormat, names: &[String]) -> Result<ServerConfig, ConfigError> {
    ServerConfig::parse(
        &source(
            format,
            "observability",
            "events",
            &serde_json::to_string(names).unwrap(),
        ),
        format,
    )
}

fn assert_safe(error: &ConfigError) {
    assert!(error.line.is_some());
    assert!(error.column.is_some());
    let rendered = format!("{error} {error:?} {}", error.diagnostic());
    assert!(!rendered.contains("secret-key"));
    assert!(!rendered.contains("secret-value"));
}

#[test]
fn omitted_and_empty_observability_sections_disable_events() {
    for (format, source) in [
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }",
        ),
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }\n[observability]",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: {}",
        ),
    ] {
        let config = ServerConfig::parse(source, format).unwrap();
        assert_eq!(config.observability.events, Vec::<String>::new());
        assert_eq!(config.limits.max_observations_per_hook, 8);
        assert_eq!(config.limits.max_observations_per_request, 64);
    }
    assert_eq!(ObservabilityConfig::default().events, Vec::<String>::new());
}

#[test]
fn event_allowlists_preserve_order_and_accept_empty_lists() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        let names = ["cache_hit", "route_2", "a"].map(str::to_owned);
        assert_eq!(
            parse_events(format, &names).unwrap().observability.events,
            names
        );
        assert_eq!(
            parse_events(format, &[]).unwrap().observability.events,
            Vec::<String>::new()
        );
    }
}

#[test]
fn event_names_follow_the_ascii_grammar_without_normalization() {
    for name in [
        "Upper",
        "camelCase",
        "_prefix",
        "1prefix",
        "a-b",
        "a.b",
        "a b",
        "a\n",
        "a中",
    ] {
        for format in [ServerFormat::Toml, ServerFormat::Yaml] {
            let error = parse_events(format, &[name.to_owned()]).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Schema);
            assert_eq!(error.field, "observability.events");
            assert!(error.line.is_some());
        }
    }
}

#[test]
fn duplicate_event_names_are_rejected() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        let error = parse_events(format, &["cache_hit".to_owned(), "cache_hit".to_owned()])
            .err()
            .unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        assert_eq!(error.field, "observability.events");
    }
}

#[test]
fn event_names_and_allowlists_have_inclusive_length_limits() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        assert!(parse_events(format, &["a".to_owned()]).is_ok());
        assert!(parse_events(format, &["a".repeat(64)]).is_ok());
        for name in [String::new(), "a".repeat(65)] {
            let error = parse_events(format, &[name]).err().unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Range);
            assert_eq!(error.field, "observability.events");
        }
        let mut names: Vec<_> = (0..64).map(|index| format!("event_{index}")).collect();
        assert_eq!(
            parse_events(format, &names)
                .unwrap()
                .observability
                .events
                .len(),
            64
        );
        names.push("extra".to_owned());
        let error = parse_events(format, &names).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Range);
        assert_eq!(error.field, "observability.events");
    }
}

#[test]
fn event_list_type_failures_preserve_safe_fields_and_locations() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        for value in ["'secret-value'", "1", "true", "[1]", "[true]", "[[]]"] {
            let error =
                ServerConfig::parse(&source(format, "observability", "events", value), format)
                    .err()
                    .unwrap();
            assert_eq!(error.kind, ConfigErrorKind::Schema);
            assert_eq!(error.field, "observability.events");
            assert_safe(&error);
        }
    }
    for value in ["null", "[null]", "[{}]", "{secret-key: secret-value}"] {
        let error = ServerConfig::parse(
            &source(ServerFormat::Yaml, "observability", "events", value),
            ServerFormat::Yaml,
        )
        .err()
        .unwrap();
        assert_eq!(error.field, "observability.events");
        assert_safe(&error);
    }
}

#[test]
fn observability_sections_reject_wrong_types_unknown_and_duplicate_fields() {
    for (format, source) in [
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }\nobservability = []",
        ),
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }\nobservability = 'secret-value'",
        ),
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }\n[observability]\nsecret-key = 'secret-value'",
        ),
        (
            ServerFormat::Toml,
            "auth = { api_key = 'test-api-key-01234567890123456789' }\n[observability]\nevents = []\nevents = []",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: null",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: []",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: secret-value",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: {secret-key: secret-value}",
        ),
        (
            ServerFormat::Yaml,
            "auth: {api_key: test-api-key-01234567890123456789}\nobservability: {events: [], events: []}",
        ),
    ] {
        let error = ServerConfig::parse(source, format).err().unwrap();
        assert_eq!(error.kind, ConfigErrorKind::Schema);
        assert_safe(&error);
    }
}

#[test]
fn invalid_event_names_retain_exact_array_location_without_echoing_names() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        let error = parse_events(format, &["secret-value".to_owned()])
            .err()
            .unwrap();
        assert_eq!(error.field, "observability.events");
        assert_eq!(error.line, Some(3));
        assert_eq!(
            error.column,
            Some(if format == ServerFormat::Toml { 10 } else { 11 })
        );
        assert_safe(&error);
    }
}

#[test]
fn observation_capacity_limits_accept_boundaries_and_keep_other_defaults() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        for (name, maximum) in [
            ("max_observations_per_hook", 64),
            ("max_observations_per_request", 4096),
        ] {
            for value in [1, maximum] {
                let config = ServerConfig::parse(
                    &source(format, "limits", name, &value.to_string()),
                    format,
                )
                .unwrap();
                let actual = if name == "max_observations_per_hook" {
                    config.limits.max_observations_per_hook
                } else {
                    config.limits.max_observations_per_request
                };
                assert_eq!(actual, value);
                assert_eq!(config.limits.max_output_events, 64);
            }
            for value in [0, maximum + 1] {
                let error = ServerConfig::parse(
                    &source(format, "limits", name, &value.to_string()),
                    format,
                )
                .err()
                .unwrap();
                assert_eq!(error.kind, ConfigErrorKind::Range);
                assert_eq!(error.field, format!("limits.{name}"));
                assert_eq!(error.line, Some(3));
                assert_eq!(
                    error.column,
                    Some(name.len() as u64 + if format == ServerFormat::Toml { 4 } else { 5 })
                );
            }
        }
    }
}

#[test]
fn observation_capacities_reject_non_integer_types_safely() {
    for format in [ServerFormat::Toml, ServerFormat::Yaml] {
        for name in ["max_observations_per_hook", "max_observations_per_request"] {
            for value in ["'secret-value'", "-1", "1.0", "true", "[]"] {
                let error = ServerConfig::parse(&source(format, "limits", name, value), format)
                    .err()
                    .unwrap();
                assert_eq!(error.kind, ConfigErrorKind::Schema);
                assert_eq!(error.field, format!("limits.{name}"));
                assert_safe(&error);
            }
        }
    }
}
