use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Debug)]
pub struct StorageOptions {
    pub dir: PathBuf,
    pub run_id: String,
    pub retention_days: u64,
    pub max_records: u64,
    pub max_db_bytes: u64,
    pub generation_concurrency: usize,
    pub management_concurrency: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub state: HealthState,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ready,
    Failed,
    Stopping,
}

macro_rules! text_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
            pub fn parse(value: &str) -> Option<Self> {
                match value { $($text => Some(Self::$variant)),+, _ => None }
            }
        }
    }
}

text_enum!(RecordState { Pending => "pending", Finalized => "finalized", Interrupted => "interrupted" });
text_enum!(Outcome { Success => "success", Failed => "failed", Cancelled => "cancelled" });
text_enum!(UsageQuality { Missing => "missing", Partial => "partial", Reported => "reported", Invalid => "invalid", Conflicting => "conflicting" });
text_enum!(GroupBy { None => "none", Day => "day", Model => "model", Provider => "provider" });

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRecord {
    pub request_id: String,
    pub run_id: String,
    pub started_at_ms: i64,
    pub client_stream: bool,
    pub input_bytes: u64,
}

/// Owned, frozen metadata only. Missing fields remain unknown rather than becoming zero.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalRecord {
    pub request_id: String,
    pub finished_at_ms: i64,
    pub outcome: Option<Outcome>,
    pub error_code: Option<String>,
    pub error_phase: Option<String>,
    pub error_step: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub configured_upstream_model: Option<String>,
    pub final_upstream_model: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_media: Option<String>,
    pub upstream_completed: Option<bool>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub usage_source: Option<String>,
    pub usage_quality: Option<UsageQuality>,
    pub outbound_bytes: Option<u64>,
    pub upstream_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub request_ms: Option<u64>,
    pub upstream_headers_ms: Option<u64>,
    pub first_output_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRecord {
    pub record_id: i64,
    pub request_id: String,
    pub run_id: String,
    #[serde(rename = "startedAt", serialize_with = "timestamp")]
    pub started_at_ms: i64,
    #[serde(rename = "finishedAt", serialize_with = "optional_timestamp")]
    pub finished_at_ms: Option<i64>,
    pub state: RecordState,
    pub outcome: Option<Outcome>,
    pub error_code: Option<String>,
    pub error_phase: Option<String>,
    pub error_step: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub configured_upstream_model: Option<String>,
    pub final_upstream_model: Option<String>,
    pub client_stream: bool,
    pub upstream_status: Option<u16>,
    pub upstream_media: Option<String>,
    pub upstream_completed: Option<bool>,
    #[serde(serialize_with = "optional_decimal")]
    pub input_tokens: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub output_tokens: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub total_tokens: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub cached_input_tokens: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub reasoning_output_tokens: Option<u64>,
    pub usage_source: Option<String>,
    pub usage_quality: Option<UsageQuality>,
    #[serde(serialize_with = "optional_decimal")]
    pub input_bytes: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub outbound_bytes: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub upstream_bytes: Option<u64>,
    #[serde(serialize_with = "optional_decimal")]
    pub output_bytes: Option<u64>,
    pub request_ms: Option<u64>,
    pub upstream_headers_ms: Option<u64>,
    pub first_output_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Filter {
    pub from_ms: i64,
    pub to_ms: i64,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub state: Option<RecordState>,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub started_at_ms: i64,
    pub record_id: i64,
}

#[derive(Clone, Debug)]
pub struct RequestQuery {
    pub filter: Filter,
    pub limit: usize,
    pub high_watermark: Option<i64>,
    pub before: Option<Position>,
}

#[derive(Debug)]
pub struct RequestPage {
    pub records: Vec<RequestRecord>,
    pub high_watermark: i64,
    pub next: Option<Position>,
}

#[derive(Clone, Debug)]
pub struct UsageQuery {
    pub filter: Filter,
    pub group_by: GroupBy,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSum {
    #[serde(serialize_with = "optional_decimal")]
    pub sum: Option<u128>,
    pub samples: u64,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: TokenSum,
    pub output: TokenSum,
    pub total: TokenSum,
    pub cached_input: TokenSum,
    pub reasoning_output: TokenSum,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub key: Option<String>,
    pub records: u64,
    pub states: BTreeMap<&'static str, u64>,
    pub outcomes: BTreeMap<&'static str, u64>,
    pub qualities: BTreeMap<&'static str, u64>,
    pub upstream_completed: BTreeMap<&'static str, u64>,
    pub tokens: TokenTotals,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Retention {
    pub retention_days: u64,
    pub max_records: u64,
    #[serde(rename = "oldestStartedAt", serialize_with = "optional_timestamp")]
    pub oldest_started_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    #[serde(rename = "from", serialize_with = "timestamp")]
    pub from_ms: i64,
    #[serde(rename = "to", serialize_with = "timestamp")]
    pub to_ms: i64,
    pub group_by: GroupBy,
    pub retention: Retention,
    pub groups: Vec<UsageGroup>,
}

#[expect(
    clippy::ref_option,
    reason = "serde serialize_with passes a reference to the field"
)]
fn optional_decimal<S: Serializer, T: ToString>(
    value: &Option<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .as_ref()
        .map(ToString::to_string)
        .serialize(serializer)
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde serialize_with signature"
)]
fn timestamp<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    let time = chrono::DateTime::from_timestamp_millis(*value)
        .ok_or_else(|| serde::ser::Error::custom("timestamp is out of range"))?;
    serializer.serialize_str(&time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

#[expect(clippy::ref_option, reason = "serde serialize_with signature")]
fn optional_timestamp<S: Serializer>(
    value: &Option<i64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => timestamp(value, serializer),
        None => serializer.serialize_none(),
    }
}
