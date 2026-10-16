//! Validated startup configuration.

mod business;
mod error;
mod json;
mod server;

pub use business::{
    BusinessConfig, HookId, Model, PipelineStep, Provider, UpstreamAuth, validate_endpoint,
    validate_json,
};
pub use error::{ConfigError, ConfigErrorKind};
pub use json::{MAX_JSON_DEPTH, parse_json_slice, parse_json_str};
pub use server::{
    ClientAuth, LogConfig, LogFormat, LogLevel, LogOutput, ObservabilityConfig, ResourceLimits,
    ScriptContextConfig, ServerConfig, ServerFormat, load_server,
};
