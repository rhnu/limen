use std::time::{Duration, Instant};

use limen_script::{Cancellation, ClientContext, HookContext, HookInput, HookMeta, ScriptState};
use serde_json::{Value, json};

use super::{
    error::{ErrorCode, Failure},
    observations::Observations,
    stats::Statistics,
};

/// Mutable data is owned by one request. Hook snapshots enter the VM by value.
pub(super) struct Lifecycle {
    pub request_id: String,
    pub cancellation: Cancellation,
    pub deadline: Instant,
    pub scope: limen_storage::PhaseScope,
    pub cleanup_timings: super::stats::Timings,
    pub client_parameters: limen_storage::GenerationParameters,
    pub upstream_parameters: Option<limen_storage::GenerationParameters>,
    pub phase: &'static str,
    pub step: Option<String>,
    pub started: Instant,
    pub started_at_ms: i64,
    pub recorded: bool,
    pub state: ScriptState,
    pub client: ClientContext,
    pub client_stream: Option<bool>,
    pub alias: Option<String>,
    pub provider: Option<String>,
    pub configured_model: Option<String>,
    pub final_model: Option<String>,
    pub target: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_media: Option<String>,
    pub input_bytes: usize,
    pub outbound_bytes: usize,
    pub upstream_bytes: usize,
    pub upstream_headers_ms: Option<u64>,
    pub output_bytes: usize,
    pub response_headers: http::HeaderMap,
    pub statistics: Statistics,
    pub observations: Observations,
}

impl Lifecycle {
    pub fn new(request_id: String, cancellation: Cancellation, request_ms: u64) -> Self {
        let started = Instant::now();
        Self {
            request_id,
            cancellation,
            deadline: started + Duration::from_millis(request_ms),
            scope: limen_storage::PhaseScope::Global,
            cleanup_timings: super::stats::Timings::default(),
            client_parameters: limen_storage::GenerationParameters::default(),
            upstream_parameters: None,
            phase: "admission",
            step: None,
            started,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            recorded: false,
            state: ScriptState::new(),
            client: ClientContext::default(),
            client_stream: None,
            alias: None,
            provider: None,
            configured_model: None,
            final_model: None,
            target: None,
            upstream_status: None,
            upstream_media: None,
            input_bytes: 0,
            outbound_bytes: 0,
            upstream_bytes: 0,
            upstream_headers_ms: None,
            output_bytes: 0,
            response_headers: http::HeaderMap::new(),
            statistics: Statistics::default(),
            observations: Observations::default(),
        }
    }

    pub fn context(&self, input: HookInput) -> HookContext {
        self.context_with_state(input, self.state.clone())
    }

    pub fn context_with_state(&self, input: HookInput, state: ScriptState) -> HookContext {
        HookContext {
            meta: HookMeta {
                request_id: self.request_id.clone(),
                model: self.alias.clone(),
                provider: self.provider.clone(),
                configured_upstream_model: self.configured_model.clone(),
                client_stream: self.client_stream,
                upstream_status: self.upstream_status,
                upstream_content_type: self.upstream_media.clone(),
            },
            client: self.client.clone(),
            state,
            input,
        }
    }

    pub fn stats(&self) -> Value {
        let mut stats = self.statistics.snapshot();
        let legacy = json!({"inputBytes":self.input_bytes,"outboundBytes":self.outbound_bytes,
            "upstreamBytes":self.upstream_bytes,"outputBytes":self.output_bytes,
            "upstreamHeadersMs":self.upstream_headers_ms,
            "elapsedMs":super::pipeline::elapsed_ms(self.started),
            "finalUpstreamModel":self.final_model,"target":self.target});
        if let (Some(stats), Some(legacy)) = (stats.as_object_mut(), legacy.as_object()) {
            stats.extend(legacy.clone());
        }
        stats
    }

    pub fn record(&self, result: &Result<(), Failure>, observation_names: &[String]) {
        let outcome = match result {
            Ok(()) => "success",
            Err(e) if e.code == ErrorCode::Cancelled => "cancelled",
            Err(_) => "failed",
        };
        let (code, phase, step) =
            result
                .as_ref()
                .err()
                .map_or((None, self.phase, self.step.as_deref()), |e| {
                    (
                        Some(e.code.as_str()),
                        e.phase,
                        e.step.as_deref().or(self.step.as_deref()),
                    )
                });
        tracing::info!(
            request_id = self.request_id,
            outcome,
            code,
            phase,
            step,
            model = self.alias,
            provider = self.provider,
            configured_model = self.configured_model,
            final_model = self.final_model,
            target = self.target,
            upstream_status = self.upstream_status,
            upstream_media = self.upstream_media,
            client_stream = self.client_stream,
            input_bytes = self.input_bytes,
            outbound_bytes = self.outbound_bytes,
            upstream_bytes = self.upstream_bytes,
            upstream_headers_ms = self.upstream_headers_ms,
            output_bytes = self.output_bytes,
            elapsed_ms = self.started.elapsed().as_millis(),
            "request finished"
        );
        super::stats::record(&self.request_id, &self.statistics.snapshot());
        self.observations
            .record(&self.request_id, observation_names);
        if let Err(failure) = result {
            if matches!(
                failure.code,
                ErrorCode::InvalidRequest
                    | ErrorCode::RequestRejected
                    | ErrorCode::Unauthorized
                    | ErrorCode::UnknownModel
                    | ErrorCode::PayloadTooLarge
                    | ErrorCode::Busy
                    | ErrorCode::Cancelled
            ) {
                tracing::warn!(
                    request_id = self.request_id,
                    code = failure.code.as_str(),
                    status = failure.http_status(),
                    phase,
                    step,
                    upstream_status = self.upstream_status,
                    upstream_error_status = failure.upstream_error_status,
                    upstream_media = self.upstream_media,
                    error_diagnostic = %failure,
                    "request rejected or cancelled"
                );
            } else {
                tracing::error!(
                    request_id = self.request_id,
                    code = failure.code.as_str(),
                    status = failure.http_status(),
                    phase,
                    step,
                    upstream_status = self.upstream_status,
                    upstream_error_status = failure.upstream_error_status,
                    upstream_media = self.upstream_media,
                    error_diagnostic = %failure,
                    "request failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
