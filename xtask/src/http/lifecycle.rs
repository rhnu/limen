//! Timeouts, disconnects, shutdown, retirement, and request admission remain bounded.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;
use tokio::time::Instant;

use super::fixture::{Laboratory, Options, PENDING_CONFIG, request};

/// Distinguish healthy hook-wait timeout from upstream cancellation at the total deadline.
pub async fn deadline(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PENDING_CONFIG),
                ..Options::default()
            },
            async |item| {
                let started = Instant::now();
                let reply = item.request(request(&[("wait", json!(true))])).await?;
                reply.error(504, "deadline_exceeded", "hook waiting deadline")?;
                ensure!(
                    started.elapsed() < Duration::from_secs(2),
                    "deadline bounded"
                );
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "timed out request does not send"
                );
                let log = item
                    .statistics(
                        reply.request_id()?,
                        &[("onRequest", false), ("onFinish", true)],
                    )
                    .await?;
                log.outcome("failed", Some("deadline_exceeded"))?;
                log.interrupted_hook("onRequest", false, false)?;
                log.no_upstream_samples()?;
                ensure!(
                    item.request(request(&[])).await?.status == 200,
                    "ordinary timeout retains healthy VM"
                );
                Ok(())
            },
        )
        .await?;
    laboratory
        .with_fixture(Options::default(), async |item| {
            item.upstream.controls.lock().await.delay = true;
            let reply = item.request(request(&[])).await?;
            reply.error(504, "deadline_exceeded", "upstream request deadline")?;
            item.upstream.wait(Duration::from_secs(1), true).await?;
            let log = item.statistics(reply.request_id()?, &[]).await?;
            log.outcome("failed", Some("deadline_exceeded"))?;
            log.no_upstream_samples()
        })
        .await
}

/// Close complete downstream requests and require cancellation before their normal deadlines.
pub async fn disconnect(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PENDING_CONFIG),
                ..Options::default()
            },
            async |item| {
                let stream = item.open_request(request(&[("wait", json!(true))])).await?;
                item.wait_pending(1).await?;
                drop(stream);
                item.wait_cancelled(1).await?;
                let finished = item
                    .wait_event("requestFinished", |event| event["outcome"] == "cancelled")
                    .await?;
                let log = item
                    .statistics(
                        finished["requestId"]
                            .as_str()
                            .context("cancelled request ID missing")?,
                        &[("onRequest", false), ("onFinish", true)],
                    )
                    .await?;
                log.outcome("cancelled", Some("cancelled"))?;
                log.interrupted_hook("onRequest", false, true)?;
                log.no_upstream_samples()
            },
        )
        .await?;
    laboratory
        .with_fixture(Options::default(), async |item| {
            item.upstream.controls.lock().await.delay = true;
            let stream = item.open_request(request(&[])).await?;
            item.upstream.wait(Duration::from_secs(1), false).await?;
            drop(stream);
            item.upstream.wait(Duration::from_millis(800), true).await?;
            let finished = item
                .wait_event("requestFinished", |event| event["outcome"] == "cancelled")
                .await?;
            let log = item
                .statistics(
                    finished["requestId"]
                        .as_str()
                        .context("cancelled request ID missing")?,
                    &[],
                )
                .await?;
            log.outcome("cancelled", Some("cancelled"))?;
            log.no_upstream_samples()
        })
        .await
}

