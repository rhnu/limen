//! Error response policy runs before delivery, independently of terminal observers.

use std::time::{Duration, Instant};

use http::HeaderValue;
use limen_script::{Cancellation, ErrorResponseInput, HookCall, HookCandidate, HookId, HookInput};

use super::{
    Gateway,
    error::{ErrorCode, Failure, FailureResponse},
    lifecycle::Lifecycle,
    logic,
    pipeline::hook_invalid,
    stats::Timings,
};

impl Gateway {
    /// Cleanup has its own cancellation token: expired requests may still send errors.
    /// A failed policy retains the primary failure and every previously committed draft.
    pub(super) async fn prepare_error(
        &self,
        failure: &mut Failure,
        lifecycle: &mut Lifecycle,
        deadline: Instant,
    ) {
        let mut hooks = Vec::new();
        if let Some(model) = lifecycle
            .alias
            .as_ref()
            .and_then(|alias| self.business.models.get(alias))
            && let Some(provider) = self.business.providers.get(&model.provider)
        {
            for step in provider.pipeline.iter().chain(&model.pipeline) {
                if let Some(hook) = step.on_error_response {
                    hooks.push((hook, Some(step.name.as_str())));
                }
            }
        }
        if let Some(hook) = self.business.on_error_response {
            hooks.push((hook, None));
        }
        for (hook, step) in hooks {
            if Instant::now() >= deadline {
                tracing::warn!(
                    phase = "onErrorResponse",
                    step,
                    reason = "cleanup_budget",
                    "error response policy not executed"
                );
                break;
            }
            if let Err(policy_error) = self
                .error_response_hook(hook, step, failure, lifecycle, deadline)
                .await
            {
                tracing::warn!(phase = "onErrorResponse", step, error = %policy_error, "error response policy failed; last committed response retained");
                break;
            }
        }
        lifecycle.output_bytes = failure.response.as_ref().map_or_else(
            || failure.error_json(&lifecycle.request_id).to_string().len(),
            |response| response.body.len(),
        );
    }

    async fn error_response_hook(
        &self,
        hook: HookId,
        step: Option<&str>,
        failure: &mut Failure,
        lifecycle: &mut Lifecycle,
        deadline: Instant,
    ) -> Result<(), Failure> {
        let default_body;
        let default_content_type = HeaderValue::from_static("application/json");
        let (body, content_type, encoded) = if let Some(response) = &failure.response {
            (
                &response.body,
                response.content_type.as_ref(),
                !response.content_encoding.is_empty(),
            )
        } else {
            default_body = failure
                .error_json(&lifecycle.request_id)
                .to_string()
                .into_bytes();
            (&default_body, Some(&default_content_type), false)
        };
        let context = lifecycle.context(HookInput::OnErrorResponse(ErrorResponseInput {
            error: failure.observer_error(&lifecycle.request_id),
            status: failure.http_status(),
            headers: logic::header_fields(&lifecycle.response_headers)?,
            body: if encoded {
                None
            } else {
                std::str::from_utf8(body).ok().map(str::to_owned)
            },
            content_type: content_type
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        }));
        let timings = Timings::default();
        let measurement = timings.hook("onErrorResponse");
        let report = self
            .runtime
            .invoke(HookCall {
                call_id: self.next_call(),
                hook_id: hook,
                context,
                deadline: deadline
                    .min(Instant::now() + Duration::from_millis(self.server.limits.hook_ms)),
                cancellation: Cancellation::new(),
            })
            .await;
        measurement.finish(&report);
        super::stats::record_cleanup(&lifecycle.request_id, &timings.snapshot());
        let output = report
            .outcome
            .map_err(|source| Failure::script(source, "onErrorResponse", step))?;
        let invalid = || hook_invalid("onErrorResponse", step.unwrap_or("global"));
        let HookCandidate::OnErrorResponse(draft) = output.candidate else {
            return Err(invalid());
        };
        let status = draft.status;
        let upstream_failure =
            failure.code == ErrorCode::UpstreamFailed && failure.phase == "upstream_status";
        if !((400..600).contains(&status) || upstream_failure && (300..400).contains(&status)) {
            return Err(invalid());
        }
        let headers = logic::headers_from_fields(&draft.headers)
            .map_err(|source| invalid().with_source(source))?;
        let content_type = draft
            .content_type
            .as_deref()
            .map(HeaderValue::from_str)
            .transpose()
            .map_err(|source| invalid().with_source(source))?;
        let (body, content_encoding) = match draft.body {
            Some(value) if value.len() <= self.server.limits.max_output_bytes => {
                (value.into_bytes(), Vec::new())
            },
            None => (
                body.clone(),
                failure
                    .response
                    .as_ref()
                    .map_or_else(Vec::new, |response| response.content_encoding.clone()),
            ),
            Some(_) => return Err(invalid()),
        };
        // All response checks precede this atomic commit, including state and events.
        lifecycle.state = output.effects.state;
        lifecycle.response_headers = headers;
        lifecycle
            .observations
            .commit(output.effects.observations, "onErrorResponse", step);
        failure.status = Some(status);
        failure.response = Some(Box::new(FailureResponse {
            body,
            content_type,
            content_encoding,
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
