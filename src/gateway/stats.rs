use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use limen_chat::UsageTracker;
use limen_script::{HookReport, HookTimings, ScriptErrorKind};
use serde_json::{Value, json};

use super::error::{ErrorCode, Failure};

/// Runtime-owned facts are sampled before policy drafts can alter them.
#[derive(Default)]
pub(super) struct Statistics {
    pub usage: UsageTracker,
    pub source: Option<&'static str>,
    pub upstream_completed: bool,
    pub timings: Timings,
    frozen: Option<Value>,
}

impl Statistics {
    pub fn freeze(&mut self, started: Instant, now: Instant) {
        if self.frozen.is_some() {
            return;
        }
        self.timings.freeze(now.saturating_duration_since(started));
        self.frozen = Some(self.snapshot());
    }

    pub fn snapshot(&self) -> Value {
        if let Some(value) = &self.frozen {
            return value.clone();
        }
        let mut usage = self.usage.snapshot().to_json();
        usage["source"] = json!(self.source);
        usage["upstreamCompleted"] = json!(self.upstream_completed);
        json!({"usage": usage, "timings": self.timings.snapshot()})
    }
}

#[derive(Clone, Default)]
pub(super) struct Timings(Arc<Mutex<TimingData>>);

#[derive(Default)]
struct TimingData {
    frozen: bool,
    request: Option<Duration>,
    decode: Option<Duration>,
    upstream_started: Option<Instant>,
    headers: Option<Duration>,
    first_byte: Option<Duration>,
    first_chunk: Option<Duration>,
    first_output: Option<Duration>,
    read: Option<Duration>,
    queue: Option<Duration>,
    drain: Option<Duration>,
    hooks: BTreeMap<&'static str, HookSummary>,
    native: BTreeMap<&'static str, NativeSummary>,
}

#[derive(Default)]
struct NativeSummary {
    calls: u64,
    failed: u64,
    interrupted: u64,
    wall: Duration,
}

impl NativeSummary {
    fn json(&self) -> Value {
        json!({"calls":self.calls,"failedCalls":self.failed,"interruptedCalls":self.interrupted,"wallMs":ms(self.wall)})
    }
}

#[derive(Default)]
struct Sample {
    elapsed: Duration,
    count: u64,
}

impl Sample {
    fn add(&mut self, value: Option<Duration>) {
        if let Some(value) = value {
            self.elapsed = self.elapsed.saturating_add(value);
            self.count = self.count.saturating_add(1);
        }
    }

    fn json(&self) -> Value { json!({"totalMs": ms(self.elapsed), "samples": self.count}) }
}

#[derive(Default)]
struct HookSummary {
    calls: u64,
    failed: u64,
    interrupted: u64,
    reported: u64,
    wall: Duration,
    queue: Sample,
    initial: Sample,
    pending: Sample,
    input: Sample,
    output: Sample,
}

impl HookSummary {
    fn report(&mut self, report: &HookReport) {
        self.reported = self.reported.saturating_add(1);
        if let Err(error) = &report.outcome {
            self.failed = self.failed.saturating_add(1);
            if matches!(
                error.kind(),
                ScriptErrorKind::Cancelled | ScriptErrorKind::Deadline
            ) {
                self.interrupted = self.interrupted.saturating_add(1);
            }
        }
        let HookTimings {
            queue,
            initial_call,
            pending,
            input_bridge,
            output_bridge,
            ..
        } = report.timings;
        self.queue.add(queue);
        self.initial.add(initial_call);
        self.pending.add(pending);
        self.input.add(Some(input_bridge));
        self.output.add(output_bridge);
    }

    fn json(&self) -> Value {
        json!({"calls":self.calls,"failedCalls":self.failed,"interruptedCalls":self.interrupted,
            "reportedCalls":self.reported,"wallMs":ms(self.wall),"queue":self.queue.json(),
            "initialCall":self.initial.json(),"pending":self.pending.json(),
            "inputBridge":self.input.json(),"outputBridge":self.output.json()})
    }
}

#[derive(Clone, Copy)]
pub(super) enum WaitKind {
    Decode,
    Read,
    Queue,
    Drain,
}

