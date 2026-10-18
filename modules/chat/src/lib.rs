//! Pure, bounded Chat Completions framing and conversion. No transport or VM state.

mod aggregate;
mod bounded;
mod chunk;
mod content;
mod convert;
mod error;
mod native_content;
mod sse;
mod usage;

pub use aggregate::Aggregator;
pub use chunk::{ChunkTracker, FinishCounts, validate_chunk};
pub use content::{
    ContentError, ContentFrame, ContentProjector, GeneratedContent, observe_generated_content,
    validate_content_event,
};
pub use convert::JsonChunks;
pub use error::Error;
pub use native_content::{
    NativeContentError, NativeContentLimits, NativeContentPolicy, NativeContentTemplate,
};
pub use sse::{DONE, Event, Frame, FrameDecoder, encode_chunk, encode_error};
pub use usage::{UsageQuality, UsageStats, UsageTracker};

#[cfg(test)]
mod tests;
