use std::{
    fmt::{self, Write as _},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use tracing::field::{Field, Visit};

use super::MAX_RECORD_BYTES;

#[derive(Clone)]
pub(super) struct Encoder {
    run_id: Arc<str>,
    sequence: Arc<AtomicU64>,
}

impl Encoder {
    pub fn new(run_id: &str) -> Self {
        Self {
            run_id: Arc::from(run_id),
            sequence: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn encode(&self, level: &str, fields: Fields) -> Vec<u8> {
        let mut record = fields.values;
        let event = event_name(fields.message.as_deref().unwrap_or_default());
        record.insert("event".into(), json!(event));
        record.insert("schemaVersion".into(), json!(1));
        record.insert(
            "timestamp".into(),
            json!(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        );
        record.insert("level".into(), json!(level.to_ascii_lowercase()));
        record.insert(
            "logId".into(),
            json!(format!(
                "limen-l-{}-{}",
                self.run_id,
                self.sequence.fetch_add(1, Ordering::Relaxed)
            )),
        );
        let mut bytes = serde_json::to_vec(&record).expect("log metadata is serializable JSON");
        if fields.truncated || bytes.len() >= MAX_RECORD_BYTES {
            record.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "schemaVersion" | "timestamp" | "level" | "logId" | "event" | "requestId"
                )
            });
            record.insert("truncated".into(), json!(true));
            bytes = serde_json::to_vec(&record).expect("bounded log summary is serializable JSON");
        }
        bytes.push(b'\n');
        bytes
    }
}

#[derive(Clone, Default)]
pub(super) struct Fields {
    pub values: Map<String, Value>,
    message: Option<String>,
    truncated: bool,
}

impl Fields {
    pub fn message(message: &str) -> Self {
        Self {
            message: Some(message.to_owned()),
            ..Self::default()
        }
    }

    fn put(&mut self, name: &str, value: Value) {
        if let Some(name) = field_name(name) {
            self.values.insert(name.into(), value);
        }
    }

    fn string(&mut self, name: &str, value: &str) {
        if name != "message" && field_name(name).is_none() {
            return;
        }
        if value.len() > 4096 {
            self.truncated = true;
            return;
        }
        if name == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.put(name, json!(value));
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) { self.string(field.name(), value); }

    fn record_bool(&mut self, field: &Field, value: bool) { self.put(field.name(), json!(value)); }

    fn record_u64(&mut self, field: &Field, value: u64) { self.put(field.name(), json!(value)); }

    fn record_i64(&mut self, field: &Field, value: i64) { self.put(field.name(), json!(value)); }

    fn record_f64(&mut self, field: &Field, value: f64) { self.put(field.name(), json!(value)); }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() != "message" && field_name(field.name()).is_none() {
            return;
        }
        let mut bounded = BoundedText(String::new());
        if write!(&mut bounded, "{value:?}").is_err() {
            self.truncated = true;
        } else {
            self.string(field.name(), &bounded.0);
        }
    }
}

struct BoundedText(String);

impl fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.0.len().saturating_add(value.len()) > 4096 {
            return Err(fmt::Error);
        }
        self.0.push_str(value);
        Ok(())
    }
}

const FIELDS: &[(&str, &str)] = &[
    ("request_id", "requestId"),
    ("phase", "phase"),
    ("code", "code"),
    ("model", "model"),
    ("provider", "provider"),
    ("configured_model", "configuredUpstreamModel"),
    ("final_model", "finalUpstreamModel"),
    ("step", "step"),
    ("scope", "scope"),
    ("target", "origin"),
    ("origin", "origin"),
    ("event", "observationEvent"),
    ("count", "count"),
    ("dropped", "dropped"),
    ("reason", "reason"),
    ("command", "command"),
    ("line", "line"),
    ("column", "column"),
    ("outcome", "outcome"),
    ("error_kind", "errorKind"),
    ("upstream_status", "upstreamStatus"),
    ("upstream_media", "upstreamMedia"),
    ("client_stream", "clientStream"),
    ("input_bytes", "inputBytes"),
    ("outbound_bytes", "outboundBytes"),
    ("upstream_bytes", "upstreamBytes"),
    ("output_bytes", "outputBytes"),
    ("upstream_headers_ms", "upstreamHeadersMs"),
    ("elapsed_ms", "elapsedMs"),
    ("request_ms", "requestMs"),
    ("connection_limit", "connectionLimit"),
    ("connections", "connections"),
    ("models", "models"),
    ("providers", "providers"),
    ("status", "status"),
    ("observation", "observation"),
    ("cleanup", "cleanup"),
    ("calls", "calls"),
    ("failed_calls", "failedCalls"),
    ("interrupted_calls", "interruptedCalls"),
    ("reported_calls", "reportedCalls"),
    ("wall_ms", "wallMs"),
    ("queue_ms", "queueMs"),
    ("queue_samples", "queueSamples"),
    ("initial_call_ms", "initialCallMs"),
    ("initial_call_samples", "initialCallSamples"),
    ("pending_ms", "pendingMs"),
    ("pending_samples", "pendingSamples"),
    ("input_bridge_ms", "inputBridgeMs"),
    ("input_bridge_samples", "inputBridgeSamples"),
    ("output_bridge_ms", "outputBridgeMs"),
    ("output_bridge_samples", "outputBridgeSamples"),
    ("request_decode_ms", "requestDecodeMs"),
    ("storage_begin_ms", "storageBeginMs"),
    ("storage_finish_ms", "storageFinishMs"),
    ("pre_send_ms", "preSendMs"),
    ("upstream_elapsed_ms", "upstreamElapsedMs"),
    ("cleanup_ms", "cleanupMs"),
    ("upstream_ttft_ms", "upstreamTtftMs"),
    ("downstream_ttft_ms", "downstreamTtftMs"),
    ("upstream_first_text_ms", "upstreamFirstTextMs"),
    ("downstream_first_text_ms", "downstreamFirstTextMs"),
    ("average_output_tps", "averageOutputTps"),
    ("generation_tps", "generationTps"),
    ("average_output_tps_status", "averageOutputTpsStatus"),
    ("generation_tps_status", "generationTpsStatus"),
    ("downstream_mode", "downstreamMode"),
    ("upstream_headers_wait_ms", "upstreamHeadersWaitMs"),
    ("upstream_first_byte_ms", "upstreamFirstByteMs"),
    ("upstream_first_chunk_ms", "upstreamFirstChunkMs"),
    ("upstream_read_wait_ms", "upstreamReadWaitMs"),
    ("downstream_first_event_ms", "downstreamFirstEventMs"),
    ("downstream_queue_wait_ms", "downstreamQueueWaitMs"),
    ("downstream_drain_ms", "downstreamDrainMs"),
    ("panicked", "panicked"),
    ("cancelled", "cancelled"),
];

