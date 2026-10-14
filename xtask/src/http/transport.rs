//! All four JSON/SSE paths retain client intent, media facts, usage, and failure semantics.

use anyhow::{Context, Result, ensure};
use serde_json::json;

use super::fixture::{Laboratory, Options, request};

const CONFIG: &str = r"export default {
 onRequest(ctx){if(ctx.body.rewrite)ctx.body.stream=false;},
 providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'control',
  beforeSend(ctx){if(ctx.body.upstreamStream)ctx.body.stream=true;},
  afterReceive(ctx){if(ctx.meta.upstreamStatus>=300){ctx.status=409;ctx.headers.set('X-Mapped','failure');}
   else{ctx.headers.set('X-Original-Media',ctx.meta.upstreamContentType);}}
 }]}},models:{chat:{provider:'p',upstream:'real-model'}}};";

/// Exercise successful conversions, an error remap, and truncation after SSE has started.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory.with_fixture(Options { config: Some(CONFIG), ..Options::default() }, async |item| {
        item.upstream.controls.lock().await.response = Some(json!({
            "id":"synthetic", "object":"chat.completion", "created":1, "model":"real-model",
            "choices":[{"index":0, "message":{"role":"assistant", "content":"hello"}, "finish_reason":"stop"}],
            "usage":{"total_tokens":2},
        }));
        let reply = item.request(request(&[("stream", json!(true)), ("rewrite", json!(true))])).await?;
        ensure!(reply.status == 200 && reply.text()?.ends_with("data: [DONE]\n\n"), "original client stream intent survives onRequest rewrite");
        {
            let upstream = item.upstream.controls.lock().await;
            ensure!(upstream.records.last().context("JSON upstream request missing")?.body["stream"] == false, "onRequest selects JSON upstream independently");
        }
        let reply = item.request(request(&[("upstreamStream", json!(true))])).await?;
        ensure!(reply.status == 200 && reply.body["model"] == "chat", "actual JSON response selects JSON parser despite upstream request intent");
        {
            let upstream = item.upstream.controls.lock().await;
            ensure!(upstream.records.last().context("stream-intent request missing")?.body["stream"] == true, "beforeSend stream is sent without second rewrite");
        }
        item.upstream.controls.lock().await.status = 500;
        let reply = item.request(request(&[])).await?;
        reply.error(409, "upstream_failed", "afterReceive maps error status without success classification")?;
        ensure!(reply.header("x-mapped")? == "failure", "error response preserves committed hook header");

        let events = [
            json!({"id":"synthetic", "object":"chat.completion.chunk", "created":1, "model":"real-model",
                "choices":[{"index":0,"delta":{"role":"assistant", "content":"hello"},"finish_reason":"stop"}]}),
            json!({"id":"synthetic", "object":"chat.completion.chunk", "created":1, "model":"real-model",
                "choices":[], "usage":{"total_tokens":2}}),
        ];
        let mut data = Vec::new();
        for event in events {
            data.extend_from_slice(b"data: ");
            data.extend_from_slice(&serde_json::to_vec(&event).context("encode synthetic SSE event")?);
            data.extend_from_slice(b"\n\n");
        }
        {
            let mut upstream = item.upstream.controls.lock().await;
            upstream.status = 200;
            upstream.media = "text/event-stream";
            let mut completed = data.clone();
            completed.extend_from_slice(b"data: [DONE]\n\n");
            upstream.raw = Some(completed);
        }
        let reply = item.request(request(&[])).await?;
        ensure!(reply.status == 200 && reply.body["choices"][0]["message"]["content"] == "hello", "SSE aggregates for original JSON client");
        ensure!(reply.body["usage"] == json!({"total_tokens":2}), "usage after finish reason is retained");
        ensure!(reply.header("x-original-media")? == "sse", "original media metadata is frozen");
        let reply = item.request(request(&[("stream", json!(true))])).await?;
        ensure!(reply.status == 200 && reply.text()?.ends_with("data: [DONE]\n\n"), "SSE passes through with one completion marker");
        ensure!(reply.text()?.matches("[DONE]").count() == 1 && reply.text()?.contains("\"model\":\"chat\""), "SSE applies alias without duplicate DONE");
        item.upstream.controls.lock().await.raw = Some(data);
        let reply = item.request(request(&[("stream", json!(true))])).await?;
        ensure!(reply.status == 200 && reply.text()?.contains("event: error\n") && !reply.text()?.contains("[DONE]"), "truncation after stream start emits error without success marker");
        Ok(())
    }).await
}
