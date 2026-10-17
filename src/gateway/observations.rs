use limen_script::ObservationBatch;
use serde_json::{Value, json};

const MAX_SAFE_COUNT: u64 = 9_007_199_254_740_991;

#[derive(Clone, Default)]
pub(super) struct Observations {
    limit: usize,
    accepted: usize,
    dropped: u64,
    entries: Vec<Entry>,
}

#[derive(Clone)]
struct Entry {
    event: u8,
    phase: &'static str,
    step: Option<String>,
    count: u64,
}

impl Observations {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    /// Only the gateway's successful stage commit can publish a private VM batch.
    pub fn commit(&mut self, batch: ObservationBatch, phase: &'static str, step: Option<&str>) {
        self.dropped = self
            .dropped
            .saturating_add(batch.dropped)
            .min(MAX_SAFE_COUNT);
        for event in batch.events {
            if self.accepted >= self.limit {
                self.dropped = self.dropped.saturating_add(1).min(MAX_SAFE_COUNT);
                continue;
            }
            self.accepted += 1;
            if let Some(entry) = self.entries.iter_mut().find(|entry| {
                entry.event == event && entry.phase == phase && entry.step.as_deref() == step
            }) {
                entry.count += 1;
            } else {
                self.entries.push(Entry {
                    event,
                    phase,
                    step: step.map(str::to_owned),
                    count: 1,
                });
            }
        }
    }

    pub fn snapshot(&self, names: &[String]) -> Value {
        let events: Vec<Value> = self.entries.iter().map(|entry| {
            json!({"event":names.get(usize::from(entry.event)),"phase":entry.phase,"step":entry.step,"count":entry.count})
        }).collect();
        json!({"events":events,"dropped":self.dropped})
    }

    pub fn record(&self, request_id: &str, names: &[String]) {
        for entry in &self.entries {
            tracing::info!(
                request_id,
                event = names.get(usize::from(entry.event)).map(String::as_str),
                phase = entry.phase,
                step = entry.step,
                count = entry.count,
                "script observation"
            );
        }
        if self.dropped != 0 {
            tracing::info!(
                request_id,
                dropped = self.dropped,
                "script observations dropped"
            );
        }
    }
}

#[cfg(test)]
mod tests;
