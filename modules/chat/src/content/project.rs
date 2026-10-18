use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::{Binding, ContentError, Kind, Lifecycle, Part, PartKey, Target};

pub(super) fn project_body(
    state: &mut Lifecycle,
    events: &mut Vec<Value>,
    bindings: &mut BTreeMap<PartKey, Binding>,
    choice: u64,
    body: &Value,
    full: bool,
    limit: usize,
) -> Result<(), ContentError> {
    let body = body.as_object().ok_or(ContentError::InvalidSource)?;
    for &(field, ref default_kind) in super::CONTENT_FIELDS {
        let Some(value) = body.get(field) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        if let Some(text) = value.as_str() {
            if state.finished.contains_key(&choice) && text.is_empty() {
                continue;
            }
            emit(
                state,
                events,
                bindings,
                choice,
                field.into(),
                Binding {
                    target: Target::Field(field.into()),
                    original: value.clone(),
                    kind: default_kind.clone(),
                    tool_type_known: true,
                },
                text,
                None,
                limit,
            )?;
        } else if let Some(blocks) = value.as_array() {
            project_blocks(
                state,
                events,
                bindings,
                choice,
                field,
                default_kind,
                blocks,
                &[],
                field,
                limit,
            )?;
        } else {
            return Err(ContentError::InvalidSource);
        }
    }
    project_tools(
        state,
        events,
        bindings,
        choice,
        body.get("tool_calls"),
        full,
        limit,
    )?;
    if let Some(tool) = body.get("function_call").filter(|v| !v.is_null()) {
        let wrapper = json!({"function":tool});
        let (text, metadata) = tool_fields(&wrapper, "function")?;
        emit(
            state,
            events,
            bindings,
            choice,
            "function_call".into(),
            Binding {
                target: Target::Legacy,
                original: tool.clone(),
                kind: Kind::Tool {
                    index: 0,
                    kind: "legacy".into(),
                },
                tool_type_known: true,
            },
            text,
            Some(metadata),
            limit,
        )?;
    }
    Ok(())
}

