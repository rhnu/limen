//! Content timing observes understood leaves without imposing projection requirements.

use serde_json::Value;

use super::{CONTENT_FIELDS, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent content observations can occur together in one chunk"
)]
pub struct GeneratedContent {
    pub generation: bool,
    pub text: bool,
    pub reasoning: bool,
    pub audio: bool,
    pub qualified: bool,
}

impl Default for GeneratedContent {
    fn default() -> Self {
        Self {
            generation: false,
            text: false,
            reasoning: false,
            audio: false,
            qualified: true,
        }
    }
}

/// Samples known generated fields. Opaque extensions affect metric qualification,
/// never the success or forwarding semantics of the original response.
pub fn observe_generated_content(value: &Value) -> GeneratedContent {
    let mut facts = GeneratedContent::default();
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(body) = choice.get("delta").or_else(|| choice.get("message")) {
                observe_body(body, &mut facts);
            }
        }
    }
    facts
}

fn observe_body(body: &Value, facts: &mut GeneratedContent) {
    let Some(body) = body.as_object() else {
        facts.qualified = false;
        return;
    };
    for (field, kind) in CONTENT_FIELDS {
        if let Some(value) = body.get(*field) {
            observe_text(value, kind, facts);
        }
    }
    if let Some(value) = body.get("function_call") {
        observe_tool(value, "arguments", facts);
    }
    if let Some(tools) = body.get("tool_calls").filter(|value| !value.is_null()) {
        if let Some(tools) = tools.as_array() {
            for tool in tools {
                if tool
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| !matches!(kind, "function" | "custom"))
                {
                    facts.qualified = false;
                }
                for (field, argument) in [("function", "arguments"), ("custom", "input")] {
                    if let Some(value) = tool.get(field) {
                        observe_tool(value, argument, facts);
                    }
                }
            }
        } else {
            facts.qualified = false;
        }
    }
    if let Some(audio) = body.get("audio").filter(|value| !value.is_null()) {
        if audio.is_object() {
            for key in ["data", "transcript"] {
                let generated = fragment(&audio[key], facts);
                facts.generation |= generated;
                facts.audio |= generated;
            }
        } else {
            facts.qualified = false;
        }
    }
    for (key, value) in body {
        if !CONTENT_FIELDS.iter().any(|(field, _)| *field == key)
            && !matches!(
                key.as_str(),
                "role"
                    | "function_call"
                    | "tool_calls"
                    | "audio"
                    | "annotations"
                    | "content_filter_results"
                    | "content_filter_offsets"
                    | "offsets"
                    | "native_finish_reason"
                    | "provider"
                    | "obfuscation"
            )
            && nonempty(value)
        {
            facts.qualified = false;
        }
    }
}

fn observe_text(value: &Value, kind: &Kind, facts: &mut GeneratedContent) {
    match value {
        Value::Null => {},
        Value::String(text) => {
            if !text.is_empty() {
                facts.generation = true;
                facts.text |= *kind == Kind::Text;
                facts.reasoning |= *kind == Kind::Reasoning;
            }
        },
        Value::Array(blocks) => {
            for block in blocks {
                let Some((kind, leaf)) =
                    super::block_kind(block.get("type").and_then(Value::as_str), kind)
                else {
                    if nonempty(block) {
                        facts.qualified = false;
                    }
                    continue;
                };
                if let Some(value) = block.get(leaf) {
                    observe_text(value, &kind, facts);
                }
            }
        },
        _ => facts.qualified = false,
    }
}

fn observe_tool(value: &Value, argument: &str, facts: &mut GeneratedContent) {
    if value.is_null() {
        return;
    }
    if value.is_object() {
        for key in ["name", argument] {
            facts.generation |= fragment(&value[key], facts);
        }
    } else {
        facts.qualified = false;
    }
}

fn fragment(value: &Value, facts: &mut GeneratedContent) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        _ => {
            facts.qualified = false;
            false
        },
    }
}

fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests;
