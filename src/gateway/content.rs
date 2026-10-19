//! One semantic transformation stage before either aggregation or delivery.

use std::{collections::VecDeque, sync::Arc, time::Instant};

use limen_chat::{
    ContentFrame, ContentProjector, Frame, NativeContentError, NativeContentLimits,
    NativeContentPolicy, validate_content_event,
};
use limen_config::{ResourceLimits, validate_json};
use serde_json::Value;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    events::EventBudget,
    json,
    lifecycle::Lifecycle,
    pipeline::hook_invalid,
    plan::{ContentExecutor, RoutePlan},
};

mod stage;

/// A released payload keeps the source envelope and its shared expansion budget.
#[derive(Debug)]
pub(super) struct ContentOutput {
    pub value: Value,
    pub envelope: Option<Arc<Frame>>,
    pub budget: EventBudget,
}

#[derive(Clone)]
struct ContentBatch {
    frame: Arc<ContentFrame>,
    envelope: Option<Arc<Frame>>,
    events: Vec<Value>,
    budget: EventBudget,
    complete: bool,
    ending: bool,
    retained: (usize, usize),
}

pub(super) struct ContentPipeline {
    source: ContentProjector,
    stages: Vec<ContentStage>,
}

#[derive(Clone)]
struct ContentStage {
    scope: limen_storage::PhaseScope,
    name: String,
    projector: ContentProjector,
    executor: ContentExecution,
    pending: VecDeque<ContentBatch>,
}

#[derive(Clone)]
enum ContentExecution {
    Script(limen_script::HookId),
    Native(NativeContentPolicy),
}

impl ContentPipeline {
    fn capacity(
        &self,
        candidate: Option<(usize, &ContentStage)>,
        limits: &ResourceLimits,
    ) -> Result<(), Failure> {
        let mut parts = self.source.retained_parts();
        let mut bytes = self.source.retained_bytes();
        for (index, projector) in self.stages.iter().enumerate() {
            let projector = candidate
                .filter(|(stage, _)| *stage == index)
                .map_or(projector, |(_, value)| value);
            parts = parts.saturating_add(projector.projector.retained_parts());
            bytes = bytes.saturating_add(projector.projector.retained_bytes());
            for batch in &projector.pending {
                parts = parts.saturating_add(batch.retained.0);
                bytes = bytes.saturating_add(batch.retained.1);
            }
            if let ContentExecution::Native(native) = &projector.executor {
                parts = parts.saturating_add(native.retained_parts());
                bytes = bytes.saturating_add(native.retained_bytes());
            }
        }
        if parts > limits.max_json_nodes || bytes > limits.max_output_bytes {
            Err(Failure::new(ErrorCode::HookFailed, "onContent"))
        } else {
            Ok(())
        }
    }
}

impl Gateway {
    pub(super) fn content_projector(&self, route: &RoutePlan<'_>) -> Option<ContentPipeline> {
        let max_parts = self.limits.max_json_nodes;
        let (provider_steps, model_steps) = route.content();
        let stages: Vec<_> = provider_steps
            .iter()
            .chain(model_steps)
            .map(|step| ContentStage {
                scope: step.scope,
                name: step.name.clone(),
                projector: ContentProjector::new(max_parts),
                pending: VecDeque::new(),
                executor: match &step.executor {
                    ContentExecutor::Script(hook) => ContentExecution::Script(*hook),
                    ContentExecutor::Native(template) => {
                        ContentExecution::Native(template.instantiate())
                    },
                },
            })
            .collect();
        (!stages.is_empty()).then(|| ContentPipeline {
            source: ContentProjector::new(max_parts),
            stages,
        })
    }
}

pub(super) enum ContentInput {
    Json(Value),
    Chunk(Value),
    End,
}

