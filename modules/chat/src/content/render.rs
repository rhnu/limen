use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::{Binding, ContentError, ContentFrame, Kind, Lifecycle, Part, PartKey, Target};

#[derive(Default)]
pub(super) struct Fragment {
    text: String,
    metadata: Map<String, Value>,
}

pub(super) fn apply_event(
    state: &mut Lifecycle,
    event: &Value,
    limit: usize,
    fragments: &mut BTreeMap<PartKey, Fragment>,
    order: &mut Vec<PartKey>,
    finishes: &mut BTreeMap<u64, Value>,
) -> Result<(), ContentError> {
    if state.ended {
        return Err(ContentError::Lifecycle);
    }
    let typ = event["type"].as_str().ok_or(ContentError::InvalidEvent)?;
    if typ == "response_end" {
        if state.parts.values().any(|part| !part.ended) {
            return Err(ContentError::Lifecycle);
        }
        state.ended = true;
        return Ok(());
    }
    let choice = event["choice"].as_u64().ok_or(ContentError::InvalidEvent)?;
    if state.finished.contains_key(&choice) {
        return Err(ContentError::Lifecycle);
    }
    if !state.choices.contains(&choice) && state.choices.len() >= limit {
        return Err(ContentError::PartLimit);
    }
    state.choices.insert(choice);
    if typ == "choice_end" {
        if state
            .parts
            .iter()
            .any(|((index, _), part)| *index == choice && !part.ended)
        {
            return Err(ContentError::Lifecycle);
        }
        state.finished.insert(
            choice,
            event["finishReason"]
                .as_str()
                .ok_or(ContentError::InvalidEvent)?
                .into(),
        );
        finishes.insert(choice, event["finishReason"].clone());
        return Ok(());
    }
    let id = event["part"]
        .as_str()
        .ok_or(ContentError::InvalidEvent)?
        .to_owned();
    let key = (choice, id);
    if typ == "part_start" {
        start_part(state, event, &key, limit)?;
    } else {
        let part = state.parts.get_mut(&key).ok_or(ContentError::Lifecycle)?;
        if part.ended {
            return Err(ContentError::Lifecycle);
        }
        if event.get("tool").is_some() && !matches!(part.kind, Kind::Tool { .. }) {
            return Err(ContentError::InvalidEvent);
        }
        if let Some(typ) = event.pointer("/tool/type").and_then(Value::as_str) {
            let Kind::Tool { kind, .. } = &mut part.kind else {
                return Err(ContentError::InvalidEvent);
            };
            if kind == "legacy" || (kind != typ && part.arguments_started) {
                return Err(ContentError::Lifecycle);
            }
            *kind = typ.into();
        }
        part.arguments_started |= event
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty());
        if typ == "part_end" {
            part.ended = true;
            return Ok(());
        }
    }
    if !fragments.contains_key(&key) {
        order.push(key.clone());
    }
    let fragment = fragments.entry(key).or_default();
    if let Some(text) = event.get("text").and_then(Value::as_str) {
        fragment.text.push_str(text);
    }
    if let Some(metadata) = event.get("tool").and_then(Value::as_object) {
        for name in ["id", "name"] {
            if let Some(value) = metadata.get(name).and_then(Value::as_str) {
                let entry = fragment.metadata.entry(name).or_insert_with(|| json!(""));
                if let Value::String(previous) = entry {
                    previous.push_str(value);
                }
            }
        }
    }
    Ok(())
}

fn start_part(
    state: &mut Lifecycle,
    event: &Value,
    key: &PartKey,
    limit: usize,
) -> Result<(), ContentError> {
    if state.parts.contains_key(key) {
        return Err(ContentError::Lifecycle);
    }
    if state.parts.len() >= limit {
        return Err(ContentError::PartLimit);
    }
    let kind = match event["kind"].as_str() {
        Some("text") => Kind::Text,
        Some("reasoning") => Kind::Reasoning,
        Some("refusal") => Kind::Refusal,
        Some("tool") => Kind::Tool {
            index: event["tool"]["index"]
                .as_u64()
                .ok_or(ContentError::InvalidEvent)?,
            kind: event["tool"]["type"]
                .as_str()
                .ok_or(ContentError::InvalidEvent)?
                .into(),
        },
        _ => return Err(ContentError::InvalidEvent),
    };
    if let Kind::Tool { index, kind } = &kind
        && state.parts.iter().any(|((other_choice, _), part)| {
            *other_choice == key.0
                && matches!(&part.kind, Kind::Tool { index: other, kind: other_kind }
                if other == index && ((other_kind == "legacy") == (kind == "legacy")))
        })
    {
        return Err(ContentError::Lifecycle);
    }
    state.parts.insert(
        key.clone(),
        Part {
            kind,
            ended: false,
            arguments_started: false,
            tool_type_known: false,
        },
    );
    Ok(())
}

pub(super) fn positional_metadata(value: &Value) -> bool {
    scope_positions(value)
        || value
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| {
                choices.iter().any(|choice| {
                    scope_positions(choice)
                        || ["message", "delta"]
                            .iter()
                            .any(|field| choice.get(*field).is_some_and(content_positions))
                })
            })
}

