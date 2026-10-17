//! Keep semantic transformation and output routing separate from upstream framing.

use super::{
    Aggregator, ChunkTracker, ContentInput, Delivery, ErrorCode, EventBudget, EventConsumer,
    Failure, Frame, Gateway, Lifecycle, RoutePlan, Value, content, protocol,
};

pub(super) struct SseOutput {
    pub content: Option<content::ContentPipeline>,
    pub aggregate: Option<Aggregator>,
    pub tracker: ChunkTracker,
}

impl Gateway {
    pub(super) async fn content_stream_event(
        &self,
        route: &RoutePlan<'_>,
        event: Value,
        frame: &Frame,
        output: &mut SseOutput,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<(), Failure> {
        let mut budget = EventBudget::new();
        let events = self
            .event_hooks(
                route,
                event,
                EventConsumer::Upstream { frame: Some(frame) },
                &mut budget,
                lifecycle,
            )
            .await?;
        for event in events {
            let event = if let Some(pipeline) = &mut output.content {
                self.content_payload(
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
                    route,
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
