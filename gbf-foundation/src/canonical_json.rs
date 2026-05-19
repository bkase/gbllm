//! Shared canonical JSON and domain-hash helpers.

use std::fmt;

use serde::Serialize;
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde_json::Value;

use crate::{Hash256, sha256};

/// Canonical JSON encoder inherited from the S1 artifact contract.
///
/// Object keys are sorted lexicographically by Unicode scalar value order. For
/// UTF-8 strings, Rust's bytewise `str` ordering preserves scalar order.
pub struct CanonicalJson;

impl CanonicalJson {
    /// Serialize `payload` to canonical JSON bytes.
    pub fn to_vec<T>(payload: &T) -> Result<Vec<u8>, CanonicalJsonError>
    where
        T: Serialize,
    {
        let value = payload.serialize(CanonicalSerializer)?;
        let mut out = Vec::new();
        write_canonical_value(&value, &mut out)?;
        Ok(out)
    }

    /// Serialize an already materialized JSON value to canonical JSON bytes.
    pub fn value_to_vec(value: &Value) -> Result<Vec<u8>, CanonicalJsonError> {
        let value = canonical_value_from_json(value)?;
        let mut out = Vec::new();
        write_canonical_value(&value, &mut out)?;
        Ok(out)
    }
}

/// Domain metadata for the RFC `DomainHash` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainHash<'a> {
    crate_name: &'a str,
    type_name: &'a str,
    schema_id: &'a str,
    schema_version: &'a str,
}

impl<'a> DomainHash<'a> {
    /// Create a domain hash context.
    #[must_use]
    pub const fn new(
        crate_name: &'a str,
        type_name: &'a str,
        schema_id: &'a str,
        schema_version: &'a str,
    ) -> Self {
        Self {
            crate_name,
            type_name,
            schema_id,
            schema_version,
        }
    }

    /// Hash a payload after canonical JSON serialization.
    ///
    /// Top-level `*_self_hash` fields are rejected here. Use
    /// [`self_hash_omitting_fields`] when computing an artifact's own self-hash.
    pub fn hash<T>(&self, payload: &T) -> Result<Hash256, CanonicalJsonError>
    where
        T: Serialize,
    {
        let value = payload.serialize(CanonicalSerializer)?;
        reject_top_level_self_hash(&value)?;
        self.hash_value_without_self_hash_check(&value)
    }

    /// Hash already canonicalized JSON bytes with this domain.
    pub fn hash_canonical_bytes(&self, canonical: &[u8]) -> Result<Hash256, CanonicalJsonError> {
        self.validate_components()?;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"gbf:");
        bytes.extend_from_slice(self.crate_name.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.type_name.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.schema_id.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.schema_version.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(canonical);
        Ok(sha256(bytes))
    }

    fn hash_value_without_self_hash_check(
        &self,
        value: &CanonicalValue,
    ) -> Result<Hash256, CanonicalJsonError> {
        let mut canonical = Vec::new();
        write_canonical_value(value, &mut canonical)?;
        self.hash_canonical_bytes(&canonical)
    }

