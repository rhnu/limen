use limen_config::{Model, Provider, ResourceLimits};
use serde_json::Value;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    json,
    lifecycle::Lifecycle,
    pipeline::{commit_state, hook_invalid},
};

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
        events: &[Value],
        limits: &ResourceLimits,
        phase: &'static str,
    ) -> Result<(), Failure> {
        let bytes = events.iter().try_fold(0_usize, |sum, event| {
            let size = json::size(event, limits.max_output_bytes, ErrorCode::HookFailed, phase)?;
            sum.checked_add(size)
                .ok_or_else(|| Failure::new(ErrorCode::HookFailed, phase))
        })?;
        if events.len() > limits.max_output_events.saturating_sub(self.events)
            || bytes > limits.max_output_bytes.saturating_sub(self.bytes)
        {
            return Err(Failure::new(ErrorCode::HookFailed, phase));
        }
        self.events += events.len();
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
        phase: &'static str,
        budget: &mut EventBudget,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<Value>, Failure> {
        self.validate_event(&event, ErrorCode::UpstreamProtocol, phase)?;
        let mut events = vec![event];
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            let hook = match phase {
                "onUpstreamEvent" => step.on_upstream_event,
                "onStreamEvent" => step.on_stream_event,
                _ => None,
            };
            let Some(hook) = hook else {
                continue;
            };
            let mut next = Vec::new();
            for event in events {
                let mut context = lifecycle.context(phase);
                context["event"] = event;
                let mut output = self
                    .hook(hook, phase, Some(&step.name), context, lifecycle)
                    .await?;
                let produced = output
                    .get("events")
                    .and_then(Value::as_array)
                    .ok_or_else(|| hook_invalid(phase, &step.name))?;
                for event in produced {
                    self.validate_event(event, ErrorCode::HookFailed, phase)?;
                }
                budget.charge(produced, &self.server.limits, phase)?;
                commit_state(lifecycle, &mut output)?;
                let Value::Array(produced) = output["events"].take() else {
                    return Err(hook_invalid(phase, &step.name));
                };
                next.extend(produced);
            }
            events = next;
        }
        Ok(events)
    }

    fn validate_event(
        &self,
        event: &Value,
        code: ErrorCode,
        phase: &'static str,
    ) -> Result<(), Failure> {
        limen_config::validate_json(
            event,
            self.server.limits.max_json_depth,
            self.server.limits.max_json_nodes,
        )
        .map_err(|source| Failure::new(code, phase).with_source(source))?;
        limen_chat::validate_chunk(event)
            .map_err(|source| Failure::new(code, phase).with_source(source))?;
        json::size(event, self.server.limits.max_event_bytes, code, phase)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
