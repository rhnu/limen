use std::{error::Error, fmt};

use limen_script::{ScriptError, ScriptErrorKind};
use serde_json::{Value, json};

/// Stable public error codes; messages are independently sanitized by adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    UnknownModel,
    PayloadTooLarge,
    Busy,
    DeadlineExceeded,
    Cancelled,
    HookFailed,
    UpstreamFailed,
    UpstreamProtocol,
    Internal,
    Unavailable,
}

impl ErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::UnknownModel => "unknown_model",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Busy => "busy",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::HookFailed => "hook_failed",
            Self::UpstreamFailed => "upstream_failed",
            Self::UpstreamProtocol => "upstream_protocol",
            Self::Internal => "internal_error",
            Self::Unavailable => "unavailable",
        }
    }

    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::UnknownModel => 400,
            Self::Unauthorized => 401,
            Self::PayloadTooLarge => 413,
            Self::Busy => 429,
            Self::DeadlineExceeded => 504,
            Self::Cancelled => 499,
            Self::HookFailed | Self::Internal => 500,
            Self::UpstreamFailed | Self::UpstreamProtocol => 502,
            Self::Unavailable => 503,
        }
    }
}

/// Success remains a body-bearing 2xx response; failures can map only to 4xx/5xx.
/// Informational statuses, redirects, and bodyless 204/205 cannot represent completions.
///
/// # Errors
/// Returns `HookFailed` when the requested mapping changes success semantics.
pub fn validate_response_status(upstream: u16, downstream: u16) -> Result<(), ErrorCode> {
    let allowed = if (200..300).contains(&upstream) {
        (200..300).contains(&downstream) && !matches!(downstream, 204 | 205)
    } else {
        (300..600).contains(&upstream) && (400..600).contains(&downstream)
    };
    if allowed {
        Ok(())
    } else {
        Err(ErrorCode::HookFailed)
    }
}

/// Safe request failure metadata; raw sources are retained only for typed inspection.
pub struct Failure {
    pub code: ErrorCode,
    pub phase: &'static str,
    pub step: Option<String>,
    pub status: Option<u16>,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl Failure {
    pub const fn new(code: ErrorCode, phase: &'static str) -> Self {
        Self {
            code,
            phase,
            step: None,
            status: None,
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn script(source: ScriptError, phase: &'static str, step: Option<&str>) -> Self {
        let code = match source.kind() {
            ScriptErrorKind::Busy => ErrorCode::Busy,
            ScriptErrorKind::Cancelled => ErrorCode::Cancelled,
            ScriptErrorKind::Deadline => ErrorCode::DeadlineExceeded,
            ScriptErrorKind::Retired | ScriptErrorKind::Stopped | ScriptErrorKind::Thread => {
                ErrorCode::Unavailable
            },
            _ => ErrorCode::HookFailed,
        };
        let mut failure = Self::new(code, phase).with_source(source);
        failure.step = step.map(str::to_owned);
        failure
    }

    pub fn upstream(source: reqwest::Error, phase: &'static str) -> Self {
        let code = if source.is_timeout() {
            ErrorCode::DeadlineExceeded
        } else {
            ErrorCode::UpstreamFailed
        };
        Self::new(code, phase).with_source(source)
    }

    pub fn body(source: axum::Error) -> Self {
        let mut cause: Option<&(dyn Error + 'static)> = Some(&source);
        let mut code = ErrorCode::InvalidRequest;
        // Concrete HTTP body adapters wrap the length error; inspect types without
        // parsing dependency messages or rendering the possibly sensitive source.
        for _ in 0..8 {
            let Some(error) = cause else {
                break;
            };
            if error.is::<http_body_util::LengthLimitError>() {
                code = ErrorCode::PayloadTooLarge;
                break;
            }
            cause = error.source();
        }
        Self::new(code, "request_body").with_source(source)
    }

    pub fn http_status(&self) -> u16 {
        self.status
            .filter(|status| (400..600).contains(status))
            .unwrap_or_else(|| self.code.status())
    }

    pub fn error_json(&self, request_id: &str) -> Value {
        json!({"error": {"code": self.code.as_str(), "message": self.message(), "request_id": request_id}})
    }

    fn message(&self) -> &'static str {
        match self.code {
            ErrorCode::InvalidRequest => "Invalid Chat Completions request",
            ErrorCode::Unauthorized => "Client authentication failed",
            ErrorCode::UnknownModel => "The requested model is not configured",
            ErrorCode::PayloadTooLarge => "The request or output exceeds the configured size limit",
            ErrorCode::Busy => "The gateway is busy",
            ErrorCode::DeadlineExceeded => "The request deadline was exceeded",
            ErrorCode::Cancelled => "The request was cancelled",
            ErrorCode::HookFailed => "A configured request hook failed",
            ErrorCode::UpstreamFailed => "The upstream request failed",
            ErrorCode::UpstreamProtocol => "The upstream response is not supported",
            ErrorCode::Internal => "The gateway could not complete the request",
            ErrorCode::Unavailable => "The gateway is unavailable",
        }
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}; phase={}", self.code.as_str(), self.phase)?;
        if let Some(step) = &self.step {
            write!(f, "; step={step}")?;
        }
        let mut source = self.source();
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

// Render typed metadata only. A dependency's message or Debug can retain URLs,
// credentials or body text even when the outer gateway error is fixed.
fn safe_cause(error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(source) = error.downcast_ref::<ScriptError>() {
        write!(f, "script={:?}", source.kind())
    } else if let Some(source) = error.downcast_ref::<limen_config::ConfigError>() {
        write!(f, "config={:?}", source.kind)
    } else if let Some(source) = error.downcast_ref::<Failure>() {
        write!(
            f,
            "gateway={}, phase={}",
            source.code.as_str(),
            source.phase
        )
    } else if let Some(source) = error.downcast_ref::<reqwest::Error>() {
        write!(
            f,
            "upstream_timeout={}, upstream_connect={}",
            source.is_timeout(),
            source.is_connect()
        )
    } else if let Some(source) = error.downcast_ref::<serde_json::Error>() {
        write!(
            f,
            "json={:?}, line={}, column={}",
            source.classify(),
            source.line(),
            source.column()
        )
    } else if let Some(source) = error.downcast_ref::<limen_chat::Error>() {
        // Chat Display renders fixed categories and never formats its JSON source.
        write!(f, "chat={source}")
    } else if let Some(source) = error.downcast_ref::<std::io::Error>() {
        write!(
            f,
            "io_kind={:?}, os_code={:?}",
            source.kind(),
            source.raw_os_error()
        )
    } else if error.is::<http_body_util::LengthLimitError>() {
        f.write_str("body_length_limit")
    } else {
        f.write_str("opaque")
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(self, f) }
}

#[cfg(test)]
mod tests;
