use std::time::Instant;

use axum::{
    body::to_bytes,
    http::{Request, header},
    response::Response,
};
use limen_config::{Model, Provider, validate_json};
use limen_script::{HookCall, HookId, HookOutput};
use serde_json::{Value, json};

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    lifecycle::Lifecycle,
    logic, native, protocol,
    response::Delivery,
    stats::WaitKind,
    upstream::is_json_media,
};

impl Gateway {
    pub(super) async fn prepare_request(
        &self,
        request: Request<axum::body::Body>,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        if !self.ready() {
            return Err(Failure::new(ErrorCode::Unavailable, "admission"));
        }
        if !request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(is_json_media)
        {
            return Err(Failure::new(ErrorCode::InvalidRequest, "request_body"));
        }
        lifecycle.phase = "request_body";
        let decode = lifecycle.statistics.timings.measure(WaitKind::Decode);
        lifecycle.client = logic::client_context(
            request.headers(),
            &self.server.script_context.client_headers,
        )?;
        let bytes = to_bytes(request.into_body(), self.server.limits.max_body_bytes)
            .await
            .map_err(Failure::body)?;
        lifecycle.input_bytes = bytes.len();
        let (body, stream) = logic::parse_request(&bytes, &self.server.limits)?;
        drop(decode);
        lifecycle.client_stream = Some(stream);
        Ok(body)
    }

    pub(super) async fn complete(
        &self,
        mut body: Value,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
    ) -> Result<Option<Response>, Failure> {
        if let Some(hook) = self.business.on_request {
            let mut context = lifecycle.context("onRequest");
            context["body"] = body;
            let mut output = self
                .hook(hook, "onRequest", None, context, lifecycle)
                .await?;
            logic::validate_body_shape(require_body(&output.context)?).map_err(|source| {
                Failure::new(ErrorCode::HookFailed, "onRequest").with_source(source)
            })?;
            commit_request(lifecycle, &mut output)?;
            body = output.context["body"].take();
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
        let body = logic::merge_body(provider, model, body)?;
        let (body, url, mut headers) = self.before_send(provider, model, body, lifecycle).await?;
        lifecycle.phase = "beforeSend";
        lifecycle.step = None;
        let bytes = protocol::encode_request(&body, &mut headers, &self.server.limits)?;
        lifecycle.outbound_bytes = bytes.len();
        lifecycle.final_model = body.get("model").and_then(Value::as_str).map(str::to_owned);
        lifecycle.target = Some(logic::safe_target(&url));
        drop(body);
        lifecycle.phase = "upstream_send";
        lifecycle.statistics.timings.start_upstream(Instant::now());
        let response = self.transport.send(url, headers, bytes).await?;
        self.receive(provider, model, response, lifecycle, delivery)
            .await
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
            if step.before_send.is_some()
                || step
                    .builtin
                    .as_ref()
                    .is_some_and(limen_config::BuiltinPolicy::has_before_send)
            {
                let mut context = lifecycle.context("beforeSend");
                context["body"] = body;
                context["url"] = json!(url);
                context["headers"] = logic::headers_to_json(&headers)?;
                let deadline = self.hook_deadline(lifecycle);
                let mut output = if let Some(policy) = &step.builtin {
                    lifecycle.phase = "beforeSend";
                    lifecycle.step = Some(step.name.clone());
                    let measurement = lifecycle.statistics.timings.native("beforeSend");
                    let result = (|| {
                        self.validate_native_draft(&context, "beforeSend", &step.name)?;
                        let observations = native::before_send(
                            policy,
                            &mut context,
                            &self.server.observability.events,
                            &step.name,
                            &mut || self.native_active(lifecycle, deadline),
                        )?;
                        self.validate_native_draft(&context, "beforeSend", &step.name)?;
                        self.native_active(lifecycle, deadline)?;
                        Ok(HookOutput {
                            context,
                            observations,
                        })
                    })();
                    measurement.finish(&result);
                    result?
                } else {
                    self.hook(
                        step.before_send
                            .ok_or_else(|| hook_invalid("beforeSend", &step.name))?,
                        "beforeSend",
                        Some(&step.name),
                        context,
                        lifecycle,
                    )
                    .await?
                };
                logic::validate_body_shape(require_body(&output.context)?)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                let next_url = output
                    .context
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| hook_invalid("beforeSend", &step.name))?;
                logic::validate_url(next_url)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                let next_headers = logic::headers_from_json(&output.context["headers"])
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                if step.builtin.is_some() {
                    self.native_active(lifecycle, deadline)?;
                } else {
                    self.request_active(lifecycle, &lifecycle.cancellation)?;
                }
                commit_request(lifecycle, &mut output)?;
                body = output.context["body"].take();
                let Value::String(next_url) = output.context["url"].take() else {
                    return Err(hook_invalid("beforeSend", &step.name));
                };
                url = next_url;
                headers = next_headers;
            }
        }
        Ok((body, url, headers))
    }

    pub(super) fn validate_native_draft(
        &self,
        value: &Value,
        phase: &'static str,
        step: &str,
    ) -> Result<(), Failure> {
        validate_json(
            value,
            self.server.limits.max_json_depth,
            self.server.limits.max_json_nodes,
        )
        .map_err(|source| hook_invalid(phase, step).with_source(source))?;
        super::json::size(
            value,
            self.server.limits.max_output_bytes,
            ErrorCode::HookFailed,
            phase,
        )?;
        Ok(())
    }

    pub(super) fn native_active(
        &self,
        lifecycle: &Lifecycle,
        deadline: Instant,
    ) -> Result<(), Failure> {
        self.request_active(lifecycle, &lifecycle.cancellation)
            .map_err(|mut failure| {
                failure.step.clone_from(&lifecycle.step);
                failure
            })?;
        if Instant::now() >= deadline {
            let mut failure = Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase);
            failure.step.clone_from(&lifecycle.step);
            Err(failure)
        } else {
            Ok(())
        }
    }

    pub(super) async fn hook(
        &self,
        hook_id: HookId,
        phase: &'static str,
        step: Option<&str>,
        context: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<HookOutput, Failure> {
        lifecycle.phase = phase;
        lifecycle.step = step.map(str::to_owned);
        let measurement = lifecycle.statistics.timings.hook(phase);
        let report = self
            .runtime
            .invoke(HookCall {
                call_id: self.next_call(),
                hook_id,
                context,
                deadline: self.hook_deadline(lifecycle),
                cancellation: lifecycle.cancellation.clone(),
            })
            .await;
        measurement.finish(&report);
        report
            .outcome
            .map_err(|source| Failure::script(source, phase, step))
    }
}

