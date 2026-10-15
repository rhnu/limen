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
/// Returns [`Error::InvalidChunk`] for malformed standard fields or duplicate indices.
pub fn validate_chunk(value: &Value) -> Result<(), Error> {
    let root = object(value)?;
    if root.get("object").and_then(Value::as_str) != Some("chat.completion.chunk")
        || !root.get("id").is_some_and(Value::is_string)
        || !root.get("model").is_some_and(Value::is_string)
        || !root.get("created").is_some_and(Value::is_u64)
    {
        return Err(Error::InvalidChunk);
    }
    if root
        .get("usage")
        .is_some_and(|v| !v.is_object() && !v.is_null())
    {
        return Err(Error::InvalidChunk);
    }
    let choices = root
        .get("choices")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidChunk)?;
    let mut seen = BTreeSet::new();
    for choice in choices {
        let choice = object(choice)?;
        if !seen.insert(index(choice.get("index").ok_or(Error::InvalidChunk)?)?) {
            return Err(Error::InvalidChunk);
        }
        let finish = choice.get("finish_reason").ok_or(Error::InvalidChunk)?;
        if !finish.is_null() && !finish.is_string() {
            return Err(Error::InvalidChunk);
        }
        let delta = object(choice.get("delta").ok_or(Error::InvalidChunk)?)?;
        optional_string(delta, "role", false)?;
        optional_string(delta, "content", true)?;
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

/// Bounded per-choice completion state, independent of transfer and aggregation.
pub struct ChunkTracker {
    choices: BTreeMap<u64, bool>,
    max_choices: usize,
}

impl ChunkTracker {
    pub fn new(max_choices: usize) -> Self {
        Self {
            choices: BTreeMap::new(),
            max_choices,
        }
    }

    /// Records each choice's completion state while allowing trailing usage chunks.
    ///
    /// # Errors
    /// Rejects malformed chunks, additional data for finished choices or excess choices.
    pub fn push(&mut self, chunk: &Value) -> Result<(), Error> {
        validate_chunk(chunk)?;
        let choices = chunk["choices"].as_array().ok_or(Error::InvalidChunk)?;
        for choice in choices {
            let index = index(&choice["index"])?;
            if self.choices.get(&index) == Some(&true) {
                return Err(Error::ChoiceFinished);
            }
            if !self.choices.contains_key(&index) && self.choices.len() >= self.max_choices {
                return Err(Error::ChoiceLimit);
            }
            self.choices
                .insert(index, !choice["finish_reason"].is_null());
        }
        Ok(())
    }

    /// Confirms every observed choice has a finish reason.
    ///
    /// # Errors
    /// Returns [`Error::UnfinishedChoices`] for empty streams or unfinished choices.
    pub fn finish(self) -> Result<(), Error> {
        if self.choices.is_empty() || self.choices.values().any(|finished| !finished) {
            Err(Error::UnfinishedChoices)
        } else {
            Ok(())
        }
    }
}
