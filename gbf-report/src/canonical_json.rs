//! Shared F-B2/F-B4 report envelope, canonical JSON, and self-hash helpers.

use std::fmt;
use std::str::FromStr;

use gbf_foundation::{Hash256, SemVer, SemVerParseError};
pub use gbf_policy::diagnostics::{DiagnosticSeverity, ValidationDiagnostic};
use serde::de::DeserializeOwned;
use serde::ser::{
    self, Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const SCHEMA_FIELD: &str = "schema";
const SCHEMA_VERSION_FIELD: &str = "schema_version";
const OUTCOME_FIELD: &str = "outcome";
const REPORT_SELF_HASH_FIELD: &str = "report_self_hash";

const ENVELOPE_FIELDS: [&str; 4] = [
    SCHEMA_FIELD,
    SCHEMA_VERSION_FIELD,
    OUTCOME_FIELD,
    REPORT_SELF_HASH_FIELD,
];

const ARTIFACT_VALIDATION_SCHEMA: &str = "artifact_validation.v1";
const POLICY_RESOLUTION_SCHEMA: &str = "policy_resolution.v1";
const STATIC_BUDGET_SCHEMA: &str = "static_budget.v1";

/// Report schema identifier carried by every F-B2/F-B4 report envelope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReportSchemaId(String);

impl ReportSchemaId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for ReportSchemaId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ReportSchemaId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ReportSchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Chunk-level validator report outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportOutcome {
    Passed,
    Failed,
}

/// Logical Rust wrapper for report bodies.
///
/// Public JSON is flat:
/// `{ schema, schema_version, outcome, report_self_hash, ...body_fields }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEnvelope<R> {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}

impl<R> ReportEnvelope<R>
where
    R: ReportBody,
{
    /// Build an envelope with the schema constants from `R` and a zero self-hash.
    pub fn new(outcome: ReportOutcome, body: R) -> Result<Self, ReportEnvelopeError> {
        Ok(Self {
            schema: ReportSchemaId::from(R::SCHEMA_ID),
            schema_version: parse_schema_version::<R>()
                .map_err(ReportEnvelopeError::SchemaVersion)?,
            outcome,
            report_self_hash: Hash256::ZERO,
            body,
        })
    }
}

impl<R> ReportEnvelope<R>
where
    R: ReportBody + Serialize,
{
    /// Return this envelope with `report_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, ReportSelfHashError> {
        self.report_self_hash = Hash256::ZERO;
        self.report_self_hash = compute_self_hash(&self)?;
        Ok(self)
    }
}

impl<R> Serialize for ReportEnvelope<R>
where
    R: ReportBody + Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        envelope_value_with_hash(self, self.report_self_hash)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de, R> Deserialize<'de> for ReportEnvelope<R>
where
    R: ReportBody + Serialize + DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        deserialize_envelope_value(value).map_err(serde::de::Error::custom)
    }
}

/// Trait implemented by each concrete F-B2/F-B4 report body.
pub trait ReportBody: Sized {
    const REPORT_TYPE: &'static str;
    const SCHEMA_ID: &'static str;
    const SCHEMA_VERSION: &'static str;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>>;
}

/// Serialize `env` to canonical JSON bytes.
pub fn canonicalize<R>(env: &ReportEnvelope<R>) -> Result<Vec<u8>, CanonicalJsonError>
where
    R: ReportBody + Serialize,
{
    validate_envelope(env)?;
    let value = envelope_value_with_hash(env, env.report_self_hash)?;
    validate_json_contract(R::SCHEMA_ID, &value)?;

    let mut bytes = Vec::new();
    emit_canonical_json(&value, &mut bytes)?;
    Ok(bytes)
}

/// Compute the domain-separated self-hash for `env`.
pub fn compute_self_hash<R>(env: &ReportEnvelope<R>) -> Result<Hash256, ReportSelfHashError>
where
    R: ReportBody + Serialize,
{
    validate_envelope(env)?;
    let value = envelope_value_with_hash(env, Hash256::ZERO)?;
    validate_json_contract(R::SCHEMA_ID, &value)?;

    let mut canonical = Vec::new();
    emit_canonical_json(&value, &mut canonical)?;

    let mut hasher = Sha256::new();
    hasher.update(domain_separator::<R>()?);
    hasher.update(&canonical);
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(Hash256::from_bytes(digest))
}

/// Verify that `env`'s stored self-hash matches the canonical fixed point and
/// that the canonical bytes deserialize back to the same report.
pub fn round_trip_self_hash<R>(env: &ReportEnvelope<R>) -> Result<(), ReportSelfHashError>
where
    R: ReportBody + Serialize + DeserializeOwned,
{
    let expected = compute_self_hash(env)?;
    if env.report_self_hash != expected {
        return Err(ReportSelfHashError::HashMismatch {
            expected,
            observed: env.report_self_hash,
        });
    }

    let bytes = canonicalize(env)?;
    let decoded: ReportEnvelope<R> =
        serde_json::from_slice(&bytes).map_err(|error| ReportSelfHashError::Json {
            reason: error.to_string(),
        })?;
    let decoded_bytes = canonicalize(&decoded)?;
    if decoded_bytes != bytes {
        return Err(ReportSelfHashError::RoundTripBytesChanged);
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportEnvelopeError {
    SchemaVersion(SemVerParseError),
}

impl fmt::Display for ReportEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersion(error) => write!(f, "invalid report schema version: {error}"),
        }
    }
}

