//! Typed startup descriptors for policies executed by Rust.

use std::collections::BTreeSet;

use http::HeaderName;
use serde::Deserialize;

use crate::{ConfigError, ConfigErrorKind, error::present_option};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BuiltinPolicy {
    CapCompletionTokens(CapCompletionTokens),
    DropRequestFields(DropRequestFields),
    ForwardClientHeader(ForwardClientHeader),
    ReplaceText(ReplaceText),
    SyntheticTextTool(SyntheticTextTool),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapCompletionTokens {
    pub max: u64,
    #[serde(default)]
    pub field: CompletionTokenField,
    #[serde(default, deserialize_with = "present_option")]
    pub on_change: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub enum CompletionTokenField {
    #[default]
    #[serde(rename = "max_completion_tokens")]
    MaxCompletionTokens,
    #[serde(rename = "max_tokens")]
    MaxTokens,
}

impl CompletionTokenField {
    pub fn name(self) -> &'static str {
        match self {
            Self::MaxCompletionTokens => "max_completion_tokens",
            Self::MaxTokens => "max_tokens",
        }
    }

    pub fn other(self) -> &'static str {
        match self {
            Self::MaxCompletionTokens => "max_tokens",
            Self::MaxTokens => "max_completion_tokens",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DropRequestFields {
    pub fields: Vec<String>,
    #[serde(default, deserialize_with = "present_option")]
    pub on_change: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardClientHeader {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceText {
    pub search: String,
    pub replacement: String,
    pub max_buffered_chars: usize,
    #[serde(default = "default_parts")]
    pub max_parts: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntheticTextTool {
    #[serde(default = "default_tool_name")]
    pub tool_name: String,
    pub max_buffered_chars: usize,
    #[serde(default = "default_parts")]
    pub max_pending_events: usize,
    #[serde(default = "default_parts")]
    pub max_parts: usize,
}

fn default_parts() -> usize { 64 }

fn default_tool_name() -> String { "limen_emit_text".to_owned() }

impl BuiltinPolicy {
    pub fn has_before_send(&self) -> bool { !matches!(self, Self::ReplaceText(_)) }

    pub fn has_content(&self) -> bool {
        matches!(self, Self::ReplaceText(_) | Self::SyntheticTextTool(_))
    }

    pub(crate) fn validate(&self, field: &str) -> Result<(), ConfigError> {
        let valid = match self {
            Self::CapCompletionTokens(options) => {
                (1..=9_007_199_254_740_991).contains(&options.max)
                    && valid_observation(options.on_change.as_deref())
            },
            Self::DropRequestFields(options) => {
                let mut names = BTreeSet::new();
                !options.fields.is_empty()
                    && options.fields.iter().all(|name| {
                        !name.is_empty()
                            && !matches!(name.as_str(), "model" | "messages" | "stream")
                            && names.insert(name)
                    })
                    && valid_observation(options.on_change.as_deref())
            },
            Self::ForwardClientHeader(options) => {
                valid_business_header(&options.from) && valid_business_header(&options.to)
            },
            Self::ReplaceText(options) => {
                !options.search.is_empty()
                    && options.max_buffered_chars > 0
                    && options.max_buffered_chars
                        >= options.search.encode_utf16().count().saturating_sub(1)
                    && options.max_parts > 0
            },
            Self::SyntheticTextTool(options) => {
                let name = options.tool_name.as_bytes();
                !name.is_empty()
                    && name.len() <= 64
                    && (name[0].is_ascii_alphabetic() || name[0] == b'_')
                    && name
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                    && options.max_buffered_chars >= name.len()
                    && options.max_pending_events > 0
                    && options.max_parts > 0
            },
        };
        if valid {
            Ok(())
        } else {
            Err(ConfigError::new(
                ConfigErrorKind::Schema,
                field,
                "invalid builtin policy options",
            ))
        }
    }
}

fn valid_observation(name: Option<&str>) -> bool {
    name.is_none_or(|name| {
        !name.is_empty()
            && name.len() <= 64
            && name.as_bytes()[0].is_ascii_lowercase()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

fn valid_business_header(name: &str) -> bool {
    HeaderName::from_bytes(name.as_bytes()).is_ok_and(|name| {
        !matches!(
            name.as_str(),
            "authorization"
                | "proxy-authorization"
                | "cookie"
                | "set-cookie"
                | "proxy-authenticate"
                | "www-authenticate"
                | "host"
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
    })
}
