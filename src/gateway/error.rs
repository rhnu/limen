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
    RequestRejected,
    HookFailed,
    UpstreamFailed,
    UpstreamProtocol,
    Internal,
    Unavailable,
    StorageUnavailable,
    NotFound,
    MethodNotAllowed,
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
            Self::RequestRejected => "request_rejected",
            Self::HookFailed => "hook_failed",
            Self::UpstreamFailed => "upstream_failed",
            Self::UpstreamProtocol => "upstream_protocol",
            Self::Internal => "internal_error",
            Self::Unavailable => "unavailable",
            Self::StorageUnavailable => "storage_unavailable",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
        }
    }

    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::UnknownModel | Self::RequestRejected => 400,
            Self::Unauthorized => 401,
            Self::PayloadTooLarge => 413,
            Self::Busy | Self::StorageUnavailable | Self::Unavailable => 503,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::DeadlineExceeded => 504,
            Self::Cancelled => 499,
            Self::HookFailed | Self::Internal => 500,
            Self::UpstreamFailed | Self::UpstreamProtocol => 502,
        }
    }
}

/// Success remains a body-bearing 2xx response; upstream failures retain 3xx/4xx/5xx.
/// Informational statuses and bodyless 204/205 cannot represent completions.
///
/// # Errors
/// Returns `HookFailed` when the requested mapping changes success semantics.
pub fn validate_response_status(upstream: u16, downstream: u16) -> Result<(), ErrorCode> {
    let allowed = if (200..300).contains(&upstream) {
        (200..300).contains(&downstream) && !matches!(downstream, 204 | 205)
    } else {
        (300..600).contains(&upstream) && (300..600).contains(&downstream)
    };
    if allowed {
        Ok(())
    } else {
        Err(ErrorCode::HookFailed)
    }
}

/// Request failure with safe diagnostics, typed sources and private response data.
pub struct Failure {
    pub code: ErrorCode,
    pub phase: &'static str,
    pub step: Option<String>,
    pub status: Option<u16>,
    /// Numeric HTTP error code declared by an upstream structured error event.
    pub upstream_error_status: Option<u16>,
    pub response: Option<Box<FailureResponse>>,
    deadline_kind: DeadlineKind,
    source: Option<Box<dyn Error + Send + Sync>>,
}

/// Retain the deadline source while sharing the public code and HTTP status.
#[derive(Clone, Copy)]
enum DeadlineKind {
    Request,
    Hook,
}

/// Private response data is never included in failure diagnostics or observations.
pub struct FailureResponse {
    pub body: Vec<u8>,
    pub content_type: Option<http::HeaderValue>,
    pub content_encoding: Vec<http::HeaderValue>,
}

impl Failure {
    pub const fn new(code: ErrorCode, phase: &'static str) -> Self {
        Self {
            code,
            phase,
            step: None,
            status: None,
            upstream_error_status: None,
            response: None,
            deadline_kind: DeadlineKind::Request,
            source: None,
        }
    }

    pub const fn hook_timeout(phase: &'static str) -> Self {
        let mut failure = Self::new(ErrorCode::DeadlineExceeded, phase);
        failure.deadline_kind = DeadlineKind::Hook;
        failure
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
        let mut failure = if code == ErrorCode::DeadlineExceeded {
            Self::hook_timeout(phase)
        } else {
            Self::new(code, phase)
        }
        .with_source(source);
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
            .filter(|status| {
                (400..600).contains(status)
                    || (self.code == ErrorCode::UpstreamFailed && (300..400).contains(status))
            })
            .unwrap_or_else(|| self.code.status())
    }

    pub fn observer_error(&self, request_id: &str) -> Value {
        let mut error = self.error_json(request_id)["error"].take();
        error["status"] = json!(self.http_status());
        error["phase"] = json!(self.phase);
        error["step"] = json!(self.step);
        error
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
            ErrorCode::DeadlineExceeded => match self.deadline_kind {
                DeadlineKind::Request => "The request deadline was exceeded",
                DeadlineKind::Hook => "The hook deadline was exceeded",
            },
            ErrorCode::Cancelled => "The request was cancelled",
            ErrorCode::RequestRejected => "The request was rejected by script policy",
            ErrorCode::HookFailed => "A configured request hook failed",
            ErrorCode::UpstreamFailed => "The upstream request failed",
            ErrorCode::UpstreamProtocol => "The upstream response is not supported",
            ErrorCode::Internal => "The gateway could not complete the request",
            ErrorCode::Unavailable => "The gateway is unavailable",
            ErrorCode::StorageUnavailable => "Request storage is unavailable",
            ErrorCode::NotFound => "The endpoint or record was not found",
            ErrorCode::MethodNotAllowed => "The request method is not allowed",
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
            "upstream_timeout={}, upstream_connect={}, upstream_builder={}, upstream_request={}, upstream_body={}, upstream_decode={}, upstream_redirect={}, upstream_status={:?}",
            source.is_timeout(),
            source.is_connect(),
            source.is_builder(),
            source.is_request(),
            source.is_body(),
            source.is_decode(),
            source.is_redirect(),
            source.status().map(|status| status.as_u16())
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
