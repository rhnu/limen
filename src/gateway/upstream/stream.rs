//! Keep semantic transformation and output routing separate from upstream framing.

use super::{
    Aggregator, ChunkTracker, ContentInput, Delivery, EventBudget, EventConsumer, Failure, Frame,
    Gateway, Lifecycle, RoutePlan, Value, content, protocol,
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
            let released = if let Some(pipeline) = &mut output.content {
                self.content_payload(
                    pipeline,
                    ContentInput::Chunk(event),
                    Some(frame),
                    &mut budget,
                    lifecycle,
                )
                .await?
            } else {
                vec![content::ContentOutput {
                    value: event,
                    envelope: Some(std::sync::Arc::new(frame.clone())),
                    budget: budget.clone(),
                }]
            };
            for mut released in released {
                if let Some(aggregate) = &mut output.aggregate {
                    aggregate
                        .push(released.value)
                        .map_err(|source| protocol(source, "sse_to_json"))?;
                } else {
                    self.send_stream_event(
                        route,
                        released.value,
                        true,
                        released.envelope.as_deref(),
                        &mut released.budget,
                        &mut output.tracker,
                        lifecycle,
                        delivery,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }
}
