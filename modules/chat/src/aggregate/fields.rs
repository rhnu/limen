use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use serde_json::{Map, Value, json};

use crate::{
    Error,
    chunk::{index, known_keys, object},
};

const MESSAGE_KEYS: &[&str] = &[
    "role",
    "content",
    "refusal",
    "reasoning",
    "reasoning_content",
    "reasoning_details",
    "function_call",
    "tool_calls",
    "audio",
    "annotations",
    "content_filter_results",
    "content_filter_offsets",
    "offsets",
    "native_finish_reason",
    "provider",
    "obfuscation",
];

/// Metadata has explicit scope: annotation arrays append, fixed values agree.
pub(crate) fn merge_metadata(
    target: &mut Map<String, Value>,
    key: &str,
    value: Value,
) -> Result<(), Error> {
    if matches!(
        key,
        "annotations" | "prompt_annotations" | "prompt_filter_results"
    ) {
        return append_array(target, key, &value);
    }
    if let Some(previous) = target.get(key) {
        if value.is_null() && !previous.is_null() {
            return Ok(());
        }
        if !previous.is_null() && !value.is_null() && previous != &value {
            if matches!(
                key,
                "content_filter_results" | "content_filter_offsets" | "offsets"
            ) {
                // Distinct scoped reports cannot be flattened into one complete
                // field without losing their relation to earlier stream events.
                return Err(Error::Unsupported);
            }
            return Err(Error::ConflictingMetadata);
        }
    }
    target.insert(key.to_owned(), value);
    Ok(())
}

pub(crate) fn auxiliary_choice(choice: &Value) -> bool { !crate::chunk::generation_choice(choice) }

pub(crate) fn merge_tail_message(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
) -> Result<(), Error> {
    known_keys(source, MESSAGE_KEYS)?;
    let mut metadata: Map<String, Value> = source
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "audio"
                    | "annotations"
                    | "content_filter_results"
                    | "content_filter_offsets"
                    | "offsets"
                    | "native_finish_reason"
                    | "provider"
            )
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(audio) = metadata.get_mut("audio").and_then(Value::as_object_mut) {
        audio.remove("data");
        audio.remove("transcript");
    }
    merge_message(target, &metadata)
}

/// Standard usage counters and token detail counters are cumulative snapshots.
/// Unrecognized usage extensions remain opaque metadata rather than deep merges.
pub(crate) fn merge_usage(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    if nullable(target, "usage", value) {
        return Ok(());
    }
    let source = object(value)?;
    let destination = target.entry("usage").or_insert_with(|| json!({}));
    if destination.is_null() {
        *destination = json!({});
    }
    let destination = destination.as_object_mut().ok_or(Error::Unsupported)?;
    for (key, value) in source {
        match key.as_str() {
            "prompt_tokens" | "completion_tokens" | "total_tokens" | "input_tokens"
            | "output_tokens" => {
                update_counter(destination, key, value)?;
            },
            "prompt_tokens_details"
            | "completion_tokens_details"
            | "input_tokens_details"
            | "output_tokens_details" => {
                if nullable(destination, key, value) {
                    continue;
                }
                let details = object(value)?;
                let detail_target = destination.entry(key).or_insert_with(|| json!({}));
                if detail_target.is_null() {
                    *detail_target = json!({});
                }
                let detail_target = detail_target.as_object_mut().ok_or(Error::Unsupported)?;
                for (key, value) in details {
                    match key.as_str() {
                        "cached_tokens"
                        | "audio_tokens"
                        | "reasoning_tokens"
                        | "accepted_prediction_tokens"
                        | "rejected_prediction_tokens"
                        | "text_tokens"
                        | "image_tokens" => update_counter(detail_target, key, value)?,
                        _ => merge_metadata(detail_target, key, value.clone())?,
                    }
                }
            },
            _ => merge_metadata(destination, key, value.clone())?,
        }
    }
    Ok(())
}

fn update_counter(target: &mut Map<String, Value>, key: &str, value: &Value) -> Result<(), Error> {
    if nullable(target, key, value) {
        return Ok(());
    }
    let valid = value.as_u64().is_some()
        || value.as_f64().is_some_and(|count| {
            count.is_finite()
                && count >= 0.0
                && count.fract() == 0.0
                && count <= 9_007_199_254_740_991.0
        });
    if !valid {
        return Err(Error::Unsupported);
    }
    target.insert(key.into(), value.clone());
    Ok(())
}

fn nullable(target: &mut Map<String, Value>, key: &str, value: &Value) -> bool {
    if value.is_null() {
        target.entry(key).or_insert(Value::Null);
        true
    } else {
        false
    }
}

