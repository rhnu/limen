//! Error response policy runs before delivery, independently of terminal observers.

use std::time::{Duration, Instant};

use http::HeaderValue;
use limen_script::{Cancellation, HookCall, HookId};
use serde_json::{Value, json};

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
        let mut context = lifecycle.context("onErrorResponse");
        context["error"] = failure.observer_error(&lifecycle.request_id);
        context["status"] = json!(failure.http_status());
        context["headers"] = logic::headers_to_json(&lifecycle.response_headers)?;
        context["body"] = if encoded {
            Value::Null
        } else {
            json!(std::str::from_utf8(body).ok())
        };
        context["contentType"] = json!(content_type.and_then(|value| value.to_str().ok()));
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
        let mut output = report
            .outcome
            .map_err(|source| Failure::script(source, "onErrorResponse", step))?;
        let invalid = || hook_invalid("onErrorResponse", step.unwrap_or("global"));
        let status = output.context["status"]
            .as_u64()
            .and_then(|status| u16::try_from(status).ok())
            .ok_or_else(invalid)?;
        let upstream_failure =
            failure.code == ErrorCode::UpstreamFailed && failure.phase == "upstream_status";
        if !((400..600).contains(&status) || upstream_failure && (300..400).contains(&status)) {
            return Err(invalid());
        }
        let headers = logic::headers_from_json(&output.context["headers"])
            .map_err(|source| invalid().with_source(source))?;
        let content_type = match &output.context["contentType"] {
            Value::Null => None,
            Value::String(value) => {
                Some(HeaderValue::from_str(value).map_err(|source| invalid().with_source(source))?)
            },
            _ => return Err(invalid()),
        };
        let (body, content_encoding) = match &output.context["body"] {
            Value::String(value) if value.len() <= self.server.limits.max_output_bytes => {
                (value.as_bytes().to_vec(), Vec::new())
            },
            Value::Null => (
                body.clone(),
                failure
                    .response
                    .as_ref()
                    .map_or_else(Vec::new, |response| response.content_encoding.clone()),
            ),
            _ => return Err(invalid()),
        };
        let state = output
            .context
            .get_mut("state")
            .filter(|state| state.is_object())
            .ok_or_else(invalid)?;
        // All response checks precede this atomic commit, including state and events.
        lifecycle.state = state.take();
        lifecycle.response_headers = headers;
        lifecycle
            .observations
            .commit(output.observations, "onErrorResponse", step);
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
