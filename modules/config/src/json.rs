//! JSON decoding with the same root-at-zero depth convention as `validate_json`.

use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// Maximum supported JSON depth across configuration, hooks and wire data.
pub const MAX_JSON_DEPTH: usize = 128;

/// Parse one complete JSON value without descending past the supported depth.
/// Node counts and numeric safety remain the caller's `validate_json` boundary.
///
/// # Errors
/// Returns the original JSON syntax error, or a JSON data error for excess depth.
pub fn parse_json_slice(data: &[u8], max_depth: usize) -> Result<Value, serde_json::Error> {
    let mut parser = serde_json::Deserializer::from_slice(data);
    // Serde JSON has no configurable non-default recursion limit. The seed
    // checks before descending and always clamps to our supported maximum.
    parser.disable_recursion_limit();
    let value = BoundedValue {
        depth: 0,
        max_depth: max_depth.min(MAX_JSON_DEPTH),
    }
    .deserialize(&mut parser)?;
    parser.end()?;
    Ok(value)
}

/// Parse a string using the same bounded JSON decoder as byte input.
///
/// # Errors
/// Returns the original JSON syntax error, or a JSON data error for excess depth.
pub fn parse_json_str(data: &str, max_depth: usize) -> Result<Value, serde_json::Error> {
    parse_json_slice(data.as_bytes(), max_depth)
}

#[derive(Clone, Copy)]
struct BoundedValue {
    depth: usize,
    max_depth: usize,
}

impl BoundedValue {
    fn child(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

impl<'de> DeserializeSeed<'de> for BoundedValue {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, parser: D) -> Result<Value, D::Error> {
        if self.depth > self.max_depth {
            return Err(de::Error::custom("JSON depth exceeds the configured limit"));
        }
        parser.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for BoundedValue {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value within the configured depth")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> { Ok(value.into()) }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> { Ok(value.into()) }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> { Ok(value.into()) }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> { Ok(value.into()) }

    fn visit_string<E>(self, value: String) -> Result<Value, E> { Ok(value.into()) }

    fn visit_unit<E>(self) -> Result<Value, E> { Ok(Value::Null) }

    fn visit_seq<A: SeqAccess<'de>>(self, mut source: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = source.next_element_seed(self.child())? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut source: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = source.next_key::<String>()? {
            values.insert(key, source.next_value_seed(self.child())?);
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests;
