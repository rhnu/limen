//! A transport-independent view of generated Chat content, with source templates.
//!
//! Projection never aggregates arguments or retains the event history. Only bounded
//! part identities and their lifecycle survive between input payloads. Rendering
//! edits understood leaves in the original payload, so opaque same-format data
//! does not have to pass through the cross-format conversion whitelist.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use thiserror::Error;

mod project;
mod render;

use project::project_body;
use render::{apply_event, positional_metadata, render_frame};

type PartKey = (u64, String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("invalid Chat semantic content event")]
    InvalidEvent,
    #[error("invalid Chat semantic source")]
    InvalidSource,
    #[error("Chat semantic content exceeds its part limit")]
    PartLimit,
    #[error("invalid Chat semantic content lifecycle")]
    Lifecycle,
    #[error("content edits conflict with positional metadata")]
    PositionalMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    Reasoning,
    Refusal,
    Tool { index: u64, kind: String },
}

impl Kind {
    fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Reasoning => "reasoning",
            Self::Refusal => "refusal",
            Self::Tool { .. } => "tool",
        }
    }
}

#[derive(Clone)]
struct Part {
    kind: Kind,
    ended: bool,
    arguments_started: bool,
    tool_type_known: bool,
}

#[derive(Clone, Default)]
struct Lifecycle {
    parts: BTreeMap<PartKey, Part>,
    choices: BTreeSet<u64>,
    finished: BTreeMap<u64, String>,
    ended: bool,
}

#[derive(Clone)]
enum Target {
    Field(String),
    Block {
        field: String,
        path: Vec<usize>,
        leaf: String,
    },
    Tool {
        position: usize,
        kind: String,
    },
    Legacy,
}

#[derive(Clone)]
struct Binding {
    target: Target,
    original: Value,
    kind: Kind,
    tool_type_known: bool,
}

/// One payload's semantic events and private source-layout bindings.
#[derive(Clone)]
pub struct ContentFrame {
    source: Value,
    events: Vec<Value>,
    bindings: BTreeMap<PartKey, Binding>,
    full: bool,
    terminal: bool,
}

impl ContentFrame {
    pub fn events(&self) -> &[Value] { &self.events }
}

/// Projects source JSON/chunks and applies transformed events to source templates.
#[derive(Clone)]
pub struct ContentProjector {
    source: Lifecycle,
    output: Lifecycle,
    max_parts: usize,
    edited: bool,
    positional: bool,
}

impl ContentProjector {
    pub fn new(max_parts: usize) -> Self {
        Self {
            source: Lifecycle::default(),
            output: Lifecycle::default(),
            max_parts,
            edited: false,
            positional: false,
        }
    }

    /// Counts retained part, choice and finish records in both lifecycles.
    /// Callers sharing a request budget sum this across all stage projectors.
    pub fn retained_parts(&self) -> usize {
        [&self.source, &self.output]
            .into_iter()
            .fold(0, |sum, state| {
                sum.saturating_add(state.parts.len())
                    .saturating_add(state.choices.len())
                    .saturating_add(state.finished.len())
            })
    }

    /// Conservatively accounts retained identifiers, subtypes and finish reasons.
    /// The fixed per-record allowance includes map-node and value overhead; this
    /// is a request-budget charge rather than an allocator measurement. Content
    /// deltas and tool arguments are never retained between source payloads.
    pub fn retained_bytes(&self) -> usize {
        const RECORD_ALLOWANCE: usize = 2048;
        let strings = [&self.source, &self.output]
            .into_iter()
            .fold(0_usize, |sum, state| {
                let parts = state.parts.iter().fold(sum, |sum, ((_, id), part)| {
                    let subtype = match &part.kind {
                        Kind::Tool { kind, .. } => kind.capacity(),
                        _ => 0,
                    };
                    sum.saturating_add(id.capacity()).saturating_add(subtype)
                });
                state
                    .finished
                    .values()
                    .fold(parts, |sum, reason| sum.saturating_add(reason.capacity()))
            });
        std::mem::size_of::<Self>()
            .saturating_add(self.retained_parts().saturating_mul(RECORD_ALLOWANCE))
            .saturating_add(strings)
    }

    /// Projects a complete JSON object without imposing a conversion whitelist.
    ///
    /// # Errors
    /// Rejects malformed understood content, invalid lifecycles or excess parts.
    pub fn project_json(&mut self, source: Value) -> Result<ContentFrame, ContentError> {
        self.project(source, true)
    }

