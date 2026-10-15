//! Authentication, one-time routing, draft commits, and upstream failure contracts.

use anyhow::{Context, Result, ensure};
use http::Method;
use serde_json::{Value, json};

use super::fixture::{Laboratory, Options, request};

/// Check both protected endpoints, explicit models, header isolation, and single sends.
pub async fn auth_routing(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(Options::default(), async |item| {
            for (method, path) in [
                (Method::GET, "/v1/models"),
                (Method::POST, "/v1/chat/completions"),
            ] {
                item.send(request(&[]), method, path, &[("Authorization", None)])
                    .await?
                    .error(401, "unauthorized", "both endpoints must authenticate")?;
            }
            let reply = item
                .send(request(&[]), Method::GET, "/v1/models", &[])
                .await?;
            let models = reply.body["data"]
                .as_array()
                .context("model list data is not an array")?;
            ensure!(
                reply.status == 200
                    && models.iter().map(|model| &model["id"]).collect::<Vec<_>>()
                        == vec![&json!("chat")],
                "explicit model list"
            );
            item.request(request(&[("model", json!("unknown"))]))
                .await?
                .error(400, "unknown_model", "unknown route rejected")?;
            ensure!(
                item.upstream.controls.lock().await.records.is_empty(),
                "rejections must not send upstream"
            );
            let reply = item
                .send(
                    request(&[
                        ("temperature", Value::Null),
                        ("unknownExtension", json!({"keep":true})),
                    ]),
                    Method::POST,
                    "/v1/chat/completions",
                    &[
                        ("Cookie", Some("synthetic-cookie-secret=1")),
                        ("X-Unrelated", Some("hidden")),
                    ],
                )
                .await?;
            ensure!(
                reply.status == 200 && reply.body["model"] == "chat",
                "response alias"
            );
            {
                let upstream = item.upstream.controls.lock().await;
                let recorded = upstream
                    .records
                    .first()
                    .context("generation request not recorded")?;
                ensure!(
                    recorded
                        .path
                        .ends_with("/custom/completion?key=synthetic-query-secret"),
                    "complete configured URL"
                );
                ensure!(
                    recorded
                        .headers
                        .get("authorization")
                        .is_some_and(|value| value == "Bearer synthetic-upstream-secret"),
                    "independent upstream auth"
                );
                ensure!(
                    !recorded.headers.contains_key("cookie")
                        && !recorded.headers.contains_key("x-unrelated"),
                    "client headers isolated"
                );
                ensure!(
                    recorded.body["model"] == "real-model"
                        && recorded.body.get("temperature") == Some(&Value::Null)
                        && recorded.body["unknownExtension"]["keep"] == true,
                    "merge null and unknown fields"
                );
            }
            ensure!(
                !reply.headers.contains_key("set-cookie"),
                "upstream cookies excluded"
            );
            item.upstream.controls.lock().await.status = 307;
            item.request(request(&[])).await?.error(
                502,
                "upstream_failed",
                "redirect is an error",
            )?;
            ensure!(
                item.upstream.controls.lock().await.records.len() == 2,
                "one upstream send per request"
            );
            Ok(())
        })
        .await
}

const PIPELINE: &str = r"export default {
 onRequest(ctx) { ctx.body.model = 'chat'; ctx.state.suffix = ctx.body.suffix || ''; },
 providers: {p: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'}, pipeline: [{name: 'provider',
  async beforeSend(ctx) { await Promise.resolve(); ctx.body.model = 'hook-model'; ctx.headers.delete('Authorization'); ctx.headers.append('X-Multiple','one'); ctx.headers.append('x-multiple','two'); ctx.url = ctx.url.replace('/custom/completion', '/modified'); },
  afterReceive(ctx) { ctx.status = 201; ctx.headers.set('X-Draft','provider'); },
  onResponse(ctx) { ctx.body.choices[0].message.content = 'hello ' + ctx.body.choices[0].message.content; }}]}},
 models: {chat: {provider: 'p', upstream: 'real-model', pipeline: [{name: 'model',
  async beforeSend(ctx) {
   if (ctx.body.fail) ctx.body.temperature = 99;
   if (ctx.body.fail === 'throw') throw new Error('synthetic-body-secret');
   if (ctx.body.fail === 'reject') await Promise.reject(new Error('synthetic-body-secret'));
   if (ctx.body.fail === 'return') return {temperature: 99};
   if (ctx.body.fail === 'cycle') ctx.body.cycle = ctx.body;
  },
  afterReceive(ctx) { ctx.headers.set('X-Draft','model'); },
  async onResponse(ctx) { await Promise.resolve(); ctx.body = {...ctx.body, model: 'explicit-alias', choices: [{...ctx.body.choices[0], message: {...ctx.body.choices[0].message, content: ctx.body.choices[0].message.content.toUpperCase() + ctx.state.suffix}}]}; }}]}}
};";

