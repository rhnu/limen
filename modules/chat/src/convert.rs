use std::collections::{BTreeSet, VecDeque};

use serde_json::{Map, Value, json};

use crate::{
    Error,
    aggregate::{CHOICE_METADATA, conversion_root, fields, validate_complete_tool},
    chunk::{index, known_keys, object},
    sse::chunk_size,
};

#[derive(Clone)]
struct Task {
    choice: Option<Value>,
    root: Map<String, Value>,
}

impl Task {
    fn delta(index: u64, delta: Value) -> Self {
        let mut choice = json!({"index":index,"finish_reason":null});
        choice["delta"] = delta;
        Self {
            choice: Some(choice),
            root: Map::new(),
        }
    }

    fn choice(index: u64, key: &str, value: Value) -> Self {
        let mut choice = json!({"index":index});
        choice[key] = value;
        Self {
            choice: Some(choice),
            root: Map::new(),
        }
    }

    fn root(key: &str, value: Value) -> Self {
        Self {
            choice: None,
            root: Map::from_iter([(key.into(), value)]),
        }
    }
}

/// Owns a complete response and lazily yields bounded semantic fragments.
pub struct JsonChunks {
    metadata: Map<String, Value>,
    tasks: VecDeque<Task>,
    limit: usize,
    failed: bool,
}

impl JsonChunks {
    /// Validates understood response fields and prepares incremental output.
    ///
    /// # Errors
    /// Rejects malformed or unsupported fields. Indivisible oversized fields fail
    /// during iteration; no field is discarded to satisfy the event byte bound.
    pub fn new(response: Value, max_event_bytes: usize) -> Result<Self, Error> {
        conversion_root(&response)?;
        if response["object"].as_str() != Some("chat.completion")
            || !response["id"].is_string()
            || !response["model"].is_string()
            || !response["created"].is_u64()
        {
            return Err(Error::InvalidChunk);
        }
        if response
            .get("usage")
            .is_some_and(|v| !v.is_null() && !v.is_object())
        {
            return Err(Error::InvalidChunk);
        }
        if let Some(usage) = response.get("usage") {
            fields::merge_usage(&mut Map::new(), usage)?;
        }
        let Value::Object(mut source) = response else {
            return Err(Error::InvalidChunk);
        };
        let choices = source.remove("choices").ok_or(Error::InvalidChunk)?;
        let choices = choices.as_array().ok_or(Error::InvalidChunk)?;
        if choices.is_empty() {
            return Err(Error::InvalidChunk);
        }
        let usage = source.remove("usage");
        let mut metadata = Map::new();
        for key in ["id", "model", "created"] {
            metadata.insert(key.into(), source.remove(key).ok_or(Error::InvalidChunk)?);
        }
        source.remove("object");
        source.remove("obfuscation");
        metadata.insert("object".into(), "chat.completion.chunk".into());
        let mut tasks = VecDeque::new();
        // Root annotations are emitted once; repeating them in every generated
        // event would multiply annotations when those events are aggregated.
        for (key, value) in source {
            let mut validated = Map::new();
            fields::merge_metadata(&mut validated, &key, value.clone())?;
            if matches!(
                key.as_str(),
                "annotations" | "prompt_annotations" | "prompt_filter_results"
            ) {
                tasks.push_back(Task::root(&key, value));
            } else {
                metadata.insert(key, value);
            }
        }
        let mut seen = BTreeSet::new();
        for choice in choices {
            let choice = object(choice)?;
            let mut keys = vec![
                "index",
                "message",
                "finish_reason",
                "logprobs",
                "obfuscation",
            ];
            keys.extend_from_slice(CHOICE_METADATA);
            known_keys(choice, &keys)?;
            let index = index(choice.get("index").ok_or(Error::InvalidChunk)?)?;
            if !seen.insert(index) {
                return Err(Error::InvalidChunk);
            }
            push_message_tasks(
                &mut tasks,
                index,
                choice.get("message").ok_or(Error::InvalidChunk)?,
            )?;
            if let Some(logprobs) = choice.get("logprobs") {
                fields::merge_logprobs(&mut Map::new(), logprobs)?;
                if !logprobs.is_null() {
                    tasks.push_back(Task::choice(index, "logprobs", logprobs.clone()));
                }
            }
            for key in CHOICE_METADATA {
                if let Some(value) = choice.get(*key) {
                    fields::merge_metadata(&mut Map::new(), key, value.clone())?;
                    tasks.push_back(Task::choice(index, key, value.clone()));
                }
            }
            let finish = choice
                .get("finish_reason")
                .filter(|v| v.is_string())
                .ok_or(Error::InvalidChunk)?;
            let mut terminal = Task::delta(index, json!({}));
            let terminal_choice = terminal.choice.as_mut().ok_or(Error::InvalidChunk)?;
            terminal_choice["finish_reason"] = finish.clone();
            if choice.get("logprobs").is_some_and(Value::is_null) {
                terminal_choice["logprobs"] = Value::Null;
            }
            tasks.push_back(terminal);
        }
        if let Some(usage) = usage {
            tasks.push_back(Task::root("usage", usage));
        }
        Ok(Self {
            metadata,
            tasks,
            limit: max_event_bytes,
            failed: false,
        })
    }

