//! Observe real stream delivery before completion, then query the persisted measurements.

use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::fixture::{Fixture, Laboratory, Options, request};

pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(include_str!("../../fixtures/http/performance.js")),
                ..Options::default()
            },
            async |item| {
                gated_first_content(item).await?;
                distinct_reads_and_restart(item).await?;
                buffered_delivery(item).await?;
                cancelled_stream(item).await
            },
        )
        .await
}

fn frame(delta: &Value, finish: &Value) -> String {
    format!(
        "data: {}\n\n",
        json!({
            "id":"synthetic", "object":"chat.completion.chunk", "created":1,
            "model":"real-model", "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
        })
    )
}

fn tail() -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id":"synthetic", "object":"chat.completion.chunk", "created":1,
            "model":"real-model", "choices":[],
            "usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}
        })
    )
}

async fn configure(item: &Fixture, head: &str, tail: &str, gate: Option<Arc<Notify>>) {
    let mut controls = item.upstream.controls.lock().await;
    controls.media = "text/event-stream";
    controls.raw = Some(format!("{head}{tail}").into_bytes());
    controls.split_at = Some(head.len());
    controls.split_gate = gate;
    controls.started = false;
    controls.disconnected = false;
}

async fn gated_first_content(item: &Fixture) -> Result<()> {
    let head = format!(
        ": heartbeat\n\n{}",
        frame(&json!({"role":"assistant"}), &Value::Null)
    );
    let rest = format!(
        "{}{}{}",
        frame(&json!({"reasoning_content":"thinking"}), &Value::Null),
        frame(&json!({"content":"answer"}), &json!("stop")),
        tail()
    );
    let gate = Arc::new(Notify::new());
    configure(item, &head, &rest, Some(Arc::clone(&gate))).await;
    let mut reply = item
        .streaming_request(request(&[
            ("stream", json!(true)),
            ("temperature", json!(0.1)),
            ("max_tokens", json!(100)),
            ("metadata", json!({"private":"synthetic-body-secret"})),
        ]))
        .await?;
    if reply.status().as_u16() != 200 {
        let status = reply.status();
        let error = reply.text().await?;
        anyhow::bail!("gated stream failed: {status}: {error}");
    }
    let id = reply
        .headers()
        .get("x-limen-request-id")
        .context("stream ID missing")?
        .to_str()?
        .to_owned();
    let mut initial = Vec::new();
    while !String::from_utf8_lossy(&initial).contains("assistant") {
        let chunk = reply
            .chunk()
            .await?
            .context("stream ended before its role frame")?;
        initial.extend_from_slice(&chunk);
        ensure!(
            initial.len() < 32 * 1024,
            "role frame was not delivered within fixture budget"
        );
    }
    ensure!(
        !String::from_utf8_lossy(&initial).contains("answer"),
        "gate did not hold generation"
    );
    let pending = item.manager(&format!("/admin/v1/requests/{id}")).await?;
    ensure!(
        pending.body["data"]["state"] == "pending",
        "role/heartbeat finalized generation"
    );
    gate.notify_one();
    while reply.chunk().await?.is_some() {}
    let record = item.record(&id).await?;
    let timings = &record["timings"];
    let metrics = &record["metrics"];
    ensure!(
        number(metrics, "upstreamTtftMs")? > number(timings, "upstreamFirstChunkUs")? / 1000.0,
        "role-only first chunk was incorrectly used as TTFT"
    );
    ensure!(
        number(metrics, "downstreamTtftMs")? >= number(timings, "downstreamFirstEventUs")? / 1000.0,
        "downstream TTFT preceded the heartbeat/role delivery"
    );
    ensure!(
        number(metrics, "upstreamFirstTextMs")? >= number(metrics, "upstreamTtftMs")?,
        "first answer preceded reasoning"
    );
    ensure!(
        metrics["downstreamMode"] == "native_stream"
            && metrics["generationTps"].is_null()
            && metrics["generationTpsStatus"] == "insufficient_observations"
            && number(metrics, "averageOutputTps")? > 0.0,
        "single-read content fabricated token timing or lost average throughput"
    );
    parameter_snapshots(&record)?;
    phase(&record, "beforeSend", "provider", "native")?;
    phase(&record, "beforeSend", "model", "script")?;
    phase(&record, "onFinish", "global", "script")?;
    ensure!(
        number(timings, "cleanupUs")? >= 0.0,
        "cleanup was not persisted"
    );
    let summary = item
        .manager("/admin/v1/usage?clientStream=true&upstreamMedia=sse")
        .await?;
    ensure!(
        summary.status == 200
            && summary.body["data"]["overall"]["performance"]["upstreamTtftMs"]["samples"] == 1,
        "native stream performance was absent from persisted aggregation: {} {}",
        summary.status,
        summary.body
    );
    Ok(())
}