/// Check transformed client responses, failed-request isolation and final upstream requests.
pub async fn pipeline(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PIPELINE),
                ..Options::default()
            },
            async |item| {
                item.upstream.controls.lock().await.response = Some(json!({
                    "id":"synthetic", "object":"chat.completion", "created":1,
                    "model":"real-model", "choices":[{"index":0,
                        "message":{"role":"assistant","content":"world"},"finish_reason":"stop"}],
                    "usage":{"total_tokens":3}, "vendor_extension":{"preserved":true}
                }));
                let reply = item
                    .request(request(&[("model", json!("reroute-once")), ("suffix", json!("!"))]))
                    .await?;
                ensure!(
                    reply.status == 201 && reply.body["model"] == "explicit-alias",
                    "status and explicit response model"
                );
                ensure!(
                    reply.body["choices"][0]["message"]["content"] == "HELLO WORLD!"
                        && reply.body["choices"][0]["finish_reason"] == "stop"
                        && reply.body["usage"]["total_tokens"] == 3
                        && reply.body["vendor_extension"] == json!({"preserved":true}),
                    "response transformations preserve completion metadata and extensions"
                );
                ensure!(reply.header("x-draft")? == "model", "later draft wins");
                {
                    let upstream = item.upstream.controls.lock().await;
                    let recorded = upstream
                        .records
                        .first()
                        .context("pipeline request not recorded")?;
                    ensure!(
                        recorded.path.starts_with("/modified?")
                            && recorded.body["model"] == "hook-model"
                            && !recorded.headers.contains_key("authorization"),
                        "final URL/model/headers respected"
                    );
                    let values = recorded
                        .headers
                        .get_all("x-multiple")
                        .iter()
                        .map(http::HeaderValue::to_str)
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .context("read multivalue upstream headers")?;
                    ensure!(values == ["one", "two"], "header multivalue preserved");
                }
                for mode in ["throw", "reject", "return", "cycle"] {
                    let reply = item.request(request(&[("fail", json!(mode))])).await?;
                    reply.error(500, "hook_failed", "invalid hook result rejects the request")?;
                    ensure!(reply.body["error"]["request_id"].as_str().is_some_and(|id| !id.is_empty()),
                        "hook errors include a client-visible request identifier");
                    ensure!(!reply.body.to_string().contains("synthetic-body-secret"),
                        "hook exception text stays out of the public error");
                    ensure!(item.upstream.controls.lock().await.records.len() == 1,
                        "failed hook sends no partial upstream request ({mode})");
                }
                let (first, second) = tokio::join!(
                    item.request(request(&[("suffix", json!(" first"))])),
                    item.request(request(&[("suffix", json!(" second"))]))
                );
                for (reply, expected) in [(first?, "HELLO WORLD first"), (second?, "HELLO WORLD second")] {
                    ensure!(reply.status == 201 && reply.body["choices"][0]["message"]["content"] == expected,
                        "concurrent requests retain their own response policy state after a failed request");
                }
                let reply = item.request(request(&[])).await?;
                ensure!(reply.status == 201 && reply.body["choices"][0]["message"]["content"] == "HELLO WORLD",
                    "later requests do not inherit an earlier response suffix");
                Ok(())
            },
        )
        .await
}

/// Reject error statuses and invalid media before stream start, including invalid hook remaps.
pub async fn failures(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(Options::default(), async |item| {
            item.upstream.controls.lock().await.status = 503;
            item.request(request(&[])).await?.error(
                502,
                "upstream_failed",
                "upstream failures classified",
            )?;
            {
                let mut upstream = item.upstream.controls.lock().await;
                upstream.status = 200;
                upstream.media = "text/event-stream";
            }
            item.request(request(&[])).await?.error(
                502,
                "upstream_protocol",
                "JSON bytes cannot pass the SSE parser",
            )?;
            item.request(request(&[("stream", json!(true))]))
                .await?
                .error(
                    502,
                    "upstream_protocol",
                    "invalid SSE fails before starting downstream SSE",
                )?;
            Ok(())
        })
        .await?;
    let config = "export default {providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'invalid-status',afterReceive(ctx){ctx.status=200;}}]}},models:{chat:{provider:'p',upstream:'real-model'}}};";
    laboratory
        .with_fixture(
            Options {
                config: Some(config),
                ..Options::default()
            },
            async |item| {
                item.upstream.controls.lock().await.status = 500;
                item.request(request(&[])).await?.error(
                    500,
                    "hook_failed",
                    "hook cannot turn failure into success",
                )
            },
        )
        .await
}
