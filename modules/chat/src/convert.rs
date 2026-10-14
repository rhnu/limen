use std::collections::{BTreeSet, VecDeque};

use serde_json::{Map, Value, json};

use crate::{
    Error,
    aggregate::{conversion_root, validate_complete_tool},
    chunk::{index, known_keys, object},
    encode_chunk,
};

struct Task {
    index: u64,
    delta: Value,
    finish: Value,
    logprobs: Option<Value>,
}

/// Owns a complete response and yields one bounded chunk at a time.
/// Text and tool ID/name/arguments are split only at UTF-8 character boundaries.
pub struct JsonChunks {
    metadata: Map<String, Value>,
    tasks: VecDeque<Task>,
    usage: Option<Value>,
    limit: usize,
    failed: bool,
}

impl JsonChunks {
    /// Validates supported complete-response fields and prepares incremental output.
    ///
    /// # Errors
    /// Rejects malformed choices and unsupported structures instead of dropping fields.
    /// An individual event that cannot fit is reported later by the iterator.
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
        let Value::Object(mut metadata) = response else {
            return Err(Error::InvalidChunk);
        };
        let choices = metadata.remove("choices").ok_or(Error::InvalidChunk)?;
        let Value::Array(choices) = choices else {
            return Err(Error::InvalidChunk);
        };
        if choices.is_empty() {
            return Err(Error::InvalidChunk);
        }
        let usage = metadata.remove("usage");
        metadata.insert("object".into(), "chat.completion.chunk".into());
        let mut tasks = VecDeque::new();
        let mut seen = BTreeSet::new();
        for choice in choices {
            let choice = object(&choice)?;
            known_keys(choice, &["index", "message", "finish_reason", "logprobs"])?;
            let index = index(choice.get("index").ok_or(Error::InvalidChunk)?)?;
            if !seen.insert(index) {
                return Err(Error::InvalidChunk);
            }
            push_message_tasks(
                &mut tasks,
                index,
                choice.get("message").ok_or(Error::InvalidChunk)?,
            )?;
            let finish = choice
                .get("finish_reason")
                .filter(|v| v.is_string())
                .ok_or(Error::InvalidChunk)?
                .clone();
            let logprobs = choice.get("logprobs").cloned();
            if logprobs.as_ref().is_some_and(|v| !v.is_null()) {
                return Err(Error::Unsupported);
            }
            tasks.push_back(Task {
                index,
                delta: json!({}),
                finish,
                logprobs,
            });
        }
        Ok(Self {
            metadata,
            tasks,
            usage,
            limit: max_event_bytes,
            failed: false,
        })
    }

    fn next_chunk(&mut self) -> Result<Option<Value>, Error> {
        loop {
            if let Some(mut task) = self.tasks.pop_front() {
                let mut chunk = self.metadata.clone();
                let mut choice =
                    json!({"index": task.index, "delta": task.delta, "finish_reason": task.finish});
                if let Some(logprobs) = &task.logprobs {
                    choice["logprobs"] = logprobs.clone();
                }
                chunk.insert("choices".into(), json!([choice]));
                let chunk = Value::Object(chunk);
                match encode_chunk(&chunk, self.limit) {
                    Ok(_) => return Ok(Some(chunk)),
                    Err(Error::EventLimit) => {
                        let Some((first, second)) = split_delta(&mut task.delta) else {
                            return Err(Error::EventLimit);
                        };
                        self.tasks.push_front(Task {
                            index: task.index,
                            delta: second,
                            finish: Value::Null,
                            logprobs: None,
                        });
                        self.tasks.push_front(Task {
                            index: task.index,
                            delta: first,
                            finish: Value::Null,
                            logprobs: None,
                        });
                    },
                    Err(error) => return Err(error),
                }
            } else if let Some(usage) = self.usage.take() {
                let mut chunk = self.metadata.clone();
                chunk.insert("choices".into(), json!([]));
                chunk.insert("usage".into(), usage);
                let chunk = Value::Object(chunk);
                encode_chunk(&chunk, self.limit)?;
                return Ok(Some(chunk));
            } else {
                return Ok(None);
            }
        }
    }
}

fn push_message_tasks(
    tasks: &mut VecDeque<Task>,
    index: u64,
    message: &Value,
) -> Result<(), Error> {
    let message = object(message)?;
    known_keys(message, &["role", "content", "refusal", "tool_calls"])?;
    if !message.get("role").is_some_and(Value::is_string) {
        return Err(Error::InvalidChunk);
    }
    for key in ["role", "content", "refusal"] {
        if let Some(value) = message.get(key) {
            if !(value.is_string() || key != "role" && value.is_null()) {
                return Err(Error::Unsupported);
            }
            let mut delta = Map::new();
            delta.insert(key.to_owned(), value.clone());
            tasks.push_back(Task {
                index,
                delta: Value::Object(delta),
                finish: Value::Null,
                logprobs: None,
            });
        }
    }
    if let Some(tools) = message.get("tool_calls") {
        let tools = tools.as_array().ok_or(Error::Unsupported)?;
        if tools.is_empty() {
            tasks.push_back(Task {
                index,
                delta: json!({"tool_calls": []}),
                finish: Value::Null,
                logprobs: None,
            });
        }
        for (tool_index, tool) in tools.iter().enumerate() {
            validate_complete_tool(tool)?;
            for key in ["id", "type"] {
                let mut call = json!({"index": tool_index});
                call[key] = tool[key].clone();
                tasks.push_back(Task {
                    index,
                    delta: json!({"tool_calls": [call]}),
                    finish: Value::Null,
                    logprobs: None,
                });
            }
            for key in ["name", "arguments"] {
                let mut call = json!({"index": tool_index, "function": {}});
                call["function"][key] = tool["function"][key].clone();
                tasks.push_back(Task {
                    index,
                    delta: json!({"tool_calls": [call]}),
                    finish: Value::Null,
                    logprobs: None,
                });
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

fn split_delta(delta: &mut Value) -> Option<(Value, Value)> {
    let pointers = [
        "/content",
        "/refusal",
        "/tool_calls/0/id",
        "/tool_calls/0/function/name",
        "/tool_calls/0/function/arguments",
    ];
    for pointer in pointers {
        let Some(text) = delta.pointer(pointer).and_then(Value::as_str) else {
            continue;
        };
        let count = text.chars().count();
        if count < 2 {
            continue;
        }
        let middle = text.char_indices().nth(count / 2)?.0;
        let first_text = text[..middle].to_owned();
        let second_text = text[middle..].to_owned();
        let mut second = delta.clone();
        *delta.pointer_mut(pointer)? = first_text.into();
        *second.pointer_mut(pointer)? = second_text.into();
        return Some((delta.clone(), second));
    }
    None
}