impl std::error::Error for ReportEnvelopeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalJsonError {
    BodySerialization {
        reason: String,
    },
    BodyMustSerializeAsObject,
    EnvelopeFieldCollision {
        field: String,
    },
    SchemaMismatch {
        expected: String,
        observed: String,
    },
    SchemaVersionParse {
        version: String,
        reason: String,
    },
    SchemaVersionMismatch {
        expected: SemVer,
        observed: SemVer,
    },
    SemanticValidation {
        diagnostics: Vec<ValidationDiagnostic>,
    },
    FloatingPointValue {
        path: String,
    },
    NonCanonicalHashString {
        path: String,
        value: String,
    },
    UnlistedNullField {
        schema: String,
        path: String,
    },
    SoftDiagnostic {
        path: String,
    },
    JsonString {
        reason: String,
    },
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BodySerialization { reason } => write!(f, "body serialization failed: {reason}"),
            Self::BodyMustSerializeAsObject => {
                f.write_str("report body must serialize as an object")
            }
            Self::EnvelopeFieldCollision { field } => {
                write!(f, "report body duplicates envelope field {field}")
            }
            Self::SchemaMismatch { expected, observed } => {
                write!(f, "expected schema {expected}, observed {observed}")
            }
            Self::SchemaVersionParse { version, reason } => {
                write!(f, "invalid schema version {version}: {reason}")
            }
            Self::SchemaVersionMismatch { expected, observed } => {
                write!(f, "expected schema version {expected}, observed {observed}")
            }
            Self::SemanticValidation { diagnostics } => {
                write!(
                    f,
                    "report semantic validation failed with {} diagnostics",
                    diagnostics.len()
                )
            }
            Self::FloatingPointValue { path } => {
                write!(f, "floating-point JSON number at {path}")
            }
            Self::NonCanonicalHashString { path, value } => {
                write!(f, "hash field {path} is not canonical sha256: hex: {value}")
            }
            Self::UnlistedNullField { schema, path } => {
                write!(f, "null at {path} is not allowed for {schema}")
            }
            Self::SoftDiagnostic { path } => {
                write!(f, "Soft diagnostic is not allowed at {path}")
            }
            Self::JsonString { reason } => write!(f, "JSON string emission failed: {reason}"),
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

impl ser::Error for CanonicalJsonError {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self::BodySerialization {
            reason: msg.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportSelfHashError {
    CanonicalJson(CanonicalJsonError),
    HashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    MissingEnvelopeField {
        field: &'static str,
    },
    EnvelopeMustBeObject,
    EnvelopeDecode {
        field: &'static str,
        reason: String,
    },
    BodyDecode {
        reason: String,
    },
    Json {
        reason: String,
    },
    RoundTripBytesChanged,
}

impl fmt::Display for ReportSelfHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::HashMismatch { expected, observed } => {
                write!(
                    f,
                    "report_self_hash {observed} does not match expected {expected}"
                )
            }
            Self::MissingEnvelopeField { field } => write!(f, "missing envelope field {field}"),
            Self::EnvelopeMustBeObject => f.write_str("report envelope must be a JSON object"),
            Self::EnvelopeDecode { field, reason } => {
                write!(f, "failed to decode envelope field {field}: {reason}")
            }
            Self::BodyDecode { reason } => write!(f, "failed to decode report body: {reason}"),
            Self::Json { reason } => write!(f, "JSON error: {reason}"),
            Self::RoundTripBytesChanged => f.write_str("canonical bytes changed after round trip"),
        }
    }
}

impl std::error::Error for ReportSelfHashError {}

impl From<CanonicalJsonError> for ReportSelfHashError {
    fn from(value: CanonicalJsonError) -> Self {
        Self::CanonicalJson(value)
    }
}

fn deserialize_envelope_value<R>(value: Value) -> Result<ReportEnvelope<R>, ReportSelfHashError>
where
    R: ReportBody + Serialize + DeserializeOwned,
{
    validate_json_contract(R::SCHEMA_ID, &value)?;

    let mut map = match value {
        Value::Object(map) => map,
        _ => return Err(ReportSelfHashError::EnvelopeMustBeObject),
    };

    let schema: ReportSchemaId = take_envelope_field(&mut map, SCHEMA_FIELD)
        .and_then(|value| decode_envelope_field(SCHEMA_FIELD, value))?;
    let schema_version = take_envelope_field(&mut map, SCHEMA_VERSION_FIELD)
        .and_then(decode_schema_version_field)?;
    let outcome: ReportOutcome = take_envelope_field(&mut map, OUTCOME_FIELD)
        .and_then(|value| decode_envelope_field(OUTCOME_FIELD, value))?;
    let report_self_hash: Hash256 = take_envelope_field(&mut map, REPORT_SELF_HASH_FIELD)
        .and_then(|value| decode_envelope_field(REPORT_SELF_HASH_FIELD, value))?;

    let body = serde_json::from_value(Value::Object(map)).map_err(|error| {
        ReportSelfHashError::BodyDecode {
            reason: error.to_string(),
        }
    })?;

    let env = ReportEnvelope {
        schema,
        schema_version,
        outcome,
        report_self_hash,
        body,
    };
    validate_envelope(&env)?;

    let expected = compute_self_hash(&env)?;
    if report_self_hash != expected {
        return Err(ReportSelfHashError::HashMismatch {
            expected,
            observed: report_self_hash,
        });
    }

    Ok(env)
}