fn field_name(name: &str) -> Option<&'static str> {
    FIELDS
        .iter()
        .find_map(|(source, target)| (*source == name).then_some(*target))
}

pub(super) fn approved_record_field(name: &str) -> bool {
    matches!(
        name,
        "schemaVersion" | "timestamp" | "level" | "logId" | "event" | "truncated"
    ) || FIELDS.iter().any(|(_, target)| *target == name)
}

const EVENTS: &[(&str, &str)] = &[
    (
        "process thread panicked; payload omitted",
        "processPanicked",
    ),
    ("configuration rejected", "configurationRejected"),
    ("configuration operation failed", "configurationFailed"),
    ("evaluating local configuration", "configurationLoading"),
    ("configuration loaded", "configurationLoaded"),
    ("configuration runtime stopped", "runtimeStopped"),
    ("HTTP listener started", "listenerStarted"),
    (
        "HTTP listener stopped; cancelling requests",
        "listenerStopping",
    ),
    (
        "HTTP connection closed with transport failure",
        "connectionFailed",
    ),
    (
        "HTTP connection deadline exceeded during response delivery or idle keepalive",
        "connectionDeadline",
    ),
    ("request finished", "requestFinished"),
    ("request rejected or cancelled", "requestRejected"),
    ("request failed", "requestFailed"),
    ("request timing", "requestTiming"),
    ("request persistence failed", "requestPersistenceFailed"),
    ("request performance", "requestPerformance"),
    (
        "request persistence completed",
        "requestPersistenceCompleted",
    ),
    ("hook statistics", "hookStatistics"),
    ("native policy statistics", "nativePolicyStatistics"),
    ("script observation", "scriptObservation"),
    ("script observations dropped", "scriptObservationsDropped"),
    ("observation not executed", "observationSkipped"),
    ("observation completed", "observationCompleted"),
    (
        "observation failed; terminal outcome retained",
        "observationFailed",
    ),
    (
        "error response policy failed; last committed response retained",
        "errorResponsePolicyFailed",
    ),
    (
        "error response policy not executed",
        "errorResponsePolicySkipped",
    ),
    (
        "request worker failed; payload omitted",
        "requestWorkerFailed",
    ),
    (
        "file logging disabled after I/O failure",
        "fileLoggingFailed",
    ),
];

fn event_name(message: &str) -> &'static str {
    EVENTS
        .iter()
        .find_map(|(source, event)| (*source == message).then_some(*event))
        .unwrap_or("diagnostic")
}

pub(super) fn approved_event(event: &str) -> bool {
    event == "diagnostic" || EVENTS.iter().any(|(_, value)| *value == event)
}

pub(super) fn text(bytes: &[u8]) -> Vec<u8> {
    let record: Value = serde_json::from_slice(bytes).expect("encoder produced valid JSON");
    let mut text = format!(
        "{} {} {}",
        record["timestamp"].as_str().unwrap_or_default(),
        record["level"].as_str().unwrap_or_default(),
        record["event"].as_str().unwrap_or_default()
    );
    if let Some(fields) = record.as_object() {
        for (key, value) in fields {
            if !matches!(key.as_str(), "timestamp" | "level" | "event") {
                write!(text, " {key}={value}").expect("formatting into String cannot fail");
            }
        }
    }
    text.push('\n');
    text.into_bytes()
}
