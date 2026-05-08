//! Canonical JSON helpers shared by F-B2/F-B4 report schemas.

use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value};

pub fn canonicalize<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalJsonError> {
    let value = serde_json::to_value(value).map_err(CanonicalJsonError::Serialize)?;
    let value = canonical_value(value)?;
    serde_json::to_vec(&value).map_err(CanonicalJsonError::Serialize)
}

pub fn canonical_value(value: Value) -> Result<Value, CanonicalJsonError> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(canonical_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(entries) => {
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, canonical_value(value)?);
            }
            Ok(Value::Object(sorted))
        }
        Value::Number(number) if number.is_f64() => Err(CanonicalJsonError::Float),
        other => Ok(other),
    }
}

#[derive(Debug)]
pub enum CanonicalJsonError {
    Float,
    Serialize(serde_json::Error),
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float => f.write_str("floating-point JSON numbers are not canonical reports"),
            Self::Serialize(error) => write!(f, "failed to serialize canonical JSON: {error}"),
        }
    }
}

impl std::error::Error for CanonicalJsonError {}
