//! Real HTTP content hooks must produce transformed deltas before upstream completion.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use http::Method;
use serde_json::json;
use tokio::{io::AsyncReadExt, net::TcpStream, sync::Notify, time::timeout};

use super::fixture::{Laboratory, Options, request};

const CONFIG: &str = include_str!("../../fixtures/http/content.js");

/// A test-owned gate keeps upstream finish/DONE unavailable until transformed text arrives.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    for alias in ["chat", "script"] {
        laboratory
        .with_fixture(
            Options {
                config: Some(CONFIG),
                ..Options::default()
            },
            async |item| {
                let first = json!({
                    "id":"content-http", "object":"chat.completion.chunk", "created":1,
                    "model":"real-model", "choices":[{"index":0,
                        "delta":{"role":"assistant","content":"Hello world"}, "finish_reason":null}]
                });
                let finish = json!({
                    "id":"content-http", "object":"chat.completion.chunk", "created":1,
                    "model":"real-model", "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
                });
                let prefix = format!("data: {first}\n\n");
                let complete = format!("{prefix}data: {finish}\n\ndata: [DONE]\n\n");
                let gate = Arc::new(Notify::new());
                {
                    let mut upstream = item.upstream.controls.lock().await;
                    upstream.media = "text/event-stream";
                    upstream.raw = Some(complete.into_bytes());
                    upstream.split_at = Some(prefix.len());
                    upstream.split_gate = Some(Arc::clone(&gate));
                }
                let mut stream = item
                    .open_request(request(&[("stream", json!(true)), ("model", json!(alias))]))
                    .await?;
                let mut received = Vec::new();
                read_until(&mut stream, &mut received, b"Hello reader").await?;
                ensure!(
                    received.starts_with(b"HTTP/1.1 200 "),
                    "content stream did not start successfully"
                );
                ensure!(
                    !contains(&received, b"[DONE]")
                        && !contains(&received, b"\"finish_reason\":\"stop\""),
                    "upstream completion escaped the content experiment gate"
                );
                ensure!(
                    !contains(&received, b"Hello world"),
                    "raw upstream text bypassed onContent"
                );
                let request_id = request_id(&received)?;
                // notify_one retains a permit even if the body has not polled its gate yet.
                gate.notify_one();
                read_until(&mut stream, &mut received, b"data: [DONE]\n\n").await?;
                ensure!(
                    !contains(&received, b"event: error"),
                    "completed content stream emitted an error"
                );
                let phases = if alias == "script" { &[("onContent", false)][..] } else { &[] };
                item.statistics(&request_id, phases)
                    .await?
                    .outcome("success", None)?;
                ensure!(
                    item.record(&request_id).await?["upstreamCompleted"] == true,
                    "gated content stream lost its native completion fact"
                );
                drop(stream);

                // A clean HTTP EOF after the same generated prefix cannot manufacture success.
                {
                    let mut upstream = item.upstream.controls.lock().await;
                    upstream.raw = Some(prefix.into_bytes());
                    upstream.split_at = None;
                    upstream.split_gate = None;
                }
                let truncated = item
                    .send(
                        request(&[("stream", json!(true)), ("model", json!(alias))]),
                        Method::POST,
                        "/v1/chat/completions",
                        &[("Connection", None)],
                    )
                    .await?;
                let text = truncated.text()?;
                ensure!(
                    truncated.status == 200
                        && text.contains("Hello reader")
                        && text.contains("event: error")
                        && !text.contains("[DONE]"),
                    "truncated transformed content was reported successful"
                );
                item.statistics(truncated.request_id()?, phases)
                    .await?
                    .outcome("failed", Some("upstream_protocol"))?;
                ensure!(
                    item.record(truncated.request_id()?).await?["upstreamCompleted"] == false,
                    "truncated transformed content fabricated native completion"
                );
                Ok(())
            },
        )
        .await?;
    }
    Ok(())
}

fn contains(bytes: &[u8], expected: &[u8]) -> bool {
    bytes
        .windows(expected.len())
        .any(|window| window == expected)
}

/// Read only bounded real socket data; no assumption about HTTP/TCP chunk boundaries.
async fn read_until(stream: &mut TcpStream, received: &mut Vec<u8>, expected: &[u8]) -> Result<()> {
    timeout(Duration::from_secs(2), async {
        let mut chunk = [0_u8; 4096];
        while !contains(received, expected) {
            let read = stream
                .read(&mut chunk)
                .await
                .context("read content experiment response")?;
            ensure!(read != 0, "content experiment ended before expected output");
            ensure!(
                received.len() + read <= 64 * 1024,
                "content experiment response exceeded limit"
            );
            received.extend_from_slice(&chunk[..read]);
        }
        Ok(())
    })
    .await
    .context("content experiment output deadline")?
}

fn request_id(response: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(response).context("decode content experiment headers")?;
    let headers = text
        .split_once("\r\n\r\n")
        .context("content experiment headers missing")?
        .0;
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("x-limen-request-id"))
        .map(|(_, value)| value.trim().to_owned())
        .context("content experiment request ID missing")
}