impl Gateway {
    pub(super) async fn content_json(
        &self,
        route: &RoutePlan<'_>,
        body: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        let Some(mut pipeline) = self.content_projector(route) else {
            return Ok(body);
        };
        let mut output = self
            .content_payload(
                &mut pipeline,
                ContentInput::Json(body),
                None,
                &mut EventBudget::new(),
                lifecycle,
            )
            .await?;
        if output.len() != 1 {
            return Err(Failure::new(ErrorCode::HookFailed, "onContent"));
        }
        let body = output
            .pop()
            .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "onContent"))?
            .value;
        self.content_end(&mut pipeline, lifecycle).await?;
        Ok(body)
    }

    pub(super) async fn content_end(
        &self,
        pipeline: &mut ContentPipeline,
        lifecycle: &mut Lifecycle,
    ) -> Result<(), Failure> {
        let tail = self
            .content_payload(
                pipeline,
                ContentInput::End,
                None,
                &mut EventBudget::new(),
                lifecycle,
            )
            .await?;
        if !tail.is_empty()
            || pipeline
                .stages
                .iter()
                .any(|stage| !stage.pending.is_empty())
        {
            return Err(Failure::new(ErrorCode::HookFailed, "onContent"));
        }
        Ok(())
    }

    pub(super) async fn content_payload(
        &self,
        projector: &mut ContentPipeline,
        input: ContentInput,
        envelope: Option<&Frame>,
        budget: &mut EventBudget,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<ContentOutput>, Failure> {
        let complete = matches!(&input, ContentInput::Json(_));
        let ending = matches!(&input, ContentInput::End);
        let frame = match input {
            ContentInput::Json(value) => projector.source.project_json(value),
            ContentInput::Chunk(value) => projector.source.project_chunk(value),
            ContentInput::End => projector.source.project_end(),
        }
        .map_err(content_failure)?;
        projector.capacity(None, &self.limits)?;
        let mut retained = frame.retained_usage();
        if let Some(envelope) = envelope {
            let bytes = std::iter::once(&envelope.event)
                .chain(envelope.id.iter())
                .chain(envelope.retry.iter())
                .chain(envelope.data.iter())
                .fold(envelope.raw().len().saturating_mul(2), |sum, value| {
                    sum.saturating_add(value.capacity())
                });
            retained.1 = retained.1.saturating_add(bytes).saturating_add(2048);
        }
        let events = frame.events().to_vec();
        let batch = ContentBatch {
            frame: Arc::new(frame),
            envelope: envelope.cloned().map(Arc::new),
            events,
            budget: budget.clone(),
            complete,
            ending,
            retained,
        };
        let mut batches = vec![(batch, None)];
        for cursor in 0..projector.stages.len() {
            let mut next = Vec::new();
            for (batch, _) in batches {
                next.extend(
                    self.content_stage(projector, cursor, batch, lifecycle)
                        .await?,
                );
            }
            batches = next;
        }
        Ok(batches
            .into_iter()
            .filter_map(|(batch, value)| {
                value.map(|value| ContentOutput {
                    value,
                    envelope: batch.envelope,
                    budget: batch.budget,
                })
            })
            .collect())
    }

    /// Rendered JSON and preserved SSE fields must pass together before any effects commit.
    fn validate_content_render(
        &self,
        value: &Value,
        complete: bool,
        envelope: Option<&Frame>,
        step: &str,
    ) -> Result<(), Failure> {
        validate_json(
            value,
            self.limits.max_json_depth,
            self.limits.max_json_nodes,
        )
        .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
        json::size(
            value,
            self.limits.max_output_bytes,
            ErrorCode::HookFailed,
            "onContent",
        )?;
        if !complete {
            limen_chat::validate_chunk(value)
                .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
            if let Some(envelope) = envelope {
                envelope
                    .encode(value, self.limits.max_event_bytes)
                    .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
            }
        }
        Ok(())
    }

    fn native_content_batch(
        &self,
        native: &mut NativeContentPolicy,
        events: Vec<Value>,
        lifecycle: &mut Lifecycle,
        deadline: Instant,
        step: &str,
    ) -> Result<Vec<Vec<Value>>, Failure> {
        lifecycle.phase = "onContent";
        lifecycle.step = Some(step.to_owned());
        let measurement = lifecycle
            .statistics
            .timings
            .native_scoped("onContent", lifecycle.scope);
        let limits = NativeContentLimits {
            max_output_events: self.limits.max_output_events,
            max_output_bytes: self.limits.max_output_bytes,
            max_retained_bytes: self.limits.max_output_bytes,
            max_retained_records: self.limits.max_json_nodes,
        };
        let calls = events.len();
        let result = native
            .apply_batch(events, &limits, &mut || {
                self.native_active(lifecycle, deadline).is_ok()
            })
            .map_err(|source| {
                if matches!(source, NativeContentError::Interrupted)
                    && let Err(failure) = self.native_active(lifecycle, deadline)
                {
                    return failure;
                }
                hook_invalid("onContent", step).with_source(source)
            });
        measurement.finish_batch(&result, calls);
        let produced = result?;
        // JS outputs already passed the VM bridge; native events need
        // their generic JSON boundary before shared semantic accounting.
        for event in produced.iter().flatten() {
            validate_json(
                event,
                self.limits.max_json_depth,
                self.limits.max_json_nodes,
            )
            .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
        }
        Ok(produced)
    }
}

fn charge(
    events: &[Value],
    budget: &mut EventBudget,
    limits: &ResourceLimits,
    step: &str,
) -> Result<(), Failure> {
    let bytes = events.iter().try_fold(0_usize, |sum, event| {
        validate_content_event(event).map_err(content_failure)?;
        let size = json::size(
            event,
            limits.max_output_bytes,
            ErrorCode::HookFailed,
            "onContent",
        )?;
        sum.checked_add(size)
            .ok_or_else(|| hook_invalid("onContent", step))
    })?;
    budget.charge(events.len(), bytes, limits, "onContent")
}

fn render(
    projector: &mut ContentProjector,
    frame: ContentFrame,
    events: &[Value],
    ending: bool,
) -> Result<Option<Value>, Failure> {
    if ending {
        projector.render_end(frame, events).map_err(content_failure)
    } else {
        projector
            .render(frame, events)
            .map(Some)
            .map_err(content_failure)
    }
}

fn content_failure(source: limen_chat::ContentError) -> Failure {
    Failure::new(ErrorCode::HookFailed, "onContent").with_source(source)
}

#[cfg(test)]
mod tests;
