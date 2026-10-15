use std::{error::Error, fmt};

use serde::Deserialize;

/// Stable configuration failure classes, independent of dependency error wording.
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

/// Safe configuration context with the original error retained for typed inspection.
///
/// Raw sources may contain credentials or configuration text. Use `diagnostic()` for
/// logs and reports rather than formatting or walking those sources directly.
#[derive(thiserror::Error)]
pub struct ConfigError {
    pub kind: ConfigErrorKind,
    pub field: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub message: &'static str,
    operation: Option<&'static str>,
    #[source]
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl ConfigError {
    pub(crate) fn new(kind: ConfigErrorKind, field: &str, message: &'static str) -> Self {
        Self {
            kind,
            field: field.to_owned(),
            line: None,
            column: None,
            message,
            operation: None,
            source: None,
        }
    }

    pub(crate) fn at(mut self, location: serde_saphyr::Location) -> Self {
        if location.line() > 0 && location.column() > 0 {
            self.line = Some(location.line());
            self.column = Some(location.column());
        }
        self
    }

    pub(crate) fn at_offset(mut self, source: &str, offset: usize) -> Self {
        let prefix = &source[..source.floor_char_boundary(offset.min(source.len()))];
        self.line = Some(prefix.split('\n').count() as u64);
        self.column = Some(
            prefix
                .rsplit('\n')
                .next()
                .map_or(1, |line| line.chars().count() as u64 + 1),
        );
        self
    }

    pub(crate) fn with_source(
        mut self,
        operation: &'static str,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        self.operation = Some(operation);
        self.source = Some(Box::new(source));
        self
    }

    /// Render schema context and a bounded, redacted classification of the cause chain.
    /// Dependency `Display` and `Debug` implementations are never invoked.
    #[must_use]
    pub fn diagnostic(&self) -> impl fmt::Display + '_ { Diagnostic(self) }
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

impl fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigError")
            .field("kind", &self.kind)
            .field("diagnostic", &format_args!("{}", self.diagnostic()))
            .finish_non_exhaustive()
    }
}

struct Diagnostic<'a>(&'a ConfigError);

impl fmt::Display for Diagnostic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        if let Some(operation) = self.0.operation {
            write!(f, "; operation: {operation}")?;
        }
        let mut source = self.0.source();
        // An opaque custom I/O source can have an arbitrary or cyclic chain.
        for _ in 0..8 {
            let Some(error) = source else {
                return Ok(());
            };
            f.write_str("; cause: ")?;
            safe_cause(error, f)?;
            source = error.source();
        }
        if source.is_some() {
            f.write_str("; further causes omitted")?;
        }
        Ok(())
    }
}

fn safe_cause(error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        write!(f, "I/O ({:?})", error.kind())
    } else if let Some(error) = error.downcast_ref::<serde_saphyr::Error>() {
        write!(f, "YAML ({})", yaml_class(error))
    } else if error.is::<toml::de::Error>() {
        f.write_str("TOML (syntax, field or type)")
    } else if let Some(error) = error.downcast_ref::<serde_json::Error>() {
        write!(
            f,
            "JSON ({:?}, line {}, column {})",
            error.classify(),
            error.line(),
            error.column()
        )
    } else if error.is::<url::ParseError>() {
        f.write_str("invalid URL")
    } else if error.is::<http::header::InvalidHeaderName>() {
        f.write_str("invalid header name")
    } else if error.is::<http::header::InvalidHeaderValue>() {
        f.write_str("invalid header value")
    } else if let Some(error) = error.downcast_ref::<std::str::Utf8Error>() {
        write!(f, "invalid UTF-8 at byte {}", error.valid_up_to())
    } else {
        f.write_str("opaque dependency error")
    }
}

fn yaml_class(error: &serde_saphyr::Error) -> &'static str {
    use serde_saphyr::Error;
    match error {
        Error::DuplicateMappingKey { .. } => "duplicate key",
        Error::SerdeUnknownField { .. } => "unknown field",
        Error::SerdeMissingField { .. } => "missing field",
        Error::SerdeUnknownVariant { .. } => "unknown variant",
        Error::SerdeInvalidType { .. }
        | Error::SerdeInvalidValue { .. }
        | Error::SerdeInvalidLength { .. }
        | Error::InvalidScalar { .. }
        | Error::InvalidBooleanStrict { .. }
        | Error::NullIntoString { .. } => "invalid type or value",
        Error::Eof { .. } => "unexpected end",
        Error::MultipleDocuments { .. } => "multiple documents",
        Error::MergeKeyNotAllowed { .. } => "merge key rejected",
        Error::Budget { .. } => "resource limit",
        Error::UnsupportedTag { .. } => "unsupported tag",
        Error::IOError { .. } => "input I/O",
        Error::AliasError { .. } => "alias",
        Error::WithSnippet { .. } => "parser context",
        Error::Message { .. } => "schema or parser error",
        _ => "invalid input",
    }
}

pub(crate) fn present_option<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    // Serde's generic custom-error API cannot carry a source. Keep a fixed class;
    // the enclosing concrete deserializer error is retained at its caller boundary.
    T::deserialize(value)
        .map(Some)
        .map_err(|_error| serde::de::Error::custom("invalid field type"))
}

#[cfg(test)]
mod tests;

/// Never echo arbitrary keys (including provider names and headers) from an error path.
pub(crate) fn safe_path(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let parts: Vec<&str> = path
        .iter()
        .filter_map(|segment| match segment {
            // Native span adapters add these fields outside the configuration schema.
            Segment::Map { key }
                if matches!(
                    key.as_str(),
                    "value" | "referenced" | "defined" | "$__serde_spanned_private_value"
                ) =>
            {
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
