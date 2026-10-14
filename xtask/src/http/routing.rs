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

const PIPELINE: &str = r"let finishes = 0, failures = 0, observed = null;
export default {
 onRequest(ctx) { ctx.body.model = 'chat'; ctx.state.order = ['request']; },
 onError(ctx) { failures++; observed = ctx.state.order; },
 onFinish(ctx) { finishes++; if (!Object.isFrozen(ctx.state)) throw new Error('readonly'); },
 providers: {p: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'}, pipeline: [{name: 'provider',
  async beforeSend(ctx) { ctx.state.order.push('provider-before'); ctx.body.model = 'hook-model'; ctx.headers.delete('Authorization'); ctx.headers.append('X-Multiple','one'); ctx.headers.append('x-multiple','two'); ctx.url = ctx.url.replace('/custom/completion', '/modified'); },
  afterReceive(ctx) { ctx.state.order.push('provider-after'); ctx.status = 201; ctx.headers.set('X-Draft','provider'); },
  onResponse(ctx) { ctx.state.order.push('provider-response'); ctx.body.order = ctx.state.order; }}]}},
 models: {chat: {provider: 'p', upstream: 'real-model', pipeline: [{name: 'model',
  beforeSend(ctx) { if (ctx.body.fail) { ctx.state.order.push('discard'); throw new Error('synthetic-body-secret'); } ctx.state.order.push('model-before'); },
  afterReceive(ctx) { ctx.state.order.push('model-after'); ctx.headers.set('X-Draft','model'); },
  onResponse(ctx) { ctx.state.order.push('model-response'); ctx.body.order = ctx.state.order; ctx.body.counters = [finishes,failures]; ctx.body.observed = observed; ctx.body.model = 'explicit-alias'; }}]}}
};";

/// Verify provider/model ordering and discarded failed drafts without rerouting.
pub async fn pipeline(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PIPELINE),
                ..Options::default()
            },
            async |item| {
                let reply = item
                    .request(request(&[("model", json!("reroute-once"))]))
                    .await?;
                ensure!(
                    reply.status == 201 && reply.body["model"] == "explicit-alias",
                    "status and explicit response model"
                );
                ensure!(
                    reply.body["order"]
                        == json!([
                            "request",
                            "provider-before",
                            "model-before",
                            "provider-after",
                            "model-after",
                            "provider-response",
                            "model-response"
                        ]),
                    "provider then model in each stage"
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
                item.request(request(&[("fail", json!(true))]))
                    .await?
                    .error(500, "hook_failed", "failed hook aborts")?;
                ensure!(
                    item.upstream.controls.lock().await.records.len() == 1,
                    "failed hook never sends"
                );
                let reply = item.request(request(&[])).await?;
                ensure!(
                    reply.status == 201 && reply.body["counters"] == json!([2, 1]),
                    "each terminal outcome observed once"
                );
                ensure!(
                    reply.body["observed"] == json!(["request", "provider-before"]),
                    "failed draft not committed to observer"
                );
                ensure!(
                    reply.body["order"]
                        .as_array()
                        .is_some_and(|order| order.len() == 7),
                    "state independently initialized"
                );
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
