use std::time::Instant;

use axum::{http::header, response::Response};
use limen_chat::{Aggregator, ChunkTracker, Decoder, Event, JsonChunks};
use limen_config::{Model, Provider, ResourceLimits, validate_json};
use serde_json::{Value, json};

use super::{
    Gateway,
    error::{ErrorCode, Failure, FailureResponse, validate_response_status},
    events::{EventBudget, EventConsumer},
    json,
    lifecycle::Lifecycle,
    logic,
    pipeline::{commit_effects, elapsed_ms, hook_invalid},
    response::{Delivery, apply_alias},
    stats::WaitKind,
};

impl Gateway {
    pub(super) async fn receive(
        &self,
        provider: &Provider,
        model: &Model,
        mut response: super::transport::Incoming,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<Option<Response>, Failure> {
        let original_status = response.status();
        lifecycle.upstream_status = Some(original_status);
        lifecycle.upstream_headers_ms = Some(elapsed_ms(lifecycle.started));
        lifecycle.statistics.timings.headers(Instant::now());
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
            return Err(self
                .receive_failure(status, &mut response, lifecycle)
                .await?);
        }
        if matches!(original_status, 204 | 205) || media == "other" {
            return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_media"));
        }
        let client_stream = lifecycle.client_stream.unwrap_or(false);
        lifecycle.statistics.source = Some(media);
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
                json::size(
                    &body,
                    self.server.limits.max_output_bytes,
                    ErrorCode::HookFailed,
                    "response",
                )?;
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
            let bytes = json::encode(
                &body,
                self.server.limits.max_output_bytes,
                ErrorCode::HookFailed,
                "response",
            )?;
            lifecycle.output_bytes = bytes.len();
            return Ok(Some(super::json_response(
                status,
                headers,
                bytes,
                &lifecycle.request_id,
            )));
        }
        Ok(None)
    }

    async fn receive_failure(
        &self,
        status: u16,
        response: &mut super::transport::Incoming,
        lifecycle: &mut Lifecycle,
    ) -> Result<Failure, Failure> {
        if !(300..600).contains(&response.status()) {
            return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_status"));
        }
        let mut error = Failure::new(ErrorCode::UpstreamFailed, "upstream_status");
        error.status = Some(status);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .filter(|_| !logic::connection_nominated(response.headers(), "content-type"))
            .cloned();
        let content_encoding =
            if logic::connection_nominated(response.headers(), "content-encoding") {
                Vec::new()
            } else {
                response
                    .headers()
                    .get_all(header::CONTENT_ENCODING)
                    .iter()
                    .cloned()
                    .collect()
            };
        let body = self.receive_bytes(response, lifecycle).await?;
        error.response = Some(Box::new(FailureResponse {
            body,
            content_type,
            content_encoding,
        }));
        Ok(error)
    }

    async fn receive_json(
        &self,
        response: &mut super::transport::Incoming,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        let bytes = self.receive_bytes(response, lifecycle).await?;
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
        lifecycle.statistics.usage.observe(body.get("usage"));
        lifecycle.statistics.upstream_completed = true;
        Ok(body)
    }

    async fn receive_bytes(
        &self,
        response: &mut super::transport::Incoming,
        lifecycle: &mut Lifecycle,
    ) -> Result<Vec<u8>, Failure> {
        let mut bytes = Vec::new();
        while let Some(chunk) = read_chunk(response, lifecycle).await? {
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
        Ok(bytes)
    }

    async fn receive_sse(
        &self,
        provider: &Provider,
        model: &Model,
        response: &mut super::transport::Incoming,
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
        'upstream: while let Some(chunk) = read_chunk(response, lifecycle).await? {
            lifecycle.upstream_bytes = lifecycle.upstream_bytes.saturating_add(chunk.len());
            for byte in chunk {
                let event = decoder
                    .push(byte)
                    .map_err(|source| protocol(source, "upstream_sse"))?;
                let Some(event) = event else {
                    continue;
                };
                match event {
                    // Stop at the terminator itself so trailing bytes cannot make
                    // success depend on how the network divided the response.
                    Event::Done => break 'upstream,
                    Event::Chunk(event) => {
                        upstream_tracker
                            .push(&event)
                            .map_err(|source| protocol(source, "upstream_sse"))?;
                        lifecycle.statistics.timings.first_chunk(Instant::now());
                        lifecycle.statistics.usage.observe(event.get("usage"));
                        let mut budget = EventBudget::new();
                        let events = self
                            .event_hooks(
                                provider,
                                model,
                                event,
                                EventConsumer::Upstream,
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
            lifecycle.phase = "upstream_receive";
            lifecycle.step = None;
        }
        decoder
            .finish()
            .map_err(|source| protocol(source, "upstream_sse"))?;
        upstream_tracker
            .finish()
            .map_err(|source| protocol(source, "upstream_sse"))?;
        lifecycle.statistics.upstream_completed = true;
        if let Some(aggregate) = aggregate {
            return finish_aggregate(aggregate, limits).map(Some);
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
        let mut frames = Vec::new();
        self.event_hooks(
            provider,
            model,
            event,
            EventConsumer::Stream {
                tracker,
                frames: &mut frames,
                max_event_bytes: self.server.limits.max_event_bytes,
            },
            budget,
            lifecycle,
        )
        .await?;
        // Hook expansion bounds the accumulated JSON payloads and event count;
        // encoding adds only the fixed framing bytes for each bounded event.
        for bytes in frames {
            lifecycle.output_bytes = lifecycle.output_bytes.saturating_add(bytes.len());
            lifecycle.phase = "response";
            lifecycle.step = None;
            delivery.emit(bytes).await?;
            lifecycle
                .statistics
                .timings
                .first_output(lifecycle.started, Instant::now());
        }
        Ok(())
    }

    pub(super) async fn after_receive(
        &self,
        provider: &Provider,
        model: &Model,
        response: &super::transport::Incoming,
        lifecycle: &mut Lifecycle,
    ) -> Result<(u16, http::HeaderMap), Failure> {
        let original_status = response.status();
        let mut status = original_status;
        let mut headers = logic::response_headers(response.headers());
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            if let Some(hook) = step.after_receive {
                let mut context = lifecycle.context("afterReceive");
                context["status"] = json!(status);
                context["headers"] = logic::headers_to_json(&headers)?;
                let mut output = self
                    .hook(hook, "afterReceive", Some(&step.name), context, lifecycle)
                    .await?;
                let next_status = output
                    .context
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .ok_or_else(|| hook_invalid("afterReceive", &step.name))?;
                validate_response_status(original_status, next_status).map_err(|source| {
                    hook_invalid("afterReceive", &step.name).with_source(StatusMapping(source))
                })?;
                let next_headers =
                    logic::headers_from_json(&output.context["headers"]).map_err(|source| {
                        hook_invalid("afterReceive", &step.name).with_source(source)
                    })?;
                commit_effects(lifecycle, &mut output)?;
                status = next_status;
                headers = next_headers;
            }
        }
        Ok((status, headers))
    }
}

async fn read_chunk(
    response: &mut super::transport::Incoming,
    lifecycle: &mut Lifecycle,
) -> Result<Option<axum::body::Bytes>, Failure> {
    let measurement = lifecycle.statistics.timings.measure(WaitKind::Read);
    let result = response.chunk().await;
    drop(measurement);
    if result
        .as_ref()
        .is_ok_and(|chunk| chunk.as_ref().is_some_and(|bytes| !bytes.is_empty()))
    {
        lifecycle.statistics.timings.first_byte(Instant::now());
    }
    result
}

/// Individual SSE events can fit the JSON limits while their combined choices or
/// tools exceed them. Establish the complete-response guarantee at aggregation.
fn finish_aggregate(aggregate: Aggregator, limits: &ResourceLimits) -> Result<Value, Failure> {
    let body = aggregate
        .finish()
        .map_err(|source| protocol(source, "sse_to_json"))?;
    validate_json(&body, limits.max_json_depth, limits.max_json_nodes).map_err(|source| {
        Failure::new(ErrorCode::UpstreamProtocol, "sse_to_json").with_source(source)
    })?;
    Ok(body)
}

#[derive(Debug, thiserror::Error)]
#[error("downstream status violates the upstream status class: {0:?}")]
struct StatusMapping(ErrorCode);

pub(crate) fn media_kind(media: &str) -> &'static str {
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

#[cfg(test)]
mod tests;