impl Timings {
    fn lock(&self) -> MutexGuard<'_, TimingData> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn measure(&self, kind: WaitKind) -> WaitGuard {
        WaitGuard {
            timings: self.clone(),
            kind,
            started: Instant::now(),
        }
    }

    pub fn hook(&self, phase: &'static str) -> HookGuard {
        HookGuard {
            timings: self.clone(),
            phase,
            started: Instant::now(),
            returned: false,
        }
    }

    pub fn native(&self, phase: &'static str) -> NativeGuard {
        NativeGuard {
            timings: self.clone(),
            phase,
            started: Instant::now(),
            result: None,
        }
    }

    pub fn start_upstream(&self, now: Instant) { self.lock().upstream_started = Some(now); }

    pub fn headers(&self, now: Instant) {
        let mut data = self.lock();
        data.headers = data
            .upstream_started
            .map(|start| now.saturating_duration_since(start));
    }

    pub fn first_byte(&self, now: Instant) {
        let mut data = self.lock();
        if data.first_byte.is_none() {
            data.first_byte = data
                .upstream_started
                .map(|start| now.saturating_duration_since(start));
        }
    }

    pub fn first_chunk(&self, now: Instant) {
        let mut data = self.lock();
        if data.first_chunk.is_none() {
            data.first_chunk = data
                .upstream_started
                .map(|start| now.saturating_duration_since(start));
        }
    }

    pub fn first_output(&self, started: Instant, now: Instant) {
        let mut data = self.lock();
        if data.first_output.is_none() && !data.frozen {
            data.first_output = Some(now.saturating_duration_since(started));
        }
    }

    fn freeze(&self, elapsed: Duration) {
        let mut data = self.lock();
        data.request = Some(elapsed);
        data.frozen = true;
    }

    pub fn snapshot(&self) -> Value {
        let data = self.lock();
        let hooks: serde_json::Map<String, Value> = data
            .hooks
            .iter()
            .map(|(phase, value)| ((*phase).to_owned(), value.json()))
            .collect();
        let native: serde_json::Map<String, Value> = data
            .native
            .iter()
            .map(|(phase, value)| ((*phase).to_owned(), value.json()))
            .collect();
        json!({"requestMs":data.request.map(ms),"requestDecodeMs":data.decode.map(ms),
            "upstreamHeadersWaitMs":data.headers.map(ms),"upstreamFirstByteMs":data.first_byte.map(ms),
            "upstreamFirstChunkMs":data.first_chunk.map(ms),"downstreamFirstEventMs":data.first_output.map(ms),
            "upstreamReadWaitMs":data.read.map(ms),"downstreamQueueWaitMs":data.queue.map(ms),
            "downstreamDrainMs":data.drain.map(ms),"hooks":hooks,"native":native})
    }
}

/// Dropping a cancelled future still accounts for time already spent waiting.
pub(super) struct WaitGuard {
    timings: Timings,
    kind: WaitKind,
    started: Instant,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        let mut data = self.timings.lock();
        if data.frozen {
            return;
        }
        let slot = match self.kind {
            WaitKind::Decode => &mut data.decode,
            WaitKind::Read => &mut data.read,
            WaitKind::Queue => &mut data.queue,
            WaitKind::Drain => &mut data.drain,
        };
        *slot = Some(
            slot.unwrap_or_default()
                .saturating_add(self.started.elapsed()),
        );
    }
}

pub(super) struct HookGuard {
    timings: Timings,
    phase: &'static str,
    started: Instant,
    returned: bool,
}

impl HookGuard {
    pub fn finish(mut self, report: &HookReport) {
        self.returned = true;
        let mut data = self.timings.lock();
        if !data.frozen {
            data.hooks.entry(self.phase).or_default().report(report);
        }
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        let mut data = self.timings.lock();
        if data.frozen {
            return;
        }
        let hook = data.hooks.entry(self.phase).or_default();
        hook.calls = hook.calls.saturating_add(1);
        hook.wall = hook.wall.saturating_add(self.started.elapsed());
        if !self.returned {
            hook.interrupted = hook.interrupted.saturating_add(1);
        }
    }
}

