use std::{
    fs::File,
    io::{self, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_saphyr::Location;

use crate::{
    ConfigError, ConfigErrorKind,
    error::{present_option, safe_path},
};

const MAX_SERVER_BYTES: usize = 1024 * 1024;

mod input;

use input::Input;

/// Both formats deserialize the same schema and apply the same validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerFormat {
    Toml,
    Yaml,
}

impl ServerFormat {
    fn from_path(path: &Path) -> Result<Self, ConfigError> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("toml") => Ok(Self::Toml),
            Some("yaml" | "yml") => Ok(Self::Yaml),
            _ => Err(ConfigError::new(
                ConfigErrorKind::Syntax,
                "server",
                "configuration extension must be .toml, .yaml or .yml",
            )),
        }
    }
}

/// Client authentication is required explicitly, including unauthenticated deployments.
#[derive(Clone)]
pub enum ClientAuth {
    None,
    Bearer(String),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    #[default]
    Stderr,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LogConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub output: LogOutput,
}

// Generate the public limits and a separate span-aware input schema together, so every
// range failure retains the parser's value location without retaining source text.
macro_rules! resource_limits {
    ($($field:ident: $ty:ty = $default:expr, $min:expr => $max:expr;)*) => {
        #[derive(Clone, Debug)]
        pub struct ResourceLimits { $(pub $field: $ty,)* }

        impl Default for ResourceLimits {
            fn default() -> Self { Self { $($field: $default,)* } }
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, default = "RawLimits::default")]
        struct RawLimits<const TOML: bool> {
            $(#[serde(deserialize_with = "strict_spanned")]
            $field: Input<$ty, TOML>,)*
        }

        impl<const TOML: bool> Default for RawLimits<TOML> {
            fn default() -> Self { Self { $($field: Input::unlocated($default),)* } }
        }

        impl<const TOML: bool> RawLimits<TOML> {
            fn validate(self, source: &str) -> Result<ResourceLimits, ConfigError> {
                $(if !($min..=$max).contains(&self.$field.value) {
                    return Err(self.$field.locate(ConfigError::new(ConfigErrorKind::Range,
                        concat!("limits.", stringify!($field)), "value outside allowed range"), source));
                })*
                Ok(ResourceLimits { $($field: self.$field.value,)* })
            }
        }
    };
}

resource_limits! {
    startup_ms: u64 = 5000, 1 => 300_000;
    hook_ms: u64 = 5000, 1 => 300_000;
    request_ms: u64 = 120_000, 1 => 3_600_000;
    execution_ms: u64 = 100, 1 => 10_000;
    vm_work_ms: u64 = 1000, 1 => 60_000;
    cleanup_ms: u64 = 100, 1 => 10_000;
    shutdown_ms: u64 = 5000, 1 => 300_000;
    memory_bytes: usize = 64 * 1024 * 1024, 1024 * 1024 => 1024 * 1024 * 1024;
    stack_bytes: usize = 512 * 1024, 64 * 1024 => 8 * 1024 * 1024;
    concurrency: usize = 32, 1 => 4096;
    queue_capacity: usize = 64, 1 => 65_536;
    max_pending: usize = 128, 1 => 65_536;
    job_batch: usize = 32, 1 => 1024;
    max_body_bytes: usize = 4 * 1024 * 1024, 1 => 256 * 1024 * 1024;
    max_event_bytes: usize = 1024 * 1024, 1 => 64 * 1024 * 1024;
    max_output_events: usize = 64, 1 => 4096;
    max_output_bytes: usize = 4 * 1024 * 1024, 1 => 256 * 1024 * 1024;
    max_aggregate_bytes: usize = 16 * 1024 * 1024, 1 => 1024 * 1024 * 1024;
    max_aggregate_events: usize = 100_000, 1 => 1_000_000;
    max_modules: usize = 128, 1 => 4096;
    max_source_bytes: usize = 4 * 1024 * 1024, 1 => 64 * 1024 * 1024;
    max_json_depth: usize = 64, 1 => 128;
    max_json_nodes: usize = 100_000, 1 => 1_000_000;
}

