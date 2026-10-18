use std::{path::Path, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request},
};
use limen_config::{ServerConfig, ServerFormat};
use limen_script::Cancellation;
use serde_json::Value;

use crate::{bootstrap::Loaded, gateway::FixtureResponse};

async fn configured(root: &Path, manager: bool) -> Loaded {
    std::fs::write(root.join("gateway.js"),"import {defineGateway} from 'limen:gateway'; export default defineGateway(gateway=>{ const p=gateway.provider('p',{endpoint:'https://fictional.invalid/v1/chat/completions'}); p.model('chat',{upstream:'synthetic'}); gateway.onRequest(()=>{throw Error('generation must not execute');}); });").unwrap();
    let mut source = String::from("[auth]\napi_key='synthetic-client-key-0123456789012345'\n");
    if manager {
        source.push_str("manager_key='synthetic-manager-key-0123456789012345'\n");
    }
    let server = ServerConfig::parse(&source, ServerFormat::Toml).unwrap();
    Loaded::start(&root.join("server.toml"), server)
        .await
        .unwrap()
}

fn request(method: Method, path: &str, key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn dispatch_preserves_authentication_namespaces_methods_and_management_cache_policy() {
    let root = tempfile::tempdir().unwrap();
    let loaded = configured(root.path(), true).await;
    let (application, trace) = crate::offline::application(
        &loaded,
        FixtureResponse {
            status: 200,
            headers: http::HeaderMap::new(),
            body: Vec::new(),
        },
    );
    for (method, path, key, status, allow, management) in [
        (Method::GET, "/v1/unknown", None, 401, None, false),
        (
            Method::GET,
            "/v1/unknown",
            Some("synthetic-client-key-0123456789012345"),
            404,
            None,
            false,
        ),
        (Method::GET, "/admin/v1/unknown", None, 401, None, true),
        (
            Method::GET,
            "/admin/v1/unknown",
            Some("synthetic-client-key-0123456789012345"),
            401,
            None,
            true,
        ),
        (
            Method::GET,
            "/admin/v1/unknown",
            Some("synthetic-manager-key-0123456789012345"),
            404,
            None,
            true,
        ),
        (
            Method::POST,
            "/v1/models",
            Some("synthetic-client-key-0123456789012345"),
            405,
            Some("GET"),
            false,
        ),
        (
            Method::GET,
            "/v1/chat/completions",
            Some("synthetic-client-key-0123456789012345"),
            405,
            Some("POST"),
            false,
        ),
        (Method::GET, "/usage", None, 404, None, false),
        (Method::GET, "/assets/missing.js", None, 404, None, false),
        (Method::HEAD, "/", None, 200, None, false),
    ] {
        let response = Arc::clone(&application)
            .handle(request(method, path, key), Cancellation::new())
            .await;
        assert_eq!(response.status().as_u16(), status, "{path}");
        assert_eq!(
            response
                .headers()
                .get("allow")
                .and_then(|value| value.to_str().ok()),
            allow,
            "{path}"
        );
        if management {
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(
                response.headers()["x-limen-request-id"]
                    .to_str()
                    .unwrap()
                    .starts_with("limen-m-")
            );
        }
    }
    assert!(trace.lock().await.sent.is_none());
    application.drain().await;
    loaded.shutdown().await.unwrap();
}

#[tokio::test]
async fn models_output_budget_keeps_payload_too_large_and_disabled_management_stays_hidden() {
    let root = tempfile::tempdir().unwrap();
    let mut loaded = configured(root.path(), false).await;
    loaded.server.limits.max_output_bytes = 1;
    let (application, trace) = crate::offline::application(
        &loaded,
        FixtureResponse {
            status: 200,
            headers: http::HeaderMap::new(),
            body: Vec::new(),
        },
    );
    let response = Arc::clone(&application)
        .handle(
            request(
                Method::GET,
                "/v1/models",
                Some("synthetic-client-key-0123456789012345"),
            ),
            Cancellation::new(),
        )
        .await;
    assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(value["error"]["code"], "payload_too_large");
    let response = Arc::clone(&application)
        .handle(
            request(
                Method::GET,
                "/admin/v1/status",
                Some("synthetic-manager-key-0123456789012345"),
            ),
            Cancellation::new(),
        )
        .await;
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(trace.lock().await.sent.is_none());
    application.drain().await;
    loaded.shutdown().await.unwrap();
}