    /// Projects one native Chat chunk. This does not validate SSE termination.
    ///
    /// # Errors
    /// Rejects malformed understood content, invalid lifecycles or excess parts.
    pub fn project_chunk(&mut self, source: Value) -> Result<ContentFrame, ContentError> {
        self.project(source, false)
    }

    fn project(&mut self, source: Value, full: bool) -> Result<ContentFrame, ContentError> {
        if !source.is_object() || self.source.ended {
            return Err(ContentError::InvalidSource);
        }
        let mut state = self.source.clone();
        let mut frame = ContentFrame {
            source,
            events: Vec::new(),
            bindings: BTreeMap::new(),
            full,
            terminal: false,
        };
        if let Some(choices) = frame.source.get("choices") {
            let choices = choices.as_array().ok_or(ContentError::InvalidSource)?;
            let mut seen = BTreeSet::new();
            for choice in choices {
                let index = choice
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or(ContentError::InvalidSource)?;
                if !seen.insert(index) {
                    return Err(ContentError::InvalidSource);
                }
                let body = choice.get(if full { "message" } else { "delta" });
                if let Some(body) = body {
                    project_body(
                        &mut state,
                        &mut frame.events,
                        &mut frame.bindings,
                        index,
                        body,
                        full,
                        self.max_parts,
                    )?;
                }
                let finish = choice
                    .get("finish_reason")
                    .filter(|v| !v.is_null())
                    .map(|value| value.as_str().ok_or(ContentError::InvalidSource))
                    .transpose()?;
                if let Some(previous) = state.finished.get(&index)
                    && finish.is_some_and(|finish| finish != previous)
                {
                    return Err(ContentError::Lifecycle);
                }
                if full || finish.is_some() {
                    if !state.choices.contains(&index) && state.choices.len() >= self.max_parts {
                        return Err(ContentError::PartLimit);
                    }
                    state.choices.insert(index);
                    if !state.finished.contains_key(&index) {
                        for ((part_choice, id), part) in &mut state.parts {
                            if *part_choice == index && !part.ended {
                                let mut event = json!({"type":"part_end","choice":index,"part":id});
                                if let Some(finish) = finish {
                                    event["finishReason"] = json!(finish);
                                }
                                frame.events.push(event);
                                part.ended = true;
                            }
                        }
                        if let Some(finish) = finish {
                            state.finished.insert(index, finish.into());
                            frame.events.push(
                                json!({"type":"choice_end","choice":index,"finishReason":finish}),
                            );
                        }
                    }
                }
            }
        }
        self.source = state;
        Ok(frame)
    }

    /// Creates the final semantic notification after the caller validates upstream
    /// success. Neither EOF nor a source finish reason may call this implicitly.
    ///
    /// # Errors
    /// Rejects duplicate termination or unfinished projected parts.
    pub fn project_end(&mut self) -> Result<ContentFrame, ContentError> {
        if self.source.ended || self.source.parts.values().any(|part| !part.ended) {
            return Err(ContentError::Lifecycle);
        }
        self.source.ended = true;
        Ok(ContentFrame {
            source: json!({"choices":[]}),
            events: vec![json!({"type":"response_end"})],
            bindings: BTreeMap::new(),
            full: false,
            terminal: true,
        })
    }

    /// Renders transformed events while retaining unmodified source fields.
    ///
    /// # Errors
    /// Rejects malformed policy output, invalid output lifecycles and edits that
    /// would invalidate positional metadata. State commits only after rendering.
    pub fn render(
        &mut self,
        frame: ContentFrame,
        events: Vec<Value>,
    ) -> Result<Value, ContentError> {
        if frame.terminal {
            return Err(ContentError::Lifecycle);
        }
        self.render_inner(frame, events)
    }

