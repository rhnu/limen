use thiserror::Error;

/// Safe to display: source JSON errors are retained without formatting their input.
#[derive(Debug, Error)]
pub enum Error {
    #[error("SSE event exceeds its byte limit")]
    EventLimit,
    #[error("Chat aggregation exceeds its byte limit")]
    AggregateBytes,
    #[error("Chat aggregation exceeds its event limit")]
    AggregateEvents,
    #[error("Chat stream exceeds its choice limit")]
    ChoiceLimit,
    #[error("invalid Chat Completions event JSON")]
    Json(#[source] serde_json::Error),
    #[error("invalid Chat Completions chunk structure")]
    InvalidChunk,
    #[error("unsupported Chat Completions conversion structure")]
    Unsupported,
    #[error("conflicting Chat Completions stream metadata")]
    ConflictingMetadata,
    #[error("Chat Completions choice has already finished")]
    ChoiceFinished,
    #[error("Chat Completions stream has unfinished choices")]
    UnfinishedChoices,
    #[error("Chat Completions stream ended without DONE")]
    Truncated,
    #[error("Chat Completions stream contains data after DONE")]
    AfterDone,
    #[error("upstream returned an SSE error event")]
    UpstreamError,
}
