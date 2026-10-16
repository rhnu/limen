use http::{HeaderMap, HeaderName, HeaderValue, header};
use limen_config::{BusinessConfig, ClientAuth, Model, Provider, ResourceLimits};
use serde_json::{Value, json};

use super::error::{ErrorCode, Failure};

pub fn authorize(auth: &ClientAuth, headers: &HeaderMap) -> Result<(), Failure> {
    let ClientAuth::Bearer(expected) = auth else {
        return Ok(());
    };
    let invalid = || Failure::new(ErrorCode::Unauthorized, "authentication");
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or_else(invalid)?;
    if values.next().is_some() {
        return Err(invalid());
    }
    let value = value
        .to_str()
        .map_err(|source| invalid().with_source(source))?;
    let (scheme, token) = value.split_once(' ').ok_or_else(invalid)?;
    let token = token.trim_start_matches(' ');
    if !scheme.eq_ignore_ascii_case("bearer")
        || !secret_equal(token.as_bytes(), expected.as_bytes())
    {
        return Err(invalid());
    }
    Ok(())
}

/// Capture only configured fields, excluding connection-nominated hop-by-hop data.
/// Header bytes remain private request data and are never added to diagnostics.
pub fn client_context(headers: &HeaderMap, allowed: &[String]) -> Result<Value, Failure> {
    let mut fields = Vec::new();
    let mut bytes = 0_usize;
    for name in allowed {
        let mut connection_field = false;
        for connection in headers.get_all(header::CONNECTION) {
            let connection = connection.to_str().map_err(|source| {
                Failure::new(ErrorCode::InvalidRequest, "client_context").with_source(source)
            })?;
            connection_field |= connection
                .split(',')
                .any(|field| field.trim().eq_ignore_ascii_case(name));
        }
        if connection_field {
            continue;
        }
        for value in headers.get_all(name) {
            bytes = bytes
                .saturating_add(name.len())
                .saturating_add(value.as_bytes().len());
            if fields.len() >= 100 || bytes > 32 * 1024 {
                return Err(Failure::new(ErrorCode::PayloadTooLarge, "client_context"));
            }
            let value = std::str::from_utf8(value.as_bytes()).map_err(|source| {
                Failure::new(ErrorCode::InvalidRequest, "client_context").with_source(source)
            })?;
            fields.push(json!({"name": name, "value": value}));
        }
    }
    Ok(json!({"headers":fields}))
}

// Compare every byte once lengths match; authentication never exits at the first difference.
fn secret_equal(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

pub fn parse_request(bytes: &[u8], limits: &ResourceLimits) -> Result<(Value, bool), Failure> {
    if bytes.len() > limits.max_body_bytes {
        return Err(Failure::new(ErrorCode::PayloadTooLarge, "request_body"));
    }
    let body = serde_json::from_slice(bytes).map_err(|source| {
        Failure::new(ErrorCode::InvalidRequest, "request_body").with_source(source)
    })?;
    validate_body(&body, limits)?;
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    Ok((body, stream))
}

pub fn validate_body(body: &Value, limits: &ResourceLimits) -> Result<(), Failure> {
    let invalid = || Failure::new(ErrorCode::InvalidRequest, "request_body");
    limen_config::validate_json(body, limits.max_json_depth, limits.max_json_nodes)
        .map_err(|source| invalid().with_source(source))?;
    validate_body_shape(body)
}

/// Check Chat fields after the caller has established the generic JSON boundary.
pub fn validate_body_shape(body: &Value) -> Result<(), Failure> {
    let invalid = || Failure::new(ErrorCode::InvalidRequest, "request_body");
    let object = body.as_object().ok_or_else(invalid)?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if model.is_empty() || model.len() > 1024 || model.chars().any(char::is_control) {
        return Err(invalid());
    }
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if messages.is_empty()
        || messages.iter().any(|message| !message.is_object())
        || object
            .get("stream")
            .is_some_and(|stream| !stream.is_boolean())
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn merge_body(provider: &Provider, model: &Model, client: Value) -> Result<Value, Failure> {
    let Value::Object(mut body) = client else {
        return Err(Failure::new(ErrorCode::InvalidRequest, "request_body"));
    };
    for defaults in [&model.defaults, &provider.defaults] {
        for (key, value) in defaults {
            if !body.contains_key(key) {
                body.insert(key.clone(), value.clone());
            }
        }
    }
    body.insert("model".to_owned(), Value::String(model.upstream.clone()));
    Ok(Value::Object(body))
}

pub fn outbound_headers(provider: &Provider) -> Result<HeaderMap, Failure> {
    let invalid = || Failure::new(ErrorCode::Internal, "upstream_headers");
    let mut headers = HeaderMap::new();
    // Static names, transport policy and authentication conflicts were validated
    // at startup. HTTP types still own the actual header-byte conversion here.
    for (name, value) in &provider.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| invalid().with_source(source))?;
        let value = HeaderValue::from_str(value).map_err(|source| invalid().with_source(source))?;
        headers.insert(name, value);
    }
    if let Some(auth) = &provider.auth {
        let value = HeaderValue::from_str(&format!("Bearer {}", auth.bearer))
            .map_err(|source| invalid().with_source(source))?;
        headers.insert(header::AUTHORIZATION, value);
    }
    Ok(headers)
}

