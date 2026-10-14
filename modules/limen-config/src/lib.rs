//! Validated startup configuration and shared protocol contracts.

mod business;
mod error;
mod protocol;
mod server;

pub use business::{BusinessConfig, HookId, Model, PipelineStep, Provider, UpstreamAuth};
pub use error::{ConfigError, ConfigErrorKind};
pub use protocol::{ErrorCode, validate_response_status};
pub use server::{
    ClientAuth, LogConfig, LogFormat, LogLevel, LogOutput, ResourceLimits, ServerConfig,
    load_server,
};
