//! Real HTTP lifecycle assertions use request IDs and phase presence, never exact timings.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use super::fixture::{Fixture, Laboratory, Options, request};

pub struct RequestLog {
    events: Vec<Value>,
}

impl RequestLog {
    pub fn event(&self, message: &str) -> Result<&Value> {
        let rows: Vec<_> = self
            .events
            .iter()
            .filter(|event| event["event"] == message)
            .collect();
        ensure!(rows.len() == 1, "request must have one {message} event");
        Ok(rows[0])
    }

    pub fn hook(&self, phase: &str, cleanup: bool) -> Result<&Value> {
        let rows: Vec<_> = self
            .events
            .iter()
            .filter(|event| {
                event["event"] == "hookStatistics"
                    && event["phase"] == phase
                    && event["cleanup"] == cleanup
            })
            .collect();
        ensure!(
            rows.len() == 1,
            "request must have one hook summary for {phase}, cleanup={cleanup}"
        );
        Ok(rows[0])
    }

    pub fn outcome(&self, outcome: &str, code: Option<&str>) -> Result<()> {
        let finished = self.event("requestFinished")?;
        ensure!(
            finished["outcome"] == outcome && finished["code"].as_str() == code,
            "terminal outcome changed: expected {outcome}/{code:?}, got {}/{}",
            finished["outcome"],
            finished["code"]
        );
        duration(self.event("requestTiming")?, "requestMs")
    }

    pub fn no_upstream_samples(&self) -> Result<()> {
        let stats = self.event("requestTiming")?;
        for field in [
            "upstreamHeadersWaitMs",
            "upstreamFirstByteMs",
            "upstreamFirstChunkMs",
            "upstreamReadWaitMs",
            "downstreamFirstEventMs",
        ] {
            ensure!(
                stats[field].is_null(),
                "unreached upstream phase was sampled: {field}"
            );
        }
        Ok(())
    }

    pub fn interrupted_hook(&self, phase: &str, cleanup: bool, dropped_future: bool) -> Result<()> {
        let hook = self.hook(phase, cleanup)?;
        ensure!(
            hook["calls"] == 1 && hook["interruptedCalls"] == 1,
            "interrupted hook was lost from statistics"
        );
        duration(hook, "wallMs")?;
        if dropped_future {
            ensure!(
                hook["reportedCalls"] == 0,
                "dropped hook future fabricated a runtime report"
            );
            for field in [
                "queueSamples",
                "initialCallSamples",
                "pendingSamples",
                "inputBridgeSamples",
                "outputBridgeSamples",
            ] {
                ensure!(
                    hook[field] == 0,
                    "dropped hook future fabricated a sample: {field}"
                );
            }
        } else {
            ensure!(
                hook["queueSamples"]
                    .as_u64()
                    .context("queue sample count missing")?
                    <= hook["reportedCalls"]
                        .as_u64()
                        .context("reported hook count missing")?,
                "queue samples exceeded actual reports"
            );
        }
        Ok(())
    }

    pub fn observation(&self, event_name: &str, phase: &str) -> Result<()> {
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|event| {
                event["event"] == "scriptObservation"
                    && event["observationEvent"] == event_name
                    && event["phase"] == phase
            })
            .collect();
        ensure!(
            events.len() == 1 && events[0]["count"] == 1,
            "committed observation missing or duplicated: {event_name}"
        );
        Ok(())
    }

    pub fn no_observation(&self, event_name: &str) -> Result<()> {
        ensure!(
            !self
                .events
                .iter()
                .any(|event| event["event"] == "scriptObservation"
                    && event["observationEvent"] == event_name),
            "uncommitted observation escaped: {event_name}"
        );
        Ok(())
    }
}

pub fn duration(event: &Value, field: &str) -> Result<()> {
    ensure!(
        event[field]
            .as_f64()
            .is_some_and(|value| value.is_finite() && value >= 0.0),
        "duration was missing or invalid: {field}"
    );
    Ok(())
}

impl Fixture {
    /// All returned events belong to the same finished request, including cleanup rows.
    pub async fn statistics(&self, request_id: &str, hooks: &[(&str, bool)]) -> Result<RequestLog> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let events: Vec<_> = self
                .events()?
                .into_iter()
                .filter(|event| event["requestId"] == request_id)
                .collect();
            let has = |message: &str| events.iter().any(|event| event["event"] == message);
            let record_complete = events
                .iter()
                .any(|event| event["event"] == "requestFinished" && event["outcome"] == "success")
                || has("requestFailed")
                || has("requestRejected");
            if has("requestFinished")
                && has("requestTiming")
                && record_complete
                && hooks.iter().all(|(phase, cleanup)| {
                    events.iter().any(|event| {
                        event["event"] == "hookStatistics"
                            && event["phase"] == *phase
                            && event["cleanup"] == *cleanup
                    })
                })
            {
                return Ok(RequestLog { events });
            }
            ensure!(
                Instant::now() < deadline,
                "request statistics not observed: {request_id}"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Controlled events and downstream backpressure retain their real terminal outcomes.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    observations(laboratory).await?;
    slow_downstream(laboratory).await
}

async fn observations(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(include_str!("../../fixtures/http/statistics.js")),
                observation_names: &["accepted", "pending_private", "cleanup_private", "finished"],
                ..Options::default()
            },
            async |item| {
                successful_observations(item).await?;
                cancelled_observations(item).await?;
                cleanup_budget_observations(item).await
            },
        )
        .await
}

