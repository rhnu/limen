use std::{fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request},
};
use limen_config::load_server;
use limen_script::Cancellation;
use serde_json::{Value, json};

use crate::{
    bootstrap::Loaded,
    gateway::{FixtureResponse, Trace},
};

struct Output {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    trace: Trace,
}

fn upstream(status: u16, media: Option<&str>, body: &[u8]) -> FixtureResponse {
    let mut headers = HeaderMap::new();
    if let Some(media) = media {
        headers.insert("content-type", HeaderValue::from_str(media).unwrap());
    }
    FixtureResponse {
        status,
        headers,
        body: body.to_vec(),
    }
}

/// Only the transport is synthetic; policy, terminal delivery and observers are real.
async fn run(policy: &str, incoming: FixtureResponse, stream: bool, limit: usize) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let server = directory.path().join("server.toml");
    fs::write(
        &server,
        "[auth]\napi_key = 'synthetic-client-secret-0123456789'\n[observability]\nevents = ['adapted']\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("gateway.js"),
        format!(
            r#"
        import {{defineGateway}} from "limen:gateway";
        export default defineGateway(gateway => {{
            const provider = gateway.provider("p", {{endpoint: "http://upstream.invalid/completions"}});
            const model = provider.model("chat", {{upstream: "real"}});
            {policy}
        }});
    "#
        ),
    )
    .unwrap();
    let (path, settings) = load_server(Some(&server)).unwrap();
    let mut loaded = Loaded::start(&path, settings).await.unwrap();
    loaded.server.limits.max_output_bytes = limit;
    loaded.server.limits.hook_ms = 50;
    let (gateway, trace) = crate::offline::generation(&loaded, incoming);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", "Bearer synthetic-client-secret-0123456789")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model":"chat","messages":[{}],"stream":stream}).to_string(),
        ))
        .unwrap();
    let response = Arc::clone(&gateway)
        .handle_generation(request, "limen-r-test-1".into(), Cancellation::new())
        .await;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 16 * 1024).await;
    gateway.drain().await;
    loaded.shutdown().await.unwrap();
    let trace = std::mem::take(&mut *trace.lock().await);
    Output {
        status,
        headers,
        body: body.unwrap().to_vec(),
        trace,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn upstream_errors_preserve_status_raw_body_media_and_failure_metadata() {
    for status in [302, 401, 429, 503] {
        for stream in [false, true] {
            let body =
                br#"{ "error": {"code":"vendor_code","message":"vendor detail"}, "model":"real" }"#;
            let mut incoming = upstream(
                status,
                Some("application/problem+json; charset=utf-8"),
                body,
            );
            for (name, value) in [
                ("retry-after", "30"),
                ("www-authenticate", "Bearer"),
                ("x-ratelimit-remaining", "0"),
                ("location", "/login"),
                ("set-cookie", "private=1"),
            ] {
                incoming
                    .headers
                    .insert(name, HeaderValue::from_static(value));
            }
            let output = run("", incoming, stream, 4096).await;
            assert_eq!(output.status, status);
            assert_eq!(output.body, body);
            assert_eq!(
                output.headers["content-type"],
                "application/problem+json; charset=utf-8"
            );
            assert_eq!(output.headers["retry-after"], "30");
            assert_eq!(output.headers["www-authenticate"], "Bearer");
            assert_eq!(output.headers["x-ratelimit-remaining"], "0");
            assert_eq!(output.headers["location"], "/login");
            assert!(!output.headers.contains_key("set-cookie"));
            assert_eq!(
                output.trace.error,
                Some(json!({"code":"upstream_failed","status":status,"phase":"upstream_status"}))
            );
            assert_eq!(output.trace.stats["outputBytes"], body.len());
            assert_eq!(output.trace.stats["upstreamBytes"], body.len());
            assert_eq!(output.trace.stats["usage"]["upstreamCompleted"], false);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn non_json_empty_and_binary_errors_do_not_require_chat_schema() {
    for (media, body) in [
        (Some("text/html"), b"<h1>unavailable</h1>".as_slice()),
        (None, b"".as_slice()),
        (Some("application/json"), b"{broken".as_slice()),
        (Some("application/octet-stream"), &[0, 255, 128][..]),
    ] {
        let output = run("", upstream(502, media, body), false, 4096).await;
        assert_eq!(output.status, 502);
        assert_eq!(output.body, body);
        assert_eq!(
            output
                .headers
                .get("content-type")
                .map(|v| v.to_str().unwrap()),
            media
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn error_policy_commits_provider_model_global_in_order_before_readonly_observers() {
    let policy = include_str!("../../../fixtures/gateway/error-response-unit/ordered-policy.js");
    let output = run(
        policy,
        upstream(
            429,
            Some("application/json"),
            br#"{"error":{"code":"vendor"}}"#,
        ),
        true,
        4096,
    )
    .await;
    assert_eq!(output.status, 409);
    assert_eq!(output.headers["content-type"], "application/problem+json");
    assert_eq!(output.headers["x-adapted"], "yes");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.body).unwrap(),
        json!({"error":{"code":"vendor"},"provider":true,"model":true})
    );
    assert_eq!(
        output.trace.state,
        json!({"order":["provider","model","global"]})
    );
    assert!(!output.trace.observation_failed);
    assert_eq!(
        output.trace.observations["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        output.trace.stats["timings"]["hooks"]
            .get("onErrorResponse")
            .is_none()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_failed_and_timed_out_error_policies_keep_last_commit_without_recursion() {
    for operation in [
        "ctx.status = 200;",
        "ctx.body = {};",
        "ctx.contentType = 'bad\\r\\nvalue';",
        "ctx.body = 'x'.repeat(5000);",
        "throw Error('private exception');",
        "return new Promise(() => {});",
    ] {
        let policy = format!(
            r#"
            provider.use({{name:"first", onErrorResponse(ctx) {{
                ctx.state.first = true; ctx.status = 418; ctx.body = "committed"; ctx.contentType = "text/plain";
            }}}});
            model.use({{name:"bad", onErrorResponse(ctx) {{
                ctx.state.bad = true; ctx.headers.set("x-bad", "yes"); ctx.observe.record("adapted"); {operation}
            }}}});
            gateway.onErrorResponse(ctx => {{ ctx.state.recursed = true; }});
        "#
        );
        let output = run(
            &policy,
            upstream(429, Some("application/json"), b"{}"),
            false,
            4096,
        )
        .await;
        assert_eq!(output.status, 418);
        assert_eq!(output.body, b"committed");
        assert!(!output.headers.contains_key("x-bad"));
        assert_eq!(output.trace.state, json!({"first":true}));
        assert_eq!(
            output.trace.error.as_ref().unwrap()["code"],
            "upstream_failed"
        );
        assert_eq!(output.trace.observations["events"], json!([]));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn global_error_policy_handles_unrouted_gateway_failures() {
    let output = run(
        include_str!("../../../fixtures/gateway/error-response-unit/unrouted-policy.js"),
        upstream(200, Some("application/json"), b"{}"),
        false,
        4096,
    )
    .await;
    assert_eq!(output.status, 422);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.body).unwrap(),
        json!({"error":{"code":"tenant_required"}})
    );
    assert!(output.trace.sent.is_none());
    assert_eq!(output.trace.state, json!({}));
}

#[tokio::test(flavor = "current_thread")]
async fn binary_or_encoded_error_body_can_be_preserved_or_explicitly_replaced() {
    for encoded in [false, true] {
        for replace in [false, true] {
            let mut incoming = upstream(503, None, &[0xFF, 0x00]);
            if encoded {
                incoming
                    .headers
                    .insert("content-encoding", HeaderValue::from_static("gzip"));
                incoming
                    .headers
                    .append("content-encoding", HeaderValue::from_static("br"));
            }
            let operation = if replace {
                "ctx.body = 'replacement'; ctx.contentType = 'text/plain';"
            } else {
                ""
            };
            let policy = format!(
                "gateway.onErrorResponse(ctx => {{ if (ctx.body !== null) throw Error(); {operation} }});"
            );
            let output = run(&policy, incoming, false, 4096).await;
            assert_eq!(output.status, 503);
            assert_eq!(
                output.body,
                if replace {
                    b"replacement".as_slice()
                } else {
                    &[0xFF, 0x00][..]
                }
            );
            assert_eq!(
                output.headers.contains_key("content-encoding"),
                encoded && !replace
            );
            if encoded && !replace {
                assert_eq!(
                    output
                        .headers
                        .get_all("content-encoding")
                        .iter()
                        .collect::<Vec<_>>(),
                    ["gzip", "br"]
                );
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn upstream_error_headers_still_strip_connection_nominated_fields() {
    let mut incoming = upstream(429, Some("text/plain"), b"limited");
    incoming
        .headers
        .insert("retry-after", HeaderValue::from_static("30"));
    incoming
        .headers
        .insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
    incoming
        .headers
        .insert("content-encoding", HeaderValue::from_static("identity"));
    incoming.headers.insert(
        "connection",
        HeaderValue::from_static(
            "Retry-After, X-RateLimit-Remaining, Content-Type, Content-Encoding",
        ),
    );
    let output = run("", incoming, false, 4096).await;
    assert_eq!(output.status, 429);
    assert_eq!(output.body, b"limited");
    for name in [
        "retry-after",
        "x-ratelimit-remaining",
        "content-type",
        "content-encoding",
        "connection",
    ] {
        assert!(!output.headers.contains_key(name));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_upstream_error_is_never_forwarded_as_a_complete_body() {
    let output = run(
        "",
        upstream(429, Some("text/plain"), &[b'x'; 4097]),
        false,
        4096,
    )
    .await;
    assert_eq!(output.status, 502);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.body).unwrap()["error"]["code"],
        "upstream_protocol"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn started_stream_skips_error_response_policy_and_never_appends_done() {
    let body = b"data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"real\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n";
    let output = run(
        "gateway.onErrorResponse(ctx => { ctx.state.rewritten = true; });",
        upstream(200, Some("text/event-stream"), body),
        true,
        4096,
    )
    .await;
    assert_eq!(output.status, 200);
    assert_eq!(output.headers["content-type"], "text/event-stream");
    let text = std::str::from_utf8(&output.body).unwrap();
    assert!(text.contains("event: error"));
    assert!(!text.contains("[DONE]"));
    assert_eq!(output.trace.state, json!({}));
    assert_eq!(
        output.trace.error.as_ref().unwrap()["code"],
        "upstream_protocol"
    );
}