fn take_envelope_field(
    map: &mut Map<String, Value>,
    field: &'static str,
) -> Result<Value, ReportSelfHashError> {
    map.remove(field)
        .ok_or(ReportSelfHashError::MissingEnvelopeField { field })
}

fn decode_envelope_field<T>(field: &'static str, value: Value) -> Result<T, ReportSelfHashError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| ReportSelfHashError::EnvelopeDecode {
        field,
        reason: error.to_string(),
    })
}

fn decode_schema_version_field(value: Value) -> Result<SemVer, ReportSelfHashError> {
    let version: String = decode_envelope_field(SCHEMA_VERSION_FIELD, value)?;
    SemVer::from_str(&version).map_err(|error| ReportSelfHashError::EnvelopeDecode {
        field: SCHEMA_VERSION_FIELD,
        reason: error.to_string(),
    })
}

fn validate_envelope<R>(env: &ReportEnvelope<R>) -> Result<(), CanonicalJsonError>
where
    R: ReportBody,
{
    if env.schema.as_str() != R::SCHEMA_ID {
        return Err(CanonicalJsonError::SchemaMismatch {
            expected: R::SCHEMA_ID.to_owned(),
            observed: env.schema.as_str().to_owned(),
        });
    }

    let expected_version =
        parse_schema_version::<R>().map_err(|error| CanonicalJsonError::SchemaVersionParse {
            version: R::SCHEMA_VERSION.to_owned(),
            reason: error.to_string(),
        })?;
    if env.schema_version != expected_version {
        return Err(CanonicalJsonError::SchemaVersionMismatch {
            expected: expected_version,
            observed: env.schema_version,
        });
    }

    if let Err(diagnostics) = env.body.validate_semantics(env.outcome) {
        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Soft)
        {
            return Err(CanonicalJsonError::SoftDiagnostic {
                path: "validate_semantics".to_owned(),
            });
        }
        return Err(CanonicalJsonError::SemanticValidation { diagnostics });
    }

    Ok(())
}

fn parse_schema_version<R>() -> Result<SemVer, SemVerParseError>
where
    R: ReportBody,
{
    SemVer::from_str(R::SCHEMA_VERSION)
}

fn envelope_value_with_hash<R>(
    env: &ReportEnvelope<R>,
    report_self_hash: Hash256,
) -> Result<Value, CanonicalJsonError>
where
    R: ReportBody + Serialize,
{
    let body = to_json_value_rejecting_non_finite(&env.body)?;
    let body = match body {
        Value::Object(body) => body,
        _ => return Err(CanonicalJsonError::BodyMustSerializeAsObject),
    };

    let mut map = Map::new();
    map.insert(
        SCHEMA_FIELD.to_owned(),
        Value::String(env.schema.to_string()),
    );
    map.insert(
        SCHEMA_VERSION_FIELD.to_owned(),
        Value::String(env.schema_version.to_string()),
    );
    map.insert(
        OUTCOME_FIELD.to_owned(),
        serde_json::to_value(env.outcome).map_err(|error| {
            CanonicalJsonError::BodySerialization {
                reason: error.to_string(),
            }
        })?,
    );
    map.insert(
        REPORT_SELF_HASH_FIELD.to_owned(),
        serde_json::to_value(report_self_hash).map_err(|error| {
            CanonicalJsonError::BodySerialization {
                reason: error.to_string(),
            }
        })?,
    );

    for (field, value) in body {
        if ENVELOPE_FIELDS.contains(&field.as_str()) {
            return Err(CanonicalJsonError::EnvelopeFieldCollision { field });
        }
        map.insert(field, value);
    }

    Ok(Value::Object(map))
}

fn to_json_value_rejecting_non_finite<T>(value: &T) -> Result<Value, CanonicalJsonError>
where
    T: Serialize + ?Sized,
{
    value.serialize(StrictValueSerializer {
        path: String::new(),
    })
}

struct StrictValueSerializer {
    path: String,
}

impl StrictValueSerializer {
    fn child(&self, segment: &str) -> Self {
        Self {
            path: join_path(&self.path, segment),
        }
    }

    fn number_from_f64(self, value: f64) -> Result<Value, CanonicalJsonError> {
        if !value.is_finite() {
            return Err(CanonicalJsonError::FloatingPointValue { path: self.path });
        }
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or(CanonicalJsonError::FloatingPointValue { path: self.path })
    }
}

