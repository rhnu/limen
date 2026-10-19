use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use limen_chat::{GeneratedContent, UsageTracker};
use limen_script::{HookReport, HookTimings, ScriptErrorKind};
use limen_storage::{
    GenerationFacts, PhaseExecutor, PhaseScope, PhaseTiming, RequestTimings, TimingSample,
};
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

#[derive(Clone)]
pub(super) struct Timings(Arc<Mutex<TimingData>>);

#[derive(Default)]
struct TimingData {
    frozen: bool,
    request: Option<Duration>,
    decode: Option<Duration>,
    storage_begin: Option<Duration>,
    routing: Option<Duration>,
    encode: Option<Duration>,
    pre_send: Option<Duration>,
    upstream_total: Option<Duration>,
    generation: GenerationFacts,
    last_content_observation: Option<Instant>,
    upstream_started: Option<Instant>,
    headers: Option<Duration>,
    first_byte: Option<Duration>,
    first_chunk: Option<Duration>,
    first_output: Option<Duration>,
    read: Option<Duration>,
    queue: Option<Duration>,
    drain: Option<Duration>,
    drained_at: Option<Instant>,
    hooks: BTreeMap<(&'static str, PhaseScope), HookSummary>,
    native: BTreeMap<(&'static str, PhaseScope), NativeSummary>,
}

#[derive(Clone, Default)]
struct NativeSummary {
    calls: u64,
    reported: u64,
    failed: u64,
    interrupted: u64,
    wall: Duration,
}

impl NativeSummary {
    fn json(&self) -> Value {
        json!({"calls":self.calls,"failedCalls":self.failed,"interruptedCalls":self.interrupted,"wallMs":ms(self.wall)})
    }
}

#[derive(Clone, Default)]
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

#[derive(Clone, Default)]
struct HookSummary {
    calls: u64,
    failed: u64,
    interrupted: u64,
    reported: u64,
    skipped_budget: u64,
    unavailable: u64,
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
    StorageBegin,
    Routing,
    Encode,
    Read,
    Queue,
    Drain,
}

impl Default for Timings {
    fn default() -> Self {
        let mut data = TimingData::default();
        data.generation.content_qualified = true;
        Self(Arc::new(Mutex::new(data)))
    }
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
        self.hook_scoped(phase, PhaseScope::Global)
    }

    pub fn hook_scoped(&self, phase: &'static str, scope: PhaseScope) -> HookGuard {
        HookGuard {
            timings: self.clone(),
            phase,
            scope,
            started: Instant::now(),
            returned: false,
        }
    }

    pub fn native_scoped(&self, phase: &'static str, scope: PhaseScope) -> NativeGuard {
        NativeGuard {
            timings: self.clone(),
            phase,
            scope,
            started: Instant::now(),
            result: None,
            calls: 1,
        }
    }

    pub fn start_upstream(&self, started: Instant, now: Instant) {
        let mut data = self.lock();
        data.upstream_started = Some(now);
        data.pre_send = Some(now.saturating_duration_since(started));
    }

    pub fn upstream_completed(&self, now: Instant) {
        let mut data = self.lock();
        data.upstream_total = data
            .upstream_started
            .map(|start| now.saturating_duration_since(start));
    }

    pub fn finishes(&self, counts: limen_chat::FinishCounts, choices: usize) {
        let mut data = self.lock();
        if data.frozen {
            return;
        }
        data.generation.choice_count = choices as u64;
        data.generation.finish_stop = counts.stop;
        data.generation.finish_length = counts.length;
        data.generation.finish_tool = counts.tool;
        data.generation.finish_filtered = counts.filtered;
        data.generation.finish_other = counts.other;
    }

    pub fn buffered(&self) { self.lock().generation.downstream_buffered = true; }

    pub fn upstream_content(&self, content: GeneratedContent, choice_count: usize, now: Instant) {
        let mut data = self.lock();
        if data.frozen {
            return;
        }
        let elapsed = data
            .upstream_started
            .map(|start| micros(now.saturating_duration_since(start)));
        data.generation.choice_count = choice_count as u64;
        data.generation.content_qualified &= content.qualified;
        data.generation.observed_reasoning |= content.reasoning;
        data.generation.observed_audio |= content.audio;
        if content.generation {
            if data.generation.upstream_first_content_us.is_none() {
                data.generation.upstream_first_content_us = elapsed;
            }
            data.generation.upstream_last_content_us = elapsed;
            if data.last_content_observation != Some(now) {
                data.generation.upstream_content_observations = data
                    .generation
                    .upstream_content_observations
                    .saturating_add(1);
                data.last_content_observation = Some(now);
            }
        }
        if content.text && data.generation.upstream_first_text_us.is_none() {
            data.generation.upstream_first_text_us = elapsed;
        }
    }

    pub fn drained(&self, now: Instant) {
        let mut data = self.lock();
        if !data.frozen && data.drained_at.is_none() {
            data.drained_at = Some(now);
        }
    }

    pub fn drained_at(&self) -> Option<Instant> { self.lock().drained_at }

    pub fn downstream(&self, started: Instant, now: Instant, generation: bool, text: bool) {
        let mut data = self.lock();
        if data.frozen {
            return;
        }
        let elapsed = now.saturating_duration_since(started);
        if data.first_output.is_none() {
            data.first_output = Some(elapsed);
        }
        if generation && data.generation.downstream_first_content_us.is_none() {
            data.generation.downstream_first_content_us = Some(micros(elapsed));
        }
        if text && data.generation.downstream_first_text_us.is_none() {
            data.generation.downstream_first_text_us = Some(micros(elapsed));
        }
    }

    pub fn skip(&self, phase: &'static str, scope: PhaseScope, unavailable: bool) {
        let mut data = self.lock();
        let summary = data.hooks.entry((phase, scope)).or_default();
        if unavailable {
            summary.unavailable = summary.unavailable.saturating_add(1);
        } else {
            summary.skipped_budget = summary.skipped_budget.saturating_add(1);
        }
    }

    pub fn owned(&self) -> (RequestTimings, GenerationFacts, Vec<PhaseTiming>) {
        let data = self.lock();
        let timings = RequestTimings {
            request_us: data.request.map(micros),
            request_decode_us: data.decode.map(micros),
            storage_begin_us: data.storage_begin.map(micros),
            pre_send_us: data.pre_send.map(micros),
            routing_us: data.routing.map(micros),
            request_encode_us: data.encode.map(micros),
            upstream_headers_wait_us: data.headers.map(micros),
            upstream_first_byte_us: data.first_byte.map(micros),
            upstream_first_chunk_us: data.first_chunk.map(micros),
            upstream_total_us: data.upstream_total.map(micros),
            downstream_first_event_us: data.first_output.map(micros),
            upstream_read_wait_us: data.read.map(micros),
            downstream_queue_wait_us: data.queue.map(micros),
            downstream_drain_us: data.drain.map(micros),
            cleanup_us: None,
        };
        let mut phases: Vec<_> = data
            .hooks
            .iter()
            .map(|(&(phase, scope), summary)| summary.owned(phase, scope))
            .collect();
        phases.extend(
            data.native
                .iter()
                .map(|(&(phase, scope), summary)| PhaseTiming {
                    phase: phase.into(),
                    scope,
                    executor: PhaseExecutor::Native,
                    calls: summary.calls,
                    failed_calls: summary.failed,
                    interrupted_calls: summary.interrupted,
                    reported_calls: summary.reported,
                    wall_us: (summary.calls > 0).then(|| micros(summary.wall)),
                    ..empty_phase()
                }),
        );
        (timings, data.generation.clone(), phases)
    }

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

    fn freeze(&self, elapsed: Duration) {
        let mut data = self.lock();
        data.request = Some(elapsed);
        data.frozen = true;
    }

    pub fn snapshot(&self) -> Value {
        let data = self.lock();
        let mut hook_summaries: BTreeMap<&str, HookSummary> = BTreeMap::new();
        for ((phase, _scope), value) in &data.hooks {
            hook_summaries.entry(phase).or_default().merge(value);
        }
        let hooks: serde_json::Map<String, Value> = hook_summaries
            .into_iter()
            .map(|(phase, value)| (phase.to_owned(), value.json()))
            .collect();
        let mut native_summaries: BTreeMap<&str, NativeSummary> = BTreeMap::new();
        for ((phase, _scope), value) in &data.native {
            let summary = native_summaries.entry(phase).or_default();
            summary.calls = summary.calls.saturating_add(value.calls);
            summary.reported = summary.reported.saturating_add(value.reported);
            summary.failed = summary.failed.saturating_add(value.failed);
            summary.interrupted = summary.interrupted.saturating_add(value.interrupted);
            summary.wall = summary.wall.saturating_add(value.wall);
        }
        let native: serde_json::Map<String, Value> = native_summaries
            .into_iter()
            .map(|(phase, value)| (phase.to_owned(), value.json()))
            .collect();
        json!({"requestMs":data.request.map(ms),"requestDecodeMs":data.decode.map(ms),
            "storageBeginMs":data.storage_begin.map(ms),"preSendMs":data.pre_send.map(ms),"routingMs":data.routing.map(ms),"requestEncodeMs":data.encode.map(ms),"upstreamElapsedMs":data.upstream_total.map(ms),"upstreamHeadersWaitMs":data.headers.map(ms),"upstreamFirstByteMs":data.first_byte.map(ms),
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
            WaitKind::StorageBegin => &mut data.storage_begin,
            WaitKind::Routing => &mut data.routing,
            WaitKind::Encode => &mut data.encode,
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
    scope: PhaseScope,
    started: Instant,
    returned: bool,
}

impl HookGuard {
    pub fn finish(mut self, report: &HookReport) {
        self.returned = true;
        let mut data = self.timings.lock();
        if !data.frozen {
            data.hooks
                .entry((self.phase, self.scope))
                .or_default()
                .report(report);
        }
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        let mut data = self.timings.lock();
        if data.frozen {
            return;
        }
        let hook = data.hooks.entry((self.phase, self.scope)).or_default();
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
    scope: PhaseScope,
    started: Instant,
    result: Option<(bool, bool)>,
    calls: u64,
}

impl NativeGuard {
    /// One timed batch still accounts for each submitted semantic event.
    pub fn finish_batch<T>(mut self, result: &Result<T, Failure>, calls: usize) {
        self.calls = u64::try_from(calls).unwrap_or(u64::MAX);
        self.finish(result);
    }

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
        let summary = data.native.entry((self.phase, self.scope)).or_default();
        summary.calls = summary.calls.saturating_add(self.calls);
        summary.wall = summary.wall.saturating_add(self.started.elapsed());
        summary.reported =
            summary
                .reported
                .saturating_add(if self.result.is_some() { self.calls } else { 0 });
        let (failed, interrupted) = self.result.unwrap_or((true, true));
        summary.failed = summary
            .failed
            .saturating_add(u64::from(failed).min(self.calls));
        summary.interrupted = summary
            .interrupted
            .saturating_add(u64::from(interrupted).min(self.calls));
    }
}

fn micros(value: Duration) -> u64 { u64::try_from(value.as_micros()).unwrap_or(u64::MAX) }

fn empty_phase() -> PhaseTiming {
    PhaseTiming {
        phase: String::new(),
        scope: PhaseScope::Global,
        executor: PhaseExecutor::Script,
        calls: 0,
        failed_calls: 0,
        interrupted_calls: 0,
        reported_calls: 0,
        skipped_budget_calls: 0,
        unavailable_calls: 0,
        wall_us: None,
        queue: TimingSample::default(),
        initial_call: TimingSample::default(),
        pending: TimingSample::default(),
        input_bridge: TimingSample::default(),
        output_bridge: TimingSample::default(),
    }
}

impl Sample {
    fn owned(&self) -> TimingSample {
        TimingSample {
            total_us: (self.count > 0).then(|| micros(self.elapsed)),
            samples: self.count,
        }
    }

    fn merge(&mut self, value: &Self) {
        self.elapsed = self.elapsed.saturating_add(value.elapsed);
        self.count = self.count.saturating_add(value.count);
    }
}

impl HookSummary {
    fn owned(&self, phase: &str, scope: PhaseScope) -> PhaseTiming {
        PhaseTiming {
            phase: phase.into(),
            scope,
            executor: PhaseExecutor::Script,
            calls: self.calls,
            failed_calls: self.failed,
            interrupted_calls: self.interrupted,
            reported_calls: self.reported,
            skipped_budget_calls: self.skipped_budget,
            unavailable_calls: self.unavailable,
            wall_us: (self.calls > 0).then(|| micros(self.wall)),
            queue: self.queue.owned(),
            initial_call: self.initial.owned(),
            pending: self.pending.owned(),
            input_bridge: self.input.owned(),
            output_bridge: self.output.owned(),
        }
    }

    fn merge(&mut self, value: &Self) {
        self.calls = self.calls.saturating_add(value.calls);
        self.failed = self.failed.saturating_add(value.failed);
        self.interrupted = self.interrupted.saturating_add(value.interrupted);
        self.reported = self.reported.saturating_add(value.reported);
        self.skipped_budget = self.skipped_budget.saturating_add(value.skipped_budget);
        self.unavailable = self.unavailable.saturating_add(value.unavailable);
        self.wall = self.wall.saturating_add(value.wall);
        self.queue.merge(&value.queue);
        self.initial.merge(&value.initial);
        self.pending.merge(&value.pending);
        self.input.merge(&value.input);
        self.output.merge(&value.output);
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

pub(super) fn record_performance(record: &limen_storage::FinalRecord, client_stream: bool) {
    let metrics = limen_storage::derive_metrics(record, client_stream);
    tracing::info!(
        request_id = record.request_id,
        upstream_ttft_ms = metrics.upstream_ttft_ms,
        downstream_ttft_ms = metrics.downstream_ttft_ms,
        upstream_first_text_ms = metrics.upstream_first_text_ms,
        downstream_first_text_ms = metrics.downstream_first_text_ms,
        average_output_tps = metrics.average_output_tps,
        average_output_tps_status = ?metrics.average_output_tps_status,
        generation_tps = metrics.generation_tps,
        generation_tps_status = ?metrics.generation_tps_status,
        downstream_mode = ?metrics.downstream_mode,
        storage_begin_ms = record.timings.storage_begin_us.map(|us| ms(Duration::from_micros(us))),
        pre_send_ms = record.timings.pre_send_us.map(|us| ms(Duration::from_micros(us))),
        upstream_elapsed_ms = record.timings.upstream_total_us.map(|us| ms(Duration::from_micros(us))),
        cleanup_ms = record.timings.cleanup_us.map(|us| ms(Duration::from_micros(us))),
        "request performance"
    );
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
