//! Pure, bounded Chat Completions framing and conversion. No transport or VM state.

mod aggregate;
mod bounded;
mod chunk;
mod convert;
mod error;
mod sse;
mod usage;

pub use aggregate::Aggregator;
pub use chunk::{ChunkTracker, validate_chunk};
pub use convert::JsonChunks;
pub use error::Error;
pub use sse::{DONE, Decoder, Event, encode_chunk, encode_error};
pub use usage::{UsageQuality, UsageStats, UsageTracker};

#[cfg(test)]
mod tests;