impl Serializer for StrictValueSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;
    type SerializeSeq = StrictSeqSerializer;
    type SerializeTuple = StrictSeqSerializer;
    type SerializeTupleStruct = StrictSeqSerializer;
    type SerializeTupleVariant = StrictTupleVariantSerializer;
    type SerializeMap = StrictMapSerializer;
    type SerializeStruct = StrictStructSerializer;
    type SerializeStructVariant = StrictStructVariantSerializer;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Number(value.into()))
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        self.number_from_f64(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        self.number_from_f64(value)
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(Value::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(Value::String(value.to_owned()))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Array(
            value
                .iter()
                .copied()
                .map(|byte| Value::Number(byte.into()))
                .collect(),
        ))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(Value::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
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
        T: Serialize + ?Sized,
    {
        let mut map = Map::new();
        map.insert(variant.to_owned(), value.serialize(self.child(variant))?);
        Ok(Value::Object(map))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(StrictSeqSerializer::new(self.path, len))
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(StrictSeqSerializer::new(self.path, Some(len)))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(StrictSeqSerializer::new(self.path, Some(len)))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(StrictTupleVariantSerializer {
            variant: variant.to_owned(),
            seq: StrictSeqSerializer::new(self.path, Some(len)),
        })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(StrictMapSerializer {
            path: self.path,
            entries: Map::with_capacity(len.unwrap_or(0)),
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StrictStructSerializer {
            path: self.path,
            entries: Map::with_capacity(len),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StrictStructVariantSerializer {
            variant: variant.to_owned(),
            inner: StrictStructSerializer {
                path: join_path(&self.path, variant),
                entries: Map::with_capacity(len),
            },
        })
    }
}

struct StrictSeqSerializer {
    path: String,
    values: Vec<Value>,
}

impl StrictSeqSerializer {
    fn new(path: String, len: Option<usize>) -> Self {
        Self {
            path,
            values: Vec::with_capacity(len.unwrap_or(0)),
        }
    }
}

impl SerializeSeq for StrictSeqSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let serializer = StrictValueSerializer {
            path: format!("{}[{}]", self.path, self.values.len()),
        };
        self.values.push(value.serialize(serializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Array(self.values))
    }
}

impl SerializeTuple for StrictSeqSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for StrictSeqSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

struct StrictTupleVariantSerializer {
    variant: String,
    seq: StrictSeqSerializer,
}

impl SerializeTupleVariant for StrictTupleVariantSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(&mut self.seq, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let mut map = Map::new();
        map.insert(self.variant, SerializeSeq::end(self.seq)?);
        Ok(Value::Object(map))
    }
}

struct StrictMapSerializer {
    path: String,
    entries: Map<String, Value>,
    next_key: Option<String>,
}

impl SerializeMap for StrictMapSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        self.next_key = Some(key.serialize(StringKeySerializer)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| serialization_error("map value serialized before key"))?;
        let child_path = join_path(&self.path, &key);
        self.entries.insert(
            key,
            value.serialize(StrictValueSerializer { path: child_path })?,
        );
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Object(self.entries))
    }
}

struct StrictStructSerializer {
    path: String,
    entries: Map<String, Value>,
}

impl SerializeStruct for StrictStructSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let child_path = join_path(&self.path, key);
        self.entries.insert(
            key.to_owned(),
            value.serialize(StrictValueSerializer { path: child_path })?,
        );
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(Value::Object(self.entries))
    }
}

struct StrictStructVariantSerializer {
    variant: String,
    inner: StrictStructSerializer,
}

impl SerializeStructVariant for StrictStructVariantSerializer {
    type Ok = Value;
    type Error = CanonicalJsonError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeStruct::serialize_field(&mut self.inner, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let mut map = Map::new();
        map.insert(self.variant, SerializeStruct::end(self.inner)?);
        Ok(Value::Object(map))
    }
}

struct StringKeySerializer;

impl Serializer for StringKeySerializer {
    type Ok = String;
    type Error = CanonicalJsonError;
    type SerializeSeq = Impossible<String, CanonicalJsonError>;
    type SerializeTuple = Impossible<String, CanonicalJsonError>;
    type SerializeTupleStruct = Impossible<String, CanonicalJsonError>;
    type SerializeTupleVariant = Impossible<String, CanonicalJsonError>;
    type SerializeMap = Impossible<String, CanonicalJsonError>;
    type SerializeStruct = Impossible<String, CanonicalJsonError>;
    type SerializeStructVariant = Impossible<String, CanonicalJsonError>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            Ok(value.to_string())
        } else {
            Err(CanonicalJsonError::FloatingPointValue {
                path: "<map-key>".to_owned(),
            })
        }
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            Ok(value.to_string())
        } else {
            Err(CanonicalJsonError::FloatingPointValue {
                path: "<map-key>".to_owned(),
            })
        }
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_owned())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(serialization_error("byte-array map keys are unsupported"))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(serialization_error("null map keys are unsupported"))
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(serialization_error("unit map keys are unsupported"))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(serialization_error("unit struct map keys are unsupported"))
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
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        Err(serialization_error(
            "newtype variant map keys are unsupported",
        ))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(serialization_error("sequence map keys are unsupported"))
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(serialization_error("tuple map keys are unsupported"))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(serialization_error("tuple struct map keys are unsupported"))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(serialization_error(
            "tuple variant map keys are unsupported",
        ))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(serialization_error("map map keys are unsupported"))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(serialization_error("struct map keys are unsupported"))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(serialization_error(
            "struct variant map keys are unsupported",
        ))
    }
}

fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}.{segment}")
    }
}

fn serialization_error(message: impl fmt::Display) -> CanonicalJsonError {
    <CanonicalJsonError as ser::Error>::custom(message)
}

fn domain_separator<R>() -> Result<Vec<u8>, ReportSelfHashError>
where
    R: ReportBody,
{
    let version = parse_schema_version::<R>().map_err(|error| {
        ReportSelfHashError::CanonicalJson(CanonicalJsonError::SchemaVersionParse {
            version: R::SCHEMA_VERSION.to_owned(),
            reason: error.to_string(),
        })
    })?;
    Ok(format!(
        "gbf:gbf-report:{}:{}:{}\0",
        R::REPORT_TYPE,
        R::SCHEMA_ID,
        version
    )
    .into_bytes())
}

