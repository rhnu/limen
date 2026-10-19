use std::sync::{Arc, Mutex};

use limen_config::ResourceLimits;
use limen_script::{EventInput, HookCandidate, HookInput, HookPhase, SseContext};
use serde_json::Value;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    json,
    lifecycle::Lifecycle,
    pipeline::{commit_effects, hook_invalid},
    plan::RoutePlan,
};

pub(super) struct EncodedEvent {
    pub bytes: Vec<u8>,
    pub content: limen_chat::GeneratedContent,
}

/// Downstream batches must be fully encodable before their hook effects commit.
pub(super) enum EventConsumer<'a> {
    Upstream {
        frame: Option<&'a limen_chat::Frame>,
    },
    Stream {
        tracker: &'a mut limen_chat::ChunkTracker,
        frames: &'a mut Vec<EncodedEvent>,
        max_event_bytes: usize,
        frame: Option<&'a limen_chat::Frame>,
        stream_started: bool,
    },
}

impl EventConsumer<'_> {
    fn phase(&self) -> HookPhase {
        match self {
            Self::Upstream { .. } => HookPhase::OnUpstreamEvent,
            Self::Stream { .. } => HookPhase::OnStreamEvent,
        }
    }

    fn frame(&self) -> Option<&limen_chat::Frame> {
        match self {
            Self::Upstream { frame } | Self::Stream { frame, .. } => *frame,
        }
    }

    fn consume(
        &mut self,
        events: &[Value],
        budget: &mut EventBudget,
        limits: &ResourceLimits,
    ) -> Result<(), Failure> {
        if let Self::Stream {
            tracker,
            frames,
            max_event_bytes,
            frame,
            stream_started,
        } = self
        {
            let mut batch = Vec::with_capacity(events.len());
            for event in events {
                // A failure terminates the request, so tracker changes need no rollback.
                tracker.push(event).map_err(stream_failure)?;
                let bytes = if let Some(frame) = frame {
                    frame.encode(event, *max_event_bytes)
                } else {
                    limen_chat::encode_chunk(event, *max_event_bytes)
                };
                let mut bytes = bytes.map_err(stream_failure)?;
                if frame.is_some_and(limen_chat::Frame::has_leading_bom)
                    && (*stream_started || !batch.is_empty())
                    && bytes.starts_with(b"\xef\xbb\xbf")
                {
                    bytes.drain(..3);
                }
                budget.charge_wire(bytes.len(), limits)?;
                batch.push(EncodedEvent {
                    bytes,
                    content: limen_chat::observe_generated_content(event),
                });
            }
            frames.extend(batch);
        }
        Ok(())
    }
}

fn stream_failure(source: limen_chat::Error) -> Failure {
    Failure::new(ErrorCode::HookFailed, "onStreamEvent").with_source(source)
}

/// One source event shares this allowance through all stages and delayed outputs.
#[derive(Clone, Debug)]
pub(super) struct EventBudget {
    work: Arc<Mutex<EventWork>>,
}

#[derive(Debug)]
struct EventWork {
    events: usize,
    bytes: usize,
    wire_bytes: usize,
}

impl EventBudget {
    pub fn new() -> Self {
        Self {
            work: Arc::new(Mutex::new(EventWork {
                events: 0,
                bytes: 0,
                wire_bytes: 0,
            })),
        }
    }

    fn charge_wire(&mut self, bytes: usize, limits: &ResourceLimits) -> Result<(), Failure> {
        let mut work = self
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if bytes > limits.max_output_bytes.saturating_sub(work.wire_bytes) {
            return Err(Failure::new(ErrorCode::HookFailed, "onStreamEvent"));
        }
        work.wire_bytes += bytes;
        Ok(())
    }

    pub(super) fn charge(
        &mut self,
        events: usize,
        bytes: usize,
        limits: &ResourceLimits,
        phase: &'static str,
    ) -> Result<(), Failure> {
        let mut work = self
            .work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if events > limits.max_output_events.saturating_sub(work.events)
            || bytes > limits.max_output_bytes.saturating_sub(work.bytes)
        {
            return Err(Failure::new(ErrorCode::HookFailed, phase));
        }
        work.events += events;
        work.bytes += bytes;
        Ok(())
    }
}

impl Gateway {
    pub(super) async fn event_hooks(
        &self,
        route: &RoutePlan<'_>,
        event: Value,
        mut consumer: EventConsumer<'_>,
        budget: &mut EventBudget,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<Value>, Failure> {
        let phase = consumer.phase();
        let phase_name = phase.as_str();
        // Callers establish source JSON and Chat guarantees before this stage.
        // Only fresh hook outputs need validation and expansion accounting here;
        // counting normalized input JSON could reject an unchanged raw SSE frame.
        let mut events = vec![event];
        let (provider_steps, model_steps) = match consumer {
            EventConsumer::Upstream { .. } => route.upstream_event(),
            EventConsumer::Stream { .. } => route.stream_event(),
        };
        let mut remaining = provider_steps.len() + model_steps.len();
        if remaining == 0 {
            consumer.consume(&events, budget, &self.limits)?;
        }
        for step in provider_steps.iter().chain(model_steps) {
            lifecycle.scope = step.scope;
            let hook = step.executor;
            remaining -= 1;
            let final_hook = remaining == 0;
            let mut next = Vec::new();
            for event in events {
                let input = EventInput {
                    event,
                    sse: sse_context(consumer.frame()),
                };
                let input = match &consumer {
                    EventConsumer::Upstream { .. } => HookInput::OnUpstreamEvent(input),
                    EventConsumer::Stream { .. } => HookInput::OnStreamEvent(input),
                };
                let output = self
                    .hook(hook, Some(&step.name), lifecycle.context(input), lifecycle)
                    .await?;
                let produced = match output.candidate {
                    HookCandidate::OnUpstreamEvent { events }
                        if phase == HookPhase::OnUpstreamEvent =>
                    {
                        events
                    },
                    HookCandidate::OnStreamEvent { events }
                        if phase == HookPhase::OnStreamEvent =>
                    {
                        events
                    },
                    _ => return Err(hook_invalid(phase_name, &step.name)),
                };
                // The VM bridge already checked the whole output's JSON tree.
                // Chat semantics and the per-event/cumulative byte budgets remain here.
                let bytes = produced.iter().try_fold(0_usize, |sum, event| {
                    let size = validate_event_payload(
                        event,
                        &self.limits,
                        ErrorCode::HookFailed,
                        phase_name,
                    )?;
                    sum.checked_add(size)
                        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, phase_name))
                })?;
                budget.charge(produced.len(), bytes, &self.limits, phase_name)?;
                // Earlier steps may still transform these events. The final step's
                // whole batch must satisfy its consumer before any of its effects commit.
                if final_hook {
                    consumer.consume(&produced, budget, &self.limits)?;
                }
                commit_effects(lifecycle, output.effects);
                next.extend(produced);
            }
            events = next;
        }
        Ok(events)
    }
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

pub(super) fn sse_context(frame: Option<&limen_chat::Frame>) -> Option<SseContext> {
    frame.map(|frame| SseContext {
        event: frame.event.clone(),
        id: frame.id.clone(),
        retry: frame.retry.clone(),
    })
}

#[cfg(test)]
mod tests;
