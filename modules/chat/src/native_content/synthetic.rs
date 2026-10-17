use super::{
    Arc, BTreeMap, Decoder, Key, NativeContentError, Output, Result, SyntheticTextTool, Value,
    json, json_size, key,
};

#[derive(Clone)]
pub(super) enum ToolPart {
    Pending(Pending),
    Actual,
    Synthetic(Decoder),
}

#[derive(Clone, Default)]
pub(super) struct Pending {
    name: String,
    events: Vec<Value>,
    chars: usize,
}

#[derive(Clone, Default)]
struct Choice {
    synthetic: bool,
    actual: bool,
    next_actual_index: u64,
}

#[derive(Clone)]
pub(super) struct Synthetic {
    options: Arc<SyntheticTextTool>,
    parts: BTreeMap<Key, ToolPart>,
    choices: BTreeMap<u64, Choice>,
}

impl Synthetic {
    pub(super) fn new(options: SyntheticTextTool) -> Self {
        Self {
            options: Arc::new(options),
            parts: BTreeMap::new(),
            choices: BTreeMap::new(),
        }
    }

    pub(super) fn retained_parts(&self) -> usize {
        self.parts.values().fold(
            self.parts.len().saturating_add(self.choices.len()),
            |sum, part| {
                if let ToolPart::Pending(Pending { events, .. }) = part {
                    // Validated events have at most two object layers; charge every
                    // field and its value as records in the shared JSON-node budget.
                    sum.saturating_add(events.iter().map(event_records).sum::<usize>())
                } else {
                    sum
                }
            },
        )
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.parts.iter().fold(
            2048_usize
                .saturating_add(self.options.tool_name.capacity())
                .saturating_add(self.choices.len().saturating_mul(2048)),
            |sum, ((_, part), state)| {
                let sum = sum.saturating_add(2048).saturating_add(part.capacity());
                if let ToolPart::Pending(Pending { name, events, .. }) = state {
                    events.iter().fold(
                        sum.saturating_add(name.capacity()).saturating_add(
                            events
                                .capacity()
                                .saturating_mul(std::mem::size_of::<Value>()),
                        ),
                        |sum, event| {
                            sum.saturating_add(event_records(event).saturating_mul(256))
                                .saturating_add(event_string_bytes(event))
                        },
                    )
                } else {
                    sum
                }
            },
        )
    }

    fn record_choice(&mut self, choice: u64, synthetic: bool) -> Result<&mut Choice> {
        if !self.choices.contains_key(&choice) && self.choices.len() >= self.options.max_parts {
            return Err(NativeContentError::Limit);
        }
        let entry = self.choices.entry(choice).or_default();
        if synthetic {
            entry.synthetic = true;
        } else {
            entry.actual = true;
        }
        Ok(entry)
    }