fn project_tools(
    state: &mut Lifecycle,
    events: &mut Vec<Value>,
    bindings: &mut BTreeMap<PartKey, Binding>,
    choice: u64,
    tools: Option<&Value>,
    full: bool,
    limit: usize,
) -> Result<(), ContentError> {
    if let Some(tools) = tools.filter(|v| !v.is_null()) {
        let tools = tools.as_array().ok_or(ContentError::InvalidSource)?;
        let mut seen = BTreeSet::new();
        for (position, tool) in tools.iter().enumerate() {
            let object = tool.as_object().ok_or(ContentError::InvalidSource)?;
            let index = if full {
                position as u64
            } else {
                object
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or(ContentError::InvalidSource)?
            };
            if !seen.insert(index) {
                return Err(ContentError::InvalidSource);
            }
            let id = format!("tool:{index}");
            let previous = state
                .parts
                .get(&(choice, id.clone()))
                .map(|part| &part.kind);
            let kind = match object.get("type").and_then(Value::as_str) {
                Some("custom") => "custom",
                Some("function") => "function",
                Some(_) => continue,
                None if object.contains_key("custom") => "custom",
                None if object.contains_key("function") => "function",
                None => match previous {
                    Some(Kind::Tool { kind, .. }) => kind.as_str(),
                    _ => "function",
                },
            }
            .to_owned();
            let (text, metadata) = tool_fields(tool, &kind)?;
            emit(
                state,
                events,
                bindings,
                choice,
                id,
                Binding {
                    target: Target::Tool {
                        position,
                        kind: kind.clone(),
                    },
                    original: tool.clone(),
                    kind: Kind::Tool { index, kind },
                    tool_type_known: object.get("type").is_some_and(Value::is_string)
                        || object.contains_key("function")
                        || object.contains_key("custom"),
                },
                text,
                Some(metadata),
                limit,
            )?;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Projection shares lifecycle, bindings and event buffers across nested fields"
)]
fn project_blocks(
    state: &mut Lifecycle,
    events: &mut Vec<Value>,
    bindings: &mut BTreeMap<PartKey, Binding>,
    choice: u64,
    field: &str,
    default_kind: &Kind,
    blocks: &[Value],
    prefix: &[usize],
    identity_prefix: &str,
    limit: usize,
) -> Result<(), ContentError> {
    for (position, block) in blocks.iter().enumerate() {
        let object = block.as_object().ok_or(ContentError::InvalidSource)?;
        let typ = object.get("type").and_then(Value::as_str);
        let Some((kind, leaf)) = super::block_kind(typ, default_kind) else {
            continue;
        };
        let Some(value) = object.get(leaf) else {
            continue;
        };
        let mut path = prefix.to_vec();
        path.push(position);
        let identity = object
            .get("index")
            .and_then(Value::as_u64)
            .map_or_else(|| position.to_string(), |index| index.to_string());
        let id = format!("{identity_prefix}:{}:{identity}", typ.unwrap_or("block"));
        if leaf == "thinking"
            && let Some(children) = value.as_array()
        {
            project_blocks(
                state,
                events,
                bindings,
                choice,
                field,
                &Kind::Reasoning,
                children,
                &path,
                &id,
                limit,
            )?;
            continue;
        }
        let text = value.as_str().ok_or(ContentError::InvalidSource)?;
        emit(
            state,
            events,
            bindings,
            choice,
            id,
            Binding {
                target: Target::Block {
                    field: field.into(),
                    path,
                    leaf: leaf.into(),
                },
                original: block.clone(),
                kind,
                tool_type_known: true,
            },
            text,
            None,
            limit,
        )?;
    }
    Ok(())
}

fn tool_fields<'a>(
    tool: &'a Value,
    kind: &str,
) -> Result<(&'a str, Map<String, Value>), ContentError> {
    let mut metadata = Map::new();
    if let Some(id) = tool.get("id").filter(|value| !value.is_null()) {
        if !id.is_string() {
            return Err(ContentError::InvalidSource);
        }
        metadata.insert("id".into(), id.clone());
    }
    let argument = if kind == "custom" {
        "input"
    } else {
        "arguments"
    };
    let fields = tool.get(kind).filter(|value| !value.is_null());
    if let Some(fields) = fields {
        let fields = fields.as_object().ok_or(ContentError::InvalidSource)?;
        if let Some(name) = fields.get("name").filter(|value| !value.is_null()) {
            if !name.is_string() {
                return Err(ContentError::InvalidSource);
            }
            metadata.insert("name".into(), name.clone());
        }
        if let Some(text) = fields.get(argument).filter(|value| !value.is_null()) {
            return Ok((text.as_str().ok_or(ContentError::InvalidSource)?, metadata));
        }
    }
    Ok(("", metadata))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Projection shares lifecycle, bindings and event buffers across nested fields"
)]
fn emit(
    state: &mut Lifecycle,
    events: &mut Vec<Value>,
    bindings: &mut BTreeMap<PartKey, Binding>,
    choice: u64,
    id: String,
    binding: Binding,
    text: &str,
    mut metadata: Option<Map<String, Value>>,
    limit: usize,
) -> Result<(), ContentError> {
    if state.finished.contains_key(&choice) {
        return Err(ContentError::Lifecycle);
    }
    let key = (choice, id);
    let id = &key.1;
    let kind = &binding.kind;
    if bindings.contains_key(&key) {
        return Err(ContentError::InvalidSource);
    }
    if let Some(part) = state.parts.get_mut(&key) {
        if part.ended {
            return Err(ContentError::Lifecycle);
        }
        if part.kind != *kind {
            let (Kind::Tool { kind: old, .. }, Kind::Tool { kind: new, .. }) = (&part.kind, kind)
            else {
                return Err(ContentError::Lifecycle);
            };
            if part.tool_type_known || part.arguments_started || old == "legacy" || new == "legacy"
            {
                return Err(ContentError::Lifecycle);
            }
            metadata
                .get_or_insert_with(Map::new)
                .insert("type".into(), json!(new));
            part.kind = kind.clone();
        }
        part.tool_type_known |= binding.tool_type_known;
        part.arguments_started |= !text.is_empty();
    } else {
        if state.parts.len() >= limit
            || (!state.choices.contains(&choice) && state.choices.len() >= limit)
        {
            return Err(ContentError::PartLimit);
        }
        state.choices.insert(choice);
        state.parts.insert(
            key.clone(),
            Part {
                kind: kind.clone(),
                ended: false,
                arguments_started: !text.is_empty(),
                tool_type_known: binding.tool_type_known,
            },
        );
        let mut event = json!({"type":"part_start","choice":choice,"part":id,"kind":kind.name()});
        if let Kind::Tool { index, kind } = kind {
            event["tool"] = json!({"index":index,"type":kind});
        }
        events.push(event);
    }
    let mut event = json!({"type":"part_delta","choice":choice,"part":id,"text":text});
    if let Some(metadata) = metadata.filter(|metadata| !metadata.is_empty()) {
        event["tool"] = Value::Object(metadata);
    }
    events.push(event);
    bindings.insert(key, binding);
    Ok(())
}
