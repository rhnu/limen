//! Single-instance, bounded request persistence. Connections never cross worker threads.

mod database;
mod metrics;
mod parameters;
mod query;
mod types;
mod worker;

pub use metrics::*;
pub use parameters::*;
pub use types::*;
pub use worker::{Storage, StorageReader, StorageWriter};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage queue is full")]
    Busy,
    #[error("storage is unavailable")]
    Unavailable,
    #[error("storage operation exceeded its deadline")]
    Timeout,
    #[error("query exceeds its bounded scope")]
    TooBroad,
    #[error("invalid storage input")]
    Invalid,
    #[error("database schema does not match this application")]
    Schema,
    #[error("record transition is not valid")]
    Conflict,
    #[error("storage capacity is exhausted")]
    Capacity,
    #[error("storage instance lock is held")]
    Locked,
    #[error("storage worker terminated unexpectedly")]
    Worker,
    #[error("database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem operation failed")]
    Io(#[from] std::io::Error),
    #[error("record encoding failed")]
    Encoding(#[from] serde_json::Error),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::TooBroad => "query_too_broad",
            Self::Invalid | Self::Encoding(_) => "invalid_record",
            Self::Schema => "schema_mismatch",
            Self::Conflict => "record_conflict",
            Self::Capacity => "capacity",
            Self::Locked => "locked",
            Self::Worker => "worker_stopped",
            Self::Database(_) => "database_error",
            Self::Io(_) => "io_error",
            Self::Unavailable => "unavailable",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
