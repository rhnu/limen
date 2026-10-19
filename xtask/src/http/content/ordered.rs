//! Ordered synthetic text must arrive while upstream completion is still gated.

use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use http::Method;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{CONFIG, contains, read_until, request_id};
use crate::http::fixture::{Fixture, Laboratory, Options, request};

fn chunk(delta: Value, finish: Option<&str>) -> String {
    let mut event = json!({"id":"ordered-http","object":"chat.completion.chunk","created":1,
        "model":"real-model","choices":[{"index":0,"finish_reason":finish}]});
    event["choices"][0]["delta"] = delta;
    format!("data: {event}\n\n")
}

pub(super) async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(CONFIG),
                ..Options::default()
            },
            async |item| {
                let first = chunk(
                    json!({"role":"assistant","tool_calls":[{"index":0,"id":"ordered-text",
                        "type":"function","function":{"name":"limen_","arguments":"{\"text\":\"order-A"}}]}),
                    None,
                );
                let second = chunk(json!({"content":"order-B"}), None);
                let third = chunk(
                    json!({"tool_calls":[{"index":0,"function":{"name":"emit_text","arguments":"order-C"}}]}),
                    None,
                );
                let unresolved = format!("{first}{second}");
                let prefix = format!("{unresolved}{third}");
                let closing = chunk(
                    json!({"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]}),
                    None,
                );
                let finish = chunk(json!({}), Some("tool_calls"));
                let complete = format!("{prefix}{closing}{finish}data: [DONE]\n\n");
                let gate = Arc::new(Notify::new());
                {
                    let mut upstream = item.upstream.controls.lock().await;
                    upstream.media = "text/event-stream";
                    upstream.raw = Some(complete.into_bytes());
                    upstream.split_at = Some(prefix.len());
                    upstream.split_gate = Some(Arc::clone(&gate));
                }
                let mut stream = item
                    .open_request(request(&[("stream", json!(true)), ("model", json!("ordered"))]))
                    .await?;
                let mut received = Vec::new();
                read_until(&mut stream, &mut received, b"order-C").await?;
                ensure!(
                    received.starts_with(b"HTTP/1.1 200 "),
                    "ordered content stream did not start successfully"
                );
                let text = std::str::from_utf8(&received).context("decode ordered content stream")?;
                let first_offset = text.find("order-A").context("first ordered text missing")?;
                let ordinary_offset = text.find("order-B").context("ordinary ordered text missing")?;
                let last_offset = text.find("order-C").context("last ordered text missing")?;
                ensure!(
                    first_offset < ordinary_offset && ordinary_offset < last_offset,
                    "ordinary text overtook earlier synthetic text"
                );
                ensure!(
                    !contains(&received, b"[DONE]")
                        && !contains(&received, b"\"finish_reason\":\"stop\"")
                        && !contains(&received, b"\"finish_reason\":\"tool_calls\""),
                    "ordered text waited for or fabricated upstream completion"
                );
                let id = request_id(&received)?;
                gate.notify_one();
                read_until(&mut stream, &mut received, b"data: [DONE]\n\n").await?;
                ensure!(
                    !contains(&received, b"event: error"),
                    "completed ordered content stream emitted an error"
                );
                item.statistics(&id, &[]).await?.outcome("success", None)?;
                ensure!(
                    item.record(&id).await?["upstreamCompleted"] == true,
                    "ordered content stream lost its native completion fact"
                );
                drop(stream);
                truncated(item, unresolved.into_bytes()).await
            },
        )
        .await
}

async fn truncated(item: &mut Fixture, unresolved: Vec<u8>) -> Result<()> {
    {
        let mut upstream = item.upstream.controls.lock().await;
        upstream.raw = Some(unresolved);
        upstream.split_at = None;
        upstream.split_gate = None;
    }
    let reply = item
        .send(
            request(&[("stream", json!(true)), ("model", json!("ordered"))]),
            Method::POST,
            "/v1/chat/completions",
            &[("Connection", None)],
        )
        .await?;
    // No source data was released, so the error can still use an HTTP status.
    reply.error(
        502,
        "upstream_protocol",
        "unresolved EOF did not fail before response start",
    )?;
    let text = reply.body.to_string();
    ensure!(
        !text.contains("order-A") && !text.contains("order-B") && !text.contains("[DONE]"),
        "unresolved EOF leaked blocked text or reported successful completion"
    );
    item.statistics(reply.request_id()?, &[])
        .await?
        .outcome("failed", Some("upstream_protocol"))?;
    ensure!(
        item.record(reply.request_id()?).await?["upstreamCompleted"] == false,
        "unresolved EOF fabricated native completion"
    );
    Ok(())
}
