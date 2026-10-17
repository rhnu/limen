//! Owned, transactional state for the built-in semantic content policies.
//!
//! The caller clones this state once per source frame, then commits that candidate
//! only after all callbacks, rendering and shared request budgets have succeeded.

use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::Arc,
};

use limen_config::{BuiltinPolicy, ReplaceText, SyntheticTextTool};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{ContentError, validate_content_event};

mod decoder;
mod synthetic;

use decoder::Decoder;
use synthetic::Synthetic;

type Key = (u64, String);

/// Per-event output limits and the remaining shared state budget for this policy.
pub struct NativeContentLimits {
    pub max_output_events: usize,
    pub max_output_bytes: usize,
    pub max_retained_bytes: usize,
    pub max_retained_records: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NativeContentError {
    #[error("invalid built-in content semantic event")]
    InvalidEvent(#[from] ContentError),
    #[error("invalid built-in content UTF-8 output")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("invalid built-in content character encoding")]
    CharacterEncoding(#[from] std::num::TryFromIntError),
    #[error("invalid built-in content policy input")]
    InvalidInput,
    #[error("built-in content policy exceeds its resource limit")]
    Limit,
    #[error("built-in content policy execution interrupted")]
    Interrupted,
}

type Result<T> = std::result::Result<T, NativeContentError>;

#[derive(Clone)]
enum Policy {
    Replace {
        options: Arc<ReplaceText>,
        fallback: Arc<Vec<usize>>,
        parts: BTreeMap<Key, usize>,
    },
    Synthetic(Synthetic),
}

/// One pipeline occurrence's state. No VM values or shared request state are used.
#[derive(Clone)]
pub struct NativeContentPolicy {
    policy: Policy,
}

impl NativeContentPolicy {
    /// Compiles immutable matching tables once, under the startup budget.
    /// Request-only built-ins return `None`.
    ///
    /// # Errors
    /// Rejects oversized policy state or an expired/cancelled startup budget.
    pub fn compile(
        policy: &BuiltinPolicy,
        limits: &NativeContentLimits,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Self>> {
        if !check() {
            return Err(NativeContentError::Interrupted);
        }
        let policy = match policy {
            BuiltinPolicy::ReplaceText(options) => {
                let search = options.search.as_bytes();
                let bytes = 2048_usize
                    .saturating_add(search.len())
                    .saturating_add(options.replacement.len())
                    .saturating_add(search.len().saturating_mul(std::mem::size_of::<usize>()));
                if bytes > limits.max_retained_bytes {
                    return Err(NativeContentError::Limit);
                }
                if search.is_empty() {
                    return Err(NativeContentError::InvalidInput);
                }
                let mut fallback = vec![0; search.len()];
                let mut work = 0_usize;
                for i in 1..search.len() {
                    work += 1;
                    if work.is_multiple_of(1024) && !check() {
                        return Err(NativeContentError::Interrupted);
                    }
                    let mut prefix = fallback[i - 1];
                    while prefix > 0 && search[i] != search[prefix] {
                        work += 1;
                        if work.is_multiple_of(1024) && !check() {
                            return Err(NativeContentError::Interrupted);
                        }
                        prefix = fallback[prefix - 1];
                    }
                    if search[i] == search[prefix] {
                        prefix += 1;
                    }
                    fallback[i] = prefix;
                }
                Policy::Replace {
                    options: Arc::new(options.clone()),
                    fallback: Arc::new(fallback),
                    parts: BTreeMap::new(),
                }
            },
            BuiltinPolicy::SyntheticTextTool(options) => {
                if 2048_usize.saturating_add(options.tool_name.len()) > limits.max_retained_bytes {
                    return Err(NativeContentError::Limit);
                }
                Policy::Synthetic(Synthetic::new(options.clone()))
            },
            _ => return Ok(None),
        };
        if !check() {
            return Err(NativeContentError::Interrupted);
        }
        Ok(Some(Self { policy }))
    }

    pub fn retained_parts(&self) -> usize {
        match &self.policy {
            Policy::Replace { parts, .. } => parts.len(),
            Policy::Synthetic(state) => state.retained_parts(),
        }
    }

    pub fn retained_records(&self) -> usize { self.retained_parts() }

    /// Includes a conservative map/value overhead allowance and owned strings.
    pub fn retained_bytes(&self) -> usize {
        match &self.policy {
            Policy::Replace {
                options,
                fallback,
                parts,
            } => parts.keys().fold(
                2048_usize
                    .saturating_add(options.search.capacity())
                    .saturating_add(options.replacement.capacity())
                    .saturating_add(
                        fallback
                            .capacity()
                            .saturating_mul(std::mem::size_of::<usize>()),
                    ),
                |sum, (_, part)| sum.saturating_add(2048).saturating_add(part.capacity()),
            ),
            Policy::Synthetic(state) => state.retained_bytes(),
        }
    }

    /// Applies one validated semantic event to a candidate state.
    ///
    /// `check` is polled throughout scanning and decoding; it must enforce the
    /// caller's cancellation and wall-clock execution deadline. A failed call
    /// invalidates this candidate; only an entirely successful frame may commit.
    ///
    /// # Errors
    /// Rejects malformed input, invalid JSON tool arguments, excess retained or
    /// expanded output, and execution whose cancellation/deadline check fails.
    pub fn apply(
        &mut self,
        event: Value,
        limits: &NativeContentLimits,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<Vec<Value>> {
        validate_content_event(&event)?;
        let mut output = Output::new(limits, check);
        output.check()?;
        if self.retained_bytes() > limits.max_retained_bytes {
            return Err(NativeContentError::Limit);
        }
        match &mut self.policy {
            Policy::Replace {
                options,
                fallback,
                parts,
            } => {
                replace(options, fallback, parts, event, &mut output)?;
            },
            Policy::Synthetic(state) => state.apply(event, &mut output)?,
        }
        if self.retained_parts() > limits.max_retained_records
            || self.retained_bytes() > limits.max_retained_bytes
        {
            return Err(NativeContentError::Limit);
        }
        output.check()?;
        Ok(output.events)
    }
}

fn key(event: &Value) -> Result<Key> {
    Ok((
        event["choice"]
            .as_u64()
            .ok_or(NativeContentError::InvalidInput)?,
        event["part"]
            .as_str()
            .ok_or(NativeContentError::InvalidInput)?
            .into(),
    ))
}

fn replace(
    options: &ReplaceText,
    fallback: &[usize],
    parts: &mut BTreeMap<Key, usize>,
    mut event: Value,
    output: &mut Output<'_>,
) -> Result<()> {
    match event["type"].as_str() {
        Some("part_start") if event["kind"] == "text" => {
            let id = key(&event)?;
            if parts.len() >= options.max_parts || parts.insert(id, 0).is_some() {
                return Err(NativeContentError::Limit);
            }
        },
        Some("part_delta" | "part_end") => {
            let id = key(&event)?;
            if let Some(matched) = parts.get_mut(&id) {
                let search = options.search.as_bytes();
                if search.is_empty() {
                    return Err(NativeContentError::InvalidInput);
                }
                if event["type"] == "part_end" {
                    if *matched > 0 {
                        output.push(json!({"type":"part_delta","choice":id.0,"part":id.1,
                            "text": &options.search[..*matched]}))?;
                    }
                    parts.remove(&id);
                } else {
                    let mut text = Vec::new();
                    let mut fallback_work = 0_usize;
                    for (offset, byte) in event["text"]
                        .as_str()
                        .ok_or(NativeContentError::InvalidInput)?
                        .bytes()
                        .enumerate()
                    {
                        if offset % 1024 == 0 {
                            output.check()?;
                        }
                        while *matched > 0 && byte != search[*matched] {
                            fallback_work += 1;
                            if fallback_work.is_multiple_of(1024) {
                                output.check()?;
                            }
                            let next = fallback[*matched - 1];
                            output.append_bytes(&mut text, &search[..*matched - next])?;
                            *matched = next;
                        }
                        if byte == search[*matched] {
                            *matched += 1;
                            if *matched == search.len() {
                                output.append_bytes(&mut text, options.replacement.as_bytes())?;
                                *matched = 0;
                            }
                        } else {
                            output.append_bytes(&mut text, &[byte])?;
                        }
                    }
                    // A valid UTF-8 suffix matched against a valid search string
                    // always ends on a scalar boundary after a complete delta.
                    if options.search[..*matched].encode_utf16().count()
                        > options.max_buffered_chars
                    {
                        return Err(NativeContentError::Limit);
                    }
                    if text.is_empty() {
                        return Ok(());
                    }
                    event["text"] = Value::String(String::from_utf8(text)?);
                }
            }
        },
        Some("response_end") if !parts.is_empty() => return Err(NativeContentError::InvalidInput),
        _ => {},
    }
    output.push(event)
}

struct Output<'a> {
    events: Vec<Value>,
    bytes: usize,
    limits: &'a NativeContentLimits,
    active: &'a mut dyn FnMut() -> bool,
}

impl<'a> Output<'a> {
    fn new(limits: &'a NativeContentLimits, active: &'a mut dyn FnMut() -> bool) -> Self {
        Self {
            events: Vec::new(),
            bytes: 0,
            limits,
            active,
        }
    }

    fn check(&mut self) -> Result<()> {
        if (self.active)() {
            Ok(())
        } else {
            Err(NativeContentError::Interrupted)
        }
    }

    fn append_bytes(&self, text: &mut Vec<u8>, fragment: &[u8]) -> Result<()> {
        if text.len().saturating_add(fragment.len())
            > self.limits.max_output_bytes.saturating_sub(self.bytes)
        {
            return Err(NativeContentError::Limit);
        }
        text.extend_from_slice(fragment);
        Ok(())
    }

    fn append_char(&self, text: &mut String, ch: char) -> Result<()> {
        if text.len().saturating_add(ch.len_utf8())
            > self.limits.max_output_bytes.saturating_sub(self.bytes)
        {
            return Err(NativeContentError::Limit);
        }
        text.push(ch);
        Ok(())
    }

    fn push(&mut self, event: Value) -> Result<()> {
        self.check()?;
        let bytes = json_size(&event, false);
        if self.events.len() >= self.limits.max_output_events
            || bytes > self.limits.max_output_bytes.saturating_sub(self.bytes)
        {
            return Err(NativeContentError::Limit);
        }
        self.bytes += bytes;
        self.events.push(event);
        Ok(())
    }
}

struct SizeWriter {
    size: usize,
    utf16: bool,
}

impl Write for SizeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let size = if self.utf16 {
            std::str::from_utf8(bytes).map_or(bytes.len(), |text| text.encode_utf16().count())
        } else {
            bytes.len()
        };
        self.size = self.size.saturating_add(size);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn json_size(event: &Value, utf16: bool) -> usize {
    let mut writer = SizeWriter { size: 0, utf16 };
    // Value serialization cannot fail when the writer accepts every byte.
    if serde_json::to_writer(&mut writer, event).is_err() {
        return usize::MAX;
    }
    writer.size
}

#[cfg(test)]
mod tests;