pub(crate) fn append_text(
    target: &mut Map<String, Value>,
    key: &str,
    value: &Value,
) -> Result<(), Error> {
    if nullable(target, key, value) {
        return Ok(());
    }
    let text = value.as_str().ok_or(Error::Unsupported)?;
    let target = target.entry(key).or_insert(Value::Null);
    if target.is_null() {
        *target = json!("");
    }
    target.as_str().ok_or(Error::Unsupported)?;
    if let Value::String(target) = target {
        target.push_str(text);
    }
    Ok(())
}

fn append_array(target: &mut Map<String, Value>, key: &str, value: &Value) -> Result<(), Error> {
    if nullable(target, key, value) {
        return Ok(());
    }
    let values = value.as_array().ok_or(Error::Unsupported)?;
    let target = target.entry(key).or_insert(Value::Null);
    if target.is_null() {
        *target = json!([]);
    }
    target
        .as_array_mut()
        .ok_or(Error::Unsupported)?
        .extend(values.iter().cloned());
    Ok(())
}

fn validate_content_block(value: &Value) -> Result<(), Error> {
    let block = object(value)?;
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            known_keys(block, &["type", "text", "annotations"])?;
            if !block.get("text").is_some_and(Value::is_string) {
                return Err(Error::Unsupported);
            }
            if block
                .get("annotations")
                .is_some_and(|v| !v.is_array() && !v.is_null())
            {
                return Err(Error::Unsupported);
            }
        },
        Some("thinking") => {
            known_keys(block, &["type", "thinking", "signature"])?;
            match block.get("thinking") {
                Some(Value::String(_)) => {},
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        validate_content_block(block)?;
                    }
                },
                _ => return Err(Error::Unsupported),
            }
            if block
                .get("signature")
                .is_some_and(|v| !v.is_string() && !v.is_null())
            {
                return Err(Error::Unsupported);
            }
        },
        Some("refusal") => {
            known_keys(block, &["type", "refusal"])?;
            if !block.get("refusal").is_some_and(Value::is_string) {
                return Err(Error::Unsupported);
            }
        },
        _ => return Err(Error::Unsupported),
    }
    Ok(())
}

fn merge_content(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    if nullable(target, "content", value) {
        return Ok(());
    }
    if let Some(blocks) = value.as_array() {
        for block in blocks {
            validate_content_block(block)?;
        }
    } else if !value.is_string() {
        return Err(Error::Unsupported);
    }
    let previous = target.entry("content").or_insert(Value::Null);
    if previous.is_null() {
        *previous = value.clone();
        return Ok(());
    }
    if let (Value::String(previous), Value::String(fragment)) = (&mut *previous, value) {
        previous.push_str(fragment);
        return Ok(());
    }
    // String/array transitions retain ordering by representing strings as text blocks.
    if let Value::String(text) = previous {
        *previous = json!([{"type":"text", "text":text}]);
    }
    let blocks = previous.as_array_mut().ok_or(Error::Unsupported)?;
    if let Some(values) = value.as_array() {
        blocks.extend(values.iter().cloned());
    } else {
        blocks.push(json!({"type":"text", "text":value}));
    }
    Ok(())
}

fn merge_function(
    target: &mut Map<String, Value>,
    key: &str,
    value: &Value,
    custom: bool,
) -> Result<(), Error> {
    if nullable(target, key, value) {
        return Ok(());
    }
    let source = object(value)?;
    let keys = if custom {
        &["name", "input"][..]
    } else {
        &["name", "arguments"][..]
    };
    known_keys(source, keys)?;
    let destination = target.entry(key).or_insert_with(|| json!({}));
    if destination.is_null() {
        *destination = json!({});
    }
    let destination = destination.as_object_mut().ok_or(Error::Unsupported)?;
    for (key, value) in source {
        // Null tool fragments add nothing and never satisfy a complete field.
        if !value.is_null() {
            append_text(destination, key, value)?;
        }
    }
    Ok(())
}

