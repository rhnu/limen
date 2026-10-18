use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::Error;

pub(crate) fn object(value: &Value) -> Result<&Map<String, Value>, Error> {
    value.as_object().ok_or(Error::InvalidChunk)
}

pub(crate) fn index(value: &Value) -> Result<u64, Error> {
    value.as_u64().ok_or(Error::InvalidChunk)
}

pub(crate) fn known_keys(object: &Map<String, Value>, keys: &[&str]) -> Result<(), Error> {
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        Err(Error::Unsupported)
    } else {
        Ok(())
    }
}

fn optional_string(object: &Map<String, Value>, key: &str, null: bool) -> Result<(), Error> {
    if object
        .get(key)
        .is_some_and(|v| !(v.is_string() || null && v.is_null()))
    {
        Err(Error::InvalidChunk)
    } else {
        Ok(())
    }
}

/// Checks understood fields while retaining arbitrary same-shape extension fields.
///
/// # Errors
/// Returns [`Error::UpstreamError`] for error envelopes, or [`Error::InvalidChunk`]
/// for malformed standard fields or duplicate indices.
pub fn validate_chunk(value: &Value) -> Result<(), Error> {
    let root = object(value)?;
    if root.get("error").is_some_and(|error| !error.is_null()) {
        return Err(Error::UpstreamError);
    }
    for key in ["object", "id", "model"] {
        optional_string(root, key, false)?;
    }
    for key in ["system_fingerprint", "service_tier"] {
        optional_string(root, key, true)?;
    }
    if root.get("created").is_some_and(|value| !value.is_u64()) {
        return Err(Error::InvalidChunk);
    }
    let Some(choices) = root.get("choices") else {
        return Ok(());
    };
    let choices = choices.as_array().ok_or(Error::InvalidChunk)?;
    let mut seen = BTreeSet::new();
    for choice in choices {
        let choice = object(choice)?;
        if !seen.insert(index(choice.get("index").ok_or(Error::InvalidChunk)?)?) {
            return Err(Error::InvalidChunk);
        }
        optional_string(choice, "finish_reason", true)?;
        let Some(delta) = choice.get("delta") else {
            continue;
        };
        let delta = object(delta)?;
        optional_string(delta, "role", false)?;
        if delta.get("content").is_some_and(|value| {
            !(value.is_null()
                || value.is_string()
                || value
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().all(Value::is_object)))
        }) {
            return Err(Error::InvalidChunk);
        }
        optional_string(delta, "refusal", true)?;
        if let Some(calls) = delta.get("tool_calls") {
            let calls = calls.as_array().ok_or(Error::InvalidChunk)?;
            let mut seen = BTreeSet::new();
            for call in calls {
                let call = object(call)?;
                if !seen.insert(index(call.get("index").ok_or(Error::InvalidChunk)?)?) {
                    return Err(Error::InvalidChunk);
                }
                optional_string(call, "id", true)?;
                optional_string(call, "type", true)?;
                if let Some(function) = call.get("function") {
                    let function = object(function)?;
                    optional_string(function, "name", true)?;
                    optional_string(function, "arguments", true)?;
                }
            }
        }
    }
    Ok(())
}

/// Trailing metadata may repeat a completion, but cannot add known generated data.
pub(crate) fn validate_tail(_chunk: &Value, choice: &Value, finish: &str) -> Result<(), Error> {
    if choice["finish_reason"]
        .as_str()
        .is_some_and(|value| value != finish)
    {
        return Err(Error::ChoiceFinished);
    }
    let Some(delta) = choice.get("delta") else {
        return Ok(());
    };
    let delta = object(delta)?;
    for key in [
        "content",
        "refusal",
        "tool_calls",
        "function_call",
        "reasoning",
        "reasoning_content",
        "reasoning_details",
    ] {
        if delta.get(key).is_some_and(has_content) {
            return Err(Error::ChoiceFinished);
        }
    }
    if let Some(audio) = delta.get("audio").and_then(Value::as_object) {
        for key in ["data", "transcript"] {
            if audio.get(key).is_some_and(has_content) {
                return Err(Error::ChoiceFinished);
            }
        }
    }
    Ok(())
}

