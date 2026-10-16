use super::*;

const AUTH: &str = "auth = { api_key = 'test-api-key-01234567890123456789' }";

#[test]
fn keys_have_explicit_size_character_and_separation_boundaries() {
    for key in [
        "a".repeat(31),
        "a".repeat(257),
        format!("{} ", "a".repeat(32)),
        "密".repeat(32),
    ] {
        let source = format!("[auth]\napi_key = {key:?}");
        assert!(ServerConfig::parse(&source, ServerFormat::Toml).is_err());
    }
    for size in [32, 256] {
        let key = "a".repeat(size);
        let source = format!("[auth]\napi_key = {key:?}");
        assert!(ServerConfig::parse(&source, ServerFormat::Toml).is_ok());
    }
    for extra in [
        "manager_key = ''",
        "manager_key = 'test-api-key-01234567890123456789'",
    ] {
        let source = format!("[auth]\napi_key = 'test-api-key-01234567890123456789'\n{extra}");
        assert!(ServerConfig::parse(&source, ServerFormat::Toml).is_err());
    }
    assert!(parse_yaml_config("auth: {api_key: 11111111111111111111111111111111}").is_err());
    assert!(
        parse_yaml_config("auth: {api_key: test-api-key-01234567890123456789, manager_key: null}")
            .is_err()
    );
}

#[test]
fn runtime_defaults_and_bounds_are_enforced() {
    let config = ServerConfig::parse(AUTH, ServerFormat::Toml).unwrap();
    assert_eq!(config.storage.dir, Path::new("../data"));
    assert_eq!(config.storage.retention_days, 90);
    assert_eq!(config.storage.max_records, 100_000);
    assert_eq!(config.storage.max_db_bytes, 1_073_741_824);
    assert_eq!(config.log.max_file_bytes, 16_777_216);
    assert_eq!(config.management.concurrency, 4);
    for section in [
        "[storage]\ndir = ''",
        "[log]\ndir = ''",
        "[storage]\nretention_days = 0",
        "[storage]\nretention_days = 3651",
        "[storage]\nmax_records = 999",
        "[storage]\nmax_records = 10000001",
        "[storage]\nmax_records = 1000\n[limits]\nconcurrency = 1000",
        "[storage]\nmax_db_bytes = 67108863",
        "[storage]\nmax_db_bytes = 67108865",
        "[storage]\nmax_db_bytes = 68719480832",
        "[storage]\noperation_ms = 49",
        "[storage]\noperation_ms = 10001",
        "[storage]\noperation_ms = 1000\n[limits]\nshutdown_ms = 1000",
        "[log]\nmax_file_bytes = 1048575",
        "[log]\nmax_file_bytes = 268435457",
        "[log]\nmax_files = 0",
        "[log]\nmax_files = 33",
        "[management]\nconcurrency = 0",
        "[management]\nconcurrency = 17",
        "[management]\ntimeout_ms = 99",
        "[management]\ntimeout_ms = 10001",
        "[log]\noutput = 'stderr'",
    ] {
        assert!(
            ServerConfig::parse(&format!("{AUTH}\n{section}"), ServerFormat::Toml).is_err(),
            "accepted {section}"
        );
    }
    for section in [
        "storage: null",
        "management: null",
        "log: null",
        "storage: {retention_days: '90'}",
        "storage: {dir: 123}",
    ] {
        assert!(
            parse_yaml_config(&format!(
                "auth: {{api_key: test-api-key-01234567890123456789}}\n{section}"
            ))
            .is_err()
        );
    }
}

#[test]
fn runtime_paths_use_selected_config_parent_and_reject_overlap() {
    let mut config = ServerConfig::parse(AUTH, ServerFormat::Toml).unwrap();
    config
        .resolve_paths(Path::new("nonexistent-runtime-fixture/config/server.toml"))
        .unwrap();
    assert!(config.storage.dir.is_absolute());
    assert_eq!(config.storage.dir.file_name().unwrap(), "data");
    assert_eq!(config.log.dir.file_name().unwrap(), "logs");
    assert_eq!(config.storage.dir.parent(), config.log.dir.parent());
    for (data, logs) in [
        (".", "../logs"),
        ("..", "../logs"),
        ("../data", "../data/logs"),
        ("../data", "../data/../data"),
    ] {
        let source = format!("{AUTH}\n[storage]\ndir = {data:?}\n[log]\ndir = {logs:?}");
        let mut config = ServerConfig::parse(&source, ServerFormat::Toml).unwrap();
        assert!(
            config
                .resolve_paths(Path::new("nonexistent-runtime-fixture/config/server.toml"))
                .is_err()
        );
    }
}
