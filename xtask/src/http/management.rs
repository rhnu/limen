//! Management and persistence checks exercise the real HTTP, filesystem and process boundaries.

use anyhow::{Context, Result, ensure};
use http::Method;
use serde_json::{Value, json};

use super::fixture::{Fixture, Laboratory, Options, PENDING_CONFIG, request};

pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(Options::default(), async |item| {
            public_web(item).await?;
            authentication(item).await?;
            projection_and_parameters(item).await?;
            let id = precision(item).await?;
            log_window(item, &id).await
        })
        .await
}

async fn public_web(item: &Fixture) -> Result<()> {
    let index = item.public(Method::GET, "/").await?;
    ensure!(
        index.status == 200
            && index
                .headers
                .get("content-type")
                .is_some_and(|value| value == "text/html; charset=utf-8")
            && index
                .headers
                .get("cache-control")
                .is_some_and(|value| value == "no-cache"),
        "public management shell unavailable or cached permanently"
    );
    let head = item.public(Method::HEAD, "/").await?;
    ensure!(
        head.status == 200
            && head.body.is_empty()
            && head.headers.get("content-length") == index.headers.get("content-length")
            && head.headers.get("content-type") == index.headers.get("content-type"),
        "public shell HEAD metadata or body changed"
    );
    let html = std::str::from_utf8(&index.body).context("decode delivered management HTML")?;
    let script = html
        .split(['"', '\''])
        .find(|value| {
            value.starts_with("/assets/")
                && std::path::Path::new(value)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
        })
        .context("delivered management shell has no local script")?;
    let asset = item.public(Method::GET, script).await?;
    ensure!(
        asset.status == 200
            && !asset.body.is_empty()
            && asset
                .headers
                .get("content-type")
                .is_some_and(|value| value == "text/javascript; charset=utf-8")
            && asset
                .headers
                .get("cache-control")
                .is_some_and(|value| value == "public, max-age=31536000, immutable")
            && asset
                .headers
                .get("x-content-type-options")
                .is_some_and(|value| value == "nosniff"),
        "compiled script delivery MIME, cache or sniffing boundary changed"
    );
    let favicon = item.public(Method::GET, "/favicon.svg").await?;
    ensure!(
        favicon.status == 200
            && favicon
                .headers
                .get("content-type")
                .is_some_and(|value| value == "image/svg+xml"),
        "public favicon unavailable"
    );
    for path in ["/assets/missing.js", "/usage"] {
        ensure!(
            item.public(Method::GET, path).await?.status == 404,
            "unknown public path received a shell fallback"
        );
    }
    let post = item.public(Method::POST, "/").await?;
    ensure!(
        post.status == 405
            && post
                .headers
                .get("allow")
                .is_some_and(|value| value == "GET, HEAD"),
        "static method boundary changed"
    );
    for path in [
        "/v1/models",
        "/v1/missing",
        "/admin/v1/status",
        "/admin/v1/missing",
    ] {
        ensure!(
            item.public(Method::GET, path).await?.status == 401,
            "API namespace lost authentication priority"
        );
    }
    let records = item.manager("/admin/v1/requests").await?;
    ensure!(
        records.status == 200 && records.body["data"].as_array().is_some_and(Vec::is_empty),
        "static requests entered generation admission"
    );
    ensure!(
        item.upstream.controls.lock().await.records.is_empty(),
        "static requests reached the generation upstream"
    );
    Ok(())
}

async fn authentication(item: &Fixture) -> Result<()> {
    item.send(Value::Null, Method::GET, "/admin/v1/status", &[])
        .await?
        .error(
            401,
            "unauthorized",
            "API key must not grant management access",
        )?;
    item.send(
        Value::Null,
        Method::GET,
        "/v1/models",
        &[(
            "Authorization",
            Some("Bearer synthetic-manager-secret-0123456789"),
        )],
    )
    .await?
    .error(
        401,
        "unauthorized",
        "manager key must not grant inference access",
    )?;
    item.send(
        Value::Null,
        Method::GET,
        "/unknown",
        &[("Authorization", None)],
    )
    .await?
    .error(
        404,
        "not_found",
        "unknown top-level route must not authenticate",
    )?;
    item.manager("/admin/v1/missing")
        .await?
        .error(404, "not_found", "unknown manager route")?;
    let method = item
        .send(
            Value::Null,
            Method::POST,
            "/admin/v1/status",
            &[(
                "Authorization",
                Some("Bearer synthetic-manager-secret-0123456789"),
            )],
        )
        .await?;
    method.error(405, "method_not_allowed", "manager method contract")?;
    ensure!(
        method
            .headers
            .get("allow")
            .is_some_and(|value| value == "GET"),
        "manager Allow header missing"
    );
    let status = item.manager("/admin/v1/status").await?;
    ensure!(
        status.status == 200 && status.body["data"]["acceptingGenerations"] == true,
        "ready management status"
    );
    ensure!(
        status
            .headers
            .get("cache-control")
            .is_some_and(|value| value == "no-store"),
        "manager response must not be cached"
    );
    ensure!(
        !status.headers.contains_key("access-control-allow-origin"),
        "unexpected cross-origin permission"
    );
    Ok(())
}