async fn successful_observations(item: &Fixture) -> Result<()> {
    item.upstream.controls.lock().await.raw = Some(serde_json::to_vec(&json!({
        "id":"synthetic", "model":"real-model", "choices":[],
        "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}
    }))?);
    let reply = item.request(request(&[])).await?;
    ensure!(
        reply.status == 200,
        "observations changed successful HTTP behavior"
    );
    let id = reply.request_id()?;
    item.wait_event("scriptObservation", |event| {
        event["requestId"] == id && event["observationEvent"] == "finished"
    })
    .await?;
    let log = item
        .statistics(
            id,
            &[
                ("onRequest", false),
                ("onResponse", false),
                ("onFinish", true),
            ],
        )
        .await?;
    log.outcome("success", None)?;
    log.observation("accepted", "onRequest")?;
    log.observation("finished", "onFinish")?;

    let stats = item.record(id).await?;
    ensure!(
        stats["usageQuality"] == "reported"
            && stats["usageSource"] == "json"
            && stats["upstreamCompleted"] == true
            && stats["inputTokens"] == "7"
            && stats["outputTokens"] == "3"
            && stats["totalTokens"] == "10",
        "HTTP statistics lost explicit upstream usage counts"
    );
    item.upstream.controls.lock().await.raw = None;
    Ok(())
}

async fn cancelled_observations(item: &Fixture) -> Result<()> {
    let held = item.open_request(request(&[("wait", json!(true))])).await?;
    item.wait_pending(1).await?;
    drop(held);
    item.wait_cancelled(1).await?;
    let finished = item
        .wait_event("requestFinished", |event| event["outcome"] == "cancelled")
        .await?;
    let id = finished["requestId"]
        .as_str()
        .context("cancelled observation request ID missing")?;
    item.wait_event("scriptObservation", |event| {
        event["requestId"] == id && event["observationEvent"] == "finished"
    })
    .await?;
    let log = item
        .statistics(
            id,
            &[("onRequest", false), ("onError", true), ("onFinish", true)],
        )
        .await?;
    log.outcome("cancelled", Some("cancelled"))?;
    log.interrupted_hook("onRequest", false, true)?;
    log.no_upstream_samples()?;
    log.no_observation("pending_private")?;
    log.observation("finished", "onFinish")?;

    Ok(())
}

async fn cleanup_budget_observations(item: &Fixture) -> Result<()> {
    let reply = item
        .request(request(&[("cleanupWait", json!(true))]))
        .await?;
    reply.error(
        403,
        "request_rejected",
        "cleanup timeout retained business rejection",
    )?;
    let id = reply.request_id()?;
    let log = item
        .statistics(id, &[("onRequest", false), ("onError", true)])
        .await?;
    log.outcome("failed", Some("request_rejected"))?;
    log.interrupted_hook("onError", true, false)?;
    log.no_upstream_samples()?;
    log.observation("accepted", "onRequest")?;
    log.no_observation("cleanup_private")?;
    log.no_observation("finished")?;
    item.wait_event("observationSkipped", |event| {
        event["requestId"] == id
            && event["phase"] == "onFinish"
            && event["reason"] == "cleanup_budget"
    })
    .await?;
    ensure!(
        item.request(request(&[])).await?.status == 200,
        "cleanup budget retained admission permit"
    );
    Ok(())
}

/// A finite response exceeds socket buffering while the client intentionally does not read.
async fn slow_downstream(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory.with_fixture(Options::default(), async |item| {
        let event = json!({"id":"synthetic", "object":"chat.completion.chunk", "created":1, "model":"real-model", "choices":[{"index":0,"delta":{"content":"x".repeat(32 * 1024)},"finish_reason":null}]});
        let frame = format!("data: {}\n\n", serde_json::to_string(&event)?).into_bytes();
        let final_event = json!({"id":"synthetic", "object":"chat.completion.chunk", "created":1, "model":"real-model", "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
        let mut body = Vec::with_capacity(frame.len() * 256 + 512);
        for _ in 0..256 { body.extend_from_slice(&frame); }
        body.extend_from_slice(format!("data: {}\n\ndata: [DONE]\n\n", serde_json::to_string(&final_event)?).as_bytes());
        {
            let mut upstream = item.upstream.controls.lock().await;
            upstream.media = "text/event-stream";
            upstream.raw = Some(body);
            // Existing delayed-tail support permits abandonment of this finite response.
            upstream.split_at = Some(frame.len());
        }
        let held = item.open_request(request(&[("stream", json!(true))])).await?;
        item.upstream.wait(Duration::from_secs(1), false).await?;
        // Await the gateway's own terminal event without draining the client socket.
        let finished = item.wait_event("requestFinished", |event| event["code"] == "deadline_exceeded").await?;
        let id = finished["requestId"].as_str().context("slow downstream request ID missing")?;
        let log = item.statistics(id, &[]).await?;
        log.outcome("failed", Some("deadline_exceeded"))?;
        ensure!(finished["phase"] == "response", "slow downstream did not stop while delivering a response");
        let stats = log.event("requestTiming")?;
        for field in ["upstreamHeadersWaitMs", "upstreamFirstByteMs", "upstreamFirstChunkMs", "upstreamReadWaitMs", "downstreamFirstEventMs", "downstreamQueueWaitMs"] {
            duration(stats, field)?;
        }
        let record = item.record(id).await?;
        ensure!(record["upstreamCompleted"] == false && record["usageQuality"] == "missing", "backpressured partial stream was reported complete");
        drop(held);
        {
            let mut upstream = item.upstream.controls.lock().await;
            upstream.raw = None;
            upstream.split_at = None;
            upstream.media = "application/json";
            upstream.delay = true;
        }
        // Two 100 Continue responses prove both admission permits were reclaimed.
        let held = [item.open_request(request(&[])).await?, item.open_request(request(&[])).await?];
        item.request(request(&[])).await?.error(503, "busy", "reclaimed capacity remains bounded")?;
        drop(held);
        Ok(())
    }).await
}
