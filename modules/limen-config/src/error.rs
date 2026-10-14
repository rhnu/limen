use std::fmt;

use serde::Deserialize;

/// Stable configuration failure classes; parser text is deliberately discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigErrorKind {
    Io,
    TooLarge,
    Syntax,
    Schema,
    Range,
    Reference,
    Endpoint,
    Header,
    Json,
}

/// A diagnostic containing only schema-owned names and source coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub kind: ConfigErrorKind,
    pub field: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub message: &'static str,
}

impl ConfigError {
    pub(crate) fn new(kind: ConfigErrorKind, field: &str, message: &'static str) -> Self {
        Self {
            kind,
            field: field.to_owned(),
            line: None,
            column: None,
            message,
        }
    }

    pub(crate) fn at(mut self, location: serde_saphyr::Location) -> Self {
        if location.line() > 0 && location.column() > 0 {
            self.line = Some(location.line());
            self.column = Some(location.column());
        }
        self
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, " (line {line}, column {column})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

pub(crate) fn present_option<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    T::deserialize(value)
        .map(Some)
        .map_err(|_error| serde::de::Error::custom("invalid field type"))
}

/// Never echo arbitrary keys (including provider names and headers) from an error path.
pub(crate) fn safe_path(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let parts: Vec<&str> = path
        .iter()
        .filter_map(|segment| match segment {
            // Spanned adds these adapter fields; they are not configuration schema.
            Segment::Map { key } if matches!(key.as_str(), "value" | "referenced" | "defined") => {
                None
            },
            Segment::Map { key } => Some(schema_name(key)),
            Segment::Seq { .. } => Some("*"),
            Segment::Enum { .. } | Segment::Unknown => Some("?"),
        })
        .collect();
    if parts.is_empty() {
        "config".to_owned()
    } else {
        parts.join(".")
    }
}

fn schema_name(key: &str) -> &str {
    match key {
        "listen"
        | "client_auth"
        | "none"
        | "bearer"
        | "log"
        | "level"
        | "format"
        | "output"
        | "limits"
        | "startup_ms"
        | "hook_ms"
        | "request_ms"
        | "execution_ms"
        | "vm_work_ms"
        | "cleanup_ms"
        | "shutdown_ms"
        | "memory_bytes"
        | "stack_bytes"
        | "queue_capacity"
        | "concurrency"
        | "max_pending"
        | "job_batch"
        | "max_body_bytes"
        | "max_event_bytes"
        | "max_output_events"
        | "max_output_bytes"
        | "max_aggregate_bytes"
        | "max_aggregate_events"
        | "max_modules"
        | "max_source_bytes"
        | "max_json_depth"
        | "max_json_nodes"
        | "providers"
        | "models"
        | "provider"
        | "upstream"
        | "endpoint"
        | "auth"
        | "headers"
        | "defaults"
        | "pipeline"
        | "name"
        | "onRequest"
        | "onError"
        | "onFinish"
        | "beforeSend"
        | "afterReceive"
        | "onResponse"
        | "onUpstreamEvent"
        | "onStreamEvent" => key,
        _ => "*",
    }
}