fn scope_positions(value: &Value) -> bool {
    [
        "logprobs",
        "annotations",
        "offsets",
        "content_filter_offsets",
    ]
    .iter()
    .filter_map(|field| value.get(*field))
    .any(|value| {
        !value.is_null()
            && !value.as_array().is_some_and(Vec::is_empty)
            && !value.as_object().is_some_and(Map::is_empty)
    })
}

fn content_positions(value: &Value) -> bool {
    // Unknown extension objects are opaque, even when they reuse a known field name.
    scope_positions(value)
        || [
            "content",
            "reasoning",
            "reasoning_content",
            "reasoning_details",
            "thinking",
        ]
        .iter()
        .filter_map(|field| value.get(*field))
        .any(|value| match value {
            Value::Array(parts) => parts.iter().any(content_positions),
            Value::Object(_) => content_positions(value),
            _ => false,
        })
}

pub(super) fn render_frame(
    frame: ContentFrame,
    state: &Lifecycle,
    mut fragments: BTreeMap<PartKey, Fragment>,
    order: &[PartKey],
    finishes: &BTreeMap<u64, Value>,
) -> Result<Value, ContentError> {
    let mut source = frame.source;
    let body_key = if frame.full { "message" } else { "delta" };
    let Value::Array(mut choices) = source
        .as_object_mut()
        .ok_or(ContentError::InvalidSource)?
        .remove("choices")
        .unwrap_or_else(|| json!([]))
    else {
        return Err(ContentError::InvalidSource);
    };
    ensure_choices(&mut choices, order, finishes);
    for choice in &mut choices {
        let index = choice["index"]
            .as_u64()
            .ok_or(ContentError::InvalidSource)?;
        let object = choice.as_object_mut().ok_or(ContentError::InvalidSource)?;
        if frame
            .events
            .iter()
            .any(|event| event["type"] == "choice_end" && event["choice"] == index)
        {
            object.remove("finish_reason");
        }
        if let Some(finish) = finishes.get(&index) {
            object.insert("finish_reason".into(), finish.clone());
        }
        let has_bindings = frame.bindings.keys().any(|(choice, _)| *choice == index);
        let has_fragments = order.iter().any(|(choice, _)| *choice == index);
        if !has_bindings && !has_fragments {
            continue;
        }
        let body = object
            .entry(body_key)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(ContentError::InvalidSource)?;
        let mut array_edits: BTreeMap<(String, Vec<usize>), BTreeMap<usize, Option<Value>>> =
            BTreeMap::new();
        for ((choice, id), binding) in &frame.bindings {
            if *choice != index {
                continue;
            }
            let key = (*choice, id.clone());
            let replacement = if let Some(fragment) = fragments.get(&key) {
                let kind = &state.parts.get(&key).ok_or(ContentError::Lifecycle)?.kind;
                bound_value(binding, kind, fragment, frame.full)?
            } else {
                None
            };
            match &binding.target {
                Target::Field(field) => {
                    body.remove(field);
                    if let Some(value) = replacement {
                        body.insert(field.clone(), value);
                        fragments.remove(&key);
                    }
                },
                Target::Block { field, path, .. } => {
                    if replacement.is_some() {
                        fragments.remove(&key);
                    }
                    let (position, parent) =
                        path.split_last().ok_or(ContentError::InvalidSource)?;
                    array_edits
                        .entry((field.clone(), parent.to_vec()))
                        .or_default()
                        .insert(*position, replacement);
                },
                Target::Tool { position, .. } => {
                    if replacement.is_some() {
                        fragments.remove(&key);
                    }
                    array_edits
                        .entry(("tool_calls".into(), Vec::new()))
                        .or_default()
                        .insert(*position, replacement);
                },
                Target::Legacy => {
                    body.remove("function_call");
                    if let Some(value) = replacement {
                        body.insert("function_call".into(), value);
                        fragments.remove(&key);
                    }
                },
            }
        }
        patch_arrays(body, array_edits)?;
        for key in order {
            if key.0 != index {
                continue;
            }
            let Some(fragment) = fragments.remove(key) else {
                continue;
            };
            let kind = &state.parts.get(key).ok_or(ContentError::Lifecycle)?.kind;
            append_synthetic(body, kind, &fragment, frame.full)?;
        }
    }
    source["choices"] = Value::Array(choices);
    Ok(source)
}

type ArrayEdits = BTreeMap<(String, Vec<usize>), BTreeMap<usize, Option<Value>>>;

fn ensure_choices(choices: &mut Vec<Value>, order: &[PartKey], finishes: &BTreeMap<u64, Value>) {
    let mut needed = BTreeSet::new();
    needed.extend(order.iter().map(|(choice, _)| *choice));
    needed.extend(finishes.keys().copied());
    for choice in needed {
        if !choices
            .iter()
            .any(|value| value["index"].as_u64() == Some(choice))
        {
            choices.push(json!({"index":choice}));
        }
    }
}

