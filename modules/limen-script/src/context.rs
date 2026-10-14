use http::{HeaderName, HeaderValue};

/// Return the normalized name only when HTTP permits the field and value.
/// Read access accepts transport fields; mutation leaves them under Rust ownership.
pub(crate) fn header_name(name: &str, value: &str, writable: bool) -> Option<String> {
    let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
    HeaderValue::from_str(value).ok()?;
    if writable
        && matches!(
            name.as_str(),
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
        return None;
    }
    Some(name.as_str().to_owned())
}

#[cfg(test)]
mod tests;
