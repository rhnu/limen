//! Check the real HTTP read boundary after an explicit SSE terminator.

use anyhow::{Result, ensure};
use http::Method;
use serde_json::json;

use super::fixture::{Laboratory, Options, request};

/// An error after DONE is ignored whether it shares a read or arrives later.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory.with_fixture(Options::default(), async |item| {
        let event = json!({"id":"synthetic", "object":"chat.completion.chunk", "created":1, "model":"real-model",
            "choices":[{"index":0,"delta":{"role":"assistant", "content":"hello"},"finish_reason":"stop"}]});
        let mut completed = b"data: ".to_vec();
        completed.extend_from_slice(&serde_json::to_vec(&event)?);
        completed.extend_from_slice(b"\n\ndata: [DONE]\n\n");
        let done_end = completed.len();
        completed.extend_from_slice(b"event: error\ndata: {}\n\n");
        for split_at in [None, Some(done_end)] {
            {
                let mut upstream = item.upstream.controls.lock().await;
                upstream.media = "text/event-stream";
                upstream.raw = Some(completed.clone());
                upstream.split_at = split_at;
            }
            for stream in [false, true] {
                // Retain the connection until the gateway has observed body drain;
                // receiving DONE alone does not establish the final lifecycle outcome.
                let reply = item.send(request(&[("stream", json!(stream))]), Method::POST,
                    "/v1/chat/completions", &[("Connection", None)]).await?;
                ensure!(reply.status == 200, "first DONE terminates independently of network splitting");
                if stream {
                    let text = reply.text()?;
                    ensure!(text.ends_with("data: [DONE]\n\n") && !text.contains("event: error"), "trailing error reached downstream");
                    ensure!(text.matches("[DONE]").count() == 1, "terminator must be emitted once");
                } else {
                    ensure!(reply.body["choices"][0]["message"]["content"] == "hello", "trailing error invalidated JSON aggregation");
                }
                let log = item.statistics(reply.request_id()?, &[]).await?;
                log.outcome("success", None)?;
                let statistics = log.event("requestTiming")?;
                let record = item.record(reply.request_id()?).await?;
                ensure!(record["upstreamCompleted"] == true && record["usageSource"] == "sse", "completed SSE lost its source facts");
                for field in ["upstreamHeadersWaitMs", "upstreamFirstByteMs", "upstreamFirstChunkMs", "upstreamReadWaitMs"] {
                    super::statistics::duration(statistics, field)?;
                }
                if stream {
                    for field in ["downstreamFirstEventMs", "downstreamQueueWaitMs", "downstreamDrainMs"] {
                        super::statistics::duration(statistics, field)?;
                    }
                } else {
                    ensure!(statistics["downstreamFirstEventMs"].is_null(), "JSON aggregation fabricated a downstream SSE sample");
                }
            }
        }
        Ok(())
    }).await?;
    protocol_frames(laboratory).await
}

/// The same protocol survives real read boundaries, metadata tails and failures.
async fn protocol_frames(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory.with_fixture(Options::default(), async |item| {
        let first = json!({"id":"protocol","object":"chat.completion.chunk","created":1,"model":"real-model",
            "choices":[{"index":0,"delta":{"role":"assistant","content":[{"type":"text","text":"hello"}],
                "audio":{"id":"audio","data":"YQ=="}},"finish_reason":null}]});
        let final_content = json!({"id":"protocol","object":"chat.completion.chunk","created":1,"model":"real-model",
            "choices":[{"index":0,"delta":{"audio":{"data":"Yg=="}},"finish_reason":"stop"}]});
        let expiry = json!({"object":"","id":"","model":"","created":0,
            "choices":[{"index":0,"delta":{"audio":{"expires_at":42}}}]});
        let annotation = json!({"choices":[{"index":0,"content_filter_offsets":{"start_offset":0,"end_offset":5}}]});
        let content = format!("\u{feff}: heartbeat\r\n\r\nevent: update\r\nid: frame-1\r\nretry: 1500\r\ndata: {first}\r\n\r\ndata: {final_content}\r\n\r\ndata: {expiry}\r\n\r\ndata: {annotation}\r\n\r\n");
        let complete = format!("{content}: final-note\nid: end\ndata: [DONE]\n\n");
        for split in [1, content.len() / 2] {
            {
                let mut upstream = item.upstream.controls.lock().await;
                upstream.media = "text/event-stream";
                upstream.raw = Some(complete.as_bytes().to_vec());
                upstream.split_at = Some(split);
            }
            for stream in [false, true] {
                let reply = item.send(request(&[("stream", json!(stream))]), Method::POST,
                    "/v1/chat/completions", &[("Connection", None)]).await?;
                ensure!(reply.status == 200, "valid protocol frames failed at an HTTP read boundary");
                if stream {
                    let text = reply.text()?;
                    for field in [": heartbeat\r\n", "event: update\r\n", "id: frame-1\r\n", "retry: 1500\r\n", ": final-note\nid: end\ndata: [DONE]\n\n"] {
                        ensure!(text.contains(field), "SSE outer field was lost");
                    }
                    ensure!(text.matches("[DONE]").count() == 1, "native terminator duplicated");
                } else {
                    ensure!(reply.body["choices"][0]["message"]["audio"]["data"] == "YWI=", "padded audio fragments were not combined as bytes");
                    ensure!(reply.body["choices"][0]["message"]["audio"]["expires_at"] == 42, "audio metadata tail was lost");
                    ensure!(reply.body["choices"][0]["content_filter_offsets"]["end_offset"] == 5, "annotation scope was lost");
                }
                item.statistics(reply.request_id()?, &[]).await?.outcome("success", None)?;
            }
        }
        for bytes in [content, ": heartbeat\n\ndata: {\"error\":{\"code\":\"capacity\",\"message\":\"synthetic-body-secret\"}}\n\n".into()] {
            {
                let mut upstream = item.upstream.controls.lock().await;
                upstream.raw = Some(bytes.into_bytes());
                upstream.split_at = None;
            }
            let reply = item.send(request(&[("stream", json!(true))]), Method::POST,
                "/v1/chat/completions", &[("Connection", None)]).await?;
            ensure!(reply.status == 200 && !reply.text()?.contains("[DONE]") && reply.text()?.contains("event: error"), "partial or failed stream was reported successful");
            let log = item.statistics(reply.request_id()?, &[]).await?;
            log.outcome("failed", Some("upstream_protocol"))?;
            ensure!(item.record(reply.request_id()?).await?["upstreamCompleted"] == false, "partial stream fabricated native completion");
        }
        Ok(())
    }).await
}
