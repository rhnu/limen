use limen_script::{EventInput, HookCandidate, HookInput};

use super::{
    ContentBatch, ContentExecution, ContentPipeline, Failure, Gateway, Lifecycle, Value, charge,
    hook_invalid, render,
};

impl Gateway {
    /// Pending native input and all newly released source frames form one
    /// candidate. A render failure cannot publish only part of that release.
    pub(super) async fn content_stage(
        &self,
        pipeline: &mut ContentPipeline,
        cursor: usize,
        mut batch: ContentBatch,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<(ContentBatch, Option<Value>)>, Failure> {
        let mut candidate = pipeline.stages[cursor].clone();
        let step = candidate.name.clone();
        lifecycle.scope = candidate.scope;
        let deadline = self.hook_deadline(lifecycle);
        let mut released = Vec::new();
        let mut effects = None;
        match &mut candidate.executor {
            ContentExecution::Native(native) => {
                let events = std::mem::take(&mut batch.events);
                candidate.pending.push_back(batch);
                let outputs =
                    self.native_content_batch(native, events, lifecycle, deadline, &step)?;
                for events in outputs {
                    let mut batch = candidate
                        .pending
                        .pop_front()
                        .ok_or_else(|| hook_invalid("onContent", &step))?;
                    charge(&events, &mut batch.budget, &self.limits, &step)?;
                    batch.events = events;
                    released.push(batch);
                }
            },
            ContentExecution::Script(hook) => {
                let mut next = Vec::new();
                for event in std::mem::take(&mut batch.events) {
                    let (state, observations) = effects.get_or_insert_with(|| {
                        (lifecycle.state.clone(), lifecycle.observations.clone())
                    });
                    let context = lifecycle.context_with_state(
                        HookInput::OnContent(EventInput {
                            event,
                            sse: super::super::events::sse_context(batch.envelope.as_deref()),
                        }),
                        std::mem::take(state),
                    );
                    let output = self.hook(*hook, Some(&step), context, lifecycle).await?;
                    let HookCandidate::OnContent { events } = output.candidate else {
                        return Err(hook_invalid("onContent", &step));
                    };
                    charge(&events, &mut batch.budget, &self.limits, &step)?;
                    *state = output.effects.state;
                    observations.commit(output.effects.observations, "onContent", Some(&step));
                    next.extend(events);
                }
                batch.events = next;
                released.push(batch);
            },
        }
        pipeline.capacity(Some((cursor, &candidate)), &self.limits)?;
        let mut output = Vec::with_capacity(released.len());
        for batch in released {
            let value = render(
                &mut candidate.projector,
                (*batch.frame).clone(),
                &batch.events,
                batch.ending,
            )?;
            if let Some(value) = &value {
                self.validate_content_render(
                    value,
                    batch.complete,
                    batch.envelope.as_deref(),
                    &step,
                )?;
            }
            output.push((batch, value));
        }
        pipeline.capacity(Some((cursor, &candidate)), &self.limits)?;
        if matches!(candidate.executor, ContentExecution::Native(_)) {
            self.native_active(lifecycle, deadline)?;
        } else {
            self.request_active(lifecycle, &lifecycle.cancellation)?;
        }
        if let Some((state, observations)) = effects {
            lifecycle.state = state;
            lifecycle.observations = observations;
        }
        pipeline.stages[cursor] = candidate;
        Ok(output)
    }
}
