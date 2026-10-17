use std::time::Instant;

use axum::{
    body::to_bytes,
    http::{Request, header},
    response::Response,
};
use limen_config::{Model, Provider};
use limen_script::{
    HookCall, HookCandidate, HookContext, HookEffects, HookId, HookInput, HookOutput,
    RejectionStatus, RequestDraft,
};
use serde_json::Value;

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
            let context = lifecycle.context(HookInput::OnRequest { body });
            let output = self.hook(hook, None, context, lifecycle).await?;
            let HookCandidate::OnRequest {
                body: next_body,
                rejection,
            } = output.candidate
            else {
                return Err(hook_invalid("onRequest", "global"));
            };
            logic::validate_body_shape(&next_body).map_err(|source| {
                Failure::new(ErrorCode::HookFailed, "onRequest").with_source(source)
            })?;
            commit_request(lifecycle, output.effects, rejection)?;
            body = next_body;
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
                let request = RequestDraft {
                    body,
                    url,
                    headers: logic::header_fields(&headers)?,
                };
                let deadline = self.hook_deadline(lifecycle);
                let output = if let Some(policy) = &step.builtin {
                    lifecycle.phase = "beforeSend";
                    lifecycle.step = Some(step.name.clone());
                    let measurement = lifecycle.statistics.timings.native("beforeSend");
                    let result = (|| {
                        let mut context = lifecycle.context(HookInput::BeforeSend(request));
                        self.validate_native_draft(&context, "beforeSend", &step.name)?;
                        let HookInput::BeforeSend(request) = &mut context.input else {
                            return Err(hook_invalid("beforeSend", &step.name));
                        };
                        let (rejection, observations) = native::before_send(
                            policy,
                            request,
                            &context.client,
                            &self.server.observability.events,
                            &step.name,
                            &mut || self.native_active(lifecycle, deadline),
                        )?;
                        self.validate_native_draft(&context, "beforeSend", &step.name)?;
                        self.native_active(lifecycle, deadline)?;
                        let HookInput::BeforeSend(request) = context.input else {
                            return Err(hook_invalid("beforeSend", &step.name));
                        };
                        Ok(HookOutput {
                            effects: HookEffects {
                                state: context.state,
                                observations,
                            },
                            candidate: HookCandidate::BeforeSend { request, rejection },
                        })
                    })();
                    measurement.finish(&result);
                    result?
                } else {
                    self.hook(
                        step.before_send
                            .ok_or_else(|| hook_invalid("beforeSend", &step.name))?,
                        Some(&step.name),
                        lifecycle.context(HookInput::BeforeSend(request)),
                        lifecycle,
                    )
                    .await?
                };
                let HookCandidate::BeforeSend { request, rejection } = output.candidate else {
                    return Err(hook_invalid("beforeSend", &step.name));
                };
                logic::validate_body_shape(&request.body)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                logic::validate_url(&request.url)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                let next_headers = logic::headers_from_fields(&request.headers)
                    .map_err(|source| hook_invalid("beforeSend", &step.name).with_source(source))?;
                if step.builtin.is_some() {
                    self.native_active(lifecycle, deadline)?;
                } else {
                    self.request_active(lifecycle, &lifecycle.cancellation)?;
                }
                commit_request(lifecycle, output.effects, rejection)?;
                body = request.body;
                url = request.url;
                headers = next_headers;
            }
        }
        Ok((body, url, headers))
    }

    pub(super) fn validate_native_draft(
        &self,
        value: &HookContext,
        phase: &'static str,
        step: &str,
    ) -> Result<(), Failure> {
        value
            .validate(limen_script::JsonLimits {
                max_depth: self.server.limits.max_json_depth,
                max_nodes: self.server.limits.max_json_nodes,
                max_bytes: self.server.limits.max_output_bytes,
            })
            .map_err(|source| hook_invalid(phase, step).with_source(source))?;
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
        step: Option<&str>,
        context: HookContext,
        lifecycle: &mut Lifecycle,
    ) -> Result<HookOutput, Failure> {
        let phase = context.input.phase().as_str();
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

/// Reject only after the ordinary draft checks and successful effects commit.
fn commit_request(
    lifecycle: &mut Lifecycle,
    effects: HookEffects,
    rejection: Option<RejectionStatus>,
) -> Result<(), Failure> {
    commit_effects(lifecycle, effects);
    if let Some(status) = rejection {
        let mut failure = Failure::new(ErrorCode::RequestRejected, lifecycle.phase);
        failure.status = Some(status.as_u16());
        failure.step.clone_from(&lifecycle.step);
        return Err(failure);
    }
    Ok(())
}

pub(super) fn commit_effects(lifecycle: &mut Lifecycle, effects: HookEffects) {
    lifecycle.state = effects.state;
    lifecycle.observations.commit(
        effects.observations,
        lifecycle.phase,
        lifecycle.step.as_deref(),
    );
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn hook_invalid(phase: &'static str, step: &str) -> Failure {
    let mut error = Failure::new(ErrorCode::HookFailed, phase);
    error.step = Some(step.to_owned());
    error
}

#[cfg(test)]
mod tests;