    fn next_chunk(&mut self) -> Result<Option<Value>, Error> {
        while let Some(task) = self.tasks.pop_front() {
            let mut chunk = self.metadata.clone();
            chunk.extend(task.root.clone());
            chunk.insert(
                "choices".into(),
                Value::Array(task.choice.iter().cloned().collect()),
            );
            let chunk = Value::Object(chunk);
            match chunk_size(&chunk, self.limit) {
                Ok(_) => return Ok(Some(chunk)),
                Err(Error::EventLimit) => {
                    let (first, second) = split_task(&task).ok_or(Error::EventLimit)?;
                    self.tasks.push_front(second);
                    self.tasks.push_front(first);
                },
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }
}

fn push_message_tasks(
    tasks: &mut VecDeque<Task>,
    index: u64,
    message: &Value,
) -> Result<(), Error> {
    fields::validate_message(message)?;
    let message = object(message)?;
    // Preserve the established synthetic Chat ordering observed by clients and
    // outgoing hooks: role, text/refusal, then the remaining semantic fields.
    let ordered = ["role", "content", "refusal"]
        .into_iter()
        .filter_map(|key| message.get_key_value(key))
        .chain(
            message
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "role" | "content" | "refusal")),
        );
    for (key, value) in ordered {
        if key == "obfuscation" {
            continue;
        }
        if key != "tool_calls" {
            let mut delta = Map::new();
            delta.insert(key.clone(), value.clone());
            tasks.push_back(Task::delta(index, Value::Object(delta)));
            continue;
        }
        let tools = value.as_array().ok_or(Error::Unsupported)?;
        if tools.is_empty() {
            tasks.push_back(Task::delta(index, json!({"tool_calls":[]})));
        }
        for (tool_index, tool) in tools.iter().enumerate() {
            validate_complete_tool(tool)?;
            for key in ["id", "type"] {
                let mut call = json!({"index":tool_index});
                call[key] = tool[key].clone();
                tasks.push_back(Task::delta(index, json!({"tool_calls":[call]})));
            }
            let (kind, argument) = if tool["type"] == "custom" {
                ("custom", "input")
            } else {
                ("function", "arguments")
            };
            for key in ["name", argument] {
                let mut call = json!({"index":tool_index});
                call[kind] = json!({});
                call[kind][key] = tool[kind][key].clone();
                tasks.push_back(Task::delta(index, json!({"tool_calls":[call]})));
            }
        }
    }
    Ok(())
}

impl Iterator for JsonChunks {
    type Item = Result<Value, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.next_chunk() {
            Ok(Some(chunk)) => Some(Ok(chunk)),
            Ok(None) => None,
            Err(error) => {
                self.failed = true;
                self.tasks.clear();
                Some(Err(error))
            },
        }
    }
}