fn validate_json_contract(schema: &str, value: &Value) -> Result<(), CanonicalJsonError> {
    validate_json_value(schema, value, "")
}

fn validate_json_value(schema: &str, value: &Value, path: &str) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => {
            if !is_allowed_null_path(schema, path) {
                return Err(CanonicalJsonError::UnlistedNullField {
                    schema: schema.to_owned(),
                    path: path.to_owned(),
                });
            }
        }
        Value::Bool(_) => {}
        Value::String(value) => {
            if is_hash_string_path(path) && !is_canonical_sha256_string(value) {
                return Err(CanonicalJsonError::NonCanonicalHashString {
                    path: path.to_owned(),
                    value: value.clone(),
                });
            }
        }
        Value::Number(number) => {
            if number.as_i64().is_none() && number.as_u64().is_none() {
                return Err(CanonicalJsonError::FloatingPointValue {
                    path: path.to_owned(),
                });
            }
        }
        Value::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                let nested_path = format!("{path}[{index}]");
                validate_json_value(schema, nested, &nested_path)?;
            }
        }
        Value::Object(map) => {
            if let Some(severity) = map.get("severity")
                && is_soft_severity(severity)
            {
                let severity_path = if path.is_empty() {
                    "severity".to_owned()
                } else {
                    format!("{path}.severity")
                };
                return Err(CanonicalJsonError::SoftDiagnostic {
                    path: severity_path,
                });
            }
            for (key, nested) in map {
                let nested_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                validate_json_value(schema, nested, &nested_path)?;
            }
        }
    }

    Ok(())
}

fn is_soft_severity(value: &Value) -> bool {
    match value {
        Value::String(value) => value == "Soft",
        Value::Object(map) => {
            matches!(map.get("kind"), Some(Value::String(kind)) if kind == "Soft")
        }
        _ => false,
    }
}

fn is_hash_string_path(path: &str) -> bool {
    let field = path
        .rsplit('.')
        .next()
        .unwrap_or(path)
        .split('[')
        .next()
        .unwrap_or(path);

    field == "digest"
        || field == "sha256"
        || field == "target_defaults"
        || field == "profile_defaults"
        || field.ends_with("_hash")
}

