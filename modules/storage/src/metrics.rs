use serde::{Deserialize, Serialize};

/// Monotonic microseconds; absent measurements are never represented by zero.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestTimings {
    pub request_us: Option<u64>,
    pub request_decode_us: Option<u64>,
    pub storage_begin_us: Option<u64>,
    pub pre_send_us: Option<u64>,
    pub routing_us: Option<u64>,
    pub request_encode_us: Option<u64>,
    pub upstream_headers_wait_us: Option<u64>,
    pub upstream_first_byte_us: Option<u64>,
    pub upstream_first_chunk_us: Option<u64>,
    pub upstream_total_us: Option<u64>,
    pub downstream_first_event_us: Option<u64>,
    pub upstream_read_wait_us: Option<u64>,
    pub downstream_queue_wait_us: Option<u64>,
    pub downstream_drain_us: Option<u64>,
    pub cleanup_us: Option<u64>,
}

/// Content milestones contain no response content or token timestamps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent observed facts, not mutually exclusive states"
)]
pub struct GenerationFacts {
    pub upstream_first_content_us: Option<u64>,
    pub upstream_last_content_us: Option<u64>,
    pub upstream_first_text_us: Option<u64>,
    pub downstream_first_content_us: Option<u64>,
    pub downstream_first_text_us: Option<u64>,
    pub upstream_content_observations: u64,
    pub choice_count: u64,
    pub observed_reasoning: bool,
    pub observed_audio: bool,
    pub content_qualified: bool,
    pub downstream_buffered: bool,
    pub finish_stop: u64,
    pub finish_length: u64,
    pub finish_tool: u64,
    pub finish_filtered: u64,
    pub finish_other: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    Available,
    NotReached,
    NotSse,
    Incomplete,
    MissingUsage,
    InvalidUsage,
    MultipleChoices,
    InsufficientTokens,
    InsufficientObservations,
    ZeroDuration,
    TokenScopeMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownstreamMode {
    NativeStream,
    Buffered,
    NotStreamed,
    NotReached,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestMetrics {
    pub upstream_ttft_ms: Option<f64>,
    pub downstream_ttft_ms: Option<f64>,
    pub upstream_first_text_ms: Option<f64>,
    pub downstream_first_text_ms: Option<f64>,
    pub average_output_tps: Option<f64>,
    pub average_output_tps_status: MetricStatus,
    pub generation_tps: Option<f64>,
    pub generation_tps_status: MetricStatus,
    pub downstream_mode: DownstreamMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseScope {
    Global,
    Provider,
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseExecutor {
    Script,
    Native,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimingSample {
    pub total_us: Option<u64>,
    pub samples: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseTiming {
    pub phase: String,
    pub scope: PhaseScope,
    pub executor: PhaseExecutor,
    pub calls: u64,
    pub failed_calls: u64,
    pub interrupted_calls: u64,
    pub reported_calls: u64,
    pub skipped_budget_calls: u64,
    pub unavailable_calls: u64,
    pub wall_us: Option<u64>,
    pub queue: TimingSample,
    pub initial_call: TimingSample,
    pub pending: TimingSample,
    pub input_bridge: TimingSample,
    pub output_bridge: TimingSample,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSummary {
    pub samples: u64,
    pub mean: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSummary {
    pub records: u64,
    pub request_ms: MetricSummary,
    pub upstream_ttft_ms: MetricSummary,
    pub downstream_ttft_ms: MetricSummary,
    pub upstream_first_text_ms: MetricSummary,
    pub downstream_first_text_ms: MetricSummary,
    pub average_output_tps: MetricSummary,
    pub generation_tps: MetricSummary,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurationAggregate {
    pub sum_ms: Option<f64>,
    pub mean_ms: Option<f64>,
    pub samples: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseAggregate {
    pub phase: String,
    pub scope: PhaseScope,
    pub executor: PhaseExecutor,
    pub calls: u64,
    pub failed_calls: u64,
    pub interrupted_calls: u64,
    pub reported_calls: u64,
    pub skipped_budget_calls: u64,
    pub unavailable_calls: u64,
    pub wall: DurationAggregate,
    pub queue: DurationAggregate,
    pub initial_call: DurationAggregate,
    pub pending: DurationAggregate,
    pub input_bridge: DurationAggregate,
    pub output_bridge: DurationAggregate,
}

#[expect(
    clippy::cast_precision_loss,
    reason = "persisted observations are validated as JSON-safe integers; derived rates use floating point"
)]
pub(crate) fn measurement_number(value: u64) -> f64 { value as f64 }

pub(crate) fn milliseconds(value: u64) -> f64 { measurement_number(value) / 1000.0 }

/// Derive every displayed request metric from the same frozen observations.
pub fn derive_metrics(record: &crate::FinalRecord, client_stream: bool) -> RequestMetrics {
    let facts = &record.generation;
    let upstream_reached = record.upstream_status.is_some()
        || record.usage_source.is_some()
        || record.timings.upstream_total_us.is_some();
    let reached = facts.downstream_first_content_us.is_some()
        || record.timings.downstream_first_event_us.is_some();
    let downstream_mode = if !client_stream {
        DownstreamMode::NotStreamed
    } else if !reached {
        DownstreamMode::NotReached
    } else if facts.downstream_buffered {
        DownstreamMode::Buffered
    } else {
        DownstreamMode::NativeStream
    };
    let usable_usage = !matches!(
        record.usage_quality,
        Some(crate::UsageQuality::Invalid | crate::UsageQuality::Conflicting)
    );
    let average_status = if !upstream_reached {
        MetricStatus::NotReached
    } else if record.upstream_completed != Some(true) {
        MetricStatus::Incomplete
    } else if !usable_usage {
        MetricStatus::InvalidUsage
    } else if record.output_tokens.is_none()
        || !matches!(
            record.usage_quality,
            Some(crate::UsageQuality::Partial | crate::UsageQuality::Reported)
        )
    {
        MetricStatus::MissingUsage
    } else if record.timings.upstream_total_us.is_none()
        || record.timings.upstream_total_us == Some(0)
    {
        MetricStatus::ZeroDuration
    } else {
        MetricStatus::Available
    };
    let average_output_tps = if average_status == MetricStatus::Available {
        record
            .output_tokens
            .zip(record.timings.upstream_total_us)
            .map(|(tokens, us)| measurement_number(tokens) * 1_000_000.0 / measurement_number(us))
    } else {
        None
    };
    let generation_status = generation_status(record, average_status, upstream_reached);
    let generation_tps = if generation_status == MetricStatus::Available {
        record
            .output_tokens
            .zip(
                facts
                    .upstream_first_content_us
                    .zip(facts.upstream_last_content_us),
            )
            .map(|(tokens, (first, last))| {
                measurement_number(tokens - 1) * 1_000_000.0 / measurement_number(last - first)
            })
    } else {
        None
    };
    RequestMetrics {
        upstream_ttft_ms: if record.usage_source.as_deref() == Some("sse") {
            facts.upstream_first_content_us.map(milliseconds)
        } else {
            None
        },
        downstream_ttft_ms: facts.downstream_first_content_us.map(milliseconds),
        upstream_first_text_ms: if record.usage_source.as_deref() == Some("sse") {
            facts.upstream_first_text_us.map(milliseconds)
        } else {
            None
        },
        downstream_first_text_ms: facts.downstream_first_text_us.map(milliseconds),
        average_output_tps,
        average_output_tps_status: average_status,
        generation_tps,
        generation_tps_status: generation_status,
        downstream_mode,
    }
}

fn generation_status(
    record: &crate::FinalRecord,
    average_status: MetricStatus,
    upstream_reached: bool,
) -> MetricStatus {
    let facts = &record.generation;
    if !upstream_reached {
        MetricStatus::NotReached
    } else if record.usage_source.as_deref() != Some("sse") {
        MetricStatus::NotSse
    } else if average_status != MetricStatus::Available {
        average_status
    } else if record.generation.observed_audio
        || !record.generation.content_qualified
        || (record
            .reasoning_output_tokens
            .is_some_and(|count| count > 0)
            && !record.generation.observed_reasoning)
    {
        MetricStatus::TokenScopeMismatch
    } else if record.generation.choice_count != 1 {
        MetricStatus::MultipleChoices
    } else if record.output_tokens.is_none_or(|tokens| tokens <= 1) {
        MetricStatus::InsufficientTokens
    } else if record.generation.upstream_content_observations < 2 {
        MetricStatus::InsufficientObservations
    } else if facts
        .upstream_first_content_us
        .zip(record.generation.upstream_last_content_us)
        .is_none_or(|(first, last)| last <= first)
    {
        MetricStatus::ZeroDuration
    } else {
        MetricStatus::Available
    }
}

#[cfg(test)]
mod tests;