pub fn response_headers(upstream: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        if matches!(
            name.as_str(),
            "content-language"
                | "x-request-id"
                | "retry-after"
                | "www-authenticate"
                | "location"
                | "ratelimit"
                | "ratelimit-policy"
        ) || name.as_str().starts_with("x-ratelimit-")
            || name.as_str().starts_with("ratelimit-")
        {
            if connection_nominated(upstream, name.as_str()) {
                continue;
            }
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

pub fn connection_nominated(headers: &HeaderMap, name: &str) -> bool {
    headers.get_all(header::CONNECTION).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(name))
        })
    })
}

pub fn headers_to_json(headers: &HeaderMap) -> Result<Value, Failure> {
    let entries = headers
        .iter()
        .map(|(name, value)| {
            let value = std::str::from_utf8(value.as_bytes()).map_err(|source| {
                Failure::new(ErrorCode::UpstreamProtocol, "headers").with_source(source)
            })?;
            Ok(json!({"name": name.as_str(), "value": value}))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    Ok(Value::Array(entries))
}

pub fn headers_from_json(value: &Value) -> Result<HeaderMap, Failure> {
    let invalid = || Failure::new(ErrorCode::HookFailed, "headers");
    let entries = value.as_array().ok_or_else(invalid)?;
    let mut headers = HeaderMap::new();
    for entry in entries {
        let object = entry.as_object().ok_or_else(invalid)?;
        if object.len() != 2 {
            return Err(invalid());
        }
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let value = object
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| invalid().with_source(source))?;
        if reserved_header(name.as_str()) {
            return Err(invalid());
        }
        let value = HeaderValue::from_str(value).map_err(|source| invalid().with_source(source))?;
        headers.append(name, value);
    }
    Ok(headers)
}

fn reserved_header(name: &str) -> bool {
    matches!(
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
}

pub fn validate_url(url: &str) -> Result<(), Failure> {
    limen_config::validate_endpoint(url)
        .map_err(|source| Failure::new(ErrorCode::HookFailed, "before_send").with_source(source))
}

/// Diagnostic target omits all path/query/userinfo: each can contain request secrets.
pub fn safe_target(endpoint: &str) -> String {
    match url::Url::parse(endpoint) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => {
            format!(
                "{}://{}",
                url.scheme(),
                &url[url::Position::BeforeHost..url::Position::AfterPort]
            )
        },
        _ => "invalid_endpoint".to_owned(),
    }
}

pub fn models_json(business: &BusinessConfig) -> Value {
    json!({"object": "list", "data": business.models.keys().map(|name| json!({"id": name, "object": "model", "created": 0, "owned_by": "limen"})).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests;