/// Exercise the platform's real shutdown signal with one pending hook and a failing observer.
pub async fn shutdown(laboratory: &mut Laboratory<'_>) -> Result<()> {
    let config = include_str!("../../fixtures/http/shutdown.js");
    laboratory
        .with_fixture(
            Options {
                config: Some(config),
                ..Options::default()
            },
            async |item| {
                let stream = item.open_request(request(&[("wait", json!(true))])).await?;
                item.wait_pending(1).await?;
                item.process.shutdown_signal()?;
                ensure!(
                    item.process
                        .wait_timeout(Duration::from_secs(2))
                        .await?
                        .success(),
                    "shutdown signal drains and exits successfully"
                );
                drop(stream);
                ensure!(
                    item.logs()?.contains("listenerStopping"),
                    "listener shutdown observed"
                );
                let events = item.events()?;
                ensure!(
                    events
                        .iter()
                        .filter(|event| event["event"] == "requestFinished"
                            && event["outcome"] == "cancelled")
                        .count()
                        == 1,
                    "shutdown records one cancelled terminal outcome"
                );
                ensure!(
                    events
                        .iter()
                        .filter(|event| event["event"] == "observationFailed"
                            && event["phase"] == "onFinish")
                        .count()
                        == 1,
                    "shutdown attempts onFinish exactly once"
                );
                let finished = events
                    .iter()
                    .find(|event| {
                        event["event"] == "requestFinished" && event["outcome"] == "cancelled"
                    })
                    .context("shutdown terminal event missing")?;
                let log = item
                    .statistics(
                        finished["requestId"]
                            .as_str()
                            .context("shutdown request ID missing")?,
                        &[("onRequest", false), ("onFinish", true)],
                    )
                    .await?;
                log.outcome("cancelled", Some("cancelled"))?;
                log.interrupted_hook("onRequest", false, true)?;
                log.no_upstream_samples()?;
                ensure!(
                    log.hook("onFinish", true)?["failedCalls"] == 1,
                    "failed shutdown observer lost its own cleanup summary"
                );
                Ok(())
            },
        )
        .await
}

/// A synchronous runaway hook retires the VM and fails the gateway without upstream sends.
pub async fn retirement(laboratory: &mut Laboratory<'_>) -> Result<()> {
    let config = include_str!("../../fixtures/http/retirement.js");
    laboratory
        .with_fixture(
            Options {
                config: Some(config),
                limits: "  execution_ms: 20\n",
                ..Options::default()
            },
            async |item| {
                match item.request(request(&[])).await {
                    Ok(reply) => ensure!(reply.status >= 400, "retired VM returned HTTP success"),
                    // Retirement may close HTTP before an error response is delivered.
                    Err(error)
                        if error
                            .downcast_ref::<reqwest::Error>()
                            .is_some_and(|source| source.is_request() || source.is_body()) => {},
                    Err(error) => return Err(error),
                }
                ensure!(
                    !item
                        .process
                        .wait_timeout(Duration::from_secs(3))
                        .await?
                        .success(),
                    "VM retirement terminates HTTP process with failure"
                );
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "retirement never sends generation"
                );
                ensure!(
                    item.logs()?.contains("listenerStopping"),
                    "retirement stops listener"
                );
                let finished = item.wait_event("requestFinished", |_| true).await?;
                ensure!(
                    finished["outcome"] != "success",
                    "retirement recorded a successful request"
                );
                let log = item
                    .statistics(
                        finished["requestId"]
                            .as_str()
                            .context("retirement request ID missing")?,
                        &[("onRequest", false)],
                    )
                    .await?;
                log.no_upstream_samples()?;
                super::statistics::duration(log.hook("onRequest", false)?, "wallMs")?;
                Ok(())
            },
        )
        .await
}

/// Reject oversized and over-capacity requests, then reclaim permits after socket closure.
pub async fn resources(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(PENDING_CONFIG),
                limits: "  max_body_bytes: 300\n",
                ..Options::default()
            },
            async |item| {
                item.request(request(&[("large", json!("x".repeat(400)))]))
                    .await?
                    .error(
                        413,
                        "payload_too_large",
                        "body limit rejects before upstream",
                    )?;
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "oversized request never sent"
                );
                Ok(())
            },
        )
        .await?;
    laboratory
        .with_fixture(
            Options {
                config: Some(PENDING_CONFIG),
                ..Options::default()
            },
            async |item| {
                let held = [
                    item.open_request(request(&[("wait", json!(true))])).await?,
                    item.open_request(request(&[("wait", json!(true))])).await?,
                ];
                item.request(request(&[])).await?.error(
                    503,
                    "busy",
                    "bounded request admission",
                )?;
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "busy request never sent"
                );
                item.wait_pending_records(2).await?;
                drop(held);
                item.wait_cancelled(2).await
            },
        )
        .await
}
