use limen_config::{Model, Provider, ResourceLimits};
use serde_json::Value;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    json,
    lifecycle::Lifecycle,
    pipeline::{commit_effects, hook_invalid},
};

/// Downstream batches must be fully encodable before their hook effects commit.
pub(super) enum EventConsumer<'a> {
    Upstream,
    Stream {
        tracker: &'a mut limen_chat::ChunkTracker,
        frames: &'a mut Vec<Vec<u8>>,
        max_event_bytes: usize,
    },
}

impl EventConsumer<'_> {
    fn phase(&self) -> &'static str {
        match self {
            Self::Upstream => "onUpstreamEvent",
            Self::Stream { .. } => "onStreamEvent",
        }
    }

    fn consume(&mut self, events: &[Value]) -> Result<(), Failure> {
        if let Self::Stream {
            tracker,
            frames,
            max_event_bytes,
        } = self
        {
            let mut batch = Vec::with_capacity(events.len());
            for event in events {
                // A failure terminates the request, so tracker changes need no rollback.
                tracker.push(event).map_err(stream_failure)?;
                batch.push(
                    limen_chat::encode_chunk(event, *max_event_bytes).map_err(stream_failure)?,
                );
            }
            frames.extend(batch);
        }
        Ok(())
    }
}

fn stream_failure(source: limen_chat::Error) -> Failure {
    Failure::new(ErrorCode::HookFailed, "onStreamEvent").with_source(source)
}

/// One source event shares this work allowance through both streaming phases.
pub(super) struct EventBudget {
    events: usize,
    bytes: usize,
}

impl EventBudget {
    pub fn new() -> Self {
        Self {
            events: 0,
            bytes: 0,
        }
    }

    fn charge(
        &mut self,
        events: usize,
        bytes: usize,
        limits: &ResourceLimits,
        phase: &'static str,
    ) -> Result<(), Failure> {
        if events > limits.max_output_events.saturating_sub(self.events)
            || bytes > limits.max_output_bytes.saturating_sub(self.bytes)
        {
            return Err(Failure::new(ErrorCode::HookFailed, phase));
        }
        self.events += events;
        self.bytes += bytes;
        Ok(())
    }
}

impl Gateway {
    pub(super) async fn event_hooks(
        &self,
        provider: &Provider,
        model: &Model,
        event: Value,
        mut consumer: EventConsumer<'_>,
        budget: &mut EventBudget,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<Value>, Failure> {
        let phase = consumer.phase();
        validate_event(
            &event,
            &self.server.limits,
            ErrorCode::UpstreamProtocol,
            phase,
        )?;
        let mut events = vec![event];
        let mut remaining = provider
            .pipeline
            .iter()
            .chain(&model.pipeline)
            .filter(|step| event_hook(step, phase).is_some())
            .count();
        if remaining == 0 {
            consumer.consume(&events)?;
        }
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            let Some(hook) = event_hook(step, phase) else {
                continue;
            };
            remaining -= 1;
            let final_hook = remaining == 0;
            let mut next = Vec::new();
            for event in events {
                let mut context = lifecycle.context(phase);
                context["event"] = event;
                let mut output = self
                    .hook(hook, phase, Some(&step.name), context, lifecycle)
                    .await?;
                let produced = output
                    .context
                    .get("events")
                    .and_then(Value::as_array)
                    .ok_or_else(|| hook_invalid(phase, &step.name))?;
                // The VM bridge already checked the whole output's JSON tree.
                // Chat semantics and the per-event/cumulative byte budgets remain here.
                let bytes = produced.iter().try_fold(0_usize, |sum, event| {
                    let size = validate_event_payload(
                        event,
                        &self.server.limits,
                        ErrorCode::HookFailed,
                        phase,
                    )?;
                    sum.checked_add(size)
                        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, phase))
                })?;
                budget.charge(produced.len(), bytes, &self.server.limits, phase)?;
                // Earlier steps may still transform these events. The final step's
                // whole batch must satisfy its consumer before any of its effects commit.
                if final_hook {
                    consumer.consume(produced)?;
                }
                commit_effects(lifecycle, &mut output)?;
                let Value::Array(produced) = output.context["events"].take() else {
                    return Err(hook_invalid(phase, &step.name));
                };
                next.extend(produced);
            }
            events = next;
        }
        Ok(events)
    }
}

fn event_hook(step: &limen_config::PipelineStep, phase: &str) -> Option<limen_script::HookId> {
    match phase {
        "onUpstreamEvent" => step.on_upstream_event,
        "onStreamEvent" => step.on_stream_event,
        _ => None,
    }
}

fn validate_event(
    event: &Value,
    limits: &ResourceLimits,
    code: ErrorCode,
    phase: &'static str,
) -> Result<usize, Failure> {
    limen_config::validate_json(event, limits.max_json_depth, limits.max_json_nodes)
        .map_err(|source| Failure::new(code, phase).with_source(source))?;
    validate_event_payload(event, limits, code, phase)
}

fn validate_event_payload(
    event: &Value,
    limits: &ResourceLimits,
    code: ErrorCode,
    phase: &'static str,
) -> Result<usize, Failure> {
    limen_chat::validate_chunk(event)
        .map_err(|source| Failure::new(code, phase).with_source(source))?;
    json::size(event, limits.max_event_bytes, code, phase)
}

#[cfg(test)]
mod tests;