fn split_at_pointer(task: Task, pointer: &str, root: bool, base64: bool) -> Option<(Task, Task)> {
    let mut first = task;
    let value = if root {
        Value::Object(first.root.clone())
    } else {
        first.choice.clone()?
    };
    let source = value.pointer(pointer)?;
    let (left, right) = if let Some(text) = source.as_str() {
        let middle = if base64 {
            // A base64 quantum must stay intact. Opaque signatures and encrypted
            // reasoning do not use this path and cannot be divided.
            if !text.is_ascii() {
                return None;
            }
            let middle = (text.len() / 8) * 4;
            if middle == 0 || middle >= text.len() {
                return None;
            }
            middle
        } else {
            let count = text.chars().count();
            if count < 2 {
                return None;
            }
            text.char_indices().nth(count / 2)?.0
        };
        (json!(&text[..middle]), json!(&text[middle..]))
    } else {
        let values = source.as_array()?;
        if values.len() < 2 {
            return None;
        }
        let middle = values.len() / 2;
        (json!(&values[..middle]), json!(&values[middle..]))
    };
    let mut second = first.clone();
    if root {
        let mut first_value = Value::Object(first.root);
        let mut second_value = Value::Object(second.root);
        *first_value.pointer_mut(pointer)? = left;
        *second_value.pointer_mut(pointer)? = right;
        let Value::Object(first_root) = first_value else {
            return None;
        };
        let Value::Object(second_root) = second_value else {
            return None;
        };
        first.root = first_root;
        second.root = second_root;
    } else {
        *first.choice.as_mut()?.pointer_mut(pointer)? = left;
        *second.choice.as_mut()?.pointer_mut(pointer)? = right;
    }
    Some((first, second))
}

fn split_task(task: &Task) -> Option<(Task, Task)> {
    if task.choice.is_none() {
        for key in ["annotations", "prompt_annotations", "prompt_filter_results"] {
            if let Some(split) = split_at_pointer(task.clone(), &format!("/{key}"), true, false) {
                return Some(split);
            }
        }
        return None;
    }
    // Separate append-bearing siblings before dividing a field. Otherwise a
    // split transcript could accidentally repeat the complete audio data.
    let choice = task.choice.as_ref()?;
    for pointer in ["/delta/audio", "/delta/function_call", "/logprobs"] {
        let Some(fields) = choice
            .pointer(pointer)
            .and_then(Value::as_object)
            .filter(|v| v.len() > 1)
        else {
            continue;
        };
        let mut first = task.clone();
        let mut second = task.clone();
        let middle = fields.len() / 2;
        *first.choice.as_mut()?.pointer_mut(pointer)? = Value::Object(
            fields
                .iter()
                .take(middle)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        *second.choice.as_mut()?.pointer_mut(pointer)? = Value::Object(
            fields
                .iter()
                .skip(middle)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        return Some((first, second));
    }
    for pointer in [
        "/delta/content",
        "/delta/refusal",
        "/delta/reasoning",
        "/delta/reasoning_content",
        "/delta/function_call/name",
        "/delta/function_call/arguments",
        "/delta/tool_calls/0/id",
        "/delta/tool_calls/0/function/name",
        "/delta/tool_calls/0/function/arguments",
        "/delta/tool_calls/0/custom/name",
        "/delta/tool_calls/0/custom/input",
        "/delta/audio/transcript",
        "/delta/reasoning_details",
        "/delta/annotations",
        "/annotations",
        "/logprobs/content",
        "/logprobs/refusal",
    ] {
        if let Some(split) = split_at_pointer(task.clone(), pointer, false, false) {
            return Some(split);
        }
    }
    if let Some(split) = split_at_pointer(task.clone(), "/delta/audio/data", false, true) {
        return Some(split);
    }
    split_reasoning_detail(task)
}

fn split_reasoning_detail(task: &Task) -> Option<(Task, Task)> {
    let choice = task.choice.as_ref()?;
    if let Some(detail) = choice.pointer("/delta/reasoning_details/0").filter(|_| {
        choice
            .pointer("/delta/reasoning_details")
            .and_then(Value::as_array)
            .is_some_and(|v| v.len() == 1)
    }) && (detail.get("id").is_some_and(Value::is_string)
        || detail.get("index").is_some_and(Value::is_u64))
    {
        let payloads: Vec<_> = [
            "text",
            "summary",
            "signature",
            "data",
            "encrypted",
            "encrypted_content",
        ]
        .into_iter()
        .filter(|key| detail.get(*key).is_some())
        .collect();
        if payloads.len() > 1 {
            let mut first = task.clone();
            let mut second = task.clone();
            for key in &payloads[1..] {
                first
                    .choice
                    .as_mut()?
                    .pointer_mut("/delta/reasoning_details/0")?
                    .as_object_mut()?
                    .remove(*key);
            }
            second
                .choice
                .as_mut()?
                .pointer_mut("/delta/reasoning_details/0")?
                .as_object_mut()?
                .remove(payloads[0]);
            return Some((first, second));
        }
        for pointer in [
            "/delta/reasoning_details/0/text",
            "/delta/reasoning_details/0/summary",
        ] {
            if let Some(split) = split_at_pointer(task.clone(), pointer, false, false) {
                return Some(split);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
