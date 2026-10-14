use std::time::{Duration, Instant};

use axum::{
    body::to_bytes,
    http::{Method, Request, header},
    response::Response,
};
use limen_config::{ErrorCode, Model, Provider, validate_json, validate_response_status};
use limen_script::{Cancellation, HookCall, HookId, RuntimeStatus};
use serde_json::{Value, json};

use super::{Gateway, error::Failure, json_response, logic};

/// Mutable data is owned by one request. Hook snapshots enter the VM by value.
pub(super) struct Lifecycle {
    pub request_id: String,
    pub cancellation: Cancellation,
    pub deadline: Instant,
    pub phase: &'static str,
    step: Option<String>,
    started: Instant,
    state: Value,
    client_stream: Option<bool>,
    alias: Option<String>,
    provider: Option<String>,
    configured_model: Option<String>,
    final_model: Option<String>,
    target: Option<String>,
    upstream_status: Option<u16>,
    upstream_media: Option<String>,
    input_bytes: usize,
    outbound_bytes: usize,
    upstream_bytes: usize,
    upstream_headers_ms: Option<u64>,
    output_bytes: usize,
    pub response_headers: http::HeaderMap,
}

impl Lifecycle {
    pub fn new(request_id: String, cancellation: Cancellation, request_ms: u64) -> Self {
        let started = Instant::now();
        Self {
            request_id,
            cancellation,
            deadline: started + Duration::from_millis(request_ms),
            phase: "admission",
            step: None,
            started,
            state: json!({}),
            client_stream: None,
            alias: None,
            provider: None,
            configured_model: None,
            final_model: None,
            target: None,
            upstream_status: None,
            upstream_media: None,
            input_bytes: 0,
            outbound_bytes: 0,
            upstream_bytes: 0,
            upstream_headers_ms: None,
            output_bytes: 0,
            response_headers: http::HeaderMap::new(),
        }
    }

    fn meta(&self, phase: &'static str) -> Value {
        json!({"requestId":self.request_id,"phase":phase,"model":self.alias,"provider":self.provider,"configuredUpstreamModel":self.configured_model,"clientStream":self.client_stream,"upstreamStatus":self.upstream_status,"upstreamContentType":self.upstream_media})
    }

    fn context(&self, phase: &'static str) -> Value {
        json!({"meta":self.meta(phase),"state":self.state})
    }

    pub fn record(&self, result: &Result<Response, Failure>) {
        let outcome = match result {
            Ok(_) => "success",
            Err(e) if e.code == ErrorCode::Cancelled => "cancelled",
            Err(_) => "failed",
        };
        let (code, phase, step) =
            result
                .as_ref()
                .err()
                .map_or((None, self.phase, self.step.as_deref()), |e| {
                    (
                        Some(e.code.as_str()),
                        e.phase,
                        e.step.as_deref().or(self.step.as_deref()),
                    )
                });
        tracing::info!(
            request_id = self.request_id,
            outcome,
            code,
            phase,
            step,
            model = self.alias,
            provider = self.provider,
            configured_model = self.configured_model,
            final_model = self.final_model,
            target = self.target,
            upstream_status = self.upstream_status,
            upstream_media = self.upstream_media,
            client_stream = self.client_stream,
            input_bytes = self.input_bytes,
            outbound_bytes = self.outbound_bytes,
            upstream_bytes = self.upstream_bytes,
            upstream_headers_ms = self.upstream_headers_ms,
            output_bytes = self.output_bytes,
            elapsed_ms = self.started.elapsed().as_millis(),
            "request finished"
        );
        if let Err(failure) = result {
            if matches!(
                failure.code,
                ErrorCode::InvalidRequest
                    | ErrorCode::Unauthorized
                    | ErrorCode::UnknownModel
                    | ErrorCode::PayloadTooLarge
                    | ErrorCode::Busy
                    | ErrorCode::Cancelled
            ) {
                tracing::warn!(phase, step, error = %failure, "request rejected or cancelled");
            } else {
                tracing::error!(phase, step, error = %failure, "request failed");
            }
        }
    }
}

impl Gateway {
    pub(super) async fn process(
        &self,
        request: Request<axum::body::Body>,
        lifecycle: &mut Lifecycle,
    ) -> Result<Response, Failure> {
        if !self.ready() {
            return Err(Failure::new(ErrorCode::Unavailable, "admission"));
        }
        lifecycle.phase = "authentication";
        logic::authorize(&self.server.client_auth, request.headers())?;
        let models = request.method() == Method::GET && request.uri().path() == "/v1/models";
        let completion =
            request.method() == Method::POST && request.uri().path() == "/v1/chat/completions";
        if !models && !completion {
            return Err(Failure::new(ErrorCode::InvalidRequest, "endpoint"));
        }
        if completion
            && !request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .is_some_and(is_json_media)
        {
            return Err(Failure::new(ErrorCode::InvalidRequest, "request_body"));
        }
        lifecycle.phase = "request_body";
        let bytes = to_bytes(request.into_body(), self.server.limits.max_body_bytes)
            .await
            .map_err(Failure::body)?;
        lifecycle.input_bytes = bytes.len();
        if models {
            let body = logic::models_json(&self.business);
            lifecycle.output_bytes = body.to_string().len();
            if lifecycle.output_bytes > self.server.limits.max_output_bytes {
                return Err(Failure::new(ErrorCode::PayloadTooLarge, "models_response"));
            }
            return Ok(json_response(
                200,
                http::HeaderMap::new(),
                &body,
                &lifecycle.request_id,
            ));
        }
        let (body, stream) = logic::parse_request(&bytes, &self.server.limits)?;
        lifecycle.client_stream = Some(stream);
        self.complete(body, stream, lifecycle).await
    }

