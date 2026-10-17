//! One semantic transformation stage before either aggregation or delivery.

use limen_chat::{ContentFrame, ContentProjector, Frame, validate_content_event};
use limen_config::{Model, Provider, ResourceLimits, validate_json};
use serde_json::{Value, json};

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    events::EventBudget,
    json,
    lifecycle::Lifecycle,
    pipeline::hook_invalid,
};

pub(super) struct ContentPipeline {
    source: ContentProjector,
    stages: Vec<ContentProjector>,
}

impl ContentPipeline {
    fn capacity(
        &self,
        candidate: Option<(usize, &ContentProjector)>,
        limits: &ResourceLimits,
    ) -> Result<(), Failure> {
        let mut parts = self.source.retained_parts();
        let mut bytes = self.source.retained_bytes();
        for (index, projector) in self.stages.iter().enumerate() {
            let projector = candidate
                .filter(|(stage, _)| *stage == index)
                .map_or(projector, |(_, value)| value);
            parts = parts.saturating_add(projector.retained_parts());
            bytes = bytes.saturating_add(projector.retained_bytes());
        }
        if parts > limits.max_json_nodes || bytes > limits.max_output_bytes {
            Err(Failure::new(ErrorCode::HookFailed, "onContent"))
        } else {
            Ok(())
        }
    }
}

pub(super) fn projector(
    provider: &Provider,
    model: &Model,
    max_parts: usize,
) -> Option<ContentPipeline> {
    let count = provider
        .pipeline
        .iter()
        .chain(&model.pipeline)
        .filter(|step| step.on_content.is_some())
        .count();
    (count > 0).then(|| ContentPipeline {
        source: ContentProjector::new(max_parts),
        stages: vec![ContentProjector::new(max_parts); count],
    })
}

pub(super) enum ContentInput {
    Json(Value),
    Chunk(Value),
    End,
}

impl Gateway {
    pub(super) async fn content_json(
        &self,
        provider: &Provider,
        model: &Model,
        body: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        let Some(mut pipeline) = projector(provider, model, self.server.limits.max_json_nodes)
        else {
            return Ok(body);
        };
        let body = self
            .content_payload(
                provider,
                model,
                &mut pipeline,
                ContentInput::Json(body),
                None,
                &mut EventBudget::new(),
                lifecycle,
            )
            .await?
            .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "onContent"))?;
        self.content_end(provider, model, &mut pipeline, lifecycle)
            .await?;
        Ok(body)
    }

    pub(super) async fn content_end(
        &self,
        provider: &Provider,
        model: &Model,
        pipeline: &mut ContentPipeline,
        lifecycle: &mut Lifecycle,
    ) -> Result<(), Failure> {
        let tail = self
            .content_payload(
                provider,
                model,
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

    #[expect(
        clippy::too_many_arguments,
        reason = "Content transformation shares routing, source envelope and request budgets"
    )]
    pub(super) async fn content_payload(
        &self,
        provider: &Provider,
        model: &Model,
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
        projector.capacity(None, &self.server.limits)?;
        let mut events = frame.events().to_vec();
        let mut rendered = None;
        let mut cursor = 0;
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            let Some(hook) = step.on_content else {
                continue;
            };
            // The complete source batch is one step transaction. Neither state nor
            // observations escape if a later event or its rendered payload is invalid.
            let mut state = lifecycle.state.clone();
            let mut observations = lifecycle.observations.clone();
            let mut next = Vec::new();
            for event in events {
                let mut context = lifecycle.context("onContent");
                context["state"] = state;
                context["event"] = event;
                context["sse"] = envelope.map_or(
                    Value::Null,
                    |frame| json!({"event":frame.event,"id":frame.id,"retry":frame.retry}),
                );
                let mut output = self
                    .hook(hook, "onContent", Some(&step.name), context, lifecycle)
                    .await?;
                let produced = output
                    .context
                    .get("events")
                    .and_then(Value::as_array)
                    .ok_or_else(|| hook_invalid("onContent", &step.name))?;
                let bytes = produced.iter().try_fold(0_usize, |sum, event| {
                    validate_content_event(event).map_err(content_failure)?;
                    let size = json::size(
                        event,
                        self.server.limits.max_output_bytes,
                        ErrorCode::HookFailed,
                        "onContent",
                    )?;
                    sum.checked_add(size)
                        .ok_or_else(|| hook_invalid("onContent", &step.name))
                })?;
                budget.charge(produced.len(), bytes, &self.server.limits, "onContent")?;
                state = output
                    .context
                    .get_mut("state")
                    .filter(|value| value.is_object())
                    .ok_or_else(|| hook_invalid("onContent", &step.name))?
                    .take();
                observations.commit(output.observations, "onContent", Some(&step.name));
                let Value::Array(produced) = output.context["events"].take() else {
                    return Err(hook_invalid("onContent", &step.name));
                };
                next.extend(produced);
            }
            // Every step validates against its own previous output. Later steps
            // may remove or change a part without changing an earlier step's input.
            let mut candidate = projector.stages[cursor].clone();
            let value = render(&mut candidate, frame.clone(), next.clone(), ending)?;
            projector.capacity(Some((cursor, &candidate)), &self.server.limits)?;
            if let Some(value) = &value {
                validate_json(
                    value,
                    self.server.limits.max_json_depth,
                    self.server.limits.max_json_nodes,
                )
                .map_err(|source| hook_invalid("onContent", &step.name).with_source(source))?;
                json::size(
                    value,
                    self.server.limits.max_output_bytes,
                    ErrorCode::HookFailed,
                    "onContent",
                )?;
                if !complete {
                    limen_chat::validate_chunk(value).map_err(|source| {
                        hook_invalid("onContent", &step.name).with_source(source)
                    })?;
                    // Check preserved outer SSE fields before committing semantic edits.
                    if let Some(envelope) = envelope {
                        envelope
                            .encode(value, self.server.limits.max_event_bytes)
                            .map_err(|source| {
                                hook_invalid("onContent", &step.name).with_source(source)
                            })?;
                    }
                }
            }
            self.request_active(lifecycle, &lifecycle.cancellation)?;
            lifecycle.state = state;
            lifecycle.observations = observations;
            events = next;
            rendered = value;
            projector.stages[cursor] = candidate;
            cursor += 1;
        }
        Ok(rendered)
    }
}

fn render(
    projector: &mut ContentProjector,
    frame: ContentFrame,
    events: Vec<Value>,
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
