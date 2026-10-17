//! Startup-bound stage executors; provider plans are shared by every referring alias.

use std::{collections::BTreeMap, time::Instant};

use limen_chat::{NativeContentError, NativeContentLimits, NativeContentTemplate};
use limen_config::{BusinessConfig, ConfigError, Model, PipelineStep, Provider, ResourceLimits};
use limen_script::HookId;

use super::native::NativeBeforeSend;

#[derive(thiserror::Error, Debug)]
pub(crate) enum Error {
    #[error("invalid native policy configuration")]
    Config(#[from] ConfigError),
    #[error("native content compilation failed")]
    Content(#[from] NativeContentError),
    #[error("execution plan compilation exceeded the startup budget")]
    Deadline,
}

pub(super) struct Step<T> {
    pub name: String,
    pub executor: T,
}

pub(super) enum BeforeSendExecutor {
    Script(HookId),
    Native(NativeBeforeSend),
}

pub(super) enum ContentExecutor {
    Script(HookId),
    Native(NativeContentTemplate),
}

#[derive(Default)]
struct PipelinePlan {
    before_send: Vec<Step<BeforeSendExecutor>>,
    after_receive: Vec<Step<HookId>>,
    response: Vec<Step<HookId>>,
    content: Vec<Step<ContentExecutor>>,
    upstream_event: Vec<Step<HookId>>,
    stream_event: Vec<Step<HookId>>,
    error_response: Vec<Step<HookId>>,
}

pub(crate) struct ExecutionPlans {
    providers: BTreeMap<String, PipelinePlan>,
    models: BTreeMap<String, PipelinePlan>,
    pub on_request: Option<HookId>,
    pub on_error_response: Option<HookId>,
    pub on_error: Option<HookId>,
    pub on_finish: Option<HookId>,
}

impl ExecutionPlans {
    pub(crate) fn compile(
        config: &BusinessConfig,
        limits: &ResourceLimits,
        observation_names: &[String],
        deadline: Instant,
    ) -> Result<Self, Error> {
        let compile = |steps: &[PipelineStep]| {
            let mut plan = PipelinePlan::default();
            for step in steps {
                active(deadline)?;
                if let Some(policy) = &step.builtin {
                    let stages = policy.stages();
                    if let Some(policy) = stages.before_send {
                        plan.before_send.push(Step {
                            name: step.name.clone(),
                            executor: BeforeSendExecutor::Native(NativeBeforeSend::compile(
                                policy,
                                observation_names,
                            )?),
                        });
                    }
                    if let Some(policy) = stages.content {
                        let template = NativeContentTemplate::compile(
                            policy,
                            &NativeContentLimits {
                                max_output_events: limits.max_output_events,
                                max_output_bytes: limits.max_output_bytes,
                                max_retained_bytes: limits.max_output_bytes,
                                max_retained_records: limits.max_json_nodes,
                            },
                            &mut || Instant::now() < deadline,
                        )
                        .map_err(|error| {
                            if error == NativeContentError::Interrupted {
                                Error::Deadline
                            } else {
                                Error::Content(error)
                            }
                        })?;
                        plan.content.push(Step {
                            name: step.name.clone(),
                            executor: ContentExecutor::Native(template),
                        });
                    }
                } else {
                    if let Some(hook) = step.before_send {
                        plan.before_send.push(Step {
                            name: step.name.clone(),
                            executor: BeforeSendExecutor::Script(hook),
                        });
                    }
                    if let Some(hook) = step.on_content {
                        plan.content.push(Step {
                            name: step.name.clone(),
                            executor: ContentExecutor::Script(hook),
                        });
                    }
                    for (hook, stage) in [
                        (step.after_receive, &mut plan.after_receive),
                        (step.on_response, &mut plan.response),
                        (step.on_upstream_event, &mut plan.upstream_event),
                        (step.on_stream_event, &mut plan.stream_event),
                        (step.on_error_response, &mut plan.error_response),
                    ] {
                        if let Some(hook) = hook {
                            stage.push(Step {
                                name: step.name.clone(),
                                executor: hook,
                            });
                        }
                    }
                }
            }
            Ok::<_, Error>(plan)
        };
        let plans = Self {
            providers: config
                .providers
                .iter()
                .map(|(name, provider)| Ok((name.clone(), compile(&provider.pipeline)?)))
                .collect::<Result<_, Error>>()?,
            models: config
                .models
                .iter()
                .map(|(name, model)| Ok((name.clone(), compile(&model.pipeline)?)))
                .collect::<Result<_, Error>>()?,
            on_request: config.on_request,
            on_error_response: config.on_error_response,
            on_error: config.on_error,
            on_finish: config.on_finish,
        };
        active(deadline)?;
        Ok(plans)
    }

    /// Routing looks up each half once; all later phases borrow the same pair.
    pub(super) fn route<'a>(
        &'a self,
        config: &'a BusinessConfig,
        alias: &str,
    ) -> Option<RoutePlan<'a>> {
        let model = config.models.get(alias)?;
        let provider = config
            .providers
            .get(&model.provider)
            .expect("business config validates every model provider");
        Some(RoutePlan {
            model,
            provider,
            provider_plan: self
                .providers
                .get(&model.provider)
                .expect("plans compile every validated provider"),
            model_plan: self
                .models
                .get(alias)
                .expect("plans compile every validated model"),
        })
    }
}

fn active(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::Deadline)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct RoutePlan<'a> {
    pub provider: &'a Provider,
    pub model: &'a Model,
    provider_plan: &'a PipelinePlan,
    model_plan: &'a PipelinePlan,
}

impl RoutePlan<'_> {
    pub fn before_send(&self) -> (&[Step<BeforeSendExecutor>], &[Step<BeforeSendExecutor>]) {
        (
            &self.provider_plan.before_send,
            &self.model_plan.before_send,
        )
    }

    pub fn content(&self) -> (&[Step<ContentExecutor>], &[Step<ContentExecutor>]) {
        (&self.provider_plan.content, &self.model_plan.content)
    }

    pub fn after_receive(&self) -> (&[Step<HookId>], &[Step<HookId>]) {
        (
            &self.provider_plan.after_receive,
            &self.model_plan.after_receive,
        )
    }

    pub fn response(&self) -> (&[Step<HookId>], &[Step<HookId>]) {
        (&self.provider_plan.response, &self.model_plan.response)
    }

    pub fn upstream_event(&self) -> (&[Step<HookId>], &[Step<HookId>]) {
        (
            &self.provider_plan.upstream_event,
            &self.model_plan.upstream_event,
        )
    }

    pub fn stream_event(&self) -> (&[Step<HookId>], &[Step<HookId>]) {
        (
            &self.provider_plan.stream_event,
            &self.model_plan.stream_event,
        )
    }

    pub fn error_response(&self) -> (&[Step<HookId>], &[Step<HookId>]) {
        (
            &self.provider_plan.error_response,
            &self.model_plan.error_response,
        )
    }
}

#[cfg(test)]
mod tests;
