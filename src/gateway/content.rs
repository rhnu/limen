//! One semantic transformation stage before either aggregation or delivery.

use std::{collections::BTreeMap, time::Instant};

use limen_chat::{
    ContentFrame, ContentProjector, Frame, NativeContentError, NativeContentLimits,
    NativeContentPolicy, validate_content_event,
};
use limen_config::{BusinessConfig, Model, Provider, ResourceLimits, validate_json};
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
    stages: Vec<ContentStage>,
}

#[derive(Clone)]
struct ContentStage {
    projector: ContentProjector,
    native: Option<NativeContentPolicy>,
}

/// Immutable policy templates are compiled once; aliases keep separate model occurrences.
#[derive(Clone, Default)]
pub(crate) struct NativeContentTemplates {
    providers: BTreeMap<String, BTreeMap<String, NativeContentPolicy>>,
    models: BTreeMap<String, BTreeMap<String, NativeContentPolicy>>,
}

impl NativeContentTemplates {
    pub(crate) fn compile(
        config: &BusinessConfig,
        limits: &ResourceLimits,
        deadline: Instant,
    ) -> Result<Self, NativeContentError> {
        let compile = |steps: &[limen_config::PipelineStep]| {
            let mut policies = BTreeMap::new();
            for step in steps {
                if Instant::now() >= deadline {
                    return Err(NativeContentError::Interrupted);
                }
                if let Some(policy) = &step.builtin {
                    let limits = NativeContentLimits {
                        max_output_events: limits.max_output_events,
                        max_output_bytes: limits.max_output_bytes,
                        max_retained_bytes: limits.max_output_bytes,
                        max_retained_records: limits.max_json_nodes,
                    };
                    if let Some(policy) =
                        NativeContentPolicy::compile(policy, &limits, &mut || {
                            Instant::now() < deadline
                        })?
                    {
                        policies.insert(step.name.clone(), policy);
                    }
                }
            }
            Ok(policies)
        };
        let templates = Self {
            providers: config
                .providers
                .iter()
                .map(|(name, provider)| Ok((name.clone(), compile(&provider.pipeline)?)))
                .collect::<Result<_, NativeContentError>>()?,
            models: config
                .models
                .iter()
                .map(|(name, model)| Ok((name.clone(), compile(&model.pipeline)?)))
                .collect::<Result<_, NativeContentError>>()?,
        };
        if Instant::now() >= deadline {
            Err(NativeContentError::Interrupted)
        } else {
            Ok(templates)
        }
    }
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
            if let Some(native) = &projector.native {
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
    pub(super) fn content_projector(
        &self,
        provider: &Provider,
        model: &Model,
        lifecycle: &Lifecycle,
    ) -> Result<Option<ContentPipeline>, Failure> {
        let max_parts = self.server.limits.max_json_nodes;
        let provider_templates = lifecycle
            .provider
            .as_ref()
            .and_then(|name| self.native_content.providers.get(name));
        let model_templates = lifecycle
            .alias
            .as_ref()
            .and_then(|name| self.native_content.models.get(name));
        let stages: Vec<_> = provider
            .pipeline
            .iter()
            .map(|step| (step, provider_templates))
            .chain(model.pipeline.iter().map(|step| (step, model_templates)))
            .filter(|(step, _)| {
                step.on_content.is_some()
                    || step
                        .builtin
                        .as_ref()
                        .is_some_and(limen_config::BuiltinPolicy::has_content)
            })
            .map(|(step, templates)| {
                let native = if step.builtin.is_some() {
                    Some(
                        templates
                            .and_then(|templates| templates.get(&step.name))
                            .cloned()
                            .ok_or_else(|| Failure::new(ErrorCode::Internal, "onContent"))?,
                    )
                } else {
                    None
                };
                Ok(ContentStage {
                    projector: ContentProjector::new(max_parts),
                    native,
                })
            })
            .collect::<Result<_, Failure>>()?;
        Ok((!stages.is_empty()).then(|| ContentPipeline {
            source: ContentProjector::new(max_parts),
            stages,
        }))
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
        provider: &Provider,
        model: &Model,
        body: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        let Some(mut pipeline) = self.content_projector(provider, model, lifecycle)? else {
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
            if step.on_content.is_none()
                && !step
                    .builtin
                    .as_ref()
                    .is_some_and(limen_config::BuiltinPolicy::has_content)
            {
                continue;
            }
            // The complete source batch is one step transaction. Neither state nor
            // observations escape if a later event or its rendered payload is invalid.
            let mut effects = None;
            let mut next = Vec::new();
            let mut candidate = projector.stages[cursor].clone();
            let native_deadline = self.hook_deadline(lifecycle);
            for event in events {
                if let Some(native) = &mut candidate.native {
                    let produced = self.native_content_event(
                        native,
                        event,
                        lifecycle,
                        native_deadline,
                        &step.name,
                    )?;
                    charge(&produced, budget, &self.server.limits, &step.name)?;
                    next.extend(produced);
                    continue;
                }
                let (state, observations) = effects.get_or_insert_with(|| {
                    (lifecycle.state.clone(), lifecycle.observations.clone())
                });
                let mut context = lifecycle.context_with_state("onContent", state.take());
                context["event"] = event;
                context["sse"] = envelope.map_or(
                    Value::Null,
                    |frame| json!({"event":frame.event,"id":frame.id,"retry":frame.retry}),
                );
                let mut output = self
                    .hook(
                        step.on_content
                            .ok_or_else(|| hook_invalid("onContent", &step.name))?,
                        "onContent",
                        Some(&step.name),
                        context,
                        lifecycle,
                    )
                    .await?;
                let produced = output
                    .context
                    .get("events")
                    .and_then(Value::as_array)
                    .ok_or_else(|| hook_invalid("onContent", &step.name))?;
                charge(produced, budget, &self.server.limits, &step.name)?;
                *state = output
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
            let value = render(&mut candidate.projector, frame.clone(), &next, ending)?;
            projector.capacity(Some((cursor, &candidate)), &self.server.limits)?;
            if let Some(value) = &value {
                self.validate_content_render(value, complete, envelope, &step.name)?;
            }
            if candidate.native.is_some() {
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
            cursor += 1;
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
            self.server.limits.max_json_depth,
            self.server.limits.max_json_nodes,
        )
        .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
        json::size(
            value,
            self.server.limits.max_output_bytes,
            ErrorCode::HookFailed,
            "onContent",
        )?;
        if !complete {
            limen_chat::validate_chunk(value)
                .map_err(|source| hook_invalid("onContent", step).with_source(source))?;
            if let Some(envelope) = envelope {
                envelope
                    .encode(value, self.server.limits.max_event_bytes)
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
            max_output_events: self.server.limits.max_output_events,
            max_output_bytes: self.server.limits.max_output_bytes,
            max_retained_bytes: self.server.limits.max_output_bytes,
            max_retained_records: self.server.limits.max_json_nodes,
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
                self.server.limits.max_json_depth,
                self.server.limits.max_json_nodes,
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