/// Reject only after the ordinary draft checks and successful state commit.
fn commit_request(lifecycle: &mut Lifecycle, output: &mut HookOutput) -> Result<(), Failure> {
    let rejection = output
        .context
        .get("rejection")
        .map(|value| {
            value
                .get("status")
                .and_then(Value::as_u64)
                .filter(|status| matches!(status, 400 | 403 | 422))
                .and_then(|status| u16::try_from(status).ok())
                .ok_or_else(|| Failure::new(ErrorCode::HookFailed, lifecycle.phase))
        })
        .transpose()?;
    commit_effects(lifecycle, output)?;
    if let Some(status) = rejection {
        let mut failure = Failure::new(ErrorCode::RequestRejected, lifecycle.phase);
        failure.status = Some(status);
        failure.step.clone_from(&lifecycle.step);
        return Err(failure);
    }
    Ok(())
}

pub(super) fn commit_effects(
    lifecycle: &mut Lifecycle,
    output: &mut HookOutput,
) -> Result<(), Failure> {
    commit_state(lifecycle, &mut output.context)?;
    lifecycle.observations.commit(
        std::mem::take(&mut output.observations),
        lifecycle.phase,
        lifecycle.step.as_deref(),
    );
    Ok(())
}

pub(super) fn commit_state(lifecycle: &mut Lifecycle, output: &mut Value) -> Result<(), Failure> {
    let state = output
        .get_mut("state")
        .filter(|v| v.is_object())
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, lifecycle.phase))?;
    lifecycle.state = state.take();
    Ok(())
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn require_body(output: &Value) -> Result<&Value, Failure> {
    output
        .get("body")
        .filter(|v| v.is_object())
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "hook_output"))
}

pub(super) fn hook_invalid(phase: &'static str, step: &str) -> Failure {
    let mut error = Failure::new(ErrorCode::HookFailed, phase);
    error.step = Some(step.to_owned());
    error
}

#[cfg(test)]
mod tests;
