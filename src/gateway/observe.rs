use std::time::Instant;

use limen_script::{Cancellation, HookCall, RuntimeStatus};
use serde_json::json;

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    lifecycle::Lifecycle,
    stats::Timings,
};

impl Gateway {
    pub(super) async fn observe(
        &self,
        lifecycle: &mut Lifecycle,
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
            context["stats"] = lifecycle.stats();
            if let Some(failure) = failure {
                context["error"] = failure.observer_error(&lifecycle.request_id);
                context["error"]["step"] =
                    json!(failure.step.as_deref().or(lifecycle.step.as_deref()));
            }
            let timings = Timings::default();
            let measurement = timings.hook(phase);
            let report = self
                .runtime
                .invoke(HookCall {
                    call_id: self.next_call(),
                    hook_id,
                    context,
                    deadline,
                    cancellation: Cancellation::new(),
                })
                .await;
            measurement.finish(&report);
            super::stats::record_cleanup(&lifecycle.request_id, &timings.snapshot());
            match report.outcome {
                Ok(output) => {
                    lifecycle
                        .observations
                        .commit(output.observations, phase, None);
                    tracing::debug!(
                        request_id = lifecycle.request_id,
                        phase,
                        observation = "completed",
                        "observation completed"
                    );
                },
                Err(source) => {
                    self.transport.observation_failed().await;
                    tracing::warn!(request_id = lifecycle.request_id, phase, error_kind = ?source.kind(), error = %source.diagnostic(), "observation failed; terminal outcome retained");
                },
            }
        }
    }
}
