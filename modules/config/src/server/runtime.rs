use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::{LogFormat, LogLevel, ResourceLimits, ServerConfig, Strict};
use crate::{ConfigError, ConfigErrorKind, error::present_option};

/// Independent credentials, deliberately without Debug or serialization.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(deserialize_with = "strict")]
    pub api_key: String,
    #[serde(default, deserialize_with = "present_option")]
    pub manager_key: Option<String>,
}

impl AuthConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        for (field, value) in [
            ("auth.api_key", Some(&self.api_key)),
            ("auth.manager_key", self.manager_key.as_ref()),
        ] {
            if let Some(value) = value
                && (!(32..=256).contains(&value.len())
                    || !value.bytes().all(|byte| byte.is_ascii_graphic()))
            {
                return Err(ConfigError::new(
                    ConfigErrorKind::Schema,
                    field,
                    "key must contain 32 to 256 printable non-whitespace ASCII bytes",
                ));
            }
        }
        if self.manager_key.as_ref() == Some(&self.api_key) {
            return Err(ConfigError::new(
                ConfigErrorKind::Schema,
                "auth.manager_key",
                "manager key must differ from API key",
            ));
        }
        Ok(())
    }
}

fn strict<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(de: D) -> Result<T, D::Error> {
    Strict::<T>::deserialize(de).map(|value| value.0)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StorageConfig {
    #[serde(deserialize_with = "strict")]
    pub dir: PathBuf,
    #[serde(deserialize_with = "strict")]
    pub retention_days: u64,
    #[serde(deserialize_with = "strict")]
    pub max_records: u64,
    #[serde(deserialize_with = "strict")]
    pub max_db_bytes: u64,
    #[serde(deserialize_with = "strict")]
    pub operation_ms: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            dir: "../data".into(),
            retention_days: 90,
            max_records: 100_000,
            max_db_bytes: 1024 * 1024 * 1024,
            operation_ms: 1000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LogConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    #[serde(deserialize_with = "strict")]
    pub dir: PathBuf,
    #[serde(deserialize_with = "strict")]
    pub max_file_bytes: u64,
    #[serde(deserialize_with = "strict")]
    pub max_files: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::default(),
            format: LogFormat::default(),
            dir: "../logs".into(),
            max_file_bytes: 16 * 1024 * 1024,
            max_files: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ManagementConfig {
    #[serde(deserialize_with = "strict")]
    pub concurrency: usize,
    #[serde(deserialize_with = "strict")]
    pub timeout_ms: u64,
}

impl Default for ManagementConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            timeout_ms: 2000,
        }
    }
}

fn range(valid: bool, field: &str) -> Result<(), ConfigError> {
    if valid {
        Ok(())
    } else {
        Err(ConfigError::new(
            ConfigErrorKind::Range,
            field,
            "value outside allowed range",
        ))
    }
}

fn directory(path: &Path, field: &str) -> Result<(), ConfigError> {
    if path.as_os_str().is_empty() {
        return Err(ConfigError::new(
            ConfigErrorKind::Schema,
            field,
            "directory must not be empty",
        ));
    }
    Ok(())
}

impl StorageConfig {
    pub(super) fn validate(&self, limits: &ResourceLimits) -> Result<(), ConfigError> {
        directory(&self.dir, "storage.dir")?;
        range(
            (1..=3650).contains(&self.retention_days),
            "storage.retention_days",
        )?;
        range(
            (1000..=10_000_000).contains(&self.max_records)
                && self.max_records > limits.concurrency as u64,
            "storage.max_records",
        )?;
        range(
            (64 * 1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&self.max_db_bytes)
                && self.max_db_bytes.is_multiple_of(4096),
            "storage.max_db_bytes",
        )?;
        range(
            (50..=10_000).contains(&self.operation_ms) && self.operation_ms < limits.shutdown_ms,
            "storage.operation_ms",
        )
    }
}

impl LogConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        directory(&self.dir, "log.dir")?;
        range(
            (1024 * 1024..=256 * 1024 * 1024).contains(&self.max_file_bytes),
            "log.max_file_bytes",
        )?;
        range((1..=32).contains(&self.max_files), "log.max_files")
    }
}

impl ManagementConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        range(
            (1..=16).contains(&self.concurrency),
            "management.concurrency",
        )?;
        range(
            (100..=10_000).contains(&self.timeout_ms),
            "management.timeout_ms",
        )
    }
}

impl ServerConfig {
    /// Resolve runtime paths against the selected configuration, without creating files.
    /// Existing ancestors are canonicalized so symlink aliases cannot bypass separation.
    ///
    /// # Errors
    /// Returns controlled errors for inaccessible or overlapping directories.
    pub fn resolve_paths(&mut self, config_path: &Path) -> Result<(), ConfigError> {
        let config_dir = resolve_directory(
            config_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?;
        let data = resolve_directory(&config_dir.join(&self.storage.dir))?;
        let logs = resolve_directory(&config_dir.join(&self.log.dir))?;
        for (left, right) in [(&config_dir, &data), (&config_dir, &logs), (&data, &logs)] {
            if nested(left, right) || nested(right, left) {
                return Err(ConfigError::new(
                    ConfigErrorKind::Schema,
                    "storage.dir",
                    "config, data and logs directories must be separate and not nested",
                ));
            }
        }
        self.storage.dir = data;
        self.log.dir = logs;
        Ok(())
    }
}

fn nested(path: &Path, parent: &Path) -> bool {
    #[cfg(windows)]
    {
        Path::new(&path.to_string_lossy().to_lowercase())
            .starts_with(parent.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.starts_with(parent)
    }
}

fn resolve_directory(path: &Path) -> Result<PathBuf, ConfigError> {
    let io_error = |error| {
        ConfigError::new(
            ConfigErrorKind::Io,
            "storage.dir",
            "cannot resolve runtime directory",
        )
        .with_source("resolve directory", error)
    };
    let absolute = std::path::absolute(path).map_err(io_error)?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                resolved.pop();
            },
            Component::Normal(name) => {
                resolved.push(name);
                match resolved.canonicalize() {
                    Ok(canonical) => resolved = canonical,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(io_error(error)),
                }
            },
            component => resolved.push(component.as_os_str()),
        }
    }
    match resolved.metadata() {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(io_error(std::io::Error::from(
                std::io::ErrorKind::NotADirectory,
            )));
        },
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(io_error(error)),
    }
    Ok(resolved)
}