fn merge_tools(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    let source = value.as_array().ok_or(Error::Unsupported)?;
    let destination = target.entry("tool_calls").or_insert_with(|| json!([]));
    let destination = destination.as_array_mut().ok_or(Error::Unsupported)?;
    for call in source {
        let call = object(call)?;
        known_keys(call, &["index", "id", "type", "function", "custom"])?;
        let call_index = index(call.get("index").ok_or(Error::InvalidChunk)?)?;
        let position = destination
            .iter()
            .position(|v| v["index"].as_u64() == Some(call_index));
        let position = position.unwrap_or_else(|| {
            destination.push(json!({"index":call_index}));
            destination.len() - 1
        });
        let target = destination[position]
            .as_object_mut()
            .ok_or(Error::Unsupported)?;
        if let Some(id) = call.get("id").filter(|v| !v.is_null()) {
            append_text(target, "id", id)?;
        }
        if let Some(kind) = call.get("type").filter(|v| !v.is_null()) {
            if !matches!(kind.as_str(), Some("function" | "custom")) {
                return Err(Error::Unsupported);
            }
            merge_metadata(target, "type", kind.clone())?;
        }
        for (key, custom) in [("function", false), ("custom", true)] {
            if let Some(value) = call.get(key) {
                merge_function(target, key, value, custom)?;
            }
        }
    }
    Ok(())
}

fn merge_audio(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    if nullable(target, "audio", value) {
        return Ok(());
    }
    let source = object(value)?;
    known_keys(source, &["id", "expires_at", "data", "transcript"])?;
    let destination = target.entry("audio").or_insert_with(|| json!({}));
    if destination.is_null() {
        *destination = json!({});
    }
    let destination = destination.as_object_mut().ok_or(Error::Unsupported)?;
    for (key, value) in source {
        match key.as_str() {
            "data" | "transcript" => append_text(destination, key, value)?,
            "expires_at" => {
                if !value.is_u64() && !value.is_null() {
                    return Err(Error::Unsupported);
                }
                if !value.is_null() || !destination.contains_key(key) {
                    destination.insert(key.clone(), value.clone());
                }
            },
            "id" => {
                if !value.is_string() && !value.is_null() {
                    return Err(Error::Unsupported);
                }
                merge_metadata(destination, key, value.clone())?;
            },
            _ => return Err(Error::Unsupported),
        }
    }
    Ok(())
}

fn merge_reasoning_details(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    if nullable(target, "reasoning_details", value) {
        return Ok(());
    }
    let details = value.as_array().ok_or(Error::Unsupported)?;
    let destination = target
        .entry("reasoning_details")
        .or_insert_with(|| json!([]));
    if destination.is_null() {
        *destination = json!([]);
    }
    let destination = destination.as_array_mut().ok_or(Error::Unsupported)?;
    for detail in details {
        let detail = object(detail)?;
        known_keys(
            detail,
            &[
                "type",
                "id",
                "index",
                "format",
                "text",
                "summary",
                "signature",
                "data",
                "encrypted",
                "encrypted_content",
            ],
        )?;
        if !matches!(
            detail.get("type").and_then(Value::as_str),
            Some(
                "reasoning.text"
                    | "reasoning.summary"
                    | "reasoning.encrypted"
                    | "text"
                    | "summary"
                    | "signature"
                    | "encrypted"
            )
        ) {
            return Err(Error::Unsupported);
        }
        for key in ["id", "format"] {
            if detail
                .get(key)
                .is_some_and(|v| !v.is_string() && !v.is_null())
            {
                return Err(Error::Unsupported);
            }
        }
        if detail
            .get("index")
            .is_some_and(|v| !v.is_u64() && !v.is_null())
        {
            return Err(Error::Unsupported);
        }
        let position = destination.iter().position(|previous| {
            previous.get("type") == detail.get("type")
                && (detail
                    .get("id")
                    .filter(|v| !v.is_null())
                    .is_some_and(|id| previous.get("id") == Some(id))
                    || detail
                        .get("index")
                        .filter(|v| !v.is_null())
                        .is_some_and(|i| previous.get("index") == Some(i)))
        });
        let position = position.unwrap_or_else(|| {
            destination.push(json!({}));
            destination.len() - 1
        });
        let target = destination[position]
            .as_object_mut()
            .ok_or(Error::Unsupported)?;
        for (key, value) in detail {
            match key.as_str() {
                "text" | "summary" => append_text(target, key, value)?,
                "signature" | "data" | "encrypted" | "encrypted_content" => {
                    if !value.is_string() && !value.is_null() {
                        return Err(Error::Unsupported);
                    }
                    // Opaque payloads are indivisible; concatenation would invent semantics.
                    merge_metadata(target, key, value.clone())?;
                },
                _ => merge_metadata(target, key, value.clone())?,
            }
        }
    }
    Ok(())
}

