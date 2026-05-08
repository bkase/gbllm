//! Flat report envelope and self-hash convention for F-B2/F-B4 reports.

use std::fmt;

use gbf_foundation::{Hash256, SemVer};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::canonical_json::{self, CanonicalJsonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReportOutcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEnvelope<R> {
    pub schema: String,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}

impl<R: ReportBody> ReportEnvelope<R> {
    #[must_use]
    pub fn new(outcome: ReportOutcome, body: R) -> Self {
        Self {
            schema: R::SCHEMA_ID.to_owned(),
            schema_version: R::SCHEMA_VERSION,
            outcome,
            report_self_hash: Hash256::ZERO,
            body,
        }
    }

    pub fn validate_semantics(&self) -> Result<(), Vec<R::Diagnostic>> {
        self.body.validate_semantics(self.outcome)
    }
}

impl<R> Serialize for ReportEnvelope<R>
where
    R: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let body = serde_json::to_value(&self.body).map_err(serde::ser::Error::custom)?;
        let Value::Object(body) = body else {
            return Err(serde::ser::Error::custom(
                "report body must serialize to a JSON object",
            ));
        };

        let mut merged = Map::new();
        insert_envelope_field(&mut merged, "schema", &self.schema)
            .map_err(serde::ser::Error::custom)?;
        insert_envelope_field(&mut merged, "schema_version", &self.schema_version)
            .map_err(serde::ser::Error::custom)?;
        insert_envelope_field(&mut merged, "outcome", &self.outcome)
            .map_err(serde::ser::Error::custom)?;
        insert_envelope_field(&mut merged, "report_self_hash", &self.report_self_hash)
            .map_err(serde::ser::Error::custom)?;
        for (key, value) in body {
            if merged.insert(key.clone(), value).is_some() {
                return Err(serde::ser::Error::custom(format!(
                    "report body duplicates envelope field {key}"
                )));
            }
        }

        Value::Object(merged).serialize(serializer)
    }
}

impl<'de, R> Deserialize<'de> for ReportEnvelope<R>
where
    R: ReportBody + DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(mut object) = value else {
            return Err(serde::de::Error::custom(
                "report envelope must be an object",
            ));
        };

        let schema: String = take_field(&mut object, "schema").map_err(serde::de::Error::custom)?;
        if schema != R::SCHEMA_ID {
            return Err(serde::de::Error::custom(format!(
                "expected schema {}, got {schema}",
                R::SCHEMA_ID
            )));
        }

        let schema_version: SemVer =
            take_field(&mut object, "schema_version").map_err(serde::de::Error::custom)?;
        if schema_version != R::SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "expected schema_version {}, got {schema_version}",
                R::SCHEMA_VERSION
            )));
        }

        let outcome = take_field(&mut object, "outcome").map_err(serde::de::Error::custom)?;
        let report_self_hash =
            take_field(&mut object, "report_self_hash").map_err(serde::de::Error::custom)?;
        let body = R::deserialize(Value::Object(object)).map_err(serde::de::Error::custom)?;

        Ok(Self {
            schema,
            schema_version,
            outcome,
            report_self_hash,
            body,
        })
    }
}

pub trait ReportBody: Sized + Serialize {
    type Diagnostic;

    const SCHEMA_ID: &'static str;
    const SCHEMA_VERSION: SemVer;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<Self::Diagnostic>>;
}

pub fn canonicalize<R>(env: &ReportEnvelope<R>) -> Result<Vec<u8>, ReportEnvelopeError>
where
    R: Serialize,
{
    canonical_json::canonicalize(env).map_err(ReportEnvelopeError::CanonicalJson)
}

pub fn compute_self_hash<R>(env: &ReportEnvelope<R>) -> Result<Hash256, ReportEnvelopeError>
where
    R: Clone + Serialize,
{
    let mut hashable = env.clone();
    hashable.report_self_hash = Hash256::ZERO;
    let canonical = canonicalize(&hashable)?;
    let mut hasher = Sha256::new();
    hasher.update(b"gbf-report/report-envelope/self-hash/v1\0");
    hasher.update(&canonical);
    Ok(Hash256::from_bytes(hasher.finalize().into()))
}

pub fn round_trip_self_hash<R>(env: &ReportEnvelope<R>) -> Result<(), ReportEnvelopeError>
where
    R: Clone + Serialize + DeserializeOwned + ReportBody,
{
    let stored = env.report_self_hash;
    let computed = compute_self_hash(env)?;
    if stored != computed {
        return Err(ReportEnvelopeError::SelfHashMismatch { stored, computed });
    }

    let canonical = canonicalize(env)?;
    let parsed: ReportEnvelope<R> =
        serde_json::from_slice(&canonical).map_err(ReportEnvelopeError::Deserialize)?;
    let reparsed = compute_self_hash(&parsed)?;
    if stored != reparsed {
        return Err(ReportEnvelopeError::SelfHashMismatch {
            stored,
            computed: reparsed,
        });
    }

    Ok(())
}

fn insert_envelope_field<T: Serialize>(
    object: &mut Map<String, Value>,
    key: &'static str,
    value: &T,
) -> Result<(), serde_json::Error> {
    object.insert(key.to_owned(), serde_json::to_value(value)?);
    Ok(())
}

fn take_field<T: DeserializeOwned>(
    object: &mut Map<String, Value>,
    key: &'static str,
) -> Result<T, serde_json::Error> {
    let value = object
        .remove(key)
        .ok_or_else(|| serde::de::Error::custom(format!("missing report envelope field {key}")))?;
    T::deserialize(value)
}

#[derive(Debug)]
pub enum ReportEnvelopeError {
    CanonicalJson(CanonicalJsonError),
    Deserialize(serde_json::Error),
    SelfHashMismatch { stored: Hash256, computed: Hash256 },
}

impl fmt::Display for ReportEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Deserialize(error) => write!(f, "failed to deserialize report: {error}"),
            Self::SelfHashMismatch { stored, computed } => {
                write!(
                    f,
                    "report_self_hash sha256:{stored} does not match sha256:{computed}"
                )
            }
        }
    }
}

impl std::error::Error for ReportEnvelopeError {}
