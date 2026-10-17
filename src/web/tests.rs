use include_dir::{DirEntry, File};

use super::*;

static FILES: Dir<'_> = Dir::new(
    "",
    &[
        DirEntry::File(File::new("index.html", b"<html></html>")),
        DirEntry::File(File::new("favicon.svg", b"<svg/>")),
        DirEntry::File(File::new("assets/app-12345678.js", b"export {}")),
        DirEntry::File(File::new("assets/app-12345678.css", b"body{}")),
        DirEntry::File(File::new("assets/icon-12345678.svg", b"<svg/>")),
        DirEntry::File(File::new("assets/font-12345678.woff2", b"font")),
        DirEntry::File(File::new("assets/image-12345678.png", b"image")),
        DirEntry::File(File::new("assets/app-12345678.js.map", b"private")),
        DirEntry::File(File::new("assets/runtime-config.json", b"private")),
        DirEntry::File(File::new("openapi.json", b"private")),
        DirEntry::File(File::new("v1/models", b"private")),
        DirEntry::File(File::new("admin/v1/status", b"private")),
    ],
);

async fn send(method: Method, path: &str, body: Body) -> Option<Response> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(body)
        .unwrap();
    serve(request, Duration::from_secs(1), &FILES).await
}

#[tokio::test]
async fn shell_is_public_html_with_revalidation_and_self_only_resources() {
    let response = send(Method::GET, "/?ignored=query", Body::empty())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "13");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    let policy = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(policy.contains("script-src 'self'"));
    assert!(policy.contains("style-src 'self' 'unsafe-inline'"));
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
    assert_eq!(
        to_bytes(response.into_body(), 100).await.unwrap(),
        "<html></html>"
    );
}

#[tokio::test]
async fn head_matches_get_metadata_and_has_no_body() {
    for path in ["/", "/assets/app-12345678.js", "/favicon.svg"] {
        let get = send(Method::GET, path, Body::empty()).await.unwrap();
        let head = send(Method::HEAD, path, Body::empty()).await.unwrap();
        assert_eq!(head.status(), get.status());
        assert_eq!(head.headers(), get.headers());
        assert!(to_bytes(head.into_body(), 0).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn compiled_assets_have_correct_mime_and_immutable_cache() {
    for (path, mime) in [
        ("/assets/app-12345678.js", "text/javascript; charset=utf-8"),
        ("/assets/app-12345678.css", "text/css; charset=utf-8"),
        ("/assets/icon-12345678.svg", "image/svg+xml"),
        ("/assets/font-12345678.woff2", "font/woff2"),
        ("/assets/image-12345678.png", "image/png"),
    ] {
        let response = send(Method::GET, path, Body::empty()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        let length = response.headers()[header::CONTENT_LENGTH]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap().len(),
            length
        );
    }
    let favicon = send(Method::GET, "/favicon.svg", Body::empty())
        .await
        .unwrap();
    assert_eq!(favicon.headers()[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn unsupported_methods_apply_only_to_existing_resources() {
    for path in ["/", "/favicon.svg", "/assets/app-12345678.js"] {
        for method in [Method::POST, Method::OPTIONS, Method::DELETE] {
            let response = send(method, path, Body::empty()).await.unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
        }
    }
    assert!(
        send(Method::POST, "/assets/missing.js", Body::empty())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn unknown_and_private_paths_never_fall_back_to_the_shell() {
    for path in [
        "/unknown",
        "/index.html",
        "/requests",
        "/assets/",
        "/assets/missing.js",
        "/assets/app-12345678.js.map",
        "/assets/runtime-config.json",
        "/openapi.json",
        "/v1",
        "/v1/models",
        "/v1/missing",
        "/admin/v1",
        "/admin/v1/status",
        "/admin/v1/missing",
    ] {
        assert!(
            send(Method::GET, path, Body::empty()).await.is_none(),
            "{path}"
        );
    }
}

#[tokio::test]
async fn traversal_and_encoded_paths_are_rejected_without_normalization() {
    for path in [
        "/assets/../index.html",
        "/assets/./app-12345678.js",
        "/assets//app-12345678.js",
        "/assets/%2e%2e/index.html",
        "/assets/%61pp-12345678.js",
        "/assets/%252e%252e/index.html",
        "/assets/app-12345678.js/",
        "/assets/.hidden.js",
        "/assets/app-12345678.js%00",
    ] {
        assert!(
            send(Method::GET, path, Body::empty()).await.is_none(),
            "{path}"
        );
    }
    assert!(resource("/assets/folder\\app-12345678.js").is_none());
}

#[tokio::test]
async fn get_and_head_reject_nonempty_bodies_with_a_zero_byte_bound() {
    for method in [Method::GET, Method::HEAD] {
        let response = send(method, "/", Body::from(vec![0_u8; 4096]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
}

struct PendingBody;

impl axum::body::HttpBody for PendingBody {
    type Data = axum::body::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn unknown_length_bodies_that_never_finish_have_a_deadline() {
    for method in [Method::GET, Method::HEAD] {
        let request = Request::builder()
            .method(method)
            .uri("/")
            .body(Body::new(PendingBody))
            .unwrap();
        let response = serve(request, Duration::ZERO, &FILES).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(to_bytes(response.into_body(), 0).await.unwrap().is_empty());
    }
}