#[derive(Clone)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub client_auth: ClientAuth,
    pub log: LogConfig,
    pub limits: ResourceLimits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer<const TOML: bool> {
    #[serde(default = "default_listen")]
    listen: Input<SocketAddr, TOML>,
    #[serde(deserialize_with = "map_spanned")]
    client_auth: Input<RawAuth, TOML>,
    #[serde(default, deserialize_with = "map_only")]
    log: LogConfig,
    #[serde(default, deserialize_with = "map_only")]
    limits: RawLimits<TOML>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuth {
    #[serde(default, deserialize_with = "present_option")]
    none: Option<bool>,
    #[serde(default, deserialize_with = "present_option")]
    bearer: Option<String>,
}

struct Strict<T>(T);

struct MapOnly<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for MapOnly<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        map_only(deserializer).map(Self)
    }
}

fn map_only<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}

fn map_spanned<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>, const TOML: bool>(
    deserializer: D,
) -> Result<Input<T, TOML>, D::Error> {
    Input::<MapOnly<T>, TOML>::deserialize(deserializer).map(|value| value.map(|value| value.0))
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Strict<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        // Generic Serde custom errors have no source field; the concrete parser
        // error produced by this fixed category is preserved by ServerConfig::parse.
        T::deserialize(value)
            .map(Self)
            .map_err(|_error| serde::de::Error::custom("invalid field type"))
    }
}

fn strict_spanned<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>, const TOML: bool>(
    deserializer: D,
) -> Result<Input<T, TOML>, D::Error> {
    Input::<Strict<T>, TOML>::deserialize(deserializer).map(|value| value.map(|value| value.0))
}

fn default_listen<const TOML: bool>() -> Input<SocketAddr, TOML> {
    Input::unlocated(SocketAddr::from(([127, 0, 0, 1], 8080)))
}

impl ServerConfig {
    /// Parse a bounded, single-document schema without exposing parser snippets.
    ///
    /// # Errors
    /// Returns sanitized syntax, schema or range diagnostics.
    pub fn parse(source: &str, format: ServerFormat) -> Result<Self, ConfigError> {
        if source.len() > MAX_SERVER_BYTES {
            return Err(ConfigError::new(
                ConfigErrorKind::TooLarge,
                "server",
                "configuration too large",
            ));
        }
        match format {
            ServerFormat::Toml => {
                let mut track = serde_path_to_error::Track::new();
                let deserializer = toml::de::Deserializer::parse(source)
                    .map_err(|error| toml_error(source, "server", error))?;
                let raw = map_only::<RawServer<true>, _>(serde_path_to_error::Deserializer::new(
                    deserializer,
                    &mut track,
                ))
                .map_err(|error| toml_error(source, &safe_path(&track.path()), error))?;
                raw.validate(source)
            },
            ServerFormat::Yaml => parse_yaml(source)?.validate(source),
        }
    }
}

fn toml_error(source: &str, field: &str, mut error: toml::de::Error) -> ConfigError {
    let mut diagnostic = ConfigError::new(
        ConfigErrorKind::Schema,
        field,
        "invalid TOML syntax, field, duplicate key or type",
    );
    if let Some(span) = error.span() {
        diagnostic = diagnostic.at_offset(source, span.start);
    }
    // The parser otherwise retains a complete copy of the input for its snippets.
    error.set_input(None);
    diagnostic.with_source("deserialize server configuration", error)
}

