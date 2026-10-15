use std::time::Instant;

use limen_script::{Cancellation, HookCall, RuntimeStatus};
use serde_json::json;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    lifecycle::Lifecycle,
    pipeline::elapsed_ms,
};

impl Gateway {
    pub(super) async fn observe(
        &self,
        lifecycle: &Lifecycle,
        failure: Option<&Failure>,
        deadline: Instant,
    ) {
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
                self.transport.observation_failed().await;
                tracing::warn!(
                    request_id = lifecycle.request_id,
                    phase,
                    reason = "runtime_unavailable",
                    "observation not executed"
                );
                continue;
            }
            if Instant::now() >= deadline {
                self.transport.observation_failed().await;
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
                    self.transport.observation_failed().await;
                    tracing::warn!(request_id = lifecycle.request_id, phase, error_kind = ?source.kind(), error = %source.diagnostic(), "observation failed; terminal outcome retained");
                },
            }
        }
    }
}
