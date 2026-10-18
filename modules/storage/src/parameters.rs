use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Only the explicit projector may create persisted generation metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GenerationParameters(pub BTreeMap<String, Value>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSnapshots {
    pub client: GenerationParameters,
    pub upstream: Option<GenerationParameters>,
}

const NUMBERS: &[&str] = &[
    "temperature",
    "top_p",
    "top_k",
    "min_p",
    "typical_p",
    "tfs_z",
    "top_a",
    "frequency_penalty",
    "presence_penalty",
    "repetition_penalty",
    "max_tokens",
    "max_completion_tokens",
    "n",
    "seed",
    "top_logprobs",
    "min_tokens",
];
const BOOLEANS: &[&str] = &[
    "stream",
    "logprobs",
    "parallel_tool_calls",
    "store",
    "ignore_eos",
];
const ENUMS: &[(&str, &[&str])] = &[
    (
        "reasoning_effort",
        &["none", "minimal", "low", "medium", "high", "xhigh"],
    ),
    ("verbosity", &["low", "medium", "high"]),
    (
        "service_tier",
        &["auto", "default", "flex", "priority", "scale"],
    ),
];

/// Project controls only; free text, tool definitions and credential-like extensions
/// never cross this boundary. Missing keys and explicit JSON null remain distinct.
pub fn project_generation_parameters(body: &Value) -> GenerationParameters {
    let mut fields = BTreeMap::new();
    if let Some(value) = body.get("model") {
        fields.insert(
            "model".into(),
            scalar(
                value,
                value.as_str().is_some_and(|value| value.len() <= 1024)
                    && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 1026),
            ),
        );
    }
    for (input, output) in [("tools", "tools_count"), ("functions", "functions_count")] {
        if let Some(value) = body.get(input) {
            fields.insert(
                output.into(),
                if value.is_null() {
                    Value::Null
                } else {
                    value
                        .as_array()
                        .map_or_else(invalid, |values| serde_json::json!(values.len()))
                },
            );
        }
    }
    for key in NUMBERS {
        if let Some(value) = body.get(*key) {
            fields.insert((*key).into(), scalar(value, value.is_number()));
        }
    }
    for key in BOOLEANS {
        if let Some(value) = body.get(*key) {
            fields.insert((*key).into(), scalar(value, value.is_boolean()));
        }
    }
    for (key, allowed) in ENUMS {
        if let Some(value) = body.get(*key) {
            fields.insert(
                (*key).into(),
                scalar(
                    value,
                    value.as_str().is_some_and(|value| allowed.contains(&value)),
                ),
            );
        }
    }
    if let Some(value) = body.get("stream_options") {
        fields.insert(
            "stream_options".into(),
            object_controls(value, &["include_usage", "include_obfuscation"]),
        );
    }
    project_structured(body, &mut fields);
    if let Some(value) = body.get("modalities") {
        fields.insert(
            "modalities".into(),
            scalar(
                value,
                value.as_array().is_some_and(|values| {
                    values.len() <= 2
                        && values.iter().all(|value| {
                            value
                                .as_str()
                                .is_some_and(|value| ["text", "audio"].contains(&value))
                        })
                }),
            ),
        );
    }
    let mut result = GenerationParameters(fields);
    if serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() > 4096)
        && let Some(bias) = result.0.get_mut("logit_bias")
    {
        let entries = bias.as_object().map_or(0, serde_json::Map::len);
        *bias = serde_json::json!({"entries":entries,"valuesRetained":false});
    }
    result
}

fn project_structured(body: &Value, fields: &mut BTreeMap<String, Value>) {
    if let Some(value) = body.get("response_format") {
        let projected = if value.is_null() {
            Value::Null
        } else if let Some(kind) = value
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| ["text", "json_object", "json_schema"].contains(kind))
        {
            let mut result =
                serde_json::Map::from_iter([("type".into(), Value::String(kind.into()))]);
            if let Some(strict) = value
                .get("json_schema")
                .and_then(|schema| schema.get("strict"))
            {
                result.insert("strict".into(), scalar(strict, strict.is_boolean()));
            }
            Value::Object(result)
        } else {
            invalid()
        };
        fields.insert("response_format".into(), projected);
    }
    if let Some(value) = body
        .get("tool_choice")
        .or_else(|| body.get("function_call"))
    {
        let projected = if value.is_null() {
            Value::Null
        } else if let Some(mode) = value
            .as_str()
            .filter(|value| ["none", "auto", "required"].contains(value))
        {
            Value::String(mode.into())
        } else if value.is_object() {
            serde_json::json!({"type":"named"})
        } else {
            invalid()
        };
        fields.insert("tool_choice".into(), projected);
    }
    if let Some(value) = body.get("stop") {
        let lengths = match value {
            Value::String(value) => Some(vec![value.len()]),
            Value::Array(values) if values.len() <= 64 && values.iter().all(Value::is_string) => {
                Some(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::len)
                        .collect(),
                )
            },
            _ => None,
        };
        fields.insert(
            "stop".into(),
            if value.is_null() {
                Value::Null
            } else {
                lengths.map_or_else(
                    invalid,
                    |lengths| serde_json::json!({"count":lengths.len(),"bytes":lengths}),
                )
            },
        );
    }
    if let Some(value) = body.get("logit_bias") {
        let projected = if value.is_null() {
            Value::Null
        } else if let Some(entries) = value.as_object() {
            if entries.len() <= 64
                && entries.iter().all(|(key, value)| {
                    key.parse::<u32>().is_ok()
                        && value
                            .as_f64()
                            .is_some_and(|value| (-100.0..=100.0).contains(&value))
                })
            {
                value.clone()
            } else {
                serde_json::json!({"entries":entries.len(),"valuesRetained":false})
            }
        } else {
            invalid()
        };
        fields.insert("logit_bias".into(), projected);
    }
}