    fn validate_components(&self) -> Result<(), CanonicalJsonError> {
        for (name, value) in [
            ("crate", self.crate_name),
            ("type", self.type_name),
            ("schema_id", self.schema_id),
            ("schema_version", self.schema_version),
        ] {
            if value
                .as_bytes()
                .iter()
                .any(|byte| *byte == b':' || *byte == 0)
            {
                return Err(CanonicalJsonError::InvalidDomainComponent {
                    component: name,
                    value: value.to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Canonical JSON bytes for a payload after omitting top-level fields.
pub fn canonical_json_bytes_omitting_fields<T>(
    payload: &T,
    omitted_fields: &[&str],
) -> Result<Vec<u8>, CanonicalJsonError>
where
    T: Serialize,
{
    let value = payload.serialize(CanonicalSerializer)?;
    let stripped = canonical_value_without_fields(value, omitted_fields)?;
    let mut out = Vec::new();
    write_canonical_value(&stripped, &mut out)?;
    Ok(out)
}

/// Compute an artifact self-hash after omitting top-level fields.
///
/// `self_hash_field` must be omitted and must end with `_self_hash`. Additional
/// omitted fields are removed before hashing. Any remaining top-level
/// `*_self_hash` field is rejected; nested self-hashes remain part of the
/// canonical payload.
pub fn self_hash_omitting_fields<T>(
    domain: DomainHash<'_>,
    payload: &T,
    self_hash_field: &str,
    extra_omitted_fields: &[&str],
) -> Result<Hash256, CanonicalJsonError>
where
    T: Serialize,
{
    validate_self_hash_field_name(self_hash_field)?;
    let value = payload.serialize(CanonicalSerializer)?;
    let mut omitted_fields = Vec::with_capacity(extra_omitted_fields.len() + 1);
    omitted_fields.push(self_hash_field);
    omitted_fields.extend_from_slice(extra_omitted_fields);
    let stripped = canonical_value_without_fields(value, &omitted_fields)?;
    reject_top_level_self_hash(&stripped)?;
    domain.hash_value_without_self_hash_check(&stripped)
}

/// Errors from canonical JSON serialization and domain hashing.
#[derive(Debug)]
pub enum CanonicalJsonError {
    /// Serde reported a custom serialization error.
    Custom(String),
    /// JSON tree materialization or scalar escaping failed.
    Json(serde_json::Error),
    /// NaN and infinities are forbidden in canonical JSON payloads.
    NonFiniteFloat,
    /// JSON object keys must serialize to strings.
    NonStringObjectKey,
    /// Object keys must be unique after string serialization.
    DuplicateObjectKey(String),
    /// A domain component would make the prefix ambiguous.
    InvalidDomainComponent {
        /// Component name.
        component: &'static str,
        /// Rejected component value.
        value: String,
    },
    /// Self-hash helpers require a top-level JSON object.
    ExpectedObjectForSelfHash,
    /// Self-hash fields must follow the `*_self_hash` convention.
    InvalidSelfHashFieldName(String),
    /// Plain domain hashing refuses to include an artifact's own hash.
    SelfHashFieldMustBeOmitted(String),
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(message) => write!(f, "{message}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::NonFiniteFloat => write!(f, "non-finite floats are forbidden in canonical JSON"),
            Self::NonStringObjectKey => write!(f, "JSON object keys must be strings"),
            Self::DuplicateObjectKey(key) => write!(f, "duplicate JSON object key {key:?}"),
            Self::InvalidDomainComponent { component, value } => {
                write!(f, "invalid domain component {component}={value:?}")
            }
            Self::ExpectedObjectForSelfHash => {
                write!(f, "self-hash helpers require a top-level object")
            }
            Self::InvalidSelfHashFieldName(field) => {
                write!(f, "self-hash field {field:?} must end with _self_hash")
            }
            Self::SelfHashFieldMustBeOmitted(field) => {
                write!(f, "field {field:?} must be omitted before domain hashing")
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

impl ser::Error for CanonicalJsonError {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self::Custom(msg.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CanonicalValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Array(Vec<CanonicalValue>),
    Object(Vec<(String, CanonicalValue)>),
}

struct CanonicalSerializer;

impl ser::Serializer for CanonicalSerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;
    type SerializeSeq = ArraySerializer;
    type SerializeTuple = ArraySerializer;
    type SerializeTupleStruct = ArraySerializer;
    type SerializeTupleVariant = TupleVariantSerializer;
    type SerializeMap = ObjectSerializer;
    type SerializeStruct = ObjectSerializer;
    type SerializeStructVariant = StructVariantSerializer;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(v))
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(v))
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        if v.is_finite() {
            Ok(CanonicalValue::F64(v))
        } else {
            Err(CanonicalJsonError::NonFiniteFloat)
        }
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(v.to_string()))
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(v.to_owned()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Array(
            v.iter()
                .copied()
                .map(|byte| CanonicalValue::U64(u64::from(byte)))
                .collect(),
        ))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Ok(CanonicalValue::Object(vec![(
            variant.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )]))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(TupleVariantSerializer {
            variant: variant.to_owned(),
            values: Vec::new(),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(ObjectSerializer {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(ObjectSerializer {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StructVariantSerializer {
            variant: variant.to_owned(),
            entries: Vec::new(),
        })
    }
}

struct KeySerializer;

impl ser::Serializer for KeySerializer {
    type Ok = String;
    type Error = CanonicalJsonError;
    type SerializeSeq = ser::Impossible<String, CanonicalJsonError>;
    type SerializeTuple = ser::Impossible<String, CanonicalJsonError>;
    type SerializeTupleStruct = ser::Impossible<String, CanonicalJsonError>;
    type SerializeTupleVariant = ser::Impossible<String, CanonicalJsonError>;
    type SerializeMap = ser::Impossible<String, CanonicalJsonError>;
    type SerializeStruct = ser::Impossible<String, CanonicalJsonError>;
    type SerializeStructVariant = ser::Impossible<String, CanonicalJsonError>;

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(v.to_owned())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_bool(self, _v: bool) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_i8(self, _v: i8) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_i16(self, _v: i16) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_i32(self, _v: i32) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_i64(self, _v: i64) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_u8(self, _v: u8) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_u16(self, _v: u16) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_u32(self, _v: u32) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_u64(self, _v: u64) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_f32(self, _v: f32) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_f64(self, _v: f64) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(v.to_string())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(CanonicalJsonError::NonStringObjectKey)
    }
}

struct ArraySerializer {
    values: Vec<CanonicalValue>,
}

impl SerializeSeq for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.values.push(value.serialize(CanonicalSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Array(self.values))
    }
}

impl SerializeTuple for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

struct TupleVariantSerializer {
    variant: String,
    values: Vec<CanonicalValue>,
}

impl SerializeTupleVariant for TupleVariantSerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.values.push(value.serialize(CanonicalSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(vec![(
            self.variant,
            CanonicalValue::Array(self.values),
        )]))
    }
}

struct ObjectSerializer {
    entries: Vec<(String, CanonicalValue)>,
    next_key: Option<String>,
}

impl SerializeMap for ObjectSerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.next_key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| CanonicalJsonError::Custom("missing object key".to_owned()))?;
        push_unique_entry(
            &mut self.entries,
            key,
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(self.entries))
    }
}

impl SerializeStruct for ObjectSerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        push_unique_entry(
            &mut self.entries,
            key.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(self.entries))
    }
}

struct StructVariantSerializer {
    variant: String,
    entries: Vec<(String, CanonicalValue)>,
}

impl SerializeStructVariant for StructVariantSerializer {
    type Ok = CanonicalValue;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        push_unique_entry(
            &mut self.entries,
            key.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(vec![(
            self.variant,
            CanonicalValue::Object(self.entries),
        )]))
    }
}

fn push_unique_entry(
    entries: &mut Vec<(String, CanonicalValue)>,
    key: String,
    value: CanonicalValue,
) -> Result<(), CanonicalJsonError> {
    if entries.iter().any(|(existing, _)| existing == &key) {
        return Err(CanonicalJsonError::DuplicateObjectKey(key));
    }
    entries.push((key, value));
    Ok(())
}

fn canonical_value_from_json(value: &Value) -> Result<CanonicalValue, CanonicalJsonError> {
    Ok(match value {
        Value::Null => CanonicalValue::Null,
        Value::Bool(value) => CanonicalValue::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                CanonicalValue::I64(value)
            } else if let Some(value) = number.as_u64() {
                CanonicalValue::U64(value)
            } else {
                CanonicalValue::F64(number.as_f64().ok_or_else(|| {
                    CanonicalJsonError::Json(serde_json::Error::io(std::io::Error::other(
                        "JSON number is not representable as finite f64",
                    )))
                })?)
            }
        }
        Value::String(value) => CanonicalValue::String(value.clone()),
        Value::Array(values) => CanonicalValue::Array(
            values
                .iter()
                .map(canonical_value_from_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(object) => {
            let mut entries = Vec::new();
            for (key, value) in object {
                push_unique_entry(&mut entries, key.clone(), canonical_value_from_json(value)?)?;
            }
            CanonicalValue::Object(entries)
        }
    })
}

fn canonical_value_without_fields(
    value: CanonicalValue,
    omitted_fields: &[&str],
) -> Result<CanonicalValue, CanonicalJsonError> {
    let CanonicalValue::Object(object) = value else {
        return Err(CanonicalJsonError::ExpectedObjectForSelfHash);
    };
    Ok(CanonicalValue::Object(
        object
            .into_iter()
            .filter(|(key, _)| !omitted_fields.contains(&key.as_str()))
            .collect(),
    ))
}

fn write_canonical_value(
    value: &CanonicalValue,
    out: &mut Vec<u8>,
) -> Result<(), CanonicalJsonError> {
    match value {
        CanonicalValue::Null => out.extend_from_slice(b"null"),
        CanonicalValue::Bool(true) => out.extend_from_slice(b"true"),
        CanonicalValue::Bool(false) => out.extend_from_slice(b"false"),
        CanonicalValue::I64(value) => out.extend_from_slice(value.to_string().as_bytes()),
        CanonicalValue::U64(value) => out.extend_from_slice(value.to_string().as_bytes()),
        CanonicalValue::F64(value) => write_canonical_f64(*value, out)?,
        CanonicalValue::String(string) => out.extend_from_slice(
            serde_json::to_string(string)
                .map_err(CanonicalJsonError::Json)?
                .as_bytes(),
        ),
        CanonicalValue::Array(values) => {
            out.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical_value(item, out)?;
            }
            out.push(b']');
        }
        CanonicalValue::Object(object) => write_canonical_object(object, out)?,
    }
    Ok(())
}