    pub(super) fn apply(&mut self, mut event: Value, output: &mut Output<'_>) -> Result<()> {
        match event["type"].as_str() {
            Some("response_end") => {
                if !self.parts.is_empty() {
                    return Err(NativeContentError::InvalidInput);
                }
                self.choices.clear();
                return output.push(event);
            },
            Some("choice_end") => {
                let choice = event["choice"]
                    .as_u64()
                    .ok_or(NativeContentError::InvalidInput)?;
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
                return output.push(event);
            },
            Some("part_start") if event["kind"] == "tool" => {
                let id = key(&event)?;
                if self.parts.len() >= self.options.max_parts || self.parts.contains_key(&id) {
                    return Err(NativeContentError::Limit);
                }
                self.parts.insert(id, ToolPart::Pending(Pending::default()));
            },
            Some("part_start") => return output.push(event),
            _ => {},
        }
        let id = key(&event)?;
        let Some(state) = self.parts.remove(&id) else {
            return output.push(event);
        };
        let end = event["type"] == "part_end";
        let state = match state {
            ToolPart::Actual => {
                output.push(event)?;
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
                decode_event(&id, &mut decoder, event, output)?;
                ToolPart::Synthetic(decoder)
            },
            ToolPart::Pending(pending) => self.apply_pending(&id, pending, event, output)?,
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
        let argument = has_argument(&event) || pending.events.iter().any(has_argument);
        let end = event["type"] == "part_end";
        // The current event is already owned and bounded by the source frame.
        // Only undecided metadata is retained across callbacks.
        if !fits || end && !exact {
            pending.events.push(event);
            self.promote_actual(id.0, pending.events, output)?;
            Ok(ToolPart::Actual)
        } else if exact && (argument || end) {
            pending.events.push(event);
            let decoder = self.promote_synthetic(id, pending.events, output)?;
            Ok(ToolPart::Synthetic(decoder))
        } else {
            pending.chars = pending.chars.saturating_add(json_size(&event, true));
            if pending.events.len() >= self.options.max_pending_events
                || pending.chars > self.options.max_buffered_chars
            {
                return Err(NativeContentError::Limit);
            }
            pending.events.push(event);
            Ok(ToolPart::Pending(pending))
        }
    }

    fn promote_actual(
        &mut self,
        choice: u64,
        mut events: Vec<Value>,
        output: &mut Output<'_>,
    ) -> Result<()> {
        let choice = self.record_choice(choice, false)?;
        let start = events.first_mut().ok_or(NativeContentError::InvalidInput)?;
        if start["type"] != "part_start" {
            return Err(NativeContentError::InvalidInput);
        }
        if start["tool"]["type"] != "legacy" {
            start["tool"]["index"] = json!(choice.next_actual_index);
            choice.next_actual_index = choice
                .next_actual_index
                .checked_add(1)
                .ok_or(NativeContentError::Limit)?;
        }
        for event in events {
            output.push(event)?;
        }
        Ok(())
    }

    fn promote_synthetic(
        &mut self,
        id: &Key,
        events: Vec<Value>,
        output: &mut Output<'_>,
    ) -> Result<Decoder> {
        if events.iter().any(|item| {
            item["type"] == "part_start" && item["tool"]["type"] != "function"
                || item["type"] == "part_delta" && item["tool"]["type"] == "custom"
        }) {
            return Err(NativeContentError::InvalidInput);
        }
        self.record_choice(id.0, true)?;
        output.push(json!({"type":"part_start","choice":id.0,"part":id.1,"kind":"text"}))?;
        let mut decoder = Decoder::default();
        for event in events {
            decode_event(id, &mut decoder, event, output)?;
        }
        Ok(decoder)
    }
}

fn has_argument(event: &Value) -> bool {
    event["type"] == "part_delta" && event["text"].as_str().is_some_and(|text| !text.is_empty())
}

fn decode_event(
    id: &Key,
    decoder: &mut Decoder,
    event: Value,
    output: &mut Output<'_>,
) -> Result<()> {
    if event["type"] == "part_delta" {
        let text = decoder.decode(
            event["text"]
                .as_str()
                .ok_or(NativeContentError::InvalidInput)?,
            output,
        )?;
        push_text(id, text, output)?;
    } else if event["type"] == "part_end" {
        finish(decoder, event, output)?;
    }
    Ok(())
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

fn push_text(id: &Key, text: String, output: &mut Output<'_>) -> Result<()> {
    if !text.is_empty() {
        let mut event = json!({"type":"part_delta","choice":id.0,"part":id.1});
        event["text"] = Value::String(text);
        output.push(event)?;
    }
    Ok(())
}

fn finish(decoder: &Decoder, event: Value, output: &mut Output<'_>) -> Result<()> {
    if !decoder.complete()
        && !matches!(
            event["finishReason"].as_str(),
            Some("length" | "content_filter")
        )
    {
        return Err(NativeContentError::InvalidInput);
    }
    output.push(event)
}
