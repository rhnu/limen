use axum::{http::header, response::Response};
use limen_chat::{Aggregator, ChunkTracker, Decoder, Event, JsonChunks};
use limen_config::{Model, Provider, validate_json};
use serde_json::{Value, json};

use super::{
    Gateway,
    error::{ErrorCode, Failure, validate_response_status},
    events::EventBudget,
    lifecycle::Lifecycle,
    logic,
    pipeline::{commit_state, elapsed_ms, hook_invalid},
    response::{Delivery, apply_alias},
};

impl Gateway {
    pub(super) async fn receive(
        &self,
        provider: &Provider,
        model: &Model,
        mut response: reqwest::Response,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<Option<Response>, Failure> {
        let original_status = response.status().as_u16();
        lifecycle.upstream_status = Some(original_status);
        lifecycle.upstream_headers_ms = Some(elapsed_ms(lifecycle.started));
        // Parser selection always uses the frozen upstream facts, never hook drafts.
        let media = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map_or("other", media_kind);
        lifecycle.upstream_media = Some(media.to_owned());
        let (status, headers) = self
            .after_receive(provider, model, &response, lifecycle)
            .await?;
        lifecycle.phase = "upstream_receive";
        lifecycle.step = None;
        lifecycle.response_headers = headers.clone();
        if !(200..300).contains(&original_status) {
            let mut error = Failure::new(ErrorCode::UpstreamFailed, "upstream_status");
            error.status = Some(status);
            return Err(error);
        }
        if matches!(original_status, 204 | 205) || media == "other" {
            return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_media"));
        }
        let client_stream = lifecycle.client_stream.unwrap_or(false);
        delivery.configure(status, headers.clone());
        let body = if media == "sse" {
            self.receive_sse(
                provider,
                model,
                &mut response,
                lifecycle,
                delivery,
                client_stream,
            )
            .await?
        } else {
            Some(self.receive_json(&mut response, lifecycle).await?)
        };
        if let Some(body) = body {
            let body = self.response_body(provider, model, body, lifecycle).await?;
            if client_stream {
                let chunks = JsonChunks::new(body, self.server.limits.max_event_bytes)
                    .map_err(|source| protocol(source, "json_to_sse"))?;
                let mut tracker = ChunkTracker::new(self.server.limits.max_json_nodes);
                for event in chunks {
                    let event = event.map_err(|source| protocol(source, "json_to_sse"))?;
                    let mut budget = EventBudget::new();
                    self.send_stream_event(
                        provider,
                        model,
                        event,
                        false,
                        &mut budget,
                        &mut tracker,
                        lifecycle,
                        delivery,
                    )
                    .await?;
                }
                tracker
                    .finish()
                    .map_err(|source| hook_protocol(source, "onStreamEvent"))?;
                lifecycle.phase = "response";
                lifecycle.step = None;
                delivery.finish().await?;
                return Ok(None);
            }
            lifecycle.phase = "response";
            lifecycle.step = None;
            lifecycle.output_bytes = body.to_string().len();
            return Ok(Some(super::json_response(
                status,
                headers,
                &body,
                &lifecycle.request_id,
            )));
        }
        Ok(None)
    }

    async fn receive_json(
        &self,
        response: &mut reqwest::Response,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| Failure::upstream(source, "upstream_receive"))?
        {
            if chunk.len()
                > self
                    .server
                    .limits
                    .max_output_bytes
                    .saturating_sub(bytes.len())
            {
                return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_body"));
            }
            bytes.extend_from_slice(&chunk);
            lifecycle.upstream_bytes = bytes.len();
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|source| {
            Failure::new(ErrorCode::UpstreamProtocol, "upstream_body").with_source(source)
        })?;
        validate_json(
            &body,
            self.server.limits.max_json_depth,
            self.server.limits.max_json_nodes,
        )
        .map_err(|source| {
            Failure::new(ErrorCode::UpstreamProtocol, "upstream_body").with_source(source)
        })?;
        if !body.is_object() {
            return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_body"));
        }
        Ok(body)
    }

