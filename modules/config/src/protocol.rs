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

#[cfg(test)]
mod tests;