fn patch_arrays(body: &mut Map<String, Value>, edits: ArrayEdits) -> Result<(), ContentError> {
    let mut edits: Vec<_> = edits.into_iter().collect();
    // Child replacements run before any parent array removes an earlier sibling.
    edits.sort_by_key(|((_, path), _)| std::cmp::Reverse(path.len()));
    for ((field, path), edits) in edits {
        let mut target = body.get_mut(&field).ok_or(ContentError::InvalidSource)?;
        for position in &path {
            target = target
                .get_mut(*position)
                .and_then(|block| block.get_mut("thinking"))
                .ok_or(ContentError::InvalidSource)?;
        }
        let array = target.as_array_mut().ok_or(ContentError::InvalidSource)?;
        *array = array
            .iter()
            .enumerate()
            .filter_map(|(position, value)| {
                edits
                    .get(&position)
                    .cloned()
                    .unwrap_or_else(|| Some(value.clone()))
            })
            .collect();
        if array.is_empty() && path.is_empty() {
            body.remove(&field);
        }
    }
    Ok(())
}

fn bound_value(
    binding: &Binding,
    kind: &Kind,
    fragment: &Fragment,
    full: bool,
) -> Result<Option<Value>, ContentError> {
    let compatible = match (&binding.kind, kind) {
        (Kind::Tool { kind: left, .. }, Kind::Tool { kind: right, .. }) => left == right,
        (left, right) => left == right,
    };
    if !compatible {
        return Ok(None);
    }
    let mut value = binding.original.clone();
    match &binding.target {
        Target::Field(field) if field_kind(field) == *kind => Ok(Some(json!(fragment.text))),
        Target::Block { leaf, .. } if !matches!(kind, Kind::Tool { .. }) => {
            value[leaf] = json!(fragment.text);
            Ok(Some(value))
        },
        Target::Tool {
            kind: source_kind, ..
        } if matches!(kind, Kind::Tool { kind, .. } if kind == source_kind) => {
            let Kind::Tool { index, kind } = kind else {
                return Err(ContentError::Lifecycle);
            };
            rewrite_tool(&mut value, *index, kind, fragment, full)?;
            Ok(Some(value))
        },
        Target::Legacy if matches!(kind, Kind::Tool { kind, .. } if kind == "legacy") => {
            let object = value.as_object_mut().ok_or(ContentError::InvalidSource)?;
            object.remove("name");
            object.insert("arguments".into(), json!(fragment.text));
            if let Some(name) = fragment.metadata.get("name") {
                object.insert("name".into(), name.clone());
            }
            Ok(Some(value))
        },
        _ => Ok(None),
    }
}

fn field_kind(field: &str) -> Kind {
    match field {
        "content" => Kind::Text,
        "refusal" => Kind::Refusal,
        _ => Kind::Reasoning,
    }
}

fn rewrite_tool(
    value: &mut Value,
    index: u64,
    kind: &str,
    fragment: &Fragment,
    full: bool,
) -> Result<(), ContentError> {
    let object = value.as_object_mut().ok_or(ContentError::InvalidSource)?;
    object.remove("id");
    if full {
        object.remove("index");
    } else {
        object.insert("index".into(), json!(index));
    }
    // Type is stable metadata: retain its original presence for native continuations.
    if let Some(id) = fragment.metadata.get("id") {
        object.insert("id".into(), id.clone());
    }
    let fields = object
        .entry(kind)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(ContentError::InvalidSource)?;
    fields.remove("name");
    fields.insert(
        if kind == "custom" {
            "input"
        } else {
            "arguments"
        }
        .into(),
        json!(fragment.text),
    );
    if let Some(name) = fragment.metadata.get("name") {
        fields.insert("name".into(), name.clone());
    }
    Ok(())
}

fn append_synthetic(
    body: &mut Map<String, Value>,
    kind: &Kind,
    fragment: &Fragment,
    full: bool,
) -> Result<(), ContentError> {
    match kind {
        Kind::Tool { kind, .. } if kind == "legacy" => {
            if body.contains_key("function_call") {
                return Err(ContentError::Lifecycle);
            }
            let mut value = json!({"arguments":fragment.text});
            if let Some(name) = fragment.metadata.get("name") {
                value["name"] = name.clone();
            }
            body.insert("function_call".into(), value);
        },
        Kind::Tool { index, kind } => {
            let mut value = json!({"type":kind});
            rewrite_tool(&mut value, *index, kind, fragment, full)?;
            body.entry("tool_calls")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or(ContentError::InvalidSource)?
                .push(value);
        },
        _ => {
            let field = match kind {
                Kind::Text => "content",
                Kind::Refusal => "refusal",
                _ => "reasoning_content",
            };
            let entry = body.entry(field).or_insert_with(|| json!(""));
            if entry.is_null() {
                *entry = json!("");
            }
            match entry {
                Value::String(text) => text.push_str(&fragment.text),
                Value::Array(blocks) if matches!(kind, Kind::Text) => {
                    blocks.push(json!({"type":"text","text":fragment.text}));
                },
                _ => return Err(ContentError::InvalidSource),
            }
        },
    }
    Ok(())
}