fn has_content(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        _ => true,
    }
}

/// Only generation-bearing choices require a finish marker; scoped metadata does not.
pub(crate) fn generation_choice(choice: &Value) -> bool {
    if choice
        .get("finish_reason")
        .is_some_and(|value| !value.is_null())
        || choice.get("logprobs").is_some_and(has_content)
    {
        return true;
    }
    let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
        return false;
    };
    delta.is_empty()
        || delta.iter().any(|(key, value)| match key.as_str() {
            "annotations"
            | "content_filter_results"
            | "content_filter_offsets"
            | "offsets"
            | "native_finish_reason"
            | "provider"
            | "obfuscation" => false,
            "audio" => value
                .as_object()
                .is_none_or(|audio| audio.contains_key("data") || audio.contains_key("transcript")),
            _ => true,
        })
}

/// Bounded per-choice completion state, independent of transfer and aggregation.
pub struct ChunkTracker {
    choices: BTreeMap<u64, Option<String>>,
    finishes: FinishCounts,
    max_choices: usize,
    failed: bool,
}

impl ChunkTracker {
    pub fn finish_counts(&self) -> FinishCounts { self.finishes }

    pub fn choice_count(&self) -> usize { self.choices.len() }

    pub fn new(max_choices: usize) -> Self {
        Self {
            choices: BTreeMap::new(),
            finishes: FinishCounts::default(),
            max_choices,
            failed: false,
        }
    }

    /// Records generation completion while allowing auxiliary and trailing metadata.
    ///
    /// # Errors
    /// Rejects malformed chunks, additional data for finished choices or excess choices.
    pub fn push(&mut self, chunk: &Value) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        let result = self.push_inner(chunk);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, chunk: &Value) -> Result<(), Error> {
        validate_chunk(chunk)?;
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return Ok(());
        };
        for choice in choices {
            let index = index(&choice["index"])?;
            if let Some(Some(finish)) = self.choices.get(&index) {
                validate_tail(chunk, choice, finish)?;
                continue;
            }
            if !generation_choice(choice) {
                continue;
            }
            if !self.choices.contains_key(&index) && self.choices.len() >= self.max_choices {
                return Err(Error::ChoiceLimit);
            }
            let finish = choice["finish_reason"].as_str();
            self.finishes.observe(finish);
            self.choices.insert(index, finish.map(str::to_owned));
        }
        Ok(())
    }

    /// Confirms every observed choice has a finish reason.
    ///
    /// # Errors
    /// Returns [`Error::UnfinishedChoices`] for empty streams or unfinished choices.
    pub fn finish(self) -> Result<(), Error> {
        if self.failed {
            Err(Error::InvalidChunk)
        } else if self.choices.is_empty() || self.choices.values().any(Option::is_none) {
            Err(Error::UnfinishedChoices)
        } else {
            Ok(())
        }
    }
}

/// Fixed finish categories contain no provider-specific reason strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinishCounts {
    pub stop: u64,
    pub length: u64,
    pub tool: u64,
    pub filtered: u64,
    pub other: u64,
}

impl FinishCounts {
    /// Complete JSON is an open object; absent or malformed fields stay unobserved.
    pub fn from_response(value: &Value) -> Self {
        let mut counts = Self::default();
        if let Some(choices) = value.get("choices").and_then(Value::as_array) {
            for choice in choices {
                counts.observe(choice.get("finish_reason").and_then(Value::as_str));
            }
        }
        counts
    }

    fn observe(&mut self, reason: Option<&str>) {
        let Some(reason) = reason else {
            return;
        };
        let count = match reason {
            "stop" => &mut self.stop,
            "length" => &mut self.length,
            "tool_calls" | "function_call" => &mut self.tool,
            "content_filter" => &mut self.filtered,
            _ => &mut self.other,
        };
        *count = count.saturating_add(1);
    }
}

#[cfg(test)]
mod tests;