fn parse_yaml(source: &str) -> Result<RawServer<false>, ConfigError> {
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        merge_keys: serde_saphyr::MergeKeyPolicy::Error,
        strict_booleans: true,
        no_schema: true,
        reject_unsupported_tags: true,
        with_snippet: false,
        budget: serde_saphyr::budget! {
            max_documents: 1,
            max_depth: 16,
            flow_nesting_limit: 16,
            max_nodes: 4096,
            max_events: 8192,
            max_aliases: 0,
            max_anchors: 0,
            max_total_scalar_bytes: MAX_SERVER_BYTES,
        },
    };
    let mut track = serde_path_to_error::Track::new();
    let parsed = serde_saphyr::with_deserializer_from_str_with_options(source, options, |de| {
        map_only::<RawServer<false>, _>(serde_path_to_error::Deserializer::new(de, &mut track))
    });
    parsed.map_err(|error| {
        let diagnostic = ConfigError::new(
            ConfigErrorKind::Schema,
            &safe_path(&track.path()),
            "invalid YAML syntax, field, duplicate key or type",
        );
        diagnostic
            .at(error.location().unwrap_or(Location::UNKNOWN))
            .with_source("deserialize server configuration", error)
    })
}

impl<const TOML: bool> RawServer<TOML> {
    fn validate(self, source: &str) -> Result<ServerConfig, ConfigError> {
        let auth = match (
            self.client_auth.value.none,
            self.client_auth.value.bearer.as_ref(),
        ) {
            (Some(true), None) => ClientAuth::None,
            (None, Some(token)) if valid_bearer(token) => ClientAuth::Bearer(token.clone()),
            _ => {
                return Err(self.client_auth.locate(
                    ConfigError::new(
                        ConfigErrorKind::Schema,
                        "client_auth",
                        "specify exactly one nonempty bearer token or none = true",
                    ),
                    source,
                ));
            },
        };
        if self.listen.value.port() == 0 {
            return Err(self.listen.locate(
                ConfigError::new(ConfigErrorKind::Range, "listen", "port must be nonzero"),
                source,
            ));
        }
        let limits = self.limits.validate(source)?;
        Ok(ServerConfig {
            listen: self.listen.value,
            client_auth: auth,
            log: self.log,
            limits,
        })
    }
}

pub(crate) fn valid_bearer(token: &str) -> bool {
    !token.is_empty() && token.len() <= 8192 && {
        let core = token.trim_end_matches('=');
        !core.is_empty()
            && core
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/".contains(&byte))
    }
}

/// Load an explicit file, or try server.toml then server.yaml only when TOML is absent.
/// Read at most one MiB plus a sentinel byte; never merge files.
///
/// # Errors
/// Returns a sanitized diagnostic for file, UTF-8 or configuration errors.
pub fn load_server(path: Option<&Path>) -> Result<(PathBuf, ServerConfig), ConfigError> {
    load_server_with(path, |path| {
        if !std::fs::metadata(path)?.is_file() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        Ok(file)
    })
}

fn load_server_with<R: Read>(
    explicit: Option<&Path>,
    mut open: impl FnMut(&Path) -> io::Result<R>,
) -> Result<(PathBuf, ServerConfig), ConfigError> {
    let io_error = |operation, error| {
        ConfigError::new(
            ConfigErrorKind::Io,
            "server",
            "cannot read configuration file",
        )
        .with_source(operation, error)
    };
    let mut path = explicit.unwrap_or_else(|| Path::new("server.toml"));
    let mut format = ServerFormat::from_path(path)?;
    let file = match open(path) {
        Err(error) if explicit.is_none() && error.kind() == io::ErrorKind::NotFound => {
            path = Path::new("server.yaml");
            format = ServerFormat::Yaml;
            open(path)
        },
        result => result,
    }
    .map_err(|error| io_error("open configuration file", error))?;
    let mut bytes = Vec::new();
    file.take((MAX_SERVER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read configuration bytes", error))?;
    if bytes.len() > MAX_SERVER_BYTES {
        return Err(ConfigError::new(
            ConfigErrorKind::TooLarge,
            "server",
            "configuration too large",
        ));
    }
    let source = std::str::from_utf8(&bytes).map_err(|error| {
        ConfigError::new(
            ConfigErrorKind::Syntax,
            "server",
            "configuration must be UTF-8",
        )
        .with_source("decode configuration as UTF-8", error)
    })?;
    ServerConfig::parse(source, format).map(|config| (path.to_path_buf(), config))
}

#[cfg(test)]
mod tests;