    async fn complete(
        &self,
        mut body: Value,
        stream: bool,
        lifecycle: &mut Lifecycle,
    ) -> Result<Response, Failure> {
        if let Some(hook) = self.business.on_request {
            let mut context = lifecycle.context("onRequest");
            context["body"] = body;
            let output = self
                .hook(hook, "onRequest", None, context, lifecycle)
                .await?;
            body = require_body(&output)?;
            logic::validate_body(&body, &self.server.limits).map_err(|source| {
                Failure::new(ErrorCode::HookFailed, "onRequest").with_source(source)
            })?;
            commit_state(lifecycle, &output)?;
        }
        lifecycle.phase = "routing";
        let alias = body
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| Failure::new(ErrorCode::InvalidRequest, "routing"))?;
        let model = self
            .business
            .models
            .get(alias)
            .ok_or_else(|| Failure::new(ErrorCode::UnknownModel, "routing"))?;
        let provider = self
            .business
            .providers
            .get(&model.provider)
            .ok_or_else(|| Failure::new(ErrorCode::Internal, "routing"))?;
        lifecycle.alias = Some(alias.to_owned());
        lifecycle.provider = Some(model.provider.clone());
        lifecycle.configured_model = Some(model.upstream.clone());
        let body = logic::merge_body(provider, model, &body);
        let (body, url, mut headers) = self.before_send(provider, model, body, lifecycle).await?;
        lifecycle.phase = "beforeSend";
        lifecycle.step = None;
        logic::validate_body(&body, &self.server.limits)?;
        logic::validate_url(&url)?;
        lifecycle.outbound_bytes = body.to_string().len();
        if lifecycle.outbound_bytes > self.server.limits.max_output_bytes {
            return Err(Failure::new(ErrorCode::PayloadTooLarge, "upstream_body"));
        }
        if stream || body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
            return Err(Failure::new(ErrorCode::InvalidRequest, "transport"));
        }
        lifecycle.final_model = body.get("model").and_then(Value::as_str).map(str::to_owned);
        lifecycle.target = Some(logic::safe_target(&url));
        lifecycle.phase = "upstream_send";
        headers.insert(
            header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        let response = self
            .client_for(&url)?
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|source| Failure::upstream(source, "upstream_send"))?;
        self.receive(provider, model, response, lifecycle).await
    }

    async fn before_send(
        &self,
        provider: &Provider,
        model: &Model,
        mut body: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<(Value, String, http::HeaderMap), Failure> {
        let mut url = provider.endpoint.clone();
        let mut headers = logic::outbound_headers(provider)?;
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            if let Some(hook) = step.before_send {
                let mut context = lifecycle.context("beforeSend");
                context["body"] = body.clone();
                context["url"] = json!(url);
                context["headers"] = logic::headers_to_json(&headers)?;
                let output = self
                    .hook(hook, "beforeSend", Some(&step.name), context, lifecycle)
                    .await?;
                let next_body = require_body(&output)?;
                logic::validate_body(&next_body, &self.server.limits)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                let next_url = output
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| hook_invalid("beforeSend", &step.name))?;
                logic::validate_url(next_url)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                let next_headers = logic::headers_from_json(&output["headers"])
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                commit_state(lifecycle, &output)?;
                body = next_body;
                url = next_url.to_owned();
                headers = next_headers;
            }
        }
        Ok((body, url, headers))
    }

    async fn receive(
        &self,
        provider: &Provider,
        model: &Model,
        mut response: reqwest::Response,
        lifecycle: &mut Lifecycle,
    ) -> Result<Response, Failure> {
        lifecycle.upstream_status = Some(response.status().as_u16());
        lifecycle.upstream_headers_ms = Some(elapsed_ms(lifecycle.started));
        // The parser choice and error branch use upstream facts, never the hook draft.
        let original_status = response.status().as_u16();
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
            // Do not read or echo arbitrary upstream error bodies.
            return Err(error);
        }
        if matches!(original_status, 204 | 205) || media != "json" {
            return Err(Failure::new(ErrorCode::UpstreamProtocol, "upstream_media"));
        }
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
        let mut body: Value = serde_json::from_slice(&bytes).map_err(|source| {
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
        if body.get("model").is_some() {
            body["model"] = json!(lifecycle.alias);
        }
        for step in provider.pipeline.iter().chain(&model.pipeline) {
            if let Some(hook) = step.on_response {
                let mut context = lifecycle.context("onResponse");
                context["body"] = body.clone();
                let output = self
                    .hook(hook, "onResponse", Some(&step.name), context, lifecycle)
                    .await?;
                let next_body = require_body(&output)?;
                commit_state(lifecycle, &output)?;
                body = next_body;
            }
        }
        lifecycle.phase = "response";
        lifecycle.step = None;
        lifecycle.output_bytes = body.to_string().len();
        if lifecycle.output_bytes > self.server.limits.max_output_bytes {
            return Err(Failure::new(ErrorCode::HookFailed, "response"));
        }
        Ok(json_response(status, headers, &body, &lifecycle.request_id))
    }

    async fn after_receive(
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

    async fn hook(
        &self,
        hook_id: HookId,
        phase: &'static str,
        step: Option<&str>,
        context: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        lifecycle.phase = phase;
        lifecycle.step = step.map(str::to_owned);
        self.runtime
            .invoke(HookCall {
                request_id: lifecycle.request_id.clone(),
                call_id: self.next_call(),
                hook_id,
                context,
                deadline: self.hook_deadline(lifecycle),
                cancellation: lifecycle.cancellation.clone(),
            })
            .await
            .map_err(|source| Failure::script(source, phase, step))
    }

    pub(super) async fn observe(&self, lifecycle: &Lifecycle, failure: Option<&Failure>) {
        let deadline = Instant::now() + Duration::from_millis(self.server.limits.cleanup_ms);
        let outcome = match failure {
            None => "success",
            Some(e) if e.code == ErrorCode::Cancelled => "cancelled",
            Some(_) => "failed",
        };
        for (phase, hook) in [
            ("onError", failure.and(self.business.on_error)),
            ("onFinish", self.business.on_finish),
        ] {
            let Some(hook_id) = hook else {
                continue;
            };
            if !matches!(self.runtime.status(), RuntimeStatus::Ready) {
                tracing::warn!(
                    request_id = lifecycle.request_id,
                    phase,
                    reason = "runtime_unavailable",
                    "observation not executed"
                );
                continue;
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    request_id = lifecycle.request_id,
                    phase,
                    reason = "cleanup_budget",
                    "observation not executed"
                );
                continue;
            }
            let mut context = lifecycle.context(phase);
            context["outcome"] = json!(outcome);
            context["stats"] = json!({"inputBytes":lifecycle.input_bytes,"outboundBytes":lifecycle.outbound_bytes,"upstreamBytes":lifecycle.upstream_bytes,"outputBytes":lifecycle.output_bytes,"upstreamHeadersMs":lifecycle.upstream_headers_ms,"elapsedMs":elapsed_ms(lifecycle.started),"finalUpstreamModel":lifecycle.final_model,"target":lifecycle.target});
            if let Some(failure) = failure {
                context["error"] = failure.error_json(&lifecycle.request_id)["error"].clone();
                context["error"]["phase"] = json!(failure.phase);
                context["error"]["step"] =
                    json!(failure.step.as_deref().or(lifecycle.step.as_deref()));
            }
            let result = self
                .runtime
                .invoke(HookCall {
                    request_id: lifecycle.request_id.clone(),
                    call_id: self.next_call(),
                    hook_id,
                    context,
                    deadline,
                    cancellation: Cancellation::new(),
                })
                .await;
            match result {
                Ok(_) => tracing::debug!(
                    request_id = lifecycle.request_id,
                    phase,
                    observation = "completed",
                    "observation completed"
                ),
                Err(source) => {
                    tracing::warn!(request_id = lifecycle.request_id, phase, error_kind = ?source.kind(), error = %source.diagnostic(), "observation failed; terminal outcome retained");
                },
            }
        }
    }
}

fn commit_state(lifecycle: &mut Lifecycle, output: &Value) -> Result<(), Failure> {
    let state = output
        .get("state")
        .filter(|v| v.is_object())
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, lifecycle.phase))?;
    lifecycle.state = state.clone();
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn require_body(output: &Value) -> Result<Value, Failure> {
    output
        .get("body")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "hook_output"))
}

fn hook_invalid(phase: &'static str, step: &str) -> Failure {
    let mut error = Failure::new(ErrorCode::HookFailed, phase);
    error.step = Some(step.to_owned());
    error
}

#[derive(Debug, thiserror::Error)]
#[error("downstream status violates the upstream status class: {0:?}")]
struct StatusMapping(ErrorCode);

fn media_kind(media: &str) -> &'static str {
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

fn is_json_media(media: &str) -> bool {
    media
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

#[cfg(test)]
mod tests;
