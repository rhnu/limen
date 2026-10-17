//! Small fixture records; assertions compare explicit, stable output categories.

use std::{collections::BTreeMap, path::PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// Absence omits an assertion; a present JSON null remains a value for JSON fields.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub(super) type Headers = BTreeMap<String, Vec<String>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Case {
    pub name: String,
    pub request: Value,
    #[serde(default)]
    pub client_headers: Headers,
    pub upstream: Upstream,
    pub expect: Expected,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Upstream {
    pub status: u16,
    #[serde(default)]
    pub headers: Headers,
    #[serde(default, deserialize_with = "present")]
    pub body: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub body_file: Option<PathBuf>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Expected {
    #[serde(default, deserialize_with = "present")]
    pub sent: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub outbound: Option<Outbound>,
    #[serde(default, deserialize_with = "present")]
    pub response: Option<Response>,
    #[serde(default, deserialize_with = "present")]
    pub state: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub error: Option<Error>,
    #[serde(default, deserialize_with = "present")]
    pub observation_failed: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub stats: Option<ExpectedStats>,
    #[serde(default, deserialize_with = "present")]
    pub observations: Option<Value>,
}

impl Expected {
    pub fn has_assertion(&self) -> bool {
        self.sent.is_some()
            || self.state.is_some()
            || self.error.is_some()
            || self.observation_failed.is_some()
            || self.observations.is_some()
            || self
                .stats
                .as_ref()
                .is_some_and(ExpectedStats::has_assertion)
            || self
                .outbound
                .as_ref()
                .is_some_and(|r| r.url.is_some() || r.headers.is_some() || r.body.is_some())
            || self.response.as_ref().is_some_and(|r| {
                r.status.is_some()
                    || r.headers.is_some()
                    || r.body.is_some()
                    || r.events.is_some()
                    || r.done.is_some()
            })
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ExpectedStats {
    #[serde(default, deserialize_with = "present")]
    pub usage: Option<Value>,
    #[serde(default)]
    pub hook_calls: BTreeMap<String, u64>,
    #[serde(default)]
    pub timing_presence: BTreeMap<String, bool>,
}

impl ExpectedStats {
    pub fn has_assertion(&self) -> bool {
        self.usage.is_some() || !self.hook_calls.is_empty() || !self.timing_presence.is_empty()
    }

    pub fn valid_fields(&self) -> bool {
        self.hook_calls.keys().all(|key| {
            matches!(
                key.as_str(),
                "onRequest"
                    | "beforeSend"
                    | "afterReceive"
                    | "onResponse"
                    | "onUpstreamEvent"
                    | "onStreamEvent"
                    | "onContent"
            )
        }) && self.timing_presence.keys().all(|key| {
            matches!(
                key.as_str(),
                "requestMs"
                    | "requestDecodeMs"
                    | "upstreamHeadersWaitMs"
                    | "upstreamFirstByteMs"
                    | "upstreamFirstChunkMs"
                    | "downstreamFirstEventMs"
                    | "upstreamReadWaitMs"
                    | "downstreamQueueWaitMs"
                    | "downstreamDrainMs"
            )
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Outbound {
    #[serde(default, deserialize_with = "present")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub headers: Option<Headers>,
    #[serde(default, deserialize_with = "present")]
    pub body: Option<Value>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Response {
    #[serde(default, deserialize_with = "present")]
    pub status: Option<u16>,
    #[serde(default, deserialize_with = "present")]
    pub headers: Option<Headers>,
    #[serde(default, deserialize_with = "present")]
    pub body: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub events: Option<Vec<Value>>,
    #[serde(default, deserialize_with = "present")]
    pub done: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Error {
    pub code: String,
    pub status: u16,
    pub phase: String,
}
