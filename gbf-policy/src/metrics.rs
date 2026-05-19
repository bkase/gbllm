//! Metric registry snapshots selected by Stage 4 observation planning.

use std::error::Error;
use std::fmt;

use gbf_foundation::{EvidenceRef, Hash256};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::{canonical_json_bytes, domain_hash};
use crate::probe::ProbeImportanceClass;

pub const METRIC_REGISTRY_VERSION: &str = "operational_probe_schema.v1";
pub const METRIC_REGISTRY_LOADED_EVENT: &str = "gbf_policy.metric_registry.loaded";
const METRIC_ID_MAX_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MetricId(String);

impl MetricId {
    pub fn from_static(s: &'static str) -> Result<Self, MetricIdError> {
        validate_metric_id(s)?;
        Ok(Self(s.to_owned()))
    }

    pub fn from_owned(s: String) -> Result<Self, MetricIdError> {
        validate_metric_id(&s)?;
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MetricId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for MetricId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MetricId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_owned(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricIdError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidChar { byte: u8, position: usize },
    LeadingDot,
    TrailingDot,
    DoubleDot { position: usize },
}

impl fmt::Display for MetricIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("metric id is empty"),
            Self::TooLong { len, max } => write!(f, "metric id length {len} exceeds max {max}"),
            Self::InvalidChar { byte, position } => {
                write!(
                    f,
                    "metric id byte 0x{byte:02x} at position {position} is invalid"
                )
            }
            Self::LeadingDot => f.write_str("metric id has a leading dot"),
            Self::TrailingDot => f.write_str("metric id has a trailing dot"),
            Self::DoubleDot { position } => {
                write!(f, "metric id has a double dot at {position}")
            }
        }
    }
}

impl Error for MetricIdError {}

