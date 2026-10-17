//! Keep semantic transformation and output routing separate from upstream framing.

use super::{
    Aggregator, ChunkTracker, ContentInput, Delivery, ErrorCode, EventBudget, EventConsumer,
    Failure, Frame, Gateway, Lifecycle, Model, Provider, Value, content, protocol,
};

pub(super) struct SseOutput {
    pub content: Option<content::ContentPipeline>,
    pub aggregate: Option<Aggregator>,
    pub tracker: ChunkTracker,
}

impl Gateway {
    #[expect(
        clippy::too_many_arguments,
        reason = "Source envelope, routing, output state and request lifecycle have separate owners"
    )]
    pub(super) async fn content_stream_event(
        &self,
        provider: &Provider,
        model: &Model,
        event: Value,
        frame: &Frame,
        output: &mut SseOutput,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<(), Failure> {
        let mut budget = EventBudget::new();
        let events = self
            .event_hooks(
                provider,
                model,
                event,
                EventConsumer::Upstream { frame: Some(frame) },
                &mut budget,
                lifecycle,
            )
            .await?;
        for event in events {
            let event = if let Some(pipeline) = &mut output.content {
                self.content_payload(
                    provider,
                    model,
                    pipeline,
                    ContentInput::Chunk(event),
                    Some(frame),
                    &mut budget,
                    lifecycle,
                )
                .await?
                .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "onContent"))?
            } else {
                event
            };
            if let Some(aggregate) = &mut output.aggregate {
                aggregate
                    .push(event)
                    .map_err(|source| protocol(source, "sse_to_json"))?;
            } else {
                self.send_stream_event(
                    provider,
                    model,
                    event,
                    true,
                    Some(frame),
                    &mut budget,
                    &mut output.tracker,
                    lifecycle,
                    delivery,
                )
                .await?;
            }
        }
        Ok(())
    }
}
