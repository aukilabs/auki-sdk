//! Strict JSON parsing helpers for v1 protocol objects.
//!
//! The protocol validates raw JSON before typed deserialization so duplicate
//! object member names cannot be silently collapsed by `serde_json::Value`.

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::{cell::RefCell, collections::HashSet, fmt};

const DUPLICATE_MEMBER_NAME_SENTINEL: &str = "duplicate json member name";

/// Errors produced by strict JSON parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// JSON text is syntactically invalid or uses unsupported JSON values.
    InvalidJson(String),
    /// A JSON object repeated a member name.
    DuplicateMemberName {
        /// Repeated member name.
        name: String,
    },
    /// The parsed JSON value was not an object.
    BodyNotObject,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(err) => write!(f, "invalid json: {err}"),
            Self::DuplicateMemberName { name } => {
                write!(f, "duplicate json member name: {name}")
            }
            Self::BodyNotObject => write!(f, "json body is not an object"),
        }
    }
}

impl std::error::Error for JsonError {}

/// Parse `input` as exactly one JSON object, rejecting duplicate member names.
pub fn parse_json_object(input: &str) -> Result<Value, JsonError> {
    let duplicate_member_name = RefCell::new(None);
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = JsonValueSeed {
        duplicate_member_name: &duplicate_member_name,
    }
    .deserialize(&mut deserializer)
    .map_err(|err| map_serde_error(err, &duplicate_member_name))?;

    deserializer
        .end()
        .map_err(|err| map_serde_error(err, &duplicate_member_name))?;

    if value.is_object() {
        Ok(value)
    } else {
        Err(JsonError::BodyNotObject)
    }
}

fn map_serde_error(
    err: serde_json::Error,
    duplicate_member_name: &RefCell<Option<String>>,
) -> JsonError {
    if let Some(name) = duplicate_member_name.borrow_mut().take() {
        JsonError::DuplicateMemberName { name }
    } else {
        JsonError::InvalidJson(err.to_string())
    }
}

struct JsonValueSeed<'a> {
    duplicate_member_name: &'a RefCell<Option<String>>,
}

impl<'de> DeserializeSeed<'de> for JsonValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonValueVisitor {
            duplicate_member_name: self.duplicate_member_name,
        })
    }
}

struct JsonValueVisitor<'a> {
    duplicate_member_name: &'a RefCell<Option<String>>,
}

impl<'de> Visitor<'de> for JsonValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a strict JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite json number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        JsonValueSeed {
            duplicate_member_name: self.duplicate_member_name,
        }
        .deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element_seed(JsonValueSeed {
            duplicate_member_name: self.duplicate_member_name,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut names = HashSet::with_capacity(map.size_hint().unwrap_or(0));
        let mut values = Map::new();

        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name.clone()) {
                *self.duplicate_member_name.borrow_mut() = Some(name);
                return Err(de::Error::custom(DUPLICATE_MEMBER_NAME_SENTINEL));
            }

            let value = map.next_value_seed(JsonValueSeed {
                duplicate_member_name: self.duplicate_member_name,
            })?;
            values.insert(name, value);
        }

        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_json_object_accepts_object() {
        assert_eq!(
            parse_json_object(r#"{"type":"example.v1","n":1}"#).unwrap(),
            json!({"type": "example.v1", "n": 1})
        );
    }

    #[test]
    fn parse_json_object_rejects_non_object_root() {
        assert_eq!(parse_json_object("true"), Err(JsonError::BodyNotObject));
    }

    #[test]
    fn parse_json_object_rejects_trailing_value() {
        assert!(matches!(
            parse_json_object(r#"{"ok":true} {"extra":true}"#),
            Err(JsonError::InvalidJson(_))
        ));
    }

    #[test]
    fn parse_json_object_rejects_top_level_duplicate_member_name() {
        assert_eq!(
            parse_json_object(r#"{"a":1,"a":2}"#),
            Err(JsonError::DuplicateMemberName {
                name: "a".to_owned()
            })
        );
    }

    #[test]
    fn parse_json_object_rejects_nested_duplicate_member_name() {
        assert_eq!(
            parse_json_object(r#"{"outer":{"a":1,"a":2}}"#),
            Err(JsonError::DuplicateMemberName {
                name: "a".to_owned()
            })
        );
    }

    #[test]
    fn parse_json_object_rejects_duplicate_member_name_inside_array() {
        assert_eq!(
            parse_json_object(r#"{"items":[{"a":1,"a":2}]}"#),
            Err(JsonError::DuplicateMemberName {
                name: "a".to_owned()
            })
        );
    }
}
