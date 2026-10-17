use std::time::Instant;

use limen_script::{
    Cancellation, HookCall, HookInput, HookOutcome, HookPhase, ObserverInput, RuntimeStatus,
};
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
            None => HookOutcome::Success,
            Some(e) if e.code == ErrorCode::Cancelled => HookOutcome::Cancelled,
            Some(_) => HookOutcome::Failed,
        };
        for (phase, hook) in [
            (HookPhase::OnError, failure.and(self.plans.on_error)),
            (HookPhase::OnFinish, self.plans.on_finish),
        ] {
            let phase_name = phase.as_str();
            let Some(hook_id) = hook else {
                continue;
            };
            if !matches!(self.runtime.status(), RuntimeStatus::Ready) {
                self.transport.observation_failed().await;
                tracing::warn!(
                    request_id = lifecycle.request_id,
                    phase = phase_name,
                    reason = "runtime_unavailable",
                    "observation not executed"
                );
                continue;
            }
            if Instant::now() >= deadline {
                self.transport.observation_failed().await;
                tracing::warn!(
                    request_id = lifecycle.request_id,
                    phase = phase_name,
                    reason = "cleanup_budget",
                    "observation not executed"
                );
                continue;
            }
            let error = failure.map(|failure| {
                let mut error = failure.observer_error(&lifecycle.request_id);
                error["step"] = json!(failure.step.as_deref().or(lifecycle.step.as_deref()));
                error
            });
            let input = ObserverInput {
                outcome,
                error,
                stats: lifecycle.stats(),
            };
            let input = if phase == HookPhase::OnError {
                HookInput::OnError(input)
            } else {
                HookInput::OnFinish(input)
            };
            let context = lifecycle.context(input);
            let timings = Timings::default();
            let measurement = timings.hook(phase_name);
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
                        .commit(output.effects.observations, phase_name, None);
                    tracing::debug!(
                        request_id = lifecycle.request_id,
                        phase = phase_name,
                        observation = "completed",
                        "observation completed"
                    );
                },
                Err(source) => {
                    self.transport.observation_failed().await;
                    tracing::warn!(request_id = lifecycle.request_id, phase = phase_name, error_kind = ?source.kind(), error = %source.diagnostic(), "observation failed; terminal outcome retained");
                },
            }
        }
    }
}