pub(crate) fn merge_logprobs(target: &mut Map<String, Value>, value: &Value) -> Result<(), Error> {
    if nullable(target, "logprobs", value) {
        return Ok(());
    }
    let source = object(value)?;
    known_keys(source, &["content", "refusal"])?;
    let destination = target.entry("logprobs").or_insert_with(|| json!({}));
    if destination.is_null() {
        *destination = json!({});
    }
    let destination = destination.as_object_mut().ok_or(Error::Unsupported)?;
    for (key, value) in source {
        append_array(destination, key, value)?;
    }
    Ok(())
}

pub(crate) fn merge_message(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
) -> Result<(), Error> {
    known_keys(source, MESSAGE_KEYS)?;
    for (key, value) in source {
        match key.as_str() {
            "obfuscation" => {},
            "role" => {
                if !value.is_string() {
                    return Err(Error::Unsupported);
                }
                merge_metadata(target, key, value.clone())?;
            },
            "content" => merge_content(target, value)?,
            "refusal" | "reasoning" | "reasoning_content" => append_text(target, key, value)?,
            "function_call" => merge_function(target, key, value, false)?,
            "tool_calls" => merge_tools(target, value)?,
            "audio" => merge_audio(target, value)?,
            "reasoning_details" => merge_reasoning_details(target, value)?,
            _ => merge_metadata(target, key, value.clone())?,
        }
    }
    Ok(())
}

pub(crate) fn validate_complete_tool(tool: &Value) -> Result<(), Error> {
    let tool = object(tool)?;
    known_keys(tool, &["id", "type", "function", "custom"])?;
    if !tool.get("id").is_some_and(Value::is_string) {
        return Err(Error::InvalidChunk);
    }
    let (key, argument) = match tool.get("type").and_then(Value::as_str) {
        Some("function") => ("function", "arguments"),
        Some("custom") => ("custom", "input"),
        _ => return Err(Error::Unsupported),
    };
    if tool.contains_key(if key == "custom" {
        "function"
    } else {
        "custom"
    }) {
        return Err(Error::Unsupported);
    }
    let function = object(tool.get(key).ok_or(Error::InvalidChunk)?)?;
    known_keys(function, &["name", argument])?;
    if !function.get("name").is_some_and(Value::is_string)
        || !function.get(argument).is_some_and(Value::is_string)
    {
        return Err(Error::InvalidChunk);
    }
    Ok(())
}

pub(crate) fn finish_message(message: &mut Map<String, Value>) -> Result<(), Error> {
    if let Some(data) = message
        .get_mut("audio")
        .and_then(Value::as_object_mut)
        .and_then(|audio| audio.get_mut("data"))
        .filter(|v| !v.is_null())
    {
        let encoded = data.as_str().ok_or(Error::Unsupported)?;
        // Independent padded audio fragments cannot be joined as base64 text.
        // Decode at padding boundaries; unpadded continuations remain one stream.
        let mut remaining = encoded;
        let mut decoded = Vec::new();
        while let Some(padding) = remaining.find('=') {
            let end = padding
                + remaining[padding..]
                    .bytes()
                    .take_while(|byte| *byte == b'=')
                    .count();
            let part = STANDARD.decode(&remaining[..end]).map_err(Error::Audio)?;
            decoded.extend(part);
            remaining = &remaining[end..];
        }
        if !remaining.is_empty() {
            decoded.extend(STANDARD_NO_PAD.decode(remaining).map_err(Error::Audio)?);
        }
        *data = STANDARD.encode(decoded).into();
    }
    if let Some(calls) = message.get_mut("tool_calls") {
        let calls = calls.as_array_mut().ok_or(Error::Unsupported)?;
        calls.sort_by_key(|call| call["index"].as_u64());
        for call in calls {
            call.as_object_mut()
                .ok_or(Error::Unsupported)?
                .remove("index");
            validate_complete_tool(call)?;
        }
    }
    if let Some(function) = message.get("function_call").filter(|v| !v.is_null()) {
        let function = object(function)?;
        if !function.get("name").is_some_and(Value::is_string)
            || !function.get("arguments").is_some_and(Value::is_string)
        {
            return Err(Error::InvalidChunk);
        }
    }
    Ok(())
}

pub(crate) fn validate_message(message: &Value) -> Result<(), Error> {
    let mut source = object(message)?.clone();
    if !source.get("role").is_some_and(Value::is_string) {
        return Err(Error::InvalidChunk);
    }
    if let Some(tools) = source.get_mut("tool_calls") {
        for (index, tool) in tools
            .as_array_mut()
            .ok_or(Error::Unsupported)?
            .iter_mut()
            .enumerate()
        {
            validate_complete_tool(tool)?;
            tool["index"] = index.into();
        }
    }
    let mut result = Map::new();
    merge_message(&mut result, &source)?;
    finish_message(&mut result)
}
