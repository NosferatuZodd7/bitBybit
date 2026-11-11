//! Strict JSON for the broker wire: duplicate keys rejected, depth ≤ 32, and typed binding that rejects any
//! property the target type does not know (also for types without `deny_unknown_fields`). serde_json already
//! rejects comments, trailing commas, numbers in strings and integer enum values for unit-variant enums.

use bitbybit_core::{Error, Result};
use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const MAX_DEPTH: usize = 32;
const MAX_ERROR_CHARS: usize = 300;

/// A JSON value parsed without duplicate object keys.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::from(v)))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::from(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<StrictValue, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::String(v.to_owned())))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_unit<E>(self) -> std::result::Result<StrictValue, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<StrictValue, A::Error> {
                let mut items = Vec::new();
                while let Some(StrictValue(v)) = seq.next_element()? {
                    items.push(v);
                }
                Ok(StrictValue(Value::Array(items)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<StrictValue, A::Error> {
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    let StrictValue(v) = map.next_value()?;
                    if out.insert(key.clone(), v).is_some() {
                        return Err(de::Error::custom(format!(
                            "duplicate property '{}'",
                            bitbybit_security::redact::redact(&key)
                        )));
                    }
                }
                Ok(StrictValue(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}

fn depth(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        Value::Object(o) => 1 + o.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

fn invalid(message: impl AsRef<str>) -> Error {
    let m = bitbybit_security::redact::redact(message.as_ref());
    let short: String = m.chars().take(MAX_ERROR_CHARS).collect();
    Error::invalid(if short.len() < m.len() { format!("{short}…") } else { short })
}

/// Parses client bytes: valid UTF-8 JSON, no duplicate keys, depth ≤ 32.
pub fn parse(bytes: &[u8]) -> Result<Value> {
    let StrictValue(v) = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    if depth(&v) > MAX_DEPTH {
        return Err(invalid("JSON nesting is too deep."));
    }
    Ok(v)
}

/// First path in `input` absent from `known` (the typed re-serialization).
fn unknown_path(input: &Value, known: &Value, path: &str) -> Option<String> {
    match (input, known) {
        (Value::Object(i), Value::Object(k)) => i.iter().find_map(|(name, v)| match k.get(name) {
            None => Some(format!("{path}.{name}")),
            Some(kv) => unknown_path(v, kv, &format!("{path}.{name}")),
        }),
        (Value::Array(i), Value::Array(k)) => {
            i.iter().zip(k).enumerate().find_map(|(n, (a, b))| unknown_path(a, b, &format!("{path}[{n}]")))
        }
        _ => None,
    }
}

/// Binds a value to `T`; returns the typed value and its canonical bytes (what will actually run). Unknown
/// properties are rejected. `null` for optional members is accepted (re-serialization may omit it).
pub fn bind<T: DeserializeOwned + Serialize>(value: &Value) -> Result<(T, Vec<u8>)> {
    let typed: T = serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))?;
    let canonical = serde_json::to_value(&typed)?;
    let stripped_nulls = strip_nulls(value);
    if let Some(path) = unknown_path(&stripped_nulls, &canonical, "$") {
        return Err(invalid(format!("unknown property {path}")));
    }
    Ok((typed, serde_json::to_vec(&canonical)?))
}

fn strip_nulls(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            Value::Object(o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), strip_nulls(v))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        inner: Inner,
    }
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Inner {
        n: u32,
    }

    fn bind_text(text: &str) -> Result<Req> {
        bind::<Req>(&parse(text.as_bytes())?).map(|(r, _)| r)
    }

    #[test]
    fn rejects_unknown_properties_and_wrong_case() {
        assert!(bind_text(r#"{"name":"a","inner":{"n":1}}"#).is_ok());
        assert!(bind_text(r#"{"name":"a","inner":{"n":1},"extra":1}"#).is_err());
        assert!(bind_text(r#"{"name":"a","inner":{"n":1,"x":2}}"#).is_err());
        assert!(bind_text(r#"{"Name":"a","inner":{"n":1}}"#).is_err());
        assert!(bind_text(r#"{"name":"a","note":null,"inner":{"n":1}}"#).is_ok());
        assert!(bind_text(r#"{"name":"a","inner":{"n":"1"}}"#).is_err(), "numbers must be numbers");
    }

    #[test]
    fn rejects_duplicates_depth_and_lax_syntax() {
        assert!(parse(br#"{"a":1,"a":2}"#).is_err());
        assert!(parse(br#"{"a":{"b":1,"b":1}}"#).is_err());
        let deep = format!("{}1{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse(deep.as_bytes()).is_err());
        let ok = format!("{}1{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(ok.as_bytes()).is_ok());
        for lax in [&br#"{"a":1,}"#[..], br#"{/*c*/"a":1}"#, br#"{'a':1}"#, b"\xff"] {
            assert!(parse(lax).is_err());
        }
    }

    #[test]
    fn canonical_form_ignores_formatting() {
        let (_, a) = bind::<Req>(&parse(br#"{ "inner": {"n": 1}, "name": "x" }"#).unwrap()).unwrap();
        let (_, b) = bind::<Req>(&parse(br#"{"name":"x","inner":{"n":1}}"#).unwrap()).unwrap();
        assert_eq!(a, b);
    }
}
