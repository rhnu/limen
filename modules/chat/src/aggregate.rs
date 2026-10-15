use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::{
    Error,
    chunk::{index, known_keys, object},
    validate_chunk,
};

const ROOT_KEYS: &[&str] = &[
    "id",
    "object",
    "created",
    "model",
    "choices",
    "usage",
    "system_fingerprint",
    "service_tier",
];

pub(crate) fn conversion_root(value: &Value) -> Result<(), Error> {
    known_keys(object(value)?, ROOT_KEYS)
}

#[derive(Default)]
struct Choice {
    role: Option<Value>,
    content: Option<Value>,
    refusal: Option<Value>,
    tools: BTreeMap<u64, Value>,
    tools_present: bool,
    finish: Option<Value>,
    logprobs: Option<Value>,
}

/// Accumulates supported Chat deltas, with cumulative input and result byte bounds.
pub struct Aggregator {
    root: Map<String, Value>,
    // Each distinct choice consumes input JSON bytes, so the cumulative byte cap also bounds this map.
    choices: BTreeMap<u64, Choice>,
    bytes: usize,
    events: usize,
    max_bytes: usize,
    max_events: usize,
    failed: bool,
}

impl Aggregator {
    pub fn new(max_bytes: usize, max_events: usize) -> Self {
        Self {
            root: Map::new(),
            choices: BTreeMap::new(),
            bytes: 0,
            events: 0,
            max_bytes,
            max_events,
            failed: false,
        }
    }

    /// Adds one chunk; a failed push permanently rejects further aggregation.
    ///
    /// # Errors
    /// Returns protocol, unsupported-conversion, metadata or resource-limit errors.
    pub fn push(&mut self, chunk: Value) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        let result = self.push_inner(chunk);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, chunk: Value) -> Result<(), Error> {
        if self.events >= self.max_events {
            return Err(Error::AggregateEvents);
        }
        let remaining = self
            .max_bytes
            .checked_sub(self.bytes)
            .ok_or(Error::AggregateBytes)?;
        let len = crate::bounded::json_size(&chunk, remaining)?.ok_or(Error::AggregateBytes)?;
        let bytes = self.bytes + len;
        conversion_root(&chunk)?;
        validate_chunk(&chunk)?;
        // Check the whole event before merging fields, preserving the completion
        // error even when the same event also contains conflicting metadata.
        for choice in chunk["choices"].as_array().ok_or(Error::InvalidChunk)? {
            let index = index(&choice["index"])?;
            if self
                .choices
                .get(&index)
                .is_some_and(|choice| choice.finish.is_some())
            {
                return Err(Error::ChoiceFinished);
            }
        }
        self.events += 1;
        self.bytes = bytes;
        let Value::Object(mut root) = chunk else {
            return Err(Error::InvalidChunk);
        };
        let choices = root.remove("choices").ok_or(Error::InvalidChunk)?;
        root.remove("object");
        for (key, value) in root {
            merge_metadata(&mut self.root, &key, value)?;
        }
        for choice in choices.as_array().ok_or(Error::InvalidChunk)? {
            self.push_choice(object(choice)?)?;
        }
        Ok(())
    }

    fn push_choice(&mut self, source: &Map<String, Value>) -> Result<(), Error> {
        known_keys(source, &["index", "delta", "finish_reason", "logprobs"])?;
        let choice_index = index(&source["index"])?;
        let target = self.choices.entry(choice_index).or_default();
        if let Some(value) = source.get("logprobs") {
            if !value.is_null() {
                return Err(Error::Unsupported);
            }
            target.logprobs = Some(value.clone());
        }
        let delta = object(&source["delta"])?;
        known_keys(delta, &["role", "content", "refusal", "tool_calls"])?;
        if let Some(role) = delta.get("role") {
            if target.role.as_ref().is_some_and(|v| v != role) {
                return Err(Error::ConflictingMetadata);
            }
            target.role = Some(role.clone());
        }
        append_nullable(&mut target.content, delta.get("content"))?;
        append_nullable(&mut target.refusal, delta.get("refusal"))?;
        if let Some(calls) = delta.get("tool_calls") {
            target.tools_present = true;
            for call in calls.as_array().ok_or(Error::InvalidChunk)? {
                let call = object(call)?;
                known_keys(call, &["index", "id", "type", "function"])?;
                let index = index(&call["index"])?;
                let target_call = target.tools.entry(index).or_insert_with(|| json!({}));
                let target_call = target_call.as_object_mut().ok_or(Error::InvalidChunk)?;
                append_field(target_call, "id", call.get("id"))?;
                if let Some(kind) = call.get("type") {
                    if kind.as_str() != Some("function") {
                        return Err(Error::Unsupported);
                    }
                    merge_metadata(target_call, "type", kind.clone())?;
                }
                if let Some(function) = call.get("function") {
                    let function = object(function)?;
                    known_keys(function, &["name", "arguments"])?;
                    let target_fn = target_call.entry("function").or_insert_with(|| json!({}));
                    let target_fn = target_fn.as_object_mut().ok_or(Error::InvalidChunk)?;
                    append_field(target_fn, "name", function.get("name"))?;
                    append_field(target_fn, "arguments", function.get("arguments"))?;
                }
            }
        }
        if !source["finish_reason"].is_null() {
            target.finish = Some(source["finish_reason"].clone());
        }
        Ok(())
    }

    /// Builds a complete response after the caller has observed upstream DONE.
    ///
    /// # Errors
    /// Rejects failed aggregation, unfinished choices, incomplete tools or oversized output.
    pub fn finish(mut self) -> Result<Value, Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        if self.choices.is_empty() || self.choices.values().any(|choice| choice.finish.is_none()) {
            return Err(Error::UnfinishedChoices);
        }
        let mut choices = Vec::with_capacity(self.choices.len());
        for (index, choice) in self.choices {
            let mut message = Map::new();
            let role = choice.role.ok_or(Error::InvalidChunk)?;
            message.insert("role".into(), role);
            if let Some(content) = choice.content {
                message.insert("content".into(), content);
            }
            if let Some(refusal) = choice.refusal {
                message.insert("refusal".into(), refusal);
            }
            if choice.tools_present {
                let mut tools = Vec::with_capacity(choice.tools.len());
                for tool in choice.tools.into_values() {
                    validate_complete_tool(&tool)?;
                    tools.push(tool);
                }
                message.insert("tool_calls".into(), Value::Array(tools));
            }
            let mut output = json!({"index": index, "message": message,
                "finish_reason": choice.finish.ok_or(Error::UnfinishedChoices)?});
            if let Some(logprobs) = choice.logprobs {
                output["logprobs"] = logprobs;
            }
            choices.push(output);
        }
        self.root.insert("object".into(), "chat.completion".into());
        self.root.insert("choices".into(), Value::Array(choices));
        let output = Value::Object(self.root);
        crate::bounded::json_size(&output, self.max_bytes)?.ok_or(Error::AggregateBytes)?;
        Ok(output)
    }
}

