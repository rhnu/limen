use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::{
    Error,
    chunk::{index, known_keys, object, validate_tail},
    validate_chunk,
};

pub(crate) mod fields;

const ROOT_KEYS: &[&str] = &[
    "id",
    "object",
    "created",
    "model",
    "choices",
    "usage",
    "system_fingerprint",
    "service_tier",
    "provider",
    "native_finish_reason",
    "annotations",
    "prompt_annotations",
    "prompt_filter_results",
    "content_filter_results",
    "content_filter_offsets",
    "offsets",
    "obfuscation",
];

pub(crate) const CHOICE_METADATA: &[&str] = &[
    "annotations",
    "content_filter_results",
    "content_filter_offsets",
    "offsets",
    "native_finish_reason",
    "provider",
];

pub(crate) fn conversion_root(value: &Value) -> Result<(), Error> {
    known_keys(object(value)?, ROOT_KEYS)
}

#[derive(Default)]
struct Choice {
    message: Map<String, Value>,
    metadata: Map<String, Value>,
    finish: Option<Value>,
    generated: bool,
}

/// Accumulates understood Chat fields without retaining an event history.
pub struct Aggregator {
    root: Map<String, Value>,
    // Cumulative input bytes bound both distinct choices and accumulated fields.
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

    /// Adds one chunk; an error permanently rejects further aggregation.
    ///
    /// # Errors
    /// Returns protocol, unsupported-conversion, metadata or resource-limit errors.
    pub fn push(&mut self, chunk: Value) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        let result = self.push_inner(&chunk);
        drop(chunk);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, chunk: &Value) -> Result<(), Error> {
        if self.events >= self.max_events {
            return Err(Error::AggregateEvents);
        }
        let remaining = self
            .max_bytes
            .checked_sub(self.bytes)
            .ok_or(Error::AggregateBytes)?;
        let size = crate::bounded::json_size(chunk, remaining)?.ok_or(Error::AggregateBytes)?;
        validate_chunk(chunk)?;
        let choices = chunk.get("choices").and_then(Value::as_array);
        // Completion violations take precedence over conflicts in the same event.
        for source in choices.into_iter().flatten() {
            if let Some(finish) = self
                .choices
                .get(&index(&source["index"])?)
                .and_then(|c| c.finish.as_ref())
                .and_then(Value::as_str)
            {
                validate_tail(chunk, source, finish)?;
            }
        }
        conversion_root(chunk)?;
        self.bytes += size;
        self.events += 1;
        let auxiliary = choices.into_iter().flatten().all(fields::auxiliary_choice);
        for (key, value) in object(chunk)? {
            match key.as_str() {
                "object" | "choices" | "obfuscation" => {},
                "usage" => {
                    fields::merge_usage(&mut self.root, value)?;
                },
                "id" | "model" if auxiliary && value.as_str() == Some("") => {},
                "created" if auxiliary && value.as_u64() == Some(0) => {},
                _ => fields::merge_metadata(&mut self.root, key, value.clone())?,
            }
        }
        for source in choices.into_iter().flatten() {
            self.push_choice(source)?;
        }
        Ok(())
    }

    fn push_choice(&mut self, source: &Value) -> Result<(), Error> {
        let generated = crate::chunk::generation_choice(source);
        let source = object(source)?;
        let mut keys = vec!["index", "delta", "finish_reason", "logprobs", "obfuscation"];
        keys.extend_from_slice(CHOICE_METADATA);
        known_keys(source, &keys)?;
        let target = self.choices.entry(index(&source["index"])?).or_default();
        if let Some(delta) = source.get("delta") {
            let delta = object(delta)?;
            if target.finish.is_some() {
                fields::merge_tail_message(&mut target.message, delta)?;
            } else {
                fields::merge_message(&mut target.message, delta)?;
            }
        }
        target.generated |= generated;
        if let Some(logprobs) = source.get("logprobs") {
            fields::merge_logprobs(&mut target.metadata, logprobs)?;
        }
        for key in CHOICE_METADATA {
            if let Some(value) = source.get(*key) {
                fields::merge_metadata(&mut target.metadata, key, value.clone())?;
            }
        }
        if let Some(finish) = source.get("finish_reason").filter(|value| !value.is_null()) {
            target.finish = Some(finish.clone());
            target.generated = true;
        }
        Ok(())
    }

    /// Builds a complete response after the caller has observed upstream DONE.
    ///
    /// # Errors
    /// Rejects failed aggregation, unfinished choices, incomplete fields or oversized output.
    pub fn finish(mut self) -> Result<Value, Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        if self.choices.is_empty()
            || self
                .choices
                .values()
                .any(|choice| choice.generated && choice.finish.is_none())
        {
            return Err(Error::UnfinishedChoices);
        }
        let mut choices = Vec::with_capacity(self.choices.len());
        for (index, mut choice) in self.choices {
            if !choice.generated {
                return Err(Error::Unsupported);
            }
            choice
                .message
                .entry("role")
                .or_insert_with(|| json!("assistant"));
            fields::finish_message(&mut choice.message)?;
            let mut output = choice.metadata;
            output.insert("index".into(), index.into());
            output.insert("message".into(), Value::Object(choice.message));
            output.insert(
                "finish_reason".into(),
                choice.finish.ok_or(Error::UnfinishedChoices)?,
            );
            choices.push(Value::Object(output));
        }
        self.root.insert("object".into(), "chat.completion".into());
        self.root.insert("choices".into(), Value::Array(choices));
        let output = Value::Object(self.root);
        crate::bounded::json_size(&output, self.max_bytes)?.ok_or(Error::AggregateBytes)?;
        Ok(output)
    }
}

pub(crate) use fields::validate_complete_tool;

#[cfg(test)]
mod tests;