fn write_canonical_object(
    object: &[(String, CanonicalValue)],
    out: &mut Vec<u8>,
) -> Result<(), CanonicalJsonError> {
    let mut entries = object.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));

    out.push(b'{');
    for (index, (key, value)) in entries.into_iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        out.extend_from_slice(
            serde_json::to_string(key)
                .map_err(CanonicalJsonError::Json)?
                .as_bytes(),
        );
        out.push(b':');
        write_canonical_value(value, out)?;
    }
    out.push(b'}');
    Ok(())
}

fn write_canonical_f64(value: f64, out: &mut Vec<u8>) -> Result<(), CanonicalJsonError> {
    if !value.is_finite() {
        return Err(CanonicalJsonError::NonFiniteFloat);
    }
    if value == 0.0 {
        out.extend_from_slice(b"0.0");
        return Ok(());
    }

    let encoded = canonical_f64_text(value)?;
    out.extend_from_slice(encoded.as_bytes());
    Ok(())
}

fn canonical_f64_text(value: f64) -> Result<String, CanonicalJsonError> {
    let mut encoded = serde_json::to_string(&value).map_err(CanonicalJsonError::Json)?;
    if let Some(exponent) = encoded.find("e+") {
        encoded.replace_range(exponent..exponent + 2, "e");
    }
    Ok(encoded)
}

