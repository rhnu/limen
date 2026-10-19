use std::collections::VecDeque;

use super::{
    Arc, BTreeMap, Decoder, Key, NativeContentError, Output, Result, SyntheticTextTool, Value,
    checked_json_size, json, key, validate_content_event,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Origin {
    batch: u64,
    event: usize,
}

#[derive(Clone)]
enum ToolPart {
    Pending(Pending),
    Actual,
    Synthetic(Decoder),
}

#[derive(Clone, Default)]
struct Pending {
    name: String,
    events: Vec<Origin>,
}

#[derive(Clone, Default)]
struct Choice {
    synthetic: bool,
    actual: bool,
    next_actual_index: u64,
}

#[derive(Clone, Default)]
struct Slot {
    source: Option<Value>,
    events: Vec<Value>,
    blocked: bool,
}

#[derive(Clone)]
struct Batch {
    id: u64,
    slots: Vec<Slot>,
}

#[derive(Clone)]
pub(super) struct Synthetic {
    options: Arc<SyntheticTextTool>,
    parts: BTreeMap<Key, ToolPart>,
    choices: BTreeMap<u64, Choice>,
    batches: VecDeque<Batch>,
    next_batch: u64,
}

impl Synthetic {
    pub(super) fn new(options: SyntheticTextTool) -> Self {
        Self {
            options: Arc::new(options),
            parts: BTreeMap::new(),
            choices: BTreeMap::new(),
            batches: VecDeque::new(),
            next_batch: 0,
        }
    }

    pub(super) fn retained_parts(&self) -> usize {
        let parts = self.parts.values().fold(
            self.parts.len().saturating_add(self.choices.len()),
            |sum, part| match part {
                ToolPart::Pending(pending) => sum.saturating_add(pending.events.len()),
                ToolPart::Actual | ToolPart::Synthetic(_) => sum,
            },
        );
        self.batches.iter().fold(parts, |sum, batch| {
            batch.slots.iter().fold(sum.saturating_add(1), |sum, slot| {
                slot.source
                    .iter()
                    .chain(&slot.events)
                    .fold(sum.saturating_add(1), |sum, event| {
                        sum.saturating_add(event_records(event))
                    })
            })
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        let parts = self.parts.iter().fold(
            2048_usize
                .saturating_add(self.options.tool_name.capacity())
                .saturating_add(self.choices.len().saturating_mul(2048))
                .saturating_add(
                    self.batches
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Batch>() + 64),
                ),
            |sum, ((_, part), state)| {
                let sum = sum.saturating_add(2048).saturating_add(part.capacity());
                match state {
                    ToolPart::Pending(pending) => sum
                        .saturating_add(pending.name.capacity())
                        .saturating_add(pending.events.capacity().saturating_mul(32)),
                    ToolPart::Actual | ToolPart::Synthetic(_) => sum,
                }
            },
        );
        self.batches.iter().fold(parts, |sum, batch| {
            batch.slots.iter().fold(
                sum.saturating_add(
                    batch
                        .slots
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Slot>() + 64),
                ),
                |sum, slot| {
                    slot.source.iter().chain(&slot.events).fold(
                        sum.saturating_add(slot.events.capacity().saturating_mul(256)),
                        |sum, event| sum.saturating_add(event_bytes(event)),
                    )
                },
            )
        })
    }

    pub(super) fn apply_batch(
        &mut self,
        events: Vec<Value>,
        output: &mut Output<'_>,
    ) -> Result<Vec<Vec<Value>>> {
        let id = self.next_batch;
        self.next_batch = id.checked_add(1).ok_or(NativeContentError::Limit)?;
        let count = events.len();
        let mut slots = Vec::with_capacity(count);
        for event in events {
            output.check()?;
            validate_content_event(&event)?;
            slots.push(Slot {
                source: Some(event),
                ..Slot::default()
            });
        }
        self.batches.push_back(Batch { id, slots });
        for event in 0..count {
            output.check()?;
            self.apply_event(Origin { batch: id, event }, output)?;
        }
        let batches = self.release(output)?;
        self.check_waiting(output)?;
        Ok(batches)
    }

    fn slot(&mut self, origin: Origin) -> Result<&mut Slot> {
        let first = self
            .batches
            .front()
            .ok_or(NativeContentError::InvalidInput)?
            .id;
        let batch = usize::try_from(
            origin
                .batch
                .checked_sub(first)
                .ok_or(NativeContentError::InvalidInput)?,
        )?;
        self.batches
            .get_mut(batch)
            .and_then(|batch| batch.slots.get_mut(origin.event))
            .ok_or(NativeContentError::InvalidInput)
    }

    fn store(&mut self, origin: Origin, event: Value, output: &mut Output<'_>) -> Result<()> {
        output.push(event)?;
        self.slot(origin)?.events.append(&mut output.events);
        Ok(())
    }

    fn apply_event(&mut self, origin: Origin, output: &mut Output<'_>) -> Result<()> {
        let event = self
            .slot(origin)?
            .source
            .take()
            .ok_or(NativeContentError::InvalidInput)?;
        match event["type"].as_str() {
            Some("response_end") => {
                if !self.parts.is_empty() {
                    return Err(NativeContentError::InvalidInput);
                }
                return self.store(origin, event, output);
            },
            Some("part_start") if event["kind"] == "tool" => {
                let id = key(&event)?;
                if self.parts.len() >= self.options.max_parts || self.parts.contains_key(&id) {
                    return Err(NativeContentError::Limit);
                }
                self.parts.insert(id, ToolPart::Pending(Pending::default()));
            },
            Some("choice_end" | "part_start") => return self.store(origin, event, output),
            _ => {},
        }
        let id = key(&event)?;
        let Some(state) = self.parts.remove(&id) else {
            return self.store(origin, event, output);
        };
        let end = event["type"] == "part_end";
        let state = match state {
            ToolPart::Actual => {
                self.store(origin, event, output)?;
                ToolPart::Actual
            },
            ToolPart::Synthetic(mut decoder) => {
                if event["type"] == "part_delta"
                    && (event["tool"]["name"]
                        .as_str()
                        .is_some_and(|name| !name.is_empty())
                        || event["tool"]["type"] == "custom")
                {
                    return Err(NativeContentError::InvalidInput);
                }
                self.decode_event(&id, &mut decoder, origin, event, output)?;
                ToolPart::Synthetic(decoder)
            },
            ToolPart::Pending(pending) => {
                self.apply_pending(&id, pending, origin, event, output)?
            },
        };
        if !end {
            self.parts.insert(id, state);
        }
        Ok(())
    }

    fn apply_pending(
        &mut self,
        id: &Key,
        mut pending: Pending,
        origin: Origin,
        event: Value,
        output: &mut Output<'_>,
    ) -> Result<ToolPart> {
        let fragment = event["tool"]["name"].as_str().unwrap_or("");
        let fits = self
            .options
            .tool_name
            .strip_prefix(pending.name.as_str())
            .is_some_and(|rest| rest.starts_with(fragment));
        if fits {
            pending.name.push_str(fragment);
        }
        let exact = fits && pending.name == self.options.tool_name;
        let end = event["type"] == "part_end";
        pending.events.push(origin);
        let slot = self.slot(origin)?;
        slot.source = Some(event);
        slot.blocked = true;
        if !fits || end && !exact {
            self.record_choice(id.0, false)?;
            for at in pending.events {
                output.check()?;
                let slot = self.slot(at)?;
                slot.blocked = false;
                let event = slot.source.take().ok_or(NativeContentError::InvalidInput)?;
                self.store(at, event, output)?;
            }
            Ok(ToolPart::Actual)
        } else {
            let mut argument = false;
            for at in &pending.events {
                output.check()?;
                argument |= self.slot(*at)?.source.as_ref().is_some_and(has_argument);
            }
            if exact && (argument || end) {
                self.record_choice(id.0, true)?;
                let mut decoder = Decoder::default();
                for at in pending.events {
                    output.check()?;
                    let slot = self.slot(at)?;
                    slot.blocked = false;
                    let event = slot.source.take().ok_or(NativeContentError::InvalidInput)?;
                    if event["type"] == "part_start" {
                        if event["tool"]["type"] != "function" {
                            return Err(NativeContentError::InvalidInput);
                        }
                        self.store(
                            at,
                            json!({"type":"part_start","choice":id.0,"part":id.1,"kind":"text"}),
                            output,
                        )?;
                    } else if event["tool"]["type"] == "custom" {
                        return Err(NativeContentError::InvalidInput);
                    }
                    self.decode_event(id, &mut decoder, at, event, output)?;
                }
                Ok(ToolPart::Synthetic(decoder))
            } else {
                Ok(ToolPart::Pending(pending))
            }
        }
    }

    fn decode_event(
        &mut self,
        id: &Key,
        decoder: &mut Decoder,
        origin: Origin,
        event: Value,
        output: &mut Output<'_>,
    ) -> Result<()> {
        let previous = decoder.pending_origin();
        let end = event["type"] == "part_end";
        if event["type"] == "part_delta" {
            let text = decoder.decode(
                event["text"]
                    .as_str()
                    .ok_or(NativeContentError::InvalidInput)?,
                origin,
                output,
            )?;
            for (at, text) in text {
                self.store(
                    at,
                    json!({"type":"part_delta","choice":id.0,"part":id.1,"text":text}),
                    output,
                )?;
            }
        } else if end {
            if !decoder.complete()
                && !matches!(
                    event["finishReason"].as_str(),
                    Some("length" | "content_filter")
                )
            {
                return Err(NativeContentError::InvalidInput);
            }
            self.store(origin, event, output)?;
        }
        if let Some(at) = previous {
            self.slot(at)?.blocked = false;
        }
        if !end && let Some(at) = decoder.pending_origin() {
            self.slot(at)?.blocked = true;
        }
        Ok(())
    }

    fn record_choice(&mut self, choice: u64, synthetic: bool) -> Result<&mut Choice> {
        if !self.choices.contains_key(&choice) && self.choices.len() >= self.options.max_parts {
            return Err(NativeContentError::Limit);
        }
        let state = self.choices.entry(choice).or_default();
        if synthetic {
            state.synthetic = true;
        } else {
            state.actual = true;
        }
        Ok(state)
    }

    fn release(&mut self, output: &mut Output<'_>) -> Result<Vec<Vec<Value>>> {
        let mut batches = Vec::new();
        let mut count = 0_usize;
        let mut bytes = 0_usize;
        loop {
            output.check()?;
            let Some(batch) = self.batches.front() else {
                break;
            };
            let mut blocked = false;
            for slot in &batch.slots {
                output.check()?;
                blocked |= slot.blocked;
            }
            if blocked {
                break;
            }
            let batch = self
                .batches
                .pop_front()
                .ok_or(NativeContentError::InvalidInput)?;
            let mut events = Vec::new();
            for slot in batch.slots {
                for mut event in slot.events {
                    output.check()?;
                    self.prepare_release(&mut event)?;
                    count = count.saturating_add(1);
                    bytes = bytes.saturating_add(checked_json_size(&event, false, output.active)?);
                    if count > output.limits.max_output_events
                        || bytes > output.limits.max_output_bytes
                    {
                        return Err(NativeContentError::Limit);
                    }
                    events.push(event);
                }
            }
            batches.push(events);
        }
        if self.batches.is_empty() {
            self.batches = VecDeque::new();
        }
        Ok(batches)
    }

    fn prepare_release(&mut self, event: &mut Value) -> Result<()> {
        let choice = event["choice"].as_u64().unwrap_or(0);
        match event["type"].as_str() {
            Some("part_start") if event["kind"] == "tool" => {
                let state = self.record_choice(choice, false)?;
                if event["tool"]["type"] != "legacy" {
                    event["tool"]["index"] = json!(state.next_actual_index);
                    state.next_actual_index = state
                        .next_actual_index
                        .checked_add(1)
                        .ok_or(NativeContentError::Limit)?;
                }
            },
            Some("choice_end") => {
                if let Some(state) = self.choices.remove(&choice)
                    && state.synthetic
                    && !state.actual
                {
                    match event["finishReason"].as_str() {
                        Some("tool_calls") => event["finishReason"] = json!("stop"),
                        Some("length" | "content_filter") => {},
                        _ => return Err(NativeContentError::InvalidInput),
                    }
                }
            },
            Some("response_end") => self.choices.clear(),
            _ => {},
        }
        Ok(())
    }

    fn check_waiting(&self, output: &mut Output<'_>) -> Result<()> {
        let mut events = 0_usize;
        let mut chars = 0_usize;
        for batch in &self.batches {
            // Empty source frames also retain an external rendering context.
            if batch.slots.is_empty() {
                events = events.saturating_add(1);
            }
            for slot in &batch.slots {
                output.check()?;
                events = events.saturating_add(1);
                if slot.source.is_none() && slot.events.is_empty() {
                    chars = chars.saturating_add(1);
                }
                for event in slot.source.iter().chain(&slot.events) {
                    output.check()?;
                    chars = chars.saturating_add(checked_json_size(event, true, output.active)?);
                }
                if events > self.options.max_pending_events
                    || chars > self.options.max_buffered_chars
                {
                    return Err(NativeContentError::Limit);
                }
            }
        }
        if events > self.options.max_pending_events {
            return Err(NativeContentError::Limit);
        }
        Ok(())
    }
}

fn has_argument(event: &Value) -> bool {
    event["type"] == "part_delta" && event["text"].as_str().is_some_and(|text| !text.is_empty())
}

fn event_records(event: &Value) -> usize {
    match event {
        Value::Object(fields) => fields.values().fold(1_usize, |sum, value| {
            sum.saturating_add(1).saturating_add(event_records(value))
        }),
        _ => 1,
    }
}

fn event_string_bytes(event: &Value) -> usize {
    match event {
        Value::Object(fields) => fields.iter().fold(0_usize, |sum, (key, value)| {
            sum.saturating_add(key.capacity())
                .saturating_add(event_string_bytes(value))
        }),
        Value::String(text) => text.capacity(),
        _ => 0,
    }
}

fn event_bytes(event: &Value) -> usize {
    event_records(event)
        .saturating_mul(256)
        .saturating_add(event_string_bytes(event))
}
