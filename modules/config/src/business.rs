use std::collections::{BTreeMap, BTreeSet};

use http::{HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value};
use url::Url;

use crate::{
    ConfigError, ConfigErrorKind, ResourceLimits,
    error::{present_option, safe_path},
    server::valid_bearer,
};

pub type HookId = u64;

/// The script runtime extracts functions to numeric IDs before static deserialization.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BusinessConfig {
    pub providers: BTreeMap<String, Provider>,
    pub models: BTreeMap<String, Model>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_request: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_error_response: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_error: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_finish: Option<HookId>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub endpoint: String,
    #[serde(default, deserialize_with = "present_option")]
    pub auth: Option<UpstreamAuth>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub defaults: Map<String, Value>,
    #[serde(default)]
    pub pipeline: Vec<PipelineStep>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAuth {
    pub bearer: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub provider: String,
    pub upstream: String,
    #[serde(default)]
    pub defaults: Map<String, Value>,
    #[serde(default)]
    pub pipeline: Vec<PipelineStep>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PipelineStep {
    pub name: String,
    #[serde(default, deserialize_with = "present_option")]
    pub before_send: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub after_receive: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_response: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_upstream_event: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_stream_event: Option<HookId>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_error_response: Option<HookId>,
}

impl BusinessConfig {
    /// Validate the static export after the runtime has saved actual function references.
    ///
    /// # Errors
    /// Returns sanitized schema, reference, endpoint, header or JSON diagnostics.
    pub fn from_value(value: Value, limits: &ResourceLimits) -> Result<Self, ConfigError> {
        validate_json(&value, limits.max_json_depth, limits.max_json_nodes)?;
        validate_shapes(&value)?;
        let mut track = serde_path_to_error::Track::new();
        let config = Self::deserialize(serde_path_to_error::Deserializer::new(value, &mut track))
            .map_err(|error| {
            ConfigError::new(
                ConfigErrorKind::Schema,
                &safe_path(&track.path()),
                "unknown field, missing field or invalid type",
            )
            .with_source("deserialize business configuration", error)
        })?;
        if config.providers.is_empty() || config.providers.len() > 1024 {
            return Err(ConfigError::new(
                ConfigErrorKind::Range,
                "providers",
                "expected 1 to 1024 providers",
            ));
        }
        if config.models.is_empty() || config.models.len() > 4096 {
            return Err(ConfigError::new(
                ConfigErrorKind::Range,
                "models",
                "expected 1 to 4096 models",
            ));
        }
        for (name, provider) in &config.providers {
            validate_name(name, "providers.*", b"-._/")?;
            validate_endpoint(&provider.endpoint)?;
            validate_headers(provider)?;
            validate_defaults(&provider.defaults, "providers.*.defaults")?;
            validate_pipeline(&provider.pipeline, "providers.*.pipeline")?;
        }
        for (name, model) in &config.models {
            validate_name(name, "models.*", b"-._/:")?;
            if !config.providers.contains_key(&model.provider) {
                return Err(ConfigError::new(
                    ConfigErrorKind::Reference,
                    "models.*.provider",
                    "provider does not exist",
                ));
            }
            if model.upstream.is_empty()
                || model.upstream.len() > 1024
                || model.upstream.chars().any(char::is_control)
            {
                return Err(ConfigError::new(
                    ConfigErrorKind::Schema,
                    "models.*.upstream",
                    "expected a nonempty model identifier",
                ));
            }
            validate_defaults(&model.defaults, "models.*.defaults")?;
            validate_pipeline(&model.pipeline, "models.*.pipeline")?;
        }
        Ok(config)
    }
}

fn validate_name(name: &str, field: &str, punctuation: &[u8]) -> Result<(), ConfigError> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || punctuation.contains(&byte))
    {
        return Err(ConfigError::new(
            ConfigErrorKind::Schema,
            field,
            "invalid or empty identifier",
        ));
    }
    Ok(())
}

/// Validate a complete HTTP(S) endpoint without exposing its credentials or query.
///
/// # Errors
/// Returns a sanitized endpoint error for malformed or forbidden URL forms.
pub fn validate_endpoint(endpoint: &str) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::new(
            ConfigErrorKind::Endpoint,
            "providers.*.endpoint",
            "expected an absolute HTTP(S) URL without credentials or fragment",
        )
    };
    // Reject URL parser repair of spaces, backslashes and a missing // authority delimiter.
    if endpoint.len() > 8192
        || endpoint
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || endpoint.contains('\\')
        || !endpoint.split_once("://").is_some_and(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        })
    {
        return Err(invalid());
    }
    let url = Url::parse(endpoint)
        .map_err(|error| invalid().with_source("parse provider endpoint", error))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    // An empty userinfo section is still forbidden, even though Url::username() is empty.
    let authority = endpoint
        .split_once("://")
        .map_or("", |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(invalid());
    }
    Ok(())
}