async fn distinct_reads_and_restart(item: &mut Fixture) -> Result<()> {
    let first = frame(&json!({"content":"first"}), &Value::Null);
    let rest = format!(
        "{}{}",
        frame(&json!({"content":"last"}), &json!("stop")),
        tail()
    );
    configure(item, &first, &rest, None).await;
    let reply = item.request(request(&[("stream", json!(true))])).await?;
    let id = reply.request_id()?.to_owned();
    let record = item.record(&id).await?;
    ensure!(
        record["metrics"]["generationTpsStatus"] == "available"
            && number(&record["metrics"], "generationTps")? > 0.0,
        "distinct source reads did not produce a qualified generation-rate sample"
    );
    item.restart().await?;
    let stored = item.manager(&format!("/admin/v1/requests/{id}")).await?;
    ensure!(
        stored.status == 200 && stored.body["data"] == record,
        "restart changed persisted timing, parameters or phase measurements"
    );
    Ok(())
}

async fn buffered_delivery(item: &Fixture) -> Result<()> {
    {
        let mut controls = item.upstream.controls.lock().await;
        controls.media = "application/json";
        controls.split_at = None;
        controls.split_gate = None;
        controls.raw = Some(serde_json::to_vec(&json!({
            "id":"synthetic", "object":"chat.completion", "created":1, "model":"real-model",
            "choices":[{"index":0,"message":{"role":"assistant","content":"buffered"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}
        }))?);
    }
    let reply = item.request(request(&[("stream", json!(true))])).await?;
    ensure!(reply.status == 200, "JSON-to-SSE fixture failed");
    let record = item.record(reply.request_id()?).await?;
    ensure!(
        record["metrics"]["downstreamMode"] == "buffered"
            && record["metrics"]["upstreamTtftMs"].is_null()
            && record["metrics"]["generationTps"].is_null()
            && number(&record["metrics"], "downstreamTtftMs")? >= 0.0,
        "buffered JSON was represented as native token timing"
    );
    let summary = item.manager("/admin/v1/usage?upstreamMedia=json").await?;
    ensure!(
        summary.status == 200
            && summary.body["data"]["overall"]["performance"]["downstreamTtftMs"]["samples"] == 0
            && summary.body["data"]["overall"]["performance"]["averageOutputTps"]["samples"] == 1,
        "buffered TTFT entered native-stream aggregation or average throughput disappeared"
    );
    Ok(())
}

async fn cancelled_stream(item: &Fixture) -> Result<()> {
    let first = frame(&json!({"content":"partial"}), &Value::Null);
    let rest = format!(
        "{}{}",
        frame(&json!({"content":"unseen"}), &json!("stop")),
        tail()
    );
    configure(item, &first, &rest, Some(Arc::new(Notify::new()))).await;
    let mut reply = item
        .streaming_request(request(&[("stream", json!(true))]))
        .await?;
    let id = reply
        .headers()
        .get("x-limen-request-id")
        .context("cancelled stream ID missing")?
        .to_str()?
        .to_owned();
    reply
        .chunk()
        .await?
        .context("cancelled stream emitted no initial content")?;
    drop(reply);
    let record = item.record(&id).await?;
    ensure!(
        record["outcome"] == "cancelled"
            && record["upstreamCompleted"] == false
            && record["metrics"]["upstreamTtftMs"].is_number()
            && record["metrics"]["generationTps"].is_null(),
        "cancellation discarded observed TTFT or fabricated complete throughput"
    );
    phase(&record, "onFinish", "global", "script")?;
    Ok(())
}

fn number(value: &Value, field: &str) -> Result<f64> {
    value[field]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .with_context(|| format!("missing or invalid performance measurement: {field}"))
}

fn parameter_snapshots(record: &Value) -> Result<()> {
    ensure!(
        record["parameters"]["client"]["temperature"] == 0.1
            && record["parameters"]["upstream"]["temperature"] == 0.9
            && record["parameters"]["client"]["max_tokens"] == 100
            && record["parameters"]["upstream"]["max_tokens"] == 16
            && record["parameters"]["upstream"]["top_p"] == 0.8,
        "parameter snapshots lost the original value or committed policy/default changes"
    );
    let params = record["parameters"].to_string();
    ensure!(
        !params.contains("synthetic-body-secret") && !params.contains("messages"),
        "parameter snapshots retained content"
    );
    Ok(())
}

fn phase(record: &Value, name: &str, scope: &str, executor: &str) -> Result<()> {
    let phases = record["phases"]
        .as_array()
        .context("phase measurements missing")?;
    ensure!(
        phases.iter().any(|phase| phase["phase"] == name
            && phase["scope"] == scope
            && phase["executor"] == executor
            && phase["calls"].as_u64().is_some_and(|calls| calls > 0)
            && phase["wallUs"].is_number()),
        "expected phase measurement missing: {name}/{scope}/{executor}"
    );
    Ok(())
}
