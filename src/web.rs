//! Serve the compiled management shell without entering generation admission or the VM.

use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use include_dir::{Dir, include_dir};

// Frontend compilation is a prerequisite for Rust builds, including debug builds.
static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// Unknown paths stay with the gateway's existing endpoint error handling.
pub(crate) async fn handle(request: Request<Body>, body_timeout: Duration) -> Option<Response> {
    serve(request, body_timeout, &DIST).await
}

async fn serve(
    request: Request<Body>,
    body_timeout: Duration,
    directory: &Dir<'static>,
) -> Option<Response> {
    let (file_path, content_type, cache) = resource(request.uri().path())?;
    let bytes = directory.get_file(file_path)?.contents();
    if request.method() != Method::GET && request.method() != Method::HEAD {
        let mut response = empty_response(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return Some(response);
    }
    let head = request.method() == Method::HEAD;
    // The same zero-byte bound applies to GET and HEAD. The timeout also bounds
    // chunked bodies whose sender never finishes, without buffering their data.
    let body = tokio::time::timeout(body_timeout, to_bytes(request.into_body(), 0)).await;
    if !matches!(body, Ok(Ok(_))) {
        return Some(empty_response(StatusCode::BAD_REQUEST));
    }
    let mut response = Response::new(if head {
        Body::empty()
    } else {
        Body::from(bytes)
    });
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    security_headers(&mut response);
    Some(response)
}

/// Resolve only the public build outputs; never decode or normalize URL paths.
fn resource(path: &str) -> Option<(&str, &'static str, &'static str)> {
    match path {
        "/" => Some(("index.html", "text/html; charset=utf-8", "no-cache")),
        "/favicon.svg" => Some(("favicon.svg", "image/svg+xml", "no-cache")),
        _ => {
            let asset = path.strip_prefix("/assets/")?;
            if asset.split('/').any(|part| {
                part.is_empty()
                    || part.starts_with('.')
                    || !part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            }) {
                return None;
            }
            let (_, extension) = asset.rsplit_once('.')?;
            let content_type = match extension {
                "js" => "text/javascript; charset=utf-8",
                "css" => "text/css; charset=utf-8",
                "svg" => "image/svg+xml",
                "woff" => "font/woff",
                "woff2" => "font/woff2",
                "ttf" => "font/ttf",
                "otf" => "font/otf",
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "avif" => "image/avif",
                "ico" => "image/vnd.microsoft.icon",
                _ => return None,
            };
            Some((
                &path[1..],
                content_type,
                "public, max-age=31536000, immutable",
            ))
        },
    }
}

fn empty_response(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    security_headers(&mut response);
    response
}

fn security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
}

#[cfg(test)]
mod tests;