    /// Renders the successful response-end hook. A no-op produces no Chat chunk.
    ///
    /// # Errors
    /// Applies the same strict validation as `render`; finished choices cannot
    /// be reopened to flush buffered text.
    pub fn render_end(
        &mut self,
        frame: ContentFrame,
        events: Vec<Value>,
    ) -> Result<Option<Value>, ContentError> {
        if !frame.terminal || events.iter().any(|event| event["type"] != "response_end") {
            return Err(ContentError::Lifecycle);
        }
        let value = self.render_inner(frame, events)?;
        if value["choices"].as_array().is_some_and(Vec::is_empty) {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn render_inner(
        &mut self,
        frame: ContentFrame,
        events: Vec<Value>,
    ) -> Result<Value, ContentError> {
        let mut state = self.output.clone();
        let mut fragments = BTreeMap::new();
        let mut order = Vec::new();
        let mut finishes = BTreeMap::new();
        let changed = frame.events != events;
        for event in events {
            validate_content_event(&event)?;
            if event["type"] == "response_end" && !frame.terminal {
                return Err(ContentError::Lifecycle);
            }
            apply_event(
                &mut state,
                &event,
                self.max_parts,
                &mut fragments,
                &mut order,
                &mut finishes,
            )?;
        }
        if frame.terminal && !state.ended {
            return Err(ContentError::Lifecycle);
        }
        let positional = positional_metadata(&frame.source);
        if (self.edited || changed) && positional || changed && self.positional {
            return Err(ContentError::PositionalMetadata);
        }
        let mut result = if changed {
            render_frame(frame, &state, fragments, &order, &finishes)?
        } else {
            frame.source
        };
        // Trailing source metadata may repeat the upstream finish marker without
        // producing another semantic event. Keep the already committed output
        // reason, while leaving the independently tracked upstream fact intact.
        if let Some(choices) = result.get_mut("choices").and_then(Value::as_array_mut) {
            for choice in choices {
                let Some(index) = choice.get("index").and_then(Value::as_u64) else {
                    continue;
                };
                if choice.get("finish_reason").is_some_and(Value::is_string) {
                    if let Some(reason) = state.finished.get(&index) {
                        choice["finish_reason"] = json!(reason);
                    } else if let Some(choice) = choice.as_object_mut() {
                        // A repeated raw marker cannot restore a semantic end
                        // deliberately removed by the policy.
                        choice.remove("finish_reason");
                    }
                }
            }
        }
        self.output = state;
        self.edited |= changed;
        self.positional |= positional;
        Ok(result)
    }
}

/// Validates the public event object independently of transport and hook state.
///
/// # Errors
/// Rejects unknown event types/keys, malformed identities and non-string deltas.
pub fn validate_content_event(event: &Value) -> Result<(), ContentError> {
    let event = event.as_object().ok_or(ContentError::InvalidEvent)?;
    let typ = event
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ContentError::InvalidEvent)?;
    let keys: &[&str] = match typ {
        "part_start" => &["type", "choice", "part", "kind", "tool"],
        "part_delta" => &["type", "choice", "part", "text", "tool"],
        "part_end" => &["type", "choice", "part", "finishReason"],
        "choice_end" => &["type", "choice", "finishReason"],
        "response_end" => &["type"],
        _ => return Err(ContentError::InvalidEvent),
    };
    if event.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(ContentError::InvalidEvent);
    }
    if typ == "response_end" {
        return Ok(());
    }
    if !event.get("choice").is_some_and(Value::is_u64) {
        return Err(ContentError::InvalidEvent);
    }
    if typ == "choice_end" {
        return if event.get("finishReason").is_some_and(Value::is_string) {
            Ok(())
        } else {
            Err(ContentError::InvalidEvent)
        };
    }
    if !event
        .get("part")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty() && id.len() <= 8192)
    {
        return Err(ContentError::InvalidEvent);
    }
    if typ == "part_delta" && !event.get("text").is_some_and(Value::is_string) {
        return Err(ContentError::InvalidEvent);
    }
    if typ == "part_end"
        && event
            .get("finishReason")
            .is_some_and(|value| !value.is_string())
    {
        return Err(ContentError::InvalidEvent);
    }
    if typ == "part_start" {
        let kind = event
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(ContentError::InvalidEvent)?;
        if !["text", "reasoning", "refusal", "tool"].contains(&kind) {
            return Err(ContentError::InvalidEvent);
        }
        if (kind == "tool") != event.contains_key("tool") {
            return Err(ContentError::InvalidEvent);
        }
    }
    if let Some(tool) = event.get("tool") {
        let tool = tool.as_object().ok_or(ContentError::InvalidEvent)?;
        let allowed: &[&str] = if typ == "part_start" {
            &["index", "type", "id", "name"]
        } else {
            &["type", "id", "name"]
        };
        if tool.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ContentError::InvalidEvent);
        }
        for key in ["id", "name"] {
            if tool.get(key).is_some_and(|value| !value.is_string()) {
                return Err(ContentError::InvalidEvent);
            }
        }
        if typ == "part_start"
            && (!tool.get("index").is_some_and(Value::is_u64)
                || !tool
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| ["function", "custom", "legacy"].contains(&kind)))
        {
            return Err(ContentError::InvalidEvent);
        }
        if typ == "part_delta"
            && tool.get("type").is_some_and(|value| {
                !value
                    .as_str()
                    .is_some_and(|kind| ["function", "custom"].contains(&kind))
            })
        {
            return Err(ContentError::InvalidEvent);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
