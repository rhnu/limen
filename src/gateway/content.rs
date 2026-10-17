//! One semantic transformation stage before either aggregation or delivery.

use std::time::Instant;

use limen_chat::{
    ContentFrame, ContentProjector, Frame, NativeContentError, NativeContentLimits,
    NativeContentPolicy, validate_content_event,
};
use limen_config::{ResourceLimits, validate_json};
use limen_script::{EventInput, HookCandidate, HookInput};
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

pub(super) struct ContentPipeline {
    source: ContentProjector,
    stages: Vec<ContentStage>,
}

#[derive(Clone)]
struct ContentStage {
    name: String,
    projector: ContentProjector,
    executor: ContentExecution,
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
                name: step.name.clone(),
                projector: ContentProjector::new(max_parts),
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
        let body = self
            .content_payload(
                &mut pipeline,
                ContentInput::Json(body),
                None,
                &mut EventBudget::new(),
                lifecycle,
            )
            .await?
            .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "onContent"))?;
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
        if tail.is_some() {
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
    ) -> Result<Option<Value>, Failure> {
        let complete = matches!(&input, ContentInput::Json(_));
        let ending = matches!(&input, ContentInput::End);
        let frame = match input {
            ContentInput::Json(value) => projector.source.project_json(value),
            ContentInput::Chunk(value) => projector.source.project_chunk(value),
            ContentInput::End => projector.source.project_end(),
        }
        .map_err(content_failure)?;
        projector.capacity(None, &self.limits)?;
        let mut events = frame.events().to_vec();
        let mut rendered = None;
        for cursor in 0..projector.stages.len() {
            // The complete source batch is one step transaction. Neither state nor
            // observations escape if a later event or its rendered payload is invalid.
            let mut effects = None;
            let mut next = Vec::new();
            let mut candidate = projector.stages[cursor].clone();
            let step_name = candidate.name.clone();
            let native_deadline = self.hook_deadline(lifecycle);
            for event in events {
                let hook = match &mut candidate.executor {
                    ContentExecution::Native(native) => {
                        let produced = self.native_content_event(
                            native,
                            event,
                            lifecycle,
                            native_deadline,
                            &step_name,
                        )?;
                        charge(&produced, budget, &self.limits, &step_name)?;
                        next.extend(produced);
                        continue;
                    },
                    ContentExecution::Script(hook) => *hook,
                };
                let (state, observations) = effects.get_or_insert_with(|| {
                    (lifecycle.state.clone(), lifecycle.observations.clone())
                });
                let context = lifecycle.context_with_state(
                    HookInput::OnContent(EventInput {
                        event,
                        sse: super::events::sse_context(envelope),
                    }),
                    std::mem::take(state),
                );
                let output = self
                    .hook(hook, Some(&step_name), context, lifecycle)
                    .await?;
                let HookCandidate::OnContent { events: produced } = output.candidate else {
                    return Err(hook_invalid("onContent", &step_name));
                };
                charge(&produced, budget, &self.limits, &step_name)?;
                *state = output.effects.state;
                observations.commit(output.effects.observations, "onContent", Some(&step_name));
                next.extend(produced);
            }
            // Every step validates against its own previous output. Later steps
            // may remove or change a part without changing an earlier step's input.
            let value = render(&mut candidate.projector, frame.clone(), &next, ending)?;
            projector.capacity(Some((cursor, &candidate)), &self.limits)?;
            if let Some(value) = &value {
                self.validate_content_render(value, complete, envelope, &step_name)?;
            }
            if matches!(candidate.executor, ContentExecution::Native(_)) {
                self.native_active(lifecycle, native_deadline)?;
            } else {
                self.request_active(lifecycle, &lifecycle.cancellation)?;
            }
            if let Some((state, observations)) = effects {
                lifecycle.state = state;
                lifecycle.observations = observations;
            }
            events = next;
            rendered = value;
            projector.stages[cursor] = candidate;
        }
        Ok(rendered)
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

    fn native_content_event(
        &self,
        native: &mut NativeContentPolicy,
        event: Value,
        lifecycle: &mut Lifecycle,
        deadline: Instant,
        step: &str,
    ) -> Result<Vec<Value>, Failure> {
        lifecycle.phase = "onContent";
        lifecycle.step = Some(step.to_owned());
        let measurement = lifecycle.statistics.timings.native("onContent");
        let limits = NativeContentLimits {
            max_output_events: self.limits.max_output_events,
            max_output_bytes: self.limits.max_output_bytes,
            max_retained_bytes: self.limits.max_output_bytes,
            max_retained_records: self.limits.max_json_nodes,
        };
        let result = native
            .apply(event, &limits, &mut || {
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
        measurement.finish(&result);
        let produced = result?;
        // JS outputs already passed the VM bridge; native events need
        // their generic JSON boundary before shared semantic accounting.
        for event in &produced {
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
