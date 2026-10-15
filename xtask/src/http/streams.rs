//! Stream policy is verified through decoded client data and complete aggregate responses.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::fixture::{Laboratory, Options, Reply, request};

const CONFIG: &str = r"export default {
 onRequest(ctx) { ctx.state.explicitName = ctx.body.explicitName === true; },
 providers: {p: {endpoint: ENDPOINT, pipeline: [
  {name: 'filter-and-split', onUpstreamEvent(ctx) {
   const content = ctx.event.choices[0]?.delta.content;
   if (content === 'remove') ctx.replaceEvents([]);
   if (content === 'hello') {
    const left = JSON.parse(JSON.stringify(ctx.event));
    const right = JSON.parse(JSON.stringify(ctx.event));
    left.choices[0].delta.content = 'he';
    right.choices[0].delta.content = 'llo';
    ctx.replaceEvents([left, right]);
   }
  }},
  {name: 'normalize', async onUpstreamEvent(ctx) {
   await Promise.resolve();
   const delta = ctx.event.choices[0]?.delta;
   if (typeof delta?.content === 'string') delta.content = delta.content.toUpperCase();
  }, onStreamEvent(ctx) {
   const delta = ctx.event.choices[0]?.delta;
   if (typeof delta?.content === 'string') delta.content = '[' + delta.content;
  }, onResponse(ctx) {
   ctx.body.choices[0].message.content = '[' + ctx.body.choices[0].message.content;
  }}
 ]}},
 models: {chat: {provider: 'p', upstream: 'real-model', pipeline: [
  {name: 'decorate', async onUpstreamEvent(ctx) {
   await Promise.resolve();
   const delta = ctx.event.choices[0]?.delta;
   if (typeof delta?.content === 'string') delta.content += '!';
  }, async onStreamEvent(ctx) {
   await Promise.resolve();
   const delta = ctx.event.choices[0]?.delta;
   if (typeof delta?.content === 'string') delta.content += ']';
   if (ctx.state.explicitName) ctx.event.model = 'stream-model';
  }, onResponse(ctx) {
   ctx.body.choices[0].message.content += ']';
   ctx.body.model = 'completed-model';
  }}
 ]}}
};";

/// Exercise drop/split, async edits, aliases and usage over both SSE output paths.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(CONFIG),
                ..Options::default()
            },
            async |item| {
                let mut events = vec![chunk(&json!({"role":"assistant"}), &Value::Null)];
                for text in ["remove", "hello", " world"] {
                    events.push(chunk(&json!({"content":text}), &Value::Null));
                }
                events.push(chunk(&json!({}), &json!("stop")));
                events.push(json!({"id":"synthetic","object":"chat.completion.chunk",
                    "created":1,"model":"real-model","choices":[],
                    "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}));
                {
                    let mut upstream = item.upstream.controls.lock().await;
                    upstream.media = "text/event-stream";
                    upstream.raw = Some(encode(&events)?);
                }
                for explicit in [false, true] {
                    let reply = item.request(request(&[
                        ("stream", json!(true)), ("explicitName", json!(explicit)),
                    ])).await?;
                    ensure!(reply.status == 200, "stream transforms produce HTTP success");
                    let events = decode(&reply)?;
                    let fragments = events.iter().filter_map(|event| event["choices"][0]["delta"]["content"].as_str()).collect::<Vec<_>>();
                    ensure!(fragments == ["[HE!]", "[LLO!]", "[ WORLD!]"], "client receives transformed fragments in order with dropped content absent");
                    ensure!(events.iter().all(|event| event["model"] == if explicit { "stream-model" } else { "chat" }), "stream alias defaults and explicit model edits survive output");
                    ensure!(events.first().context("stream role chunk missing")?["choices"][0]["delta"]["role"] == "assistant", "role survives stream transforms");
                    ensure!(events.iter().any(|event| event["choices"][0]["finish_reason"] == "stop"), "finish reason survives stream transforms");
                    ensure!(events.last().context("usage chunk missing")?["usage"]["total_tokens"] == 3, "usage after finish reaches client");
                }
                let reply = item.request(request(&[])).await?;
                ensure!(reply.status == 200 && reply.header("content-type")? == "application/json", "SSE aggregate is delivered as JSON");
                ensure!(reply.body["choices"][0]["message"] == json!({"role":"assistant","content":"[HE!LLO! WORLD!]"}), "aggregate uses transformed upstream events and complete-response edits");
                ensure!(reply.body["model"] == "completed-model" && reply.body["usage"]["total_tokens"] == 3, "aggregate preserves explicit response model and trailing usage");

                // A JSON upstream exercises onResponse followed by outgoing stream policy.
                {
                    let mut upstream = item.upstream.controls.lock().await;
                    upstream.media = "application/json";
                    upstream.raw = None;
                    upstream.response = Some(json!({"id":"synthetic","object":"chat.completion",
                        "created":1,"model":"real-model","choices":[{"index":0,
                            "message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],
                        "usage":{"total_tokens":3}}));
                }
                let reply = item.request(request(&[("stream", json!(true))])).await?;
                ensure!(reply.status == 200, "JSON converts to transformed SSE");
                let events = decode(&reply)?;
                let content = events.iter().filter_map(|event| event["choices"][0]["delta"]["content"].as_str()).collect::<String>();
                ensure!(content == "[[hello]]", "complete-response and outgoing-event edits reach JSON-to-SSE client");
                ensure!(events.iter().all(|event| event["model"] == "completed-model"), "JSON-to-SSE retains explicit response model");
                Ok(())
            },
        )
        .await
}

/// Build synthetic Chat input without depending on the gateway's conversion code.
fn chunk(delta: &Value, finish: &Value) -> Value {
    json!({"id":"synthetic","object":"chat.completion.chunk","created":1,
        "model":"real-model","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}

/// The fixture uses single-line data frames; Hyper remains responsible for HTTP framing.
fn encode(events: &[Value]) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    for event in events {
        data.extend_from_slice(b"data: ");
        data.extend_from_slice(&serde_json::to_vec(event).context("encode stream fixture event")?);
        data.extend_from_slice(b"\n\n");
    }
    data.extend_from_slice(b"data: [DONE]\n\n");
    Ok(data)
}

/// Read Limen's public single-line JSON data frames independently of its SSE implementation.
fn decode(reply: &Reply) -> Result<Vec<Value>> {
    ensure!(
        reply.header("content-type")? == "text/event-stream",
        "client receives SSE media type"
    );
    let text = reply.text()?;
    ensure!(
        !text.contains("event: error")
            && text.ends_with("data: [DONE]\n\n")
            && text.matches("data: [DONE]").count() == 1,
        "successful stream ends exactly once without an error event"
    );
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).context("decode client stream event"))
        .collect()
}
