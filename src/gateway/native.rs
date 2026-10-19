//! Rust policy execution uses the same owned drafts and gateway commit checks as JS.

use limen_config::{
    BeforeSendPolicy, CapCompletionTokens, ConfigError, DropRequestFields, ForwardClientHeader,
    SyntheticTextTool,
};
use limen_script::{ClientContext, HeaderField, ObservationBatch, RejectionStatus, RequestDraft};
use serde_json::{Value, json};

use super::{
    error::{ErrorCode, Failure},
    pipeline::hook_invalid,
};

/// Startup-bound request executors contain only supported policies and approved IDs.
pub(super) enum NativeBeforeSend {
    CapCompletionTokens {
        options: CapCompletionTokens,
        observation: Option<u8>,
    },
    DropRequestFields {
        options: DropRequestFields,
        observation: Option<u8>,
    },
    ForwardClientHeader(ForwardClientHeader),
    SyntheticTextTool(SyntheticTextTool),
}

impl NativeBeforeSend {
    pub(super) fn compile(
        policy: BeforeSendPolicy<'_>,
        names: &[String],
    ) -> Result<Self, ConfigError> {
        let observation = policy.observation_id(names)?;
        Ok(match policy {
            BeforeSendPolicy::CapCompletionTokens(options) => Self::CapCompletionTokens {
                options: options.clone(),
                observation,
            },
            BeforeSendPolicy::DropRequestFields(options) => Self::DropRequestFields {
                options: options.clone(),
                observation,
            },
            BeforeSendPolicy::ForwardClientHeader(options) => {
                Self::ForwardClientHeader(options.clone())
            },
            BeforeSendPolicy::SyntheticTextTool(options) => {
                Self::SyntheticTextTool(options.clone())
            },
        })
    }
}

pub(super) fn before_send(
    policy: &NativeBeforeSend,
    request: &mut RequestDraft,
    client: &ClientContext,
    step: &str,
    check: &mut dyn FnMut() -> Result<(), Failure>,
) -> Result<(Option<RejectionStatus>, ObservationBatch), Failure> {
    check()?;
    let mut changed_event = None;
    let mut rejection = None;
    match policy {
        NativeBeforeSend::CapCompletionTokens {
            options,
            observation,
        } => {
            let max = serde_json::Number::from(options.max)
                .as_f64()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let body = request
                .body
                .as_object_mut()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let field = options.field.name();
            if body.contains_key(options.field.other()) {
                rejection = Some(RejectionStatus::UnprocessableEntity);
            } else if let Some(value) = body.get(field) {
                if !value.as_f64().is_some_and(|value| {
                    (1.0..=9_007_199_254_740_991.0).contains(&value) && value.fract() == 0.0
                }) {
                    rejection = Some(RejectionStatus::UnprocessableEntity);
                } else if value.as_f64().is_some_and(|value| value > max) {
                    body.insert(field.to_owned(), json!(options.max));
                    changed_event = *observation;
                }
            } else {
                body.insert(field.to_owned(), json!(options.max));
                changed_event = *observation;
            }
        },
        NativeBeforeSend::DropRequestFields {
            options,
            observation,
        } => {
            let body = request
                .body
                .as_object_mut()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let mut changed = false;
            for field in &options.fields {
                check()?;
                changed |= body.remove(field).is_some();
            }
            if changed {
                changed_event = *observation;
            }
        },
        NativeBeforeSend::ForwardClientHeader(options) => {
            let mut values = client
                .headers
                .iter()
                .filter(|entry| entry.name.eq_ignore_ascii_case(&options.from));
            let value = values.next().map(|entry| entry.value.clone());
            if values.next().is_some() {
                rejection = Some(RejectionStatus::UnprocessableEntity);
            } else if let Some(value) = value {
                request
                    .headers
                    .retain(|entry| !entry.name.eq_ignore_ascii_case(&options.to));
                request.headers.push(HeaderField {
                    name: options.to.to_ascii_lowercase(),
                    value,
                });
            }
        },
        NativeBeforeSend::SyntheticTextTool(options) => {
            rejection = synthetic_before_send(options, &mut request.body, check)?;
        },
    }
    let mut observations = ObservationBatch::default();
    if let Some(id) = changed_event {
        observations.events.push(id);
    }
    check()?;
    Ok((rejection, observations))
}

fn synthetic_before_send(
    options: &SyntheticTextTool,
    body: &mut Value,
    check: &mut dyn FnMut() -> Result<(), Failure>,
) -> Result<Option<RejectionStatus>, Failure> {
    if ["functions", "function_call", "response_format"]
        .iter()
        .any(|field| body.get(*field).is_some())
        || body
            .get("n")
            .is_some_and(|value| value.as_f64() != Some(1.0))
        || body
            .get("parallel_tool_calls")
            .is_some_and(|value| value == true)
        || body.get("tool_choice").is_some_and(|value| value != "auto")
    {
        return Ok(Some(RejectionStatus::UnprocessableEntity));
    }
    if let Some(tools) = body.get("tools") {
        let Some(tools) = tools.as_array() else {
            return Ok(Some(RejectionStatus::UnprocessableEntity));
        };
        for tool in tools {
            check()?;
            if tool.get("type").is_none_or(|value| value != "function")
                || tool
                    .get("function")
                    .is_none_or(|function| !function.is_object())
                || tool["function"]["name"]
                    .as_str()
                    .is_none_or(|name| name == options.tool_name)
            {
                return Ok(Some(RejectionStatus::UnprocessableEntity));
            }
        }
    }
    let messages = body["messages"]
        .as_array_mut()
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "beforeSend"))?;
    let prompt = options.prompt.clone().unwrap_or_else(|| {
        format!("Transport instruction: To answer with text, call {} with exactly {{\"text\":\"your answer\"}}. Other declared tools remain available.", options.tool_name)
    });
    let index = messages.len().saturating_sub(options.injection_depth);
    messages.insert(index, json!({"role":"user","content":prompt}));
    if body.get("tools").is_none() {
        body["tools"] = json!([]);
    }
    let tools = body["tools"]
        .as_array_mut()
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "beforeSend"))?;
    tools.push(json!({"type":"function","function":{"name":options.tool_name,
        "description":"Return the assistant's text answer through the transport.",
        "parameters":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}}}));
    body["tool_choice"] = json!("required");
    body["parallel_tool_calls"] = json!(false);
    Ok(None)
}

#[cfg(test)]
mod tests;
