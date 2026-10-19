//! Validated startup configuration.

mod builtin;
mod business;
mod error;
mod json;
mod server;

pub use builtin::{
    BeforeSendPolicy, BuiltinPolicy, BuiltinStages, CapCompletionTokens, CompletionTokenField,
    ContentPolicy, DropRequestFields, ForwardClientHeader, ReplaceText, SyntheticTextTool,
};
pub use business::{
    BusinessConfig, HookId, Model, PipelineStep, Provider, UpstreamAuth, validate_endpoint,
    validate_json, validate_json_structure,
};
pub use error::{ConfigError, ConfigErrorKind};
pub use json::{MAX_JSON_DEPTH, parse_json_slice, parse_json_str};
pub use server::{
    AuthConfig, LogConfig, LogFormat, LogLevel, ManagementConfig, ObservabilityConfig,
    ResourceLimits, ScriptContextConfig, ServerConfig, ServerFormat, StorageConfig, load_server,
};
