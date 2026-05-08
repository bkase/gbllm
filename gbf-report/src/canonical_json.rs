//! Shared F-B2/F-B4 report envelope, canonical JSON, and self-hash helpers.

use std::any::type_name;
use std::fmt;
use std::str::FromStr;

use gbf_foundation::{Hash256, SemVer, SemVerParseError};
use serde::de::DeserializeOwned;
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
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReportOutcome {
    Passed,
    Failed,
}

/// Diagnostic severity for F-B2/F-B4 semantic validation checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DiagnosticSeverity {
    Hard,
    Soft,
}

/// Minimal diagnostic carrier used by `ReportBody::validate_semantics`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
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
    let schema_version: SemVer = take_envelope_field(&mut map, SCHEMA_VERSION_FIELD)
        .and_then(|value| decode_envelope_field(SCHEMA_VERSION_FIELD, value))?;
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
    let body =
        serde_json::to_value(&env.body).map_err(|error| CanonicalJsonError::BodySerialization {
            reason: error.to_string(),
        })?;
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
        serde_json::to_value(env.schema_version).map_err(|error| {
            CanonicalJsonError::BodySerialization {
                reason: error.to_string(),
            }
        })?,
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
        short_type_name::<R>(),
        R::SCHEMA_ID,
        version
    )
    .into_bytes())
}

fn short_type_name<R>() -> &'static str {
    type_name::<R>()
        .rsplit("::")
        .next()
        .unwrap_or(type_name::<R>())
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

    field == "digest" || field == "sha256" || field.ends_with("_hash")
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

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn zero_hash_json() -> &'static str {
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
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
        const SCHEMA_ID: &'static str = POLICY_RESOLUTION_SCHEMA;
        const SCHEMA_VERSION: &'static str = "1.0.0";

        fn validate_semantics(
            &self,
            outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            if outcome == ReportOutcome::Passed && self.result.is_none() {
                return Err(vec![ValidationDiagnostic {
                    severity: DiagnosticSeverity::Hard,
                }]);
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
        const SCHEMA_ID: &'static str = STATIC_BUDGET_SCHEMA;
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
                "{{\"alpha\":{{\"a\":1,\"b\":[true,false]}},\"outcome\":{{\"kind\":\"Passed\"}},\"report_self_hash\":\"{}\",\"schema\":\"byte_stable_report.v1\",\"schema_version\":{{\"major\":1,\"minor\":0,\"patch\":0}},\"zeta\":2}}",
                zero_hash_json()
            )
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
            diagnostics: vec![ValidationDiagnostic {
                severity: DiagnosticSeverity::Soft,
            }],
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
    fn f_b2_f_b4_reports_reject_unlisted_null_fields() {
        let allowed = ReportEnvelope::new(
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
        canonicalize(&allowed).expect("listed null fields are accepted");

        let unlisted = ReportEnvelope::new(
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
            canonicalize(&unlisted),
            Err(CanonicalJsonError::UnlistedNullField {
                schema: ARTIFACT_VALIDATION_SCHEMA.to_owned(),
                path: "identity.unexpected_missing_hash".to_owned()
            })
        );
    }

    #[test]
    fn report_envelope_rejects_noncanonical_hash_strings() {
        let json = concat!(
            "{\"alpha\":{\"a\":1,\"b\":[]},",
            "\"outcome\":{\"kind\":\"Passed\"},",
            "\"report_self_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",",
            "\"schema\":\"byte_stable_report.v1\",",
            "\"schema_version\":{\"major\":1,\"minor\":0,\"patch\":0},",
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
            "{{\"schema\":\"{schema}\",\"schema_version\":{{\"major\":1,\"minor\":0,\"patch\":0}},\"outcome\":{{\"kind\":\"Passed\"}},\"report_self_hash\":\"{}\",{}}}",
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
