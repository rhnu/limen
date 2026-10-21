//! Proactive comments must cross real HTTP while the upstream body remains gated.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::json;
use tokio::{
    sync::Notify,
    time::{Instant, timeout},
};

use super::fixture::{Fixture, Laboratory, Options, request};

const HEARTBEAT: &str = ": keepalive\n\n";

pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                limits: "  sse_keepalive_ms: 50\n",
                ..Options::default()
            },
            async |item| {
                for sse in [true, false] {
                    delayed_success(item, sse).await?;
                }
                json_silence(item).await?;
                failure_silence(item).await?;
                timeout_after_commit(item).await?;
                item.upstream.controls.lock().await.delay = true;
                let reply = item.request(request(&[("stream", json!(true))])).await?;
                reply.error(
                    504,
                    "deadline_exceeded",
                    "heartbeat committed before upstream headers",
                )
            },
        )
        .await?;
    laboratory
        .with_fixture(
            Options {
                limits: "  sse_keepalive_ms: 0\n",
                ..Options::default()
            },
            async |item| {
                let gate = configure(item, true).await?;
                let pending = item.streaming_request(request(&[("stream", json!(true))]));
                tokio::pin!(pending);
                ensure!(
                    timeout(Duration::from_millis(200), &mut pending)
                        .await
                        .is_err(),
                    "disabled heartbeat committed a quiet stream"
                );
                gate.notify_one();
                let mut response = pending.await?;
                let body = read_body(&mut response, Vec::new()).await?;
                ensure!(
                    !body.contains(HEARTBEAT) && body.contains("data: [DONE]\n\n"),
                    "disabled heartbeat changed successful stream output"
                );
                Ok(())
            },
        )
        .await
}

async fn configure(item: &Fixture, sse: bool) -> Result<Arc<Notify>> {
    let gate = Arc::new(Notify::new());
    let mut controls = item.upstream.controls.lock().await;
    controls.status = 200;
    controls.media = if sse {
        "text/event-stream"
    } else {
        "application/json"
    };
    let choice = if sse {
        json!({"index":0,"delta":{"role":"assistant","content":"quiet answer"},"finish_reason":"stop"})
    } else {
        json!({"index":0,"message":{"role":"assistant","content":"quiet answer"},"finish_reason":"stop"})
    };
    let body = json!({"id":"quiet","object":if sse {"chat.completion.chunk"} else {"chat.completion"},
        "created":1,"model":"real-model","choices":[choice]});
    controls.raw = Some(if sse {
        format!("\u{feff}: native-heartbeat\n\ndata: {body}\n\ndata: [DONE]\n\n").into_bytes()
    } else {
        serde_json::to_vec(&body)?
    });
    controls.body_gate = Some(Arc::clone(&gate));
    controls.started = false;
    controls.disconnected = false;
    Ok(gate)
}

async fn delayed_success(item: &Fixture, sse: bool) -> Result<()> {
    let gate = configure(item, sse).await?;
    let mut response = item
        .streaming_request(request(&[("stream", json!(true))]))
        .await?;
    ensure!(
        response.status().as_u16() == 200,
        "quiet upstream did not commit SSE success"
    );
    let id = response
        .headers()
        .get("x-limen-request-id")
        .context("heartbeat response ID missing")?
        .to_str()?
        .to_owned();
    let prefix = read_heartbeats(&mut response).await?;
    ensure!(
        prefix
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == HEARTBEAT.as_bytes()[index % HEARTBEAT.len()]),
        "quiet upstream emitted content before its body gate opened"
    );
    gate.notify_one();
    let body = read_body(&mut response, prefix).await?;
    ensure!(
        !body.contains('\u{feff}'),
        "upstream BOM appeared after proactive comments"
    );
    ensure!(
        body.contains("quiet answer")
            && body.ends_with("data: [DONE]\n\n")
            && !body.contains("event: error")
            && body.matches("[DONE]").count() == 1,
        "heartbeat interrupted successful delayed upstream body"
    );
    let record = item.record(&id).await?;
    ensure!(
        record["upstreamCompleted"] == true && record["outcome"] == "success",
        "heartbeat changed successful upstream completion facts"
    );
    Ok(())
}

async fn json_silence(item: &Fixture) -> Result<()> {
    let gate = configure(item, true).await?;
    let pending = item.streaming_request(request(&[("stream", json!(false))]));
    tokio::pin!(pending);
    ensure!(
        timeout(Duration::from_millis(200), &mut pending)
            .await
            .is_err(),
        "JSON downstream committed heartbeat output"
    );
    gate.notify_one();
    let response = pending.await?;
    ensure!(
        response.status().as_u16() == 200,
        "quiet JSON downstream failed"
    );
    let body: serde_json::Value = serde_json::from_slice(&response.bytes().await?)?;
    ensure!(
        body["choices"][0]["message"]["content"] == "quiet answer",
        "quiet JSON downstream lost generated content"
    );
    Ok(())
}

async fn failure_silence(item: &Fixture) -> Result<()> {
    let gate = configure(item, false).await?;
    item.upstream.controls.lock().await.status = 503;
    let pending = item.streaming_request(request(&[("stream", json!(true))]));
    tokio::pin!(pending);
    ensure!(
        timeout(Duration::from_millis(200), &mut pending)
            .await
            .is_err(),
        "upstream HTTP failure committed heartbeat output"
    );
    gate.notify_one();
    let response = pending.await?;
    ensure!(
        response.status().as_u16() == 503,
        "upstream HTTP failure status changed"
    );
    let body = response.text().await?;
    ensure!(
        !body.contains(HEARTBEAT),
        "heartbeat contaminated upstream HTTP failure"
    );
    Ok(())
}

async fn timeout_after_commit(item: &Fixture) -> Result<()> {
    let _gate = configure(item, true).await?;
    let started = Instant::now();
    let mut response = item
        .streaming_request(request(&[("stream", json!(true))]))
        .await?;
    ensure!(
        response.status().as_u16() == 200,
        "heartbeat did not commit before deadline"
    );
    let prefix = read_heartbeats(&mut response).await?;
    let body = read_body(&mut response, prefix).await?;
    ensure!(
        body.contains("event: error")
            && body.contains("deadline_exceeded")
            && !body.contains("[DONE]"),
        "deadline after heartbeat was not a terminal SSE error"
    );
    ensure!(
        started.elapsed() < Duration::from_millis(2200),
        "heartbeats extended the absolute request deadline"
    );
    item.upstream.wait(Duration::from_secs(1), true).await
}

async fn read_heartbeats(response: &mut reqwest::Response) -> Result<Vec<u8>> {
    timeout(Duration::from_millis(800), async {
        let mut received = Vec::new();
        while received.len() < 3 * HEARTBEAT.len() {
            let chunk = response
                .chunk()
                .await?
                .context("stream ended before repeated heartbeats")?;
            ensure!(
                received.len() + chunk.len() <= 4096,
                "heartbeat prefix exceeded its bound"
            );
            received.extend_from_slice(&chunk);
        }
        Ok(received)
    })
    .await
    .context("quiet upstream heartbeat deadline")?
}

async fn read_body(response: &mut reqwest::Response, mut received: Vec<u8>) -> Result<String> {
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            received.len() + chunk.len() <= 64 * 1024,
            "heartbeat response exceeded its bound"
        );
        received.extend_from_slice(&chunk);
    }
    String::from_utf8(received).context("decode heartbeat response")
}