fn scalar(value: &Value, valid: bool) -> Value {
    if value.is_null() || valid {
        value.clone()
    } else {
        invalid()
    }
}

fn invalid() -> Value { serde_json::json!({"status":"invalid_type"}) }

fn object_controls(value: &Value, keys: &[&str]) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    if !value.is_object() {
        return invalid();
    }
    Value::Object(
        keys.iter()
            .filter_map(|key| {
                value
                    .get(*key)
                    .map(|value| ((*key).into(), scalar(value, value.is_boolean())))
            })
            .collect(),
    )
}

pub(crate) fn validate_parameters(parameters: &GenerationParameters) -> bool {
    let invalid = invalid();
    let valid = parameters.0.iter().all(|(key, value)| {
        let known = NUMBERS.contains(&key.as_str())
            || BOOLEANS.contains(&key.as_str())
            || ENUMS.iter().any(|(name, _)| *name == key)
            || [
                "model",
                "tools_count",
                "functions_count",
                "stream_options",
                "response_format",
                "tool_choice",
                "stop",
                "logit_bias",
                "modalities",
            ]
            .contains(&key.as_str());
        if !known {
            return false;
        }
        if value.is_null() || *value == invalid {
            return true;
        }
        if NUMBERS.contains(&key.as_str()) {
            return value.is_number();
        }
        if BOOLEANS.contains(&key.as_str()) {
            return value.is_boolean();
        }
        if let Some((_, allowed)) = ENUMS.iter().find(|(name, _)| *name == key) {
            return value.as_str().is_some_and(|value| allowed.contains(&value));
        }
        match key.as_str() {
            "model" => value.as_str().is_some_and(|value| value.len() <= 1024),
            "tools_count" | "functions_count" => value.as_u64().is_some(),
            "stream_options" => value.as_object().is_some_and(|values| {
                values.iter().all(|(key, value)| {
                    ["include_usage", "include_obfuscation"].contains(&key.as_str())
                        && (value.is_null() || value.is_boolean() || *value == invalid)
                })
            }),
            "response_format" => value.as_object().is_some_and(|values| {
                values
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| ["text", "json_object", "json_schema"].contains(&kind))
                    && values.iter().all(|(key, value)| {
                        key == "type"
                            || key == "strict"
                                && (value.is_null() || value.is_boolean() || *value == invalid)
                    })
            }),
            "tool_choice" => {
                value
                    .as_str()
                    .is_some_and(|value| ["none", "auto", "required"].contains(&value))
                    || *value == serde_json::json!({"type":"named"})
            },
            "stop" => value.as_object().is_some_and(|values| {
                values.len() == 2
                    && values
                        .get("bytes")
                        .and_then(Value::as_array)
                        .is_some_and(|lengths| {
                            lengths.len() <= 64
                                && values.get("count").and_then(Value::as_u64)
                                    == Some(lengths.len() as u64)
                                && lengths.iter().all(|value| value.as_u64().is_some())
                        })
            }),
            "logit_bias" => value.as_object().is_some_and(|values| {
                (values.len() == 2
                    && values
                        .get("entries")
                        .is_some_and(|value| value.as_u64().is_some())
                    && values.get("valuesRetained") == Some(&Value::Bool(false)))
                    || (values.len() <= 64
                        && values.iter().all(|(key, value)| {
                            key.parse::<u32>().is_ok()
                                && value
                                    .as_f64()
                                    .is_some_and(|value| (-100.0..=100.0).contains(&value))
                        }))
            }),
            "modalities" => value.as_array().is_some_and(|values| {
                values.len() <= 2
                    && values.iter().all(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| ["text", "audio"].contains(&value))
                    })
            }),
            _ => false,
        }
    });
    valid && serde_json::to_vec(parameters).is_ok_and(|bytes| bytes.len() <= 4096)
}

#[cfg(test)]
mod tests;
