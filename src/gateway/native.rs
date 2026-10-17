//! Rust policy execution uses the same owned drafts and gateway commit checks as JS.

use limen_config::{BuiltinPolicy, SyntheticTextTool};
use limen_script::ObservationBatch;
use serde_json::{Value, json};

use super::{
    error::{ErrorCode, Failure},
    pipeline::hook_invalid,
};

pub(super) fn before_send(
    policy: &BuiltinPolicy,
    context: &mut Value,
    names: &[String],
    step: &str,
    check: &mut dyn FnMut() -> Result<(), Failure>,
) -> Result<ObservationBatch, Failure> {
    check()?;
    let mut changed_event = None;
    match policy {
        BuiltinPolicy::CapCompletionTokens(options) => {
            let max = serde_json::Number::from(options.max)
                .as_f64()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let body = context["body"]
                .as_object_mut()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let field = options.field.name();
            if body.contains_key(options.field.other()) {
                reject(context);
            } else if let Some(value) = body.get(field) {
                if !value.as_f64().is_some_and(|value| {
                    (1.0..=9_007_199_254_740_991.0).contains(&value) && value.fract() == 0.0
                }) {
                    reject(context);
                } else if value.as_f64().is_some_and(|value| value > max) {
                    body.insert(field.to_owned(), json!(options.max));
                    changed_event = options.on_change.as_deref();
                }
            } else {
                body.insert(field.to_owned(), json!(options.max));
                changed_event = options.on_change.as_deref();
            }
        },
        BuiltinPolicy::DropRequestFields(options) => {
            let body = context["body"]
                .as_object_mut()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let mut changed = false;
            for field in &options.fields {
                check()?;
                changed |= body.remove(field).is_some();
            }
            if changed {
                changed_event = options.on_change.as_deref();
            }
        },
        BuiltinPolicy::ForwardClientHeader(options) => {
            let fields = context["client"]["headers"]
                .as_array()
                .ok_or_else(|| hook_invalid("beforeSend", step))?;
            let mut values = fields.iter().filter(|entry| {
                entry["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&options.from))
            });
            let value = values.next().map(|entry| entry["value"].clone());
            if values.next().is_some() {
                reject(context);
            } else if let Some(value) = value {
                let fields = context["headers"]
                    .as_array_mut()
                    .ok_or_else(|| hook_invalid("beforeSend", step))?;
                fields.retain(|entry| {
                    !entry["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&options.to))
                });
                fields.push(json!({"name":options.to.to_ascii_lowercase(),"value":value}));
            }
        },
        BuiltinPolicy::SyntheticTextTool(options) => {
            synthetic_before_send(options, context, check)?;
        },
        BuiltinPolicy::ReplaceText(_) => return Err(hook_invalid("beforeSend", step)),
    }
    let mut observations = ObservationBatch::default();
    if let Some(name) = changed_event {
        let id = names
            .iter()
            .position(|candidate| candidate == name)
            .and_then(|id| u8::try_from(id).ok())
            .ok_or_else(|| hook_invalid("beforeSend", step))?;
        observations.events.push(id);
    }
    check()?;
    Ok(observations)
}

fn reject(context: &mut Value) { context["rejection"] = json!({"status":422}); }

fn synthetic_before_send(
    options: &SyntheticTextTool,
    context: &mut Value,
    check: &mut dyn FnMut() -> Result<(), Failure>,
) -> Result<(), Failure> {
    let body = &mut context["body"];
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
        reject(context);
        return Ok(());
    }
    if let Some(tools) = body.get("tools") {
        let Some(tools) = tools.as_array() else {
            reject(context);
            return Ok(());
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
                reject(context);
                return Ok(());
            }
        }
    }
    let messages = body["messages"]
        .as_array_mut()
        .ok_or_else(|| Failure::new(ErrorCode::HookFailed, "beforeSend"))?;
    messages.push(json!({"role":"user","content":format!("Transport instruction: To answer with text, call {} with exactly {{\"text\":\"your answer\"}}. Other declared tools remain available.",options.tool_name)}));
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
    Ok(())
}

#[cfg(test)]
mod tests;