pub(crate) fn validate_complete_tool(tool: &Value) -> Result<(), Error> {
    let tool = object(tool)?;
    known_keys(tool, &["id", "type", "function"])?;
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return Err(Error::Unsupported);
    }
    if !tool.get("id").is_some_and(Value::is_string) {
        return Err(Error::InvalidChunk);
    }
    let function = object(tool.get("function").ok_or(Error::InvalidChunk)?)?;
    known_keys(function, &["name", "arguments"])?;
    if !function.get("name").is_some_and(Value::is_string)
        || !function.get("arguments").is_some_and(Value::is_string)
    {
        return Err(Error::InvalidChunk);
    }
    Ok(())
}

fn merge_metadata(target: &mut Map<String, Value>, key: &str, value: Value) -> Result<(), Error> {
    if let Some(previous) = target.get(key) {
        if !previous.is_null() && !value.is_null() && previous != &value {
            return Err(Error::ConflictingMetadata);
        }
        if value.is_null() && !previous.is_null() {
            return Ok(());
        }
    }
    target.insert(key.to_owned(), value);
    Ok(())
}

fn append_nullable(target: &mut Option<Value>, source: Option<&Value>) -> Result<(), Error> {
    let Some(source) = source else { return Ok(()) };
    if source.is_null() {
        if target.is_none() {
            *target = Some(Value::Null);
        }
        return Ok(());
    }
    let fragment = source.as_str().ok_or(Error::Unsupported)?;
    let value = target.get_or_insert(Value::Null);
    if value.is_null() {
        *value = Value::String(String::new());
    }
    let Value::String(text) = value else {
        return Err(Error::Unsupported);
    };
    text.push_str(fragment);
    Ok(())
}

fn append_field(
    target: &mut Map<String, Value>,
    key: &str,
    source: Option<&Value>,
) -> Result<(), Error> {
    if let Some(source) = source {
        let fragment = source.as_str().ok_or(Error::Unsupported)?;
        let value = target
            .entry(key)
            .or_insert_with(|| Value::String(String::new()));
        let Value::String(text) = value else {
            return Err(Error::Unsupported);
        };
        text.push_str(fragment);
    }
    Ok(())
}