fn is_canonical_sha256_string(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_allowed_null_path(schema: &str, path: &str) -> bool {
    match schema {
        ARTIFACT_VALIDATION_SCHEMA => matches!(
            path,
            "identity.artifact_source_hash"
                | "identity.artifact_effective_core_hash"
                | "identity.artifact_manifest_hash"
                | "identity.semantic_core_hash"
                | "identity.artifact_aux_hash"
                | "identity.lowering_manifest_hash"
                | "identity.calibration_hash"
                | "identity.compatibility_adapter_hash"
                | "compatibility.decision"
        ),
        POLICY_RESOLUTION_SCHEMA => path == "result",
        STATIC_BUDGET_SCHEMA => {
            matches!(
                path,
                "identity.runtime_chrome_budget_hash"
                    | "runtime_chrome_budget"
                    | "projections.common_bank_footprint.shared_dense_ffn_bytes"
            ) || is_projected_switches_expected_path(path)
        }
        _ => false,
    }
}

fn is_projected_switches_expected_path(path: &str) -> bool {
    path.starts_with("projections.projected_")
        && path.ends_with("_switches_per_token.expected_q16_16")
}

fn emit_canonical_json(value: &Value, bytes: &mut Vec<u8>) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => bytes.extend_from_slice(b"null"),
        Value::Bool(value) => {
            bytes.extend_from_slice(if *value { b"true" } else { b"false" });
        }
        Value::Number(number) => bytes.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => {
            serde_json::to_writer(bytes, value).map_err(|error| CanonicalJsonError::JsonString {
                reason: error.to_string(),
            })?
        }
        Value::Array(values) => {
            bytes.push(b'[');
            for (index, nested) in values.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                emit_canonical_json(nested, bytes)?;
            }
            bytes.push(b']');
        }
        Value::Object(map) => {
            bytes.push(b'{');
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                serde_json::to_writer(&mut *bytes, key).map_err(|error| {
                    CanonicalJsonError::JsonString {
                        reason: error.to_string(),
                    }
                })?;
                bytes.push(b':');
                emit_canonical_json(&map[*key], bytes)?;
            }
            bytes.push(b'}');
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_policy::diagnostics::{ValidationCode, ValidationDetail, ValidationOrigin};

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn zero_hash_json() -> &'static str {
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    }

    fn diagnostic(severity: DiagnosticSeverity) -> ValidationDiagnostic {
        ValidationDiagnostic::new(
            severity,
            ValidationOrigin::Schema,
            ValidationCode::SchemaEpochUnsupported,
            ValidationDetail::None,
            Vec::new(),
        )
    }

    fn hard_diagnostic() -> ValidationDiagnostic {
        diagnostic(DiagnosticSeverity::Hard)
    }

    fn soft_diagnostic() -> ValidationDiagnostic {
        diagnostic(DiagnosticSeverity::Soft)
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ByteStableBody {
        zeta: u32,
        alpha: ByteStableNested,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ByteStableNested {
        b: Vec<bool>,
        a: u32,
    }

    impl ReportBody for ByteStableBody {
        const REPORT_TYPE: &'static str = "ByteStableReport";
        const SCHEMA_ID: &'static str = "byte_stable_report.v1";
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FloatBody {
        diagnostics: Vec<ValidationDiagnostic>,
        score: f64,
    }

    impl ReportBody for FloatBody {
        const REPORT_TYPE: &'static str = "ArtifactValidationReport";
        const SCHEMA_ID: &'static str = ARTIFACT_VALIDATION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct DiagnosticBody {
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for DiagnosticBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            if self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Soft)
            {
                return Err(self.diagnostics.clone());
            }
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ArtifactNullBody {
        identity: ArtifactNullIdentity,
        compatibility: ArtifactCompatibility,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ArtifactNullIdentity {
        artifact_source_hash: Option<Hash256>,
        unexpected_missing_hash: Option<Hash256>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ArtifactCompatibility {
        decision: Option<String>,
    }

    impl ReportBody for ArtifactNullBody {
        const REPORT_TYPE: &'static str = "ArtifactValidationReport";
        const SCHEMA_ID: &'static str = ARTIFACT_VALIDATION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ArtifactValidationFixtureBody {
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for ArtifactValidationFixtureBody {
        const REPORT_TYPE: &'static str = "ArtifactValidationReport";
        const SCHEMA_ID: &'static str = ARTIFACT_VALIDATION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PolicyResolutionFixtureBody {
        result: Option<String>,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for PolicyResolutionFixtureBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            if outcome == ReportOutcome::Passed && self.result.is_none() {
                return Err(vec![hard_diagnostic()]);
            }
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticBudgetFixtureBody {
        runtime_chrome_budget: Option<String>,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for StaticBudgetFixtureBody {
        const REPORT_TYPE: &'static str = "StaticBudgetReport";
        const SCHEMA_ID: &'static str = STATIC_BUDGET_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SoftScanBody {
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for SoftScanBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct NullableFloatBody {
        result: f64,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for NullableFloatBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PolicyNullBody {
        result: Option<String>,
        unexpected_missing_hash: Option<Hash256>,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    impl ReportBody for PolicyNullBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticNullBody {
        identity: StaticNullIdentity,
        runtime_chrome_budget: Option<String>,
        projections: StaticNullProjections,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticNullIdentity {
        runtime_chrome_budget_hash: Option<Hash256>,
        unexpected_missing_hash: Option<Hash256>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticNullProjections {
        common_bank_footprint: StaticCommonBankFootprint,
        projected_bank_switches_per_token: StaticSwitchProjection,
        projected_sram_page_switches_per_token: StaticSwitchProjection,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticCommonBankFootprint {
        shared_dense_ffn_bytes: Option<u32>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StaticSwitchProjection {
        expected_q16_16: Option<u32>,
    }

    impl ReportBody for StaticNullBody {
        const REPORT_TYPE: &'static str = "StaticBudgetReport";
        const SCHEMA_ID: &'static str = STATIC_BUDGET_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PolicyHashPathsBody {
        result: Option<PolicyHashPathsResult>,
        diagnostics: Vec<ValidationDiagnostic>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PolicyHashPathsResult {
        provenance: PolicyHashPathsProvenance,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PolicyHashPathsProvenance {
        target_defaults: String,
        profile_defaults: String,
    }

    impl ReportBody for PolicyHashPathsBody {
        const REPORT_TYPE: &'static str = "PolicyResolutionReport";
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[test]
    fn canonical_json_emitter_is_byte_stable() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            ByteStableBody {
                zeta: 2,
                alpha: ByteStableNested {
                    b: vec![true, false],
                    a: 1,
                },
            },
        )
        .expect("envelope");

        let json = String::from_utf8(canonicalize(&env).expect("canonical json")).expect("utf8");

        assert_eq!(
            json,
            format!(
                "{{\"alpha\":{{\"a\":1,\"b\":[true,false]}},\"outcome\":\"Passed\",\"report_self_hash\":\"{}\",\"schema\":\"byte_stable_report.v1\",\"schema_version\":\"1.0.0\",\"zeta\":2}}",
                zero_hash_json()
            )
        );
    }

    #[test]
    fn report_envelope_public_shape_matches_rfc_examples() {
        let env = ReportEnvelope::new(
            ReportOutcome::Failed,
            ByteStableBody {
                zeta: 2,
                alpha: ByteStableNested {
                    b: vec![true],
                    a: 1,
                },
            },
        )
        .expect("envelope");

        assert_eq!(
            serde_json::to_value(&env).expect("public JSON value"),
            serde_json::json!({
                "schema": "byte_stable_report.v1",
                "schema_version": "1.0.0",
                "outcome": "Failed",
                "report_self_hash": zero_hash_json(),
                "alpha": {
                    "a": 1,
                    "b": [true]
                },
                "zeta": 2
            })
        );
    }

    #[test]
    fn report_body_validation_uses_shared_policy_diagnostic_type() {
        let report_diagnostic: ValidationDiagnostic = soft_diagnostic();
        let policy_diagnostic: gbf_policy::diagnostics::ValidationDiagnostic =
            report_diagnostic.clone();

        assert_eq!(policy_diagnostic, report_diagnostic);
        assert_eq!(
            CanonicalJsonError::SemanticValidation {
                diagnostics: vec![policy_diagnostic]
            },
            CanonicalJsonError::SemanticValidation {
                diagnostics: vec![soft_diagnostic()]
            }
        );
    }

    #[test]
    fn canonical_json_rejects_float_values_in_f_b2_f_b4_reports() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            FloatBody {
                diagnostics: Vec::new(),
                score: 1.25,
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&env),
            Err(CanonicalJsonError::FloatingPointValue {
                path: "score".to_owned()
            })
        );
    }

    #[test]
    fn canonical_json_rejects_non_finite_float_before_nullable_null_scan() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let env = ReportEnvelope::new(
                ReportOutcome::Failed,
                NullableFloatBody {
                    result: value,
                    diagnostics: Vec::new(),
                },
            )
            .expect("envelope");

            assert_eq!(
                canonicalize(&env),
                Err(CanonicalJsonError::FloatingPointValue {
                    path: "result".to_owned()
                })
            );
        }
    }

    #[test]
    fn self_hash_golden_pins_domain_and_canonical_bytes() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            ByteStableBody {
                zeta: 7,
                alpha: ByteStableNested {
                    b: vec![false],
                    a: 3,
                },
            },
        )
        .expect("envelope");
        let canonical = canonicalize(&env).expect("canonical json");
        let domain = domain_separator::<ByteStableBody>().expect("domain separator");

        assert_eq!(
            canonical,
            format!(
                "{{\"alpha\":{{\"a\":3,\"b\":[false]}},\"outcome\":\"Passed\",\"report_self_hash\":\"{}\",\"schema\":\"byte_stable_report.v1\",\"schema_version\":\"1.0.0\",\"zeta\":7}}",
                zero_hash_json()
            )
            .into_bytes()
        );
        assert_eq!(
            domain,
            b"gbf:gbf-report:ByteStableReport:byte_stable_report.v1:1.0.0\0"
        );
        assert_eq!(
            compute_self_hash(&env).expect("self hash"),
            Hash256::from_str(
                "sha256:c27bf548d583fda2a456d46a7185196c3ffdcf92e01d2a62132596534d49a6ae"
            )
            .expect("expected hash")
        );
    }

    #[test]
    fn self_hash_round_trip() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            ByteStableBody {
                zeta: 7,
                alpha: ByteStableNested {
                    b: vec![false],
                    a: 3,
                },
            },
        )
        .expect("envelope")
        .with_computed_self_hash()
        .expect("self hash");

        round_trip_self_hash(&env).expect("round trip");

        let mut tampered = env.clone();
        tampered.body.zeta += 1;
        assert!(matches!(
            round_trip_self_hash(&tampered),
            Err(ReportSelfHashError::HashMismatch { .. })
        ));
    }

    #[test]
    fn f_b2_f_b4_report_bodies_deny_unknown_fields() {
        assert_unknown_body_field_rejected::<ArtifactValidationFixtureBody>(
            ARTIFACT_VALIDATION_SCHEMA,
            "{\"diagnostics\":[],\"unexpected\":true}",
        );
        assert_unknown_body_field_rejected::<PolicyResolutionFixtureBody>(
            POLICY_RESOLUTION_SCHEMA,
            "{\"diagnostics\":[],\"result\":\"ok\",\"unexpected\":true}",
        );
        assert_unknown_body_field_rejected::<StaticBudgetFixtureBody>(
            STATIC_BUDGET_SCHEMA,
            "{\"diagnostics\":[],\"runtime_chrome_budget\":null,\"unexpected\":true}",
        );
    }

    #[test]
    fn f_b2_f_b4_reports_reject_soft_diagnostics() {
        let body = DiagnosticBody {
            diagnostics: vec![soft_diagnostic()],
        };
        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());

        let env = ReportEnvelope::new(ReportOutcome::Passed, body).expect("envelope");

        assert_eq!(
            canonicalize(&env),
            Err(CanonicalJsonError::SoftDiagnostic {
                path: "validate_semantics".to_owned()
            })
        );
    }

    #[test]
    fn f_b2_f_b4_reports_reject_soft_diagnostics_from_json_scan() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            SoftScanBody {
                diagnostics: vec![soft_diagnostic()],
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&env),
            Err(CanonicalJsonError::SoftDiagnostic {
                path: "diagnostics[0].severity".to_owned()
            })
        );
    }

    #[test]
    fn f_b2_f_b4_reports_reject_unlisted_null_fields() {
        let allowed_artifact = ReportEnvelope::new(
            ReportOutcome::Passed,
            ArtifactNullBody {
                identity: ArtifactNullIdentity {
                    artifact_source_hash: None,
                    unexpected_missing_hash: Some(hash(1)),
                },
                compatibility: ArtifactCompatibility { decision: None },
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");
        canonicalize(&allowed_artifact).expect("artifact_validation listed null fields accepted");

        let unlisted_artifact = ReportEnvelope::new(
            ReportOutcome::Passed,
            ArtifactNullBody {
                identity: ArtifactNullIdentity {
                    artifact_source_hash: None,
                    unexpected_missing_hash: None,
                },
                compatibility: ArtifactCompatibility { decision: None },
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&unlisted_artifact),
            Err(CanonicalJsonError::UnlistedNullField {
                schema: ARTIFACT_VALIDATION_SCHEMA.to_owned(),
                path: "identity.unexpected_missing_hash".to_owned()
            })
        );

        let allowed_policy = ReportEnvelope::new(
            ReportOutcome::Failed,
            PolicyNullBody {
                result: None,
                unexpected_missing_hash: Some(hash(2)),
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");
        canonicalize(&allowed_policy).expect("policy_resolution listed null fields accepted");

        let unlisted_policy = ReportEnvelope::new(
            ReportOutcome::Failed,
            PolicyNullBody {
                result: None,
                unexpected_missing_hash: None,
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&unlisted_policy),
            Err(CanonicalJsonError::UnlistedNullField {
                schema: POLICY_RESOLUTION_SCHEMA.to_owned(),
                path: "unexpected_missing_hash".to_owned()
            })
        );

        let allowed_static_budget = ReportEnvelope::new(
            ReportOutcome::Failed,
            StaticNullBody {
                identity: StaticNullIdentity {
                    runtime_chrome_budget_hash: None,
                    unexpected_missing_hash: Some(hash(3)),
                },
                runtime_chrome_budget: None,
                projections: StaticNullProjections {
                    common_bank_footprint: StaticCommonBankFootprint {
                        shared_dense_ffn_bytes: None,
                    },
                    projected_bank_switches_per_token: StaticSwitchProjection {
                        expected_q16_16: None,
                    },
                    projected_sram_page_switches_per_token: StaticSwitchProjection {
                        expected_q16_16: None,
                    },
                },
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");
        canonicalize(&allowed_static_budget).expect("static_budget listed null fields accepted");

        let unlisted_static_budget = ReportEnvelope::new(
            ReportOutcome::Failed,
            StaticNullBody {
                identity: StaticNullIdentity {
                    runtime_chrome_budget_hash: None,
                    unexpected_missing_hash: None,
                },
                runtime_chrome_budget: None,
                projections: StaticNullProjections {
                    common_bank_footprint: StaticCommonBankFootprint {
                        shared_dense_ffn_bytes: None,
                    },
                    projected_bank_switches_per_token: StaticSwitchProjection {
                        expected_q16_16: None,
                    },
                    projected_sram_page_switches_per_token: StaticSwitchProjection {
                        expected_q16_16: None,
                    },
                },
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&unlisted_static_budget),
            Err(CanonicalJsonError::UnlistedNullField {
                schema: STATIC_BUDGET_SCHEMA.to_owned(),
                path: "identity.unexpected_missing_hash".to_owned()
            })
        );
    }

    #[test]
    fn policy_resolution_known_hash_paths_require_sha256_prefix() {
        let target_defaults_env = ReportEnvelope::new(
            ReportOutcome::Passed,
            PolicyHashPathsBody {
                result: Some(PolicyHashPathsResult {
                    provenance: PolicyHashPathsProvenance {
                        target_defaults:
                            "0000000000000000000000000000000000000000000000000000000000000000"
                                .to_owned(),
                        profile_defaults:
                            "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                                .to_owned(),
                    },
                }),
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&target_defaults_env),
            Err(CanonicalJsonError::NonCanonicalHashString {
                path: "result.provenance.target_defaults".to_owned(),
                value: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned()
            })
        );

        let profile_defaults_env = ReportEnvelope::new(
            ReportOutcome::Passed,
            PolicyHashPathsBody {
                result: Some(PolicyHashPathsResult {
                    provenance: PolicyHashPathsProvenance {
                        target_defaults:
                            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                                .to_owned(),
                        profile_defaults:
                            "1111111111111111111111111111111111111111111111111111111111111111"
                                .to_owned(),
                    },
                }),
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope");

        assert_eq!(
            canonicalize(&profile_defaults_env),
            Err(CanonicalJsonError::NonCanonicalHashString {
                path: "result.provenance.profile_defaults".to_owned(),
                value: "1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned()
            })
        );
    }

    #[test]
    fn report_envelope_rejects_noncanonical_hash_strings() {
        let json = concat!(
            "{\"alpha\":{\"a\":1,\"b\":[]},",
            "\"outcome\":\"Passed\",",
            "\"report_self_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",",
            "\"schema\":\"byte_stable_report.v1\",",
            "\"schema_version\":\"1.0.0\",",
            "\"zeta\":2}"
        );

        let error = serde_json::from_str::<ReportEnvelope<ByteStableBody>>(json)
            .expect_err("unprefixed hash must reject");
        assert!(error.to_string().contains("report_self_hash"), "{error}");
    }

    fn assert_unknown_body_field_rejected<R>(schema: &str, body_json: &str)
    where
        R: ReportBody + Serialize + DeserializeOwned + fmt::Debug,
    {
        let json = format!(
            "{{\"schema\":\"{schema}\",\"schema_version\":\"1.0.0\",\"outcome\":\"Passed\",\"report_self_hash\":\"{}\",{}}}",
            zero_hash_json(),
            body_json.trim_start_matches('{').trim_end_matches('}')
        );
        let error = serde_json::from_str::<ReportEnvelope<R>>(&json)
            .expect_err("unknown body field must reject");
        assert!(
            error.to_string().contains("unknown field `unexpected`"),
            "{error}"
        );
    }
}
