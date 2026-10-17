use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use limen_storage::{FinalRecord, Outcome, PendingRecord, StorageWriter, UsageQuality};

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    lifecycle::Lifecycle,
};

/// The offline runner selects an in-memory recorder explicitly; production always uses SQLite.
pub(crate) enum RecordSink {
    Persistent(StorageWriter),
    Memory(Arc<Mutex<MemoryRecord>>),
}

#[derive(Default)]
pub(crate) struct MemoryRecord {
    pub pending: Option<PendingRecord>,
    pub terminal: Option<FinalRecord>,
    pub fail_begin: bool,
    pub fail_finish: bool,
    #[cfg(test)]
    pub begin_gate: Option<BeginGate>,
}

#[cfg(test)]
pub(crate) struct BeginGate {
    pub entered: Arc<tokio::sync::Notify>,
    pub release: tokio::sync::oneshot::Receiver<()>,
}

impl RecordSink {
    pub fn memory() -> Self { Self::Memory(Arc::default()) }

    pub async fn begin(
        &self,
        record: PendingRecord,
        deadline: Instant,
    ) -> limen_storage::Result<()> {
        match self {
            Self::Persistent(storage) => storage.begin(record, deadline).await,
            Self::Memory(state) => {
                {
                    let mut state = state
                        .lock()
                        .map_err(|_error| limen_storage::Error::Worker)?;
                    if state.fail_begin {
                        return Err(limen_storage::Error::Unavailable);
                    }
                    // One offline case owns one gateway and one request, without unbounded history.
                    if state.pending.is_some() {
                        return Err(limen_storage::Error::Capacity);
                    }
                    state.pending = Some(record);
                }
                #[cfg(test)]
                {
                    let gate = state
                        .lock()
                        .map_err(|_error| limen_storage::Error::Worker)?
                        .begin_gate
                        .take();
                    if let Some(gate) = gate {
                        gate.entered.notify_one();
                        match tokio::time::timeout_at(deadline.into(), gate.release).await {
                            Ok(Ok(())) => {},
                            outcome => {
                                state
                                    .lock()
                                    .map_err(|_error| limen_storage::Error::Worker)?
                                    .fail_begin = true;
                                return Err(if outcome.is_err() {
                                    limen_storage::Error::Timeout
                                } else {
                                    limen_storage::Error::Worker
                                });
                            },
                        }
                    }
                }
                Ok(())
            },
        }
    }

    pub async fn finish(
        &self,
        record: FinalRecord,
        deadline: Instant,
    ) -> limen_storage::Result<()> {
        match self {
            Self::Persistent(storage) => storage.finish(record, deadline).await,
            Self::Memory(state) => {
                let mut state = state
                    .lock()
                    .map_err(|_error| limen_storage::Error::Worker)?;
                if state.fail_finish {
                    state.fail_begin = true;
                    return Err(limen_storage::Error::Unavailable);
                }
                state.terminal = Some(record);
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests;

impl Gateway {
    pub(super) async fn record_begin(&self, lifecycle: &mut Lifecycle) -> Result<(), Failure> {
        lifecycle.phase = "storage_begin";
        let deadline = lifecycle
            .deadline
            .min(Instant::now() + Duration::from_millis(self.storage_operation_ms));
        self.records
            .begin(
                PendingRecord {
                    request_id: lifecycle.request_id.clone(),
                    run_id: self.run_id.clone(),
                    started_at_ms: lifecycle.started_at_ms,
                    client_stream: lifecycle.client_stream.unwrap_or(false),
                    input_bytes: lifecycle.input_bytes as u64,
                },
                deadline,
            )
            .await
            .map_err(|error| {
                let code = if matches!(error, limen_storage::Error::Busy) {
                    ErrorCode::Busy
                } else {
                    ErrorCode::StorageUnavailable
                };
                Failure::new(code, "storage_begin").with_source(error)
            })?;
        lifecycle.recorded = true;
        Ok(())
    }
}

/// Copy only frozen, approved metadata before response hooks can change their private drafts.
pub(super) fn terminal(lifecycle: &Lifecycle, failure: Option<&Failure>) -> Option<FinalRecord> {
    if !lifecycle.recorded {
        return None;
    }
    let usage = lifecycle.statistics.usage.snapshot();
    let timings = lifecycle.statistics.snapshot();
    let millis = |key: &str| {
        timings["timings"][key]
            .as_f64()
            .map(|value| Duration::from_secs_f64(value / 1000.0).as_millis())
            .and_then(|value| u64::try_from(value).ok())
    };
    Some(FinalRecord {
        request_id: lifecycle.request_id.clone(),
        finished_at_ms: chrono::Utc::now().timestamp_millis(),
        outcome: Some(match failure {
            None => Outcome::Success,
            Some(failure) if failure.code == ErrorCode::Cancelled => Outcome::Cancelled,
            Some(_) => Outcome::Failed,
        }),
        error_code: failure.map(|error| error.code.as_str().to_owned()),
        error_phase: failure.map(|error| error.phase.to_owned()),
        error_step: failure.and_then(|error| error.step.clone().or_else(|| lifecycle.step.clone())),
        model: lifecycle.alias.clone(),
        provider: lifecycle.provider.clone(),
        configured_upstream_model: lifecycle.configured_model.clone(),
        final_upstream_model: lifecycle.final_model.clone(),
        upstream_status: lifecycle.upstream_status,
        upstream_media: lifecycle.upstream_media.as_deref().map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        }),
        upstream_completed: lifecycle
            .statistics
            .source
            .map(|_| lifecycle.statistics.upstream_completed),
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
        usage_source: lifecycle.statistics.source.map(str::to_owned),
        usage_quality: UsageQuality::parse(usage.quality.as_str()),
        outbound_bytes: lifecycle
            .final_model
            .as_ref()
            .map(|_| lifecycle.outbound_bytes as u64),
        upstream_bytes: lifecycle
            .upstream_status
            .map(|_| lifecycle.upstream_bytes as u64),
        output_bytes: Some(lifecycle.output_bytes as u64),
        request_ms: millis("requestMs"),
        upstream_headers_ms: lifecycle.upstream_headers_ms,
        first_output_ms: millis("downstreamFirstEventMs"),
    })
}