fn reject_top_level_self_hash(value: &CanonicalValue) -> Result<(), CanonicalJsonError> {
    if let CanonicalValue::Object(object) = value
        && let Some((field, _)) = object.iter().find(|(key, _)| key.ends_with("_self_hash"))
    {
        return Err(CanonicalJsonError::SelfHashFieldMustBeOmitted(
            field.clone(),
        ));
    }
    Ok(())
}

fn validate_self_hash_field_name(field: &str) -> Result<(), CanonicalJsonError> {
    if field.ends_with("_self_hash") {
        Ok(())
    } else {
        Err(CanonicalJsonError::InvalidSelfHashFieldName(
            field.to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_keys_and_preserves_array_order() {
        let value = json!({
            "z": [3, 2, 1],
            "a": {
                "b": true,
                "a": "first"
            }
        });

        assert_eq!(
            String::from_utf8(CanonicalJson::value_to_vec(&value).expect("canonical JSON"))
                .expect("utf8"),
            r#"{"a":{"a":"first","b":true},"z":[3,2,1]}"#
        );
    }

    #[test]
    fn canonical_json_uses_s1_float_spelling() {
        assert_eq!(
            CanonicalJson::to_vec(&-0.0_f64).expect("negative zero canonicalizes"),
            b"0.0"
        );
        assert_eq!(
            CanonicalJson::to_vec(&0.0_f64).expect("zero canonicalizes"),
            b"0.0"
        );
        assert_eq!(
            CanonicalJson::to_vec(&1.23456789012345_f64).expect("finite float canonicalizes"),
            serde_json::to_vec(&1.23456789012345_f64).expect("serde JSON float")
        );
    }

    #[test]
    fn canonical_json_rejects_non_finite_floats() {
        assert!(matches!(
            CanonicalJson::to_vec(&f64::NAN),
            Err(CanonicalJsonError::NonFiniteFloat)
        ));
        assert!(matches!(
            CanonicalJson::to_vec(&f64::INFINITY),
            Err(CanonicalJsonError::NonFiniteFloat)
        ));
    }

    #[test]
    fn domain_hash_uses_five_part_preimage() {
        let domain = DomainHash::new("gbf-foundation", "Example", "example.v1", "1");
        let canonical = br#"{"a":1}"#;

        let mut preimage = Vec::new();
        preimage.extend_from_slice(b"gbf:gbf-foundation:Example:example.v1:1\0");
        preimage.extend_from_slice(canonical);

        assert_eq!(
            domain.hash_canonical_bytes(canonical).expect("domain hash"),
            sha256(preimage)
        );
    }

    #[test]
    fn self_hash_omitting_fields_rejects_remaining_top_level_self_hashes() {
        #[derive(Serialize)]
        struct Artifact<'a> {
            schema: &'a str,
            payload: u64,
            artifact_self_hash: Hash256,
            other_self_hash: Hash256,
        }

        let artifact = Artifact {
            schema: "artifact.v1",
            payload: 7,
            artifact_self_hash: Hash256::ZERO,
            other_self_hash: Hash256::ZERO,
        };
        let err = self_hash_omitting_fields(
            DomainHash::new("gbf-foundation", "Artifact", "artifact.v1", "1"),
            &artifact,
            "artifact_self_hash",
            &[],
        )
        .expect_err("remaining self hash rejects");

        assert!(matches!(
            err,
            CanonicalJsonError::SelfHashFieldMustBeOmitted(field) if field == "other_self_hash"
        ));
    }
}