async fn projection_and_parameters(item: &Fixture) -> Result<()> {
    for section in ["server", "providers", "models"] {
        let reply = item
            .manager(&format!("/admin/v1/config?section={section}"))
            .await?;
        ensure!(reply.status == 200, "configuration projection unavailable");
        let text = reply.body.to_string();
        for secret in [
            "synthetic-client-secret",
            "synthetic-manager-secret",
            "synthetic-upstream-secret",
            "synthetic-query-secret",
            "gateway.js",
            "fixture-data",
        ] {
            ensure!(
                !text.contains(secret),
                "configuration projection leaked private data"
            );
        }
    }
    for path in [
        "/admin/v1/requests?limit=0",
        "/admin/v1/requests?limit=101",
        "/admin/v1/requests?state=bogus",
        "/admin/v1/requests?limit=1&limit=2",
        "/admin/v1/requests?from=2026-01-01T00:00:00Z",
        "/admin/v1/requests?sql=select",
        "/admin/v1/usage?groupBy=anything",
        "/admin/v1/config?section=raw",
        "/admin/v1/logs?path=/etc/passwd",
        "/admin/v1/logs?limit=201",
    ] {
        item.manager(path).await?.error(
            400,
            "invalid_query",
            "invalid management parameters accepted",
        )?;
    }
    Ok(())
}

async fn precision(item: &Fixture) -> Result<String> {
    item.upstream.controls.lock().await.raw = Some(serde_json::to_vec(&json!({
        "model":"real-model", "choices":[],
        "usage":{"prompt_tokens":9_007_199_254_740_991_u64,"completion_tokens":0}
    }))?);
    let reply = item.request(request(&[])).await?;
    ensure!(reply.status == 200, "generation for precision check failed");
    let record = item.record(reply.request_id()?).await?;
    ensure!(
        record["inputTokens"] == "9007199254740991"
            && record["outputTokens"] == "0"
            && record["totalTokens"].is_null(),
        "persisted integers lost precision, zero or missing semantics"
    );
    let usage = item.manager("/admin/v1/usage?groupBy=none").await?;
    ensure!(usage.status == 200, "usage aggregation failed");
    let tokens = &usage.body["data"]["groups"][0]["tokens"];
    ensure!(
        tokens["input"]["sum"] == "9007199254740991"
            && tokens["input"]["samples"] == 1
            && tokens["output"]["sum"] == "0"
            && tokens["total"]["sum"].is_null()
            && tokens["total"]["samples"] == 0,
        "aggregate precision or missing sample semantics changed"
    );
    Ok(reply.request_id()?.to_owned())
}

async fn log_window(item: &Fixture, id: &str) -> Result<()> {
    // A completed lifecycle event is a synchronization signal for the lossy writer.
    item.statistics(id, &[]).await?;
    let files = item.manager("/admin/v1/logs/files").await?;
    ensure!(
        files.status == 200
            && files.body["data"]
                .as_array()
                .is_some_and(|files| !files.is_empty()),
        "real application logs unavailable"
    );
    let logs = item.manager("/admin/v1/logs?limit=1").await?;
    ensure!(
        logs.status == 200
            && logs.body["data"]
                .as_array()
                .is_some_and(|rows| rows.len() == 1),
        "bounded log window unavailable"
    );
    ensure!(
        logs.body["data"][0]["schemaVersion"] == 1,
        "log schema missing"
    );
    Ok(())
}

pub async fn persistence(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PENDING_CONFIG),
                ..Options::default()
            },
            async |item| {
                let first = item.request(request(&[])).await?;
                ensure!(first.status == 200, "initial persisted request failed");
                let first_id = first.request_id()?.to_owned();
                let first_record = item.record(&first_id).await?;
                item.check_instance_lock().await?;
                let held = item.open_request(request(&[("wait", json!(true))])).await?;
                item.wait_pending(1).await?;
                let pending = item.manager("/admin/v1/requests?state=pending").await?;
                ensure!(pending.status == 200, "pending records unavailable");
                let pending_rows = pending.body["data"]
                    .as_array()
                    .context("pending list shape")?;
                ensure!(
                    pending_rows.len() == 1,
                    "accepted pending request not tracked exactly once"
                );
                let pending_id = pending_rows[0]["requestId"]
                    .as_str()
                    .context("pending ID missing")?
                    .to_owned();
                let page = item.manager("/admin/v1/requests?limit=1").await?;
                let cursor = page.body["nextCursor"]
                    .as_str()
                    .context("request pagination cursor missing")?
                    .to_owned();
                let status = item.manager("/admin/v1/status").await?;
                let previous_run = status.body["data"]["runId"].clone();
                item.restart().await?;
                drop(held);
                let status = item.manager("/admin/v1/status").await?;
                ensure!(
                    status.status == 200 && status.body["data"]["runId"] != previous_run,
                    "restart reused process identity"
                );
                let persisted = item
                    .manager(&format!("/admin/v1/requests/{first_id}"))
                    .await?;
                ensure!(
                    persisted.status == 200 && persisted.body["data"] == first_record,
                    "committed request changed across restart"
                );
                let interrupted = item
                    .manager(&format!("/admin/v1/requests/{pending_id}"))
                    .await?;
                ensure!(
                    interrupted.status == 200
                        && interrupted.body["data"]["state"] == "interrupted"
                        && interrupted.body["data"]["outcome"].is_null()
                        && interrupted.body["data"]["finishedAt"].is_null()
                        && interrupted.body["data"]["inputTokens"].is_null(),
                    "crash recovery fabricated an outcome or usage"
                );
                item.manager(&format!("/admin/v1/requests?cursor={cursor}"))
                    .await?
                    .error(
                        410,
                        "cursor_expired",
                        "restart must invalidate query cursors",
                    )?;
                let files = item.manager("/admin/v1/logs/files").await?;
                ensure!(
                    files.status == 200
                        && files.body["data"]
                            .as_array()
                            .is_some_and(|files| files.len() >= 2),
                    "startup did not retain old log generation"
                );
                ensure!(
                    item.request(request(&[])).await?.status == 200,
                    "recovered service did not admit generation"
                );
                Ok(())
            },
        )
        .await
}