    async fn receive_sse(
        &self,
        provider: &Provider,
        model: &Model,
        response: &mut reqwest::Response,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
        client_stream: bool,
    ) -> Result<Option<Value>, Failure> {
        let limits = &self.server.limits;
        let mut decoder = Decoder::new(limits.max_event_bytes);
        // Tracking is bounded independently of aggregation event count: one event
        // may legally contain multiple choices, and passthrough never aggregates.
        let mut upstream_tracker = ChunkTracker::new(limits.max_json_nodes);
        let mut downstream_tracker = ChunkTracker::new(limits.max_json_nodes);
        let mut aggregate = (!client_stream)
            .then(|| Aggregator::new(limits.max_aggregate_bytes, limits.max_aggregate_events));
        let mut done = false;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| Failure::upstream(source, "upstream_receive"))?
        {
            lifecycle.upstream_bytes = lifecycle.upstream_bytes.saturating_add(chunk.len());
            for byte in chunk {
                let event = decoder
                    .push(byte)
                    .map_err(|source| protocol(source, "upstream_sse"))?;
                let Some(event) = event else {
                    continue;
                };
                match event {
                    Event::Done => {
                        done = true;
                    },
                    Event::Chunk(event) => {
                        upstream_tracker
                            .push(&event)
                            .map_err(|source| protocol(source, "upstream_sse"))?;
                        let mut budget = EventBudget::new();
                        let events = self
                            .event_hooks(
                                provider,
                                model,
                                event,
                                "onUpstreamEvent",
                                &mut budget,
                                lifecycle,
                            )
                            .await?;
                        for event in events {
                            if let Some(aggregate) = &mut aggregate {
                                aggregate
                                    .push(event)
                                    .map_err(|source| protocol(source, "sse_to_json"))?;
                            } else {
                                self.send_stream_event(
                                    provider,
                                    model,
                                    event,
                                    true,
                                    &mut budget,
                                    &mut downstream_tracker,
                                    lifecycle,
                                    delivery,
                                )
                                .await?;
                            }
                        }
                    },
                }
            }
            // DONE is a protocol terminator; do not await an upstream socket EOF.
            if done {
                break;
            }
            lifecycle.phase = "upstream_receive";
            lifecycle.step = None;
        }
        decoder
            .finish()
            .map_err(|source| protocol(source, "upstream_sse"))?;
        upstream_tracker
            .finish()
            .map_err(|source| protocol(source, "upstream_sse"))?;
        if let Some(aggregate) = aggregate {
            return aggregate
                .finish()
                .map(Some)
                .map_err(|source| protocol(source, "sse_to_json"));
        }
        downstream_tracker
            .finish()
            .map_err(|source| hook_protocol(source, "onStreamEvent"))?;
        lifecycle.phase = "response";
        lifecycle.step = None;
        delivery.finish().await?;
        Ok(None)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "A stream event borrows both semantic trackers and the shared request lifetime"
    )]
    async fn send_stream_event(
        &self,
        provider: &Provider,
        model: &Model,
        mut event: Value,
        alias: bool,
        budget: &mut EventBudget,
        tracker: &mut ChunkTracker,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<(), Failure> {
        if alias {
            apply_alias(&mut event, lifecycle);
        }
        let events = self
            .event_hooks(provider, model, event, "onStreamEvent", budget, lifecycle)
            .await?;
        for event in events {
            tracker
                .push(&event)
                .map_err(|source| hook_protocol(source, "onStreamEvent"))?;
            let bytes = limen_chat::encode_chunk(&event, self.server.limits.max_event_bytes)
                .map_err(|source| hook_protocol(source, "onStreamEvent"))?;
            lifecycle.output_bytes = lifecycle.output_bytes.saturating_add(bytes.len());
            lifecycle.phase = "response";
            lifecycle.step = None;
            delivery.emit(bytes).await?;
        }
        Ok(())
    }

    pub(super) async fn after_receive(
        &self,
        provider: &Provider,
        model: &Model,
        response: &reqwest::Response,
        lifecycle: &mut Lifecycle,
    ) -> Result<(u16, http::HeaderMap), Failure> {
        let original_status = response.status().as_u16();
        let mut status = if (300..600).contains(&original_status) {
            502
        } else {
            original_status
        };
        let mut headers = logic::response_headers(response.headers());
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            if let Some(hook) = step.after_receive {
                let mut context = lifecycle.context("afterReceive");
                context["status"] = json!(status);
                context["headers"] = logic::headers_to_json(&headers)?;
                let output = self
                    .hook(hook, "afterReceive", Some(&step.name), context, lifecycle)
                    .await?;
                let next_status = output
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .ok_or_else(|| hook_invalid("afterReceive", &step.name))?;
                validate_response_status(original_status, next_status).map_err(|source| {
                    hook_invalid("afterReceive", &step.name).with_source(StatusMapping(source))
                })?;
                let next_headers =
                    logic::headers_from_json(&output["headers"]).map_err(|source| {
                        hook_invalid("afterReceive", &step.name).with_source(source)
                    })?;
                commit_state(lifecycle, &output)?;
                status = next_status;
                headers = next_headers;
            }
        }
        Ok((status, headers))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("downstream status violates the upstream status class: {0:?}")]
struct StatusMapping(ErrorCode);

pub(super) fn media_kind(media: &str) -> &'static str {
    if is_json_media(media) {
        "json"
    } else if media
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        "sse"
    } else {
        "other"
    }
}

pub(super) fn is_json_media(media: &str) -> bool {
    media
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn protocol(source: limen_chat::Error, phase: &'static str) -> Failure {
    Failure::new(ErrorCode::UpstreamProtocol, phase).with_source(source)
}

fn hook_protocol(source: limen_chat::Error, phase: &'static str) -> Failure {
    Failure::new(ErrorCode::HookFailed, phase).with_source(source)
}
