//! Timeouts, disconnects, shutdown, retirement, and request admission remain bounded.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::json;
use tokio::time::{Instant, sleep};

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
                item.request(request(&[("wait", json!(true))]))
                    .await?
                    .error(504, "deadline_exceeded", "hook waiting deadline")?;
                ensure!(
                    started.elapsed() < Duration::from_secs(2),
                    "deadline bounded"
                );
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "timed out request does not send"
                );
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
            item.request(request(&[])).await?.error(
                504,
                "deadline_exceeded",
                "upstream request deadline",
            )?;
            item.upstream.wait(Duration::from_secs(1), true).await
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
                sleep(Duration::from_millis(80)).await;
                drop(stream);
                item.wait_cancelled(1).await
            },
        )
        .await?;
    laboratory
        .with_fixture(Options::default(), async |item| {
            item.upstream.controls.lock().await.delay = true;
            let stream = item.open_request(request(&[])).await?;
            item.upstream.wait(Duration::from_secs(1), false).await?;
            drop(stream);
            item.upstream.wait(Duration::from_millis(800), true).await
        })
        .await
}

/// Exercise the platform's real shutdown signal with one pending hook and a failing observer.
pub async fn shutdown(laboratory: &mut Laboratory<'_>) -> Result<()> {
    let config = PENDING_CONFIG.replace(
        "finishes++;if(ctx.outcome==='cancelled')cancelled++;",
        "finishes++;if(ctx.outcome==='cancelled')cancelled++;throw new Error('observer failure');",
    );
    laboratory
        .with_fixture(
            Options {
                config: Some(&config),
                ..Options::default()
            },
            async |item| {
                let stream = item.open_request(request(&[("wait", json!(true))])).await?;
                sleep(Duration::from_millis(100)).await;
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
                    item.logs()?.contains("HTTP listener stopped"),
                    "listener shutdown observed"
                );
                let events = item.events()?;
                ensure!(
                    events
                        .iter()
                        .filter(|event| event["message"] == "request finished"
                            && event["outcome"] == "cancelled")
                        .count()
                        == 1,
                    "shutdown records one cancelled terminal outcome"
                );
                ensure!(
                    events
                        .iter()
                        .filter(|event| event["message"]
                            == "observation failed; terminal outcome retained"
                            && event["phase"] == "onFinish")
                        .count()
                        == 1,
                    "shutdown attempts onFinish exactly once"
                );
                Ok(())
            },
        )
        .await
}

/// A synchronous runaway hook retires the VM and fails the gateway without upstream sends.
pub async fn retirement(laboratory: &mut Laboratory<'_>) -> Result<()> {
    let config = "export default {onRequest(){while(true){}},providers:{p:{endpoint:ENDPOINT}},models:{chat:{provider:'p',upstream:'real-model'}}};";
    laboratory
        .with_fixture(
            Options {
                config: Some(config),
                limits: "  execution_ms: 20\n",
                ..Options::default()
            },
            async |item| {
                match item.request(request(&[])).await {
                    Ok(_) => {},
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
                    item.logs()?.contains("HTTP listener stopped"),
                    "retirement stops listener"
                );
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
                let held = [
                    item.open_request(request(&[("wait", json!(true))])).await?,
                    item.open_request(request(&[("wait", json!(true))])).await?,
                ];
                sleep(Duration::from_millis(100)).await;
                item.request(request(&[])).await?.error(
                    429,
                    "busy",
                    "bounded request admission",
                )?;
                ensure!(
                    item.upstream.controls.lock().await.records.is_empty(),
                    "busy request never sent"
                );
                drop(held);
                item.wait_cancelled(2).await
            },
        )
        .await
}
