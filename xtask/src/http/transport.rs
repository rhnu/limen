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
                let statistics = log.event("request statistics")?;
                ensure!(statistics["upstream_completed"] == true && statistics["usage_source"] == "sse", "completed SSE lost its source facts");
                for field in ["upstream_headers_wait_ms", "upstream_first_byte_ms", "upstream_first_chunk_ms", "upstream_read_wait_ms"] {
                    super::statistics::duration(statistics, field)?;
                }
                if stream {
                    for field in ["downstream_first_event_ms", "downstream_queue_wait_ms", "downstream_drain_ms"] {
                        super::statistics::duration(statistics, field)?;
                    }
                } else {
                    ensure!(statistics["downstream_first_event_ms"].is_null(), "JSON aggregation fabricated a downstream SSE sample");
                }
            }
        }
        Ok(())
    }).await
}