fn validate_headers(provider: &Provider) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::new(
            ConfigErrorKind::Header,
            "providers.*.headers",
            "invalid, duplicate, reserved or conflicting static header",
        )
    };
    let mut names = BTreeSet::new();
    for (name, value) in &provider.headers {
        let header = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| invalid().with_source("parse static header name", error))?;
        HeaderValue::from_str(value)
            .map_err(|error| invalid().with_source("parse static header value", error))?;
        let name = header.as_str();
        if !names.insert(name.to_owned())
            || matches!(
                name,
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "content-encoding"
                    | "content-type"
                    | "proxy-connection"
                    | "keep-alive"
                    | "te"
                    | "trailer"
                    | "upgrade"
            )
        {
            return Err(invalid());
        }
    }
    if let Some(auth) = &provider.auth
        && (!valid_bearer(&auth.bearer) || names.contains("authorization"))
    {
        return Err(ConfigError::new(
            ConfigErrorKind::Header,
            "providers.*.auth",
            "invalid bearer token or conflicting Authorization header",
        ));
    }
    Ok(())
}

fn validate_defaults(defaults: &Map<String, Value>, field: &str) -> Result<(), ConfigError> {
    if ["model", "messages", "stream"]
        .iter()
        .any(|key| defaults.contains_key(*key))
    {
        return Err(ConfigError::new(
            ConfigErrorKind::Schema,
            field,
            "defaults contain a reserved control field",
        ));
    }
    Ok(())
}

fn validate_pipeline(pipeline: &[PipelineStep], field: &str) -> Result<(), ConfigError> {
    if pipeline.len() > 256 {
        return Err(ConfigError::new(
            ConfigErrorKind::Range,
            field,
            "pipeline exceeds 256 steps",
        ));
    }
    let mut names = BTreeSet::new();
    for step in pipeline {
        validate_name(&step.name, field, b"-._/")?;
        let hooks = [
            step.before_send,
            step.after_receive,
            step.on_response,
            step.on_upstream_event,
            step.on_stream_event,
            step.on_error_response,
        ];
        if !names.insert(&step.name) || hooks.iter().all(Option::is_none) {
            return Err(ConfigError::new(
                ConfigErrorKind::Schema,
                field,
                "steps require unique names and at least one hook",
            ));
        }
    }
    Ok(())
}

/// Share the runtime's conservative JSON boundary, independent of Serde coercion.
///
/// # Errors
/// Returns a sanitized error when JSON depth, nodes or numbers exceed safe limits.
pub fn validate_json(value: &Value, max_depth: usize, max_nodes: usize) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::new(
            ConfigErrorKind::Json,
            "config",
            "JSON depth, nodes or number exceeds safe limits",
        )
    };
    let mut stack = vec![(value, 0)];
    let mut nodes = 0;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > max_nodes || depth > max_depth {
            return Err(invalid());
        }
        match value {
            Value::Object(map) => {
                if map.len() > max_nodes - nodes || stack.len() > max_nodes - map.len() {
                    return Err(invalid());
                }
                stack.extend(map.values().map(|value| (value, depth + 1)));
            },
            Value::Array(array) => {
                if array.len() > max_nodes - nodes || stack.len() > max_nodes - array.len() {
                    return Err(invalid());
                }
                stack.extend(array.iter().map(|value| (value, depth + 1)));
            },
            Value::Number(number)
                if number.as_i64().is_some_and(|n| {
                    !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n)
                }) || number.as_u64().is_some_and(|n| n > 9_007_199_254_740_991)
                    || number
                        .as_f64()
                        .is_some_and(|n| n.fract() == 0.0 && n.abs() > 9_007_199_254_740_991.0) =>
            {
                return Err(invalid());
            },
            _ => {},
        }
    }
    Ok(())
}

fn validate_shapes(value: &Value) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::new(
            ConfigErrorKind::Schema,
            "config",
            "configuration records must be objects",
        )
    };
    let object = value.as_object().ok_or_else(invalid)?;
    for key in ["providers", "models"] {
        if let Some(records) = object.get(key).and_then(Value::as_object) {
            for record in records.values() {
                let record = record.as_object().ok_or_else(invalid)?;
                if record.get("auth").is_some_and(|auth| !auth.is_object()) {
                    return Err(invalid());
                }
                if let Some(steps) = record.get("pipeline").and_then(Value::as_array)
                    && steps.iter().any(|step| !step.is_object())
                {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