fn validate_metric_id(s: &str) -> Result<(), MetricIdError> {
    if s.is_empty() {
        return Err(MetricIdError::Empty);
    }
    if s.len() > METRIC_ID_MAX_LEN {
        return Err(MetricIdError::TooLong {
            len: s.len(),
            max: METRIC_ID_MAX_LEN,
        });
    }
    if s.starts_with('.') {
        return Err(MetricIdError::LeadingDot);
    }
    if s.ends_with('.') {
        return Err(MetricIdError::TrailingDot);
    }

    let mut previous_dot = false;
    for (position, byte) in s.bytes().enumerate() {
        let valid =
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'.';
        if !valid {
            return Err(MetricIdError::InvalidChar { byte, position });
        }
        if byte == b'.' {
            if previous_dot {
                return Err(MetricIdError::DoubleDot { position });
            }
            previous_dot = true;
        } else {
            previous_dot = false;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum MetricSource {
    PerPass,
    PerToken,
    PerSliceReserved,
    PerBankSwitch,
    PerFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind")]
pub enum MetricAggregation {
    Sum,
    Mean,
    Max,
    Min,
    P50,
    P90,
    P99,
    Histogram { bucket_count: u8 },
}

impl MetricAggregation {
    pub fn validate(self) -> Result<(), MetricRegistryError> {
        match self {
            Self::Histogram { bucket_count: 0 } => {
                Err(MetricRegistryError::HistogramBucketCountZero)
            }
            _ => Ok(()),
        }
    }
}

impl<'de> Deserialize<'de> for MetricAggregation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        enum Repr {
            Sum,
            Mean,
            Max,
            Min,
            P50,
            P90,
            P99,
            Histogram { bucket_count: u8 },
        }

        let aggregation = match Repr::deserialize(deserializer)? {
            Repr::Sum => Self::Sum,
            Repr::Mean => Self::Mean,
            Repr::Max => Self::Max,
            Repr::Min => Self::Min,
            Repr::P50 => Self::P50,
            Repr::P90 => Self::P90,
            Repr::P99 => Self::P99,
            Repr::Histogram { bucket_count } => Self::Histogram { bucket_count },
        };
        aggregation.validate().map_err(serde::de::Error::custom)?;
        Ok(aggregation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricRegistryEntry {
    pub metric: MetricId,
    pub source: MetricSource,
    pub aggregation: MetricAggregation,
    pub importance: ProbeImportanceClass,
    pub weight: u16,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricRegistrySnapshot {
    pub entries: Vec<MetricRegistryEntry>,
}

impl MetricRegistrySnapshot {
    pub fn new(mut entries: Vec<MetricRegistryEntry>) -> Result<Self, MetricRegistryError> {
        if entries.is_empty() {
            return Err(MetricRegistryError::EmptySnapshot);
        }

        entries.sort_by(|left, right| left.metric.cmp(&right.metric));

        for entry in &entries {
            if entry.weight == 0 {
                return Err(MetricRegistryError::ZeroWeight {
                    metric: entry.metric.clone(),
                });
            }
            if entry.source == MetricSource::PerSliceReserved {
                return Err(MetricRegistryError::PerSliceReserved {
                    metric: entry.metric.clone(),
                });
            }
            entry.aggregation.validate()?;
        }

        for pair in entries.windows(2) {
            if pair[0].metric == pair[1].metric {
                return Err(MetricRegistryError::DuplicateMetricId {
                    metric: pair[0].metric.clone(),
                });
            }
        }

        Ok(Self { entries })
    }
}

impl<'de> Deserialize<'de> for MetricRegistrySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            entries: Vec<MetricRegistryEntry>,
        }

        Self::new(Repr::deserialize(deserializer)?.entries).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricRegistryError {
    EmptySnapshot,
    DuplicateMetricId { metric: MetricId },
    HistogramBucketCountZero,
    PerSliceReserved { metric: MetricId },
    ZeroWeight { metric: MetricId },
}

impl fmt::Display for MetricRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySnapshot => f.write_str("metric registry must contain at least one entry"),
            Self::DuplicateMetricId { metric } => write!(f, "duplicate metric id {metric}"),
            Self::HistogramBucketCountZero => {
                f.write_str("metric histogram bucket_count must be greater than zero")
            }
            Self::PerSliceReserved { metric } => {
                write!(
                    f,
                    "metric {metric} uses PerSliceReserved, which is reserved in v1"
                )
            }
            Self::ZeroWeight { metric } => write!(f, "metric {metric} has zero weight"),
        }
    }
}

impl Error for MetricRegistryError {}

pub fn metric_registry_hash(
    snapshot: &MetricRegistrySnapshot,
) -> Result<Hash256, serde_json::Error> {
    domain_hash(
        "gbf-policy",
        "MetricRegistry",
        METRIC_REGISTRY_VERSION,
        snapshot,
    )
}

pub fn emit_metric_registry_loaded(
    snapshot: &MetricRegistrySnapshot,
) -> Result<Hash256, serde_json::Error> {
    let hash = metric_registry_hash(snapshot)?;
    let hash_hex = hash.to_hex();
    let entries = u32::try_from(snapshot.entries.len())
        .expect("metric registry entry count fits telemetry u32");

    tracing::info!(
        event = METRIC_REGISTRY_LOADED_EVENT,
        entries = entries,
        hash = hash_hex.as_str(),
    );

    Ok(hash)
}

pub fn metric_registry_canonical_json_bytes(
    snapshot: &MetricRegistrySnapshot,
) -> Result<Vec<u8>, serde_json::Error> {
    canonical_json_bytes(snapshot)
}

pub fn metric_registry_v1() -> MetricRegistrySnapshot {
    MetricRegistrySnapshot::new(vec![
        metric_entry(
            "pass.count",
            MetricSource::PerPass,
            MetricAggregation::Sum,
            ProbeImportanceClass::Required,
            1,
        ),
        metric_entry(
            "token.latency",
            MetricSource::PerToken,
            MetricAggregation::Mean,
            ProbeImportanceClass::Important,
            2,
        ),
        metric_entry(
            "bank.switches",
            MetricSource::PerBankSwitch,
            MetricAggregation::Sum,
            ProbeImportanceClass::Diagnostic,
            2,
        ),
        metric_entry(
            "frame.bytes",
            MetricSource::PerFrame,
            MetricAggregation::Max,
            ProbeImportanceClass::BestEffort,
            1,
        ),
    ])
    .expect("v1 metric registry is sorted, unique, weighted, and v1-feasible")
}

pub fn load_metric_registry_v1() -> MetricRegistrySnapshot {
    let snapshot = metric_registry_v1();
    emit_metric_registry_loaded(&snapshot).expect("v1 metric registry telemetry emits");
    snapshot
}

fn metric_entry(
    metric: &'static str,
    source: MetricSource,
    aggregation: MetricAggregation,
    importance: ProbeImportanceClass,
    weight: u16,
) -> MetricRegistryEntry {
    MetricRegistryEntry {
        metric: MetricId::from_static(metric).expect("static metric id is valid"),
        source,
        aggregation,
        importance,
        weight,
        evidence: EvidenceRef {
            kind: "F-B6-F-B7".to_owned(),
            reference: format!("metrics/{metric}"),
            hash: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    #[test]
    fn metric_id_validates_dotted_grammar() {
        assert_eq!(
            MetricId::from_static("token.latency").unwrap().as_str(),
            "token.latency"
        );
        assert_eq!(MetricId::from_static(""), Err(MetricIdError::Empty));
        assert!(matches!(
            MetricId::from_static(".token"),
            Err(MetricIdError::LeadingDot)
        ));
        assert!(matches!(
            MetricId::from_static("token."),
            Err(MetricIdError::TrailingDot)
        ));
        assert!(matches!(
            MetricId::from_static("token..latency"),
            Err(MetricIdError::DoubleDot { .. })
        ));
        assert!(matches!(
            MetricId::from_static("Token"),
            Err(MetricIdError::InvalidChar { .. })
        ));
    }

    #[test]
    fn metric_registry_snapshot_serde_round_trip() {
        let snapshot = metric_registry_v1();
        let encoded = serde_json::to_string(&snapshot).expect("snapshot serializes");
        let decoded: MetricRegistrySnapshot =
            serde_json::from_str(&encoded).expect("snapshot deserializes");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn metric_registry_snapshot_domain_hash_deterministic() {
        let snapshot = metric_registry_v1();

        assert_eq!(
            metric_registry_hash(&snapshot).expect("hash computes"),
            metric_registry_hash(&snapshot).expect("hash recomputes")
        );
        assert_eq!(
            metric_registry_canonical_json_bytes(&snapshot).expect("canonical json computes"),
            metric_registry_canonical_json_bytes(&snapshot).expect("canonical json recomputes")
        );
    }

    #[test]
    fn metric_registry_loaded_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::callsite::rebuild_interest_cache();
        let snapshot = tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            load_metric_registry_v1()
        });
        tracing::callsite::rebuild_interest_cache();
        let hash = metric_registry_hash(&snapshot)
            .expect("hash computes")
            .to_hex();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str) == Some(METRIC_REGISTRY_LOADED_EVENT)
            })
            .expect("metric registry loaded event is captured");

        assert_eq!(event.fields.get("entries").map(String::as_str), Some("4"));
        assert_eq!(
            event.fields.get("hash").map(String::as_str),
            Some(hash.as_str())
        );
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn metric_registry_histogram_bucket_count_zero_is_rejected_at_load_time() {
        let value = serde_json::json!({
            "entries": [{
                "metric": "histogram.zero",
                "source": { "kind": "PerPass" },
                "aggregation": { "kind": "Histogram", "bucket_count": 0 },
                "importance": { "kind": "Diagnostic" },
                "weight": 1,
                "evidence": { "kind": "Fixture", "reference": "histogram.zero", "hash": null }
            }]
        });

        assert!(serde_json::from_value::<MetricRegistrySnapshot>(value).is_err());
    }

    #[test]
    fn metric_source_per_slice_reserved_is_rejected_at_load_time() {
        let entry = metric_entry(
            "slice.reserved",
            MetricSource::PerSliceReserved,
            MetricAggregation::Sum,
            ProbeImportanceClass::Diagnostic,
            1,
        );

        assert!(matches!(
            MetricRegistrySnapshot::new(vec![entry]),
            Err(MetricRegistryError::PerSliceReserved { .. })
        ));
    }

    #[test]
    fn metric_registry_rejects_empty_and_zero_weight() {
        assert!(matches!(
            MetricRegistrySnapshot::new(Vec::new()),
            Err(MetricRegistryError::EmptySnapshot)
        ));

        let entry = metric_entry(
            "weight.zero",
            MetricSource::PerPass,
            MetricAggregation::Sum,
            ProbeImportanceClass::Diagnostic,
            0,
        );
        assert!(matches!(
            MetricRegistrySnapshot::new(vec![entry]),
            Err(MetricRegistryError::ZeroWeight { .. })
        ));
    }

    #[test]
    fn metric_registry_v1_fixture_pins_canonical_json() {
        let snapshot = metric_registry_v1();
        let canonical = String::from_utf8(
            metric_registry_canonical_json_bytes(&snapshot).expect("canonical json computes"),
        )
        .expect("canonical json is utf-8");

        assert_eq!(
            canonical,
            include_str!("../fixtures/registries/metric_registry.v1.json").trim_end()
        );
    }

    #[derive(Clone, Default)]
    struct TraceCapture {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Debug)]
    struct CapturedEvent {
        fields: BTreeMap<String, String>,
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("capture lock is healthy")
                .push(CapturedEvent {
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }
    }
}