pub(super) struct NativeGuard {
    timings: Timings,
    phase: &'static str,
    started: Instant,
    result: Option<(bool, bool)>,
}

impl NativeGuard {
    pub fn finish<T>(mut self, result: &Result<T, Failure>) {
        self.result = Some((
            result.is_err(),
            result.as_ref().err().is_some_and(|failure| {
                matches!(
                    failure.code,
                    ErrorCode::Cancelled | ErrorCode::DeadlineExceeded
                )
            }),
        ));
    }
}

impl Drop for NativeGuard {
    fn drop(&mut self) {
        let mut data = self.timings.lock();
        if data.frozen {
            return;
        }
        let summary = data.native.entry(self.phase).or_default();
        summary.calls = summary.calls.saturating_add(1);
        summary.wall = summary.wall.saturating_add(self.started.elapsed());
        let (failed, interrupted) = self.result.unwrap_or((true, true));
        summary.failed = summary.failed.saturating_add(u64::from(failed));
        summary.interrupted = summary.interrupted.saturating_add(u64::from(interrupted));
    }
}

fn ms(value: Duration) -> f64 { value.as_secs_f64() * 1000.0 }

pub(super) fn record(request_id: &str, stats: &Value) {
    let timings = &stats["timings"];
    tracing::info!(
        request_id,
        request_ms = timings["requestMs"].as_f64(),
        request_decode_ms = timings["requestDecodeMs"].as_f64(),
        upstream_headers_wait_ms = timings["upstreamHeadersWaitMs"].as_f64(),
        upstream_first_byte_ms = timings["upstreamFirstByteMs"].as_f64(),
        upstream_first_chunk_ms = timings["upstreamFirstChunkMs"].as_f64(),
        downstream_first_event_ms = timings["downstreamFirstEventMs"].as_f64(),
        upstream_read_wait_ms = timings["upstreamReadWaitMs"].as_f64(),
        downstream_queue_wait_ms = timings["downstreamQueueWaitMs"].as_f64(),
        downstream_drain_ms = timings["downstreamDrainMs"].as_f64(),
        "request timing"
    );
    record_hooks(request_id, timings, false);
    if let Some(native) = timings["native"].as_object() {
        for (phase, summary) in native {
            tracing::info!(
                request_id,
                phase,
                calls = summary["calls"].as_u64(),
                failed_calls = summary["failedCalls"].as_u64(),
                interrupted_calls = summary["interruptedCalls"].as_u64(),
                wall_ms = summary["wallMs"].as_f64(),
                "native policy statistics"
            );
        }
    }
}

pub(super) fn record_cleanup(request_id: &str, timings: &Value) {
    record_hooks(request_id, timings, true);
}

fn record_hooks(request_id: &str, timings: &Value, cleanup: bool) {
    if let Some(hooks) = timings["hooks"].as_object() {
        for (phase, hook) in hooks {
            tracing::info!(
                request_id,
                phase,
                cleanup,
                calls = hook["calls"].as_u64(),
                failed_calls = hook["failedCalls"].as_u64(),
                interrupted_calls = hook["interruptedCalls"].as_u64(),
                reported_calls = hook["reportedCalls"].as_u64(),
                wall_ms = hook["wallMs"].as_f64(),
                queue_ms = hook["queue"]["totalMs"].as_f64(),
                queue_samples = hook["queue"]["samples"].as_u64(),
                initial_call_ms = hook["initialCall"]["totalMs"].as_f64(),
                initial_call_samples = hook["initialCall"]["samples"].as_u64(),
                pending_ms = hook["pending"]["totalMs"].as_f64(),
                pending_samples = hook["pending"]["samples"].as_u64(),
                input_bridge_ms = hook["inputBridge"]["totalMs"].as_f64(),
                input_bridge_samples = hook["inputBridge"]["samples"].as_u64(),
                output_bridge_ms = hook["outputBridge"]["totalMs"].as_f64(),
                output_bridge_samples = hook["outputBridge"]["samples"].as_u64(),
                "hook statistics"
            );
        }
    }
}

#[cfg(test)]
mod tests;
