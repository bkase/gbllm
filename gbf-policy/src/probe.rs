//! Policy-side probe governance types and sealed probe registry snapshots.

use std::error::Error;
use std::fmt;

use gbf_abi::SemanticCheckpointId;
use gbf_abi::trace::ProbeLevel;
use gbf_foundation::{EvidenceRef, Hash256};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::{canonical_json_bytes, domain_hash};
use crate::diagnostics::TraceProbeId;
use crate::trace_event_layout::{
    TraceEventLayoutRegistrySnapshot, TraceEventPayloadLayout, TraceEventShape,
    TraceEventTupleSpecId, trace_shape,
};

pub const PROBE_REGISTRY_VERSION: &str = "operational_probe_schema.v1";
pub const PROBE_REGISTRY_LOADED_EVENT: &str = "gbf_policy.probe_registry.loaded";

/// Build-time importance class used by Stage 4 probe governance.
///
/// This is intentionally separate from `gbf_abi::trace::ProbeBudgetClass`,
/// which describes runtime trace budget windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind")]
pub enum ProbeImportanceClass {
    Required,
    Important,
    Diagnostic,
    BestEffort,
}

impl ProbeImportanceClass {
    pub const ALL: [Self; 4] = [
        Self::Required,
        Self::Important,
        Self::Diagnostic,
        Self::BestEffort,
    ];
}

impl<'de> Deserialize<'de> for ProbeImportanceClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            kind: ProbeImportanceClassKind,
        }

        #[derive(Deserialize)]
        enum ProbeImportanceClassKind {
            Required,
            Important,
            Diagnostic,
            BestEffort,
        }

        match Repr::deserialize(deserializer)?.kind {
            ProbeImportanceClassKind::Required => Ok(Self::Required),
            ProbeImportanceClassKind::Important => Ok(Self::Important),
            ProbeImportanceClassKind::Diagnostic => Ok(Self::Diagnostic),
            ProbeImportanceClassKind::BestEffort => Ok(Self::BestEffort),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProbeTiming {
    PreEntry,
    PostEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum InferOpTag {
    Classify,
    CombineResidual,
    DecodeToken,
    Embedding,
    ExpertMatVec,
    FfnActivation,
    Norm,
    RouteTop1,
    RouterMatVec,
    SelectExpertTop1,
    SequenceRead,
    SequenceStep,
    SequenceWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EffectClass {
    FaultBoundary,
    Rng,
    SequenceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueRole {
    Activation,
    DecodedToken,
    EmbeddingOutput,
    ExpertCandidate,
    ExpertIntermediate,
    ExpertOutput,
    GateWeight,
    InputToken,
    LogitVector,
    NormalizedActivation,
    RouterDecision,
    RouterScore,
    SequenceBlockOutput,
    SequenceStateNext,
    SequenceStateRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum ProbeSourceSelector {
    ByAnchorCheckpoint {
        checkpoint: SemanticCheckpointId,
        timing: ProbeTiming,
    },
    ByInferOpTag {
        op_tag: InferOpTag,
        timing: ProbeTiming,
    },
    ByEffectClass {
        class: EffectClass,
    },
    ByValueRole {
        role: ValueRole,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceFrequencyBound {
    PerPass { max_events: u32 },
    PerToken { max_events_per_token: u32 },
    PerNodeExecution { max_events_per_execution: u32 },
    PerFrame { max_events_per_frame: u32 },
    FaultOnly { max_events_per_frame: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRegistryEntry {
    pub probe_id: TraceProbeId,
    pub source_selector: ProbeSourceSelector,
    #[serde(with = "probe_level_json")]
    pub level: ProbeLevel,
    pub importance: ProbeImportanceClass,
    pub event_shape: TraceEventShape,
    pub frequency_bound: TraceFrequencyBound,
    pub weight: u16,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRegistrySnapshot {
    pub entries: Vec<ProbeRegistryEntry>,
}

impl ProbeRegistrySnapshot {
    pub fn new(mut entries: Vec<ProbeRegistryEntry>) -> Result<Self, ProbeRegistryError> {
        if entries.is_empty() {
            return Err(ProbeRegistryError::EmptySnapshot);
        }

        entries.sort_by_key(|entry| entry.probe_id);

        for entry in &entries {
            if entry.weight == 0 {
                return Err(ProbeRegistryError::ZeroWeight {
                    probe_id: entry.probe_id,
                });
            }
        }

        for pair in entries.windows(2) {
            if pair[0].probe_id == pair[1].probe_id {
                return Err(ProbeRegistryError::DuplicateTraceProbeId {
                    probe_id: pair[0].probe_id,
                });
            }
        }

        Ok(Self { entries })
    }
}

impl<'de> Deserialize<'de> for ProbeRegistrySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            entries: Vec<ProbeRegistryEntry>,
        }

        Self::new(Repr::deserialize(deserializer)?.entries).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeRegistryError {
    EmptySnapshot,
    DuplicateTraceProbeId {
        probe_id: TraceProbeId,
    },
    ZeroWeight {
        probe_id: TraceProbeId,
    },
    MissingTraceEventLayout {
        probe_id: TraceProbeId,
        stable_id: TraceEventTupleSpecId,
    },
    TraceEventLayoutMismatch {
        probe_id: TraceProbeId,
        stable_id: TraceEventTupleSpecId,
    },
}

impl fmt::Display for ProbeRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySnapshot => f.write_str("probe registry must contain at least one entry"),
            Self::DuplicateTraceProbeId { probe_id } => {
                write!(f, "duplicate trace probe id {}", probe_id.0)
            }
            Self::ZeroWeight { probe_id } => {
                write!(f, "probe registry entry {} has zero weight", probe_id.0)
            }
            Self::MissingTraceEventLayout {
                probe_id,
                stable_id,
            } => write!(
                f,
                "probe registry entry {} references missing trace event layout {}",
                probe_id.0, stable_id.0
            ),
            Self::TraceEventLayoutMismatch {
                probe_id,
                stable_id,
            } => write!(
                f,
                "probe registry entry {} does not match trace event layout {}",
                probe_id.0, stable_id.0
            ),
        }
    }
}

impl Error for ProbeRegistryError {}

pub fn probe_registry_hash(snapshot: &ProbeRegistrySnapshot) -> Result<Hash256, serde_json::Error> {
    domain_hash(
        "gbf-policy",
        "ProbeRegistry",
        PROBE_REGISTRY_VERSION,
        snapshot,
    )
}

pub fn emit_probe_registry_loaded(
    snapshot: &ProbeRegistrySnapshot,
) -> Result<Hash256, serde_json::Error> {
    let hash = probe_registry_hash(snapshot)?;
    let hash_hex = hash.to_hex();
    let entries = u32::try_from(snapshot.entries.len())
        .expect("probe registry entry count fits telemetry u32");

    tracing::info!(
        event = PROBE_REGISTRY_LOADED_EVENT,
        entries = entries,
        hash = hash_hex.as_str(),
    );

    Ok(hash)
}

pub fn probe_registry_canonical_json_bytes(
    snapshot: &ProbeRegistrySnapshot,
) -> Result<Vec<u8>, serde_json::Error> {
    canonical_json_bytes(snapshot)
}

/// Ensures every probe shape is backed by the sealed trace-event layout registry.
///
/// Stage 4 may copy the probe-local `event_shape`, but the stable-id/layout pair
/// must stay cross-registry-consistent so K4 binds the same payload contract that
/// downstream trace encoding uses.
pub fn validate_probe_registry_event_shapes(
    probes: &ProbeRegistrySnapshot,
    layouts: &TraceEventLayoutRegistrySnapshot,
) -> Result<(), ProbeRegistryError> {
    for probe in &probes.entries {
        let Some(layout) = layouts
            .entries
            .iter()
            .find(|entry| entry.shape.stable_id == probe.event_shape.stable_id)
        else {
            return Err(ProbeRegistryError::MissingTraceEventLayout {
                probe_id: probe.probe_id,
                stable_id: probe.event_shape.stable_id.clone(),
            });
        };

        if layout.shape.payload_layout != probe.event_shape.payload_layout
            || layout.shape.max_payload_bytes != probe.event_shape.max_payload_bytes
        {
            return Err(ProbeRegistryError::TraceEventLayoutMismatch {
                probe_id: probe.probe_id,
                stable_id: probe.event_shape.stable_id.clone(),
            });
        }
    }

    Ok(())
}

pub fn probe_registry_v1() -> ProbeRegistrySnapshot {
    ProbeRegistrySnapshot::new(vec![
        ProbeRegistryEntry {
            probe_id: TraceProbeId(1),
            source_selector: ProbeSourceSelector::ByAnchorCheckpoint {
                checkpoint: SemanticCheckpointId::from_static("embedding.post")
                    .expect("static checkpoint id is valid"),
                timing: ProbeTiming::PostEntry,
            },
            level: ProbeLevel::Always,
            importance: ProbeImportanceClass::Required,
            event_shape: trace_shape("checkpoint.empty", TraceEventPayloadLayout::Empty, 0),
            frequency_bound: TraceFrequencyBound::PerToken {
                max_events_per_token: 1,
            },
            weight: 1,
            evidence: registry_evidence("probe/by_anchor_checkpoint"),
        },
        ProbeRegistryEntry {
            probe_id: TraceProbeId(2),
            source_selector: ProbeSourceSelector::ByInferOpTag {
                op_tag: InferOpTag::RouterMatVec,
                timing: ProbeTiming::PostEntry,
            },
            level: ProbeLevel::Always,
            importance: ProbeImportanceClass::Important,
            event_shape: trace_shape("op.counter_u16", TraceEventPayloadLayout::U16, 2),
            frequency_bound: TraceFrequencyBound::PerNodeExecution {
                max_events_per_execution: 1,
            },
            weight: 2,
            evidence: registry_evidence("probe/by_infer_op_tag"),
        },
        ProbeRegistryEntry {
            probe_id: TraceProbeId(3),
            source_selector: ProbeSourceSelector::ByEffectClass {
                class: EffectClass::FaultBoundary,
            },
            level: ProbeLevel::OnError,
            importance: ProbeImportanceClass::Diagnostic,
            event_shape: trace_shape("effect.fault_u32", TraceEventPayloadLayout::U32, 4),
            frequency_bound: TraceFrequencyBound::FaultOnly {
                max_events_per_frame: 1,
            },
            weight: 2,
            evidence: registry_evidence("probe/by_effect_class"),
        },
        ProbeRegistryEntry {
            probe_id: TraceProbeId(4),
            source_selector: ProbeSourceSelector::ByValueRole {
                role: ValueRole::LogitVector,
            },
            level: ProbeLevel::Verbose,
            importance: ProbeImportanceClass::BestEffort,
            event_shape: trace_shape("value.q8_8", TraceEventPayloadLayout::Q8_8, 2),
            frequency_bound: TraceFrequencyBound::PerPass { max_events: 1 },
            weight: 1,
            evidence: registry_evidence("probe/by_value_role"),
        },
    ])
    .expect("v1 probe registry is unique and weighted")
}

pub fn load_probe_registry_v1() -> ProbeRegistrySnapshot {
    let snapshot = probe_registry_v1();
    emit_probe_registry_loaded(&snapshot).expect("v1 probe registry telemetry emits");
    snapshot
}

fn registry_evidence(reference: &'static str) -> EvidenceRef {
    EvidenceRef {
        kind: "F-B6-F-B7".to_owned(),
        reference: reference.to_owned(),
        hash: None,
    }
}

mod probe_level_json {
    use super::*;

    pub fn serialize<S>(level: &ProbeLevel, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Repr {
            kind: ProbeLevelKind,
        }

        Repr {
            kind: ProbeLevelKind::from(*level),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ProbeLevel, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            kind: ProbeLevelKind,
        }

        Ok(Repr::deserialize(deserializer)?.kind.into())
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    enum ProbeLevelKind {
        Always,
        OnError,
        Verbose,
    }

    impl From<ProbeLevel> for ProbeLevelKind {
        fn from(value: ProbeLevel) -> Self {
            match value {
                ProbeLevel::Always => Self::Always,
                ProbeLevel::OnError => Self::OnError,
                ProbeLevel::Verbose => Self::Verbose,
            }
        }
    }

    impl From<ProbeLevelKind> for ProbeLevel {
        fn from(value: ProbeLevelKind) -> Self {
            match value {
                ProbeLevelKind::Always => Self::Always,
                ProbeLevelKind::OnError => Self::OnError,
                ProbeLevelKind::Verbose => Self::Verbose,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_event_layout::trace_event_layout_registry_v1;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    #[test]
    fn probe_importance_class_total_ordering() {
        assert_eq!(
            ProbeImportanceClass::ALL,
            [
                ProbeImportanceClass::Required,
                ProbeImportanceClass::Important,
                ProbeImportanceClass::Diagnostic,
                ProbeImportanceClass::BestEffort,
            ]
        );
        assert!(ProbeImportanceClass::Required < ProbeImportanceClass::Important);
        assert!(ProbeImportanceClass::Important < ProbeImportanceClass::Diagnostic);
        assert!(ProbeImportanceClass::Diagnostic < ProbeImportanceClass::BestEffort);
    }

    #[test]
    fn probe_importance_class_serde_round_trip() {
        for class in ProbeImportanceClass::ALL {
            let encoded = serde_json::to_string(&class).expect("class serializes");
            let decoded: ProbeImportanceClass =
                serde_json::from_str(&encoded).expect("class deserializes");

            assert_eq!(decoded, class);
        }

        assert_eq!(
            serde_json::to_string(&ProbeImportanceClass::Required)
                .expect("class serializes canonically"),
            r#"{"kind":"Required"}"#
        );
    }

    #[test]
    fn probe_importance_class_public_json_shapes_are_pinned() {
        let cases = [
            (ProbeImportanceClass::Required, "Required"),
            (ProbeImportanceClass::Important, "Important"),
            (ProbeImportanceClass::Diagnostic, "Diagnostic"),
            (ProbeImportanceClass::BestEffort, "BestEffort"),
        ];

        for (class, kind) in cases {
            let expected = serde_json::json!({ "kind": kind });
            assert_eq!(
                serde_json::to_value(class).expect("class serializes"),
                expected
            );
            assert_eq!(
                serde_json::from_value::<ProbeImportanceClass>(expected)
                    .expect("class deserializes"),
                class
            );
        }
    }

    #[test]
    fn probe_importance_class_serde_rejects_unknown() {
        assert!(
            serde_json::from_value::<ProbeImportanceClass>(
                serde_json::json!({"kind": "Experimental"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProbeImportanceClass>(
                serde_json::json!({"kind": "Required", "unexpected": true})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<ProbeImportanceClass>(serde_json::json!({})).is_err());
        assert!(
            serde_json::from_value::<ProbeImportanceClass>(serde_json::json!("Required")).is_err()
        );
        assert!(
            serde_json::from_value::<ProbeImportanceClass>(serde_json::json!({"kind": "required"}))
                .is_err()
        );
    }

    #[test]
    fn probe_registry_snapshot_serde_round_trip() {
        let snapshot = probe_registry_v1();
        let encoded = serde_json::to_string(&snapshot).expect("snapshot serializes");
        let decoded: ProbeRegistrySnapshot =
            serde_json::from_str(&encoded).expect("snapshot deserializes");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn probe_registry_snapshot_domain_hash_deterministic() {
        let snapshot = probe_registry_v1();

        assert_eq!(
            probe_registry_hash(&snapshot).expect("hash computes"),
            probe_registry_hash(&snapshot).expect("hash recomputes")
        );
        assert_eq!(
            probe_registry_canonical_json_bytes(&snapshot).expect("canonical json computes"),
            probe_registry_canonical_json_bytes(&snapshot).expect("canonical json recomputes")
        );
    }

    #[test]
    fn probe_registry_loaded_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::callsite::rebuild_interest_cache();
        let snapshot = tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            load_probe_registry_v1()
        });
        tracing::callsite::rebuild_interest_cache();
        let hash = probe_registry_hash(&snapshot)
            .expect("hash computes")
            .to_hex();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str) == Some(PROBE_REGISTRY_LOADED_EVENT)
            })
            .expect("probe registry loaded event is captured");

        assert_eq!(event.fields.get("entries").map(String::as_str), Some("4"));
        assert_eq!(
            event.fields.get("hash").map(String::as_str),
            Some(hash.as_str())
        );
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn probe_registry_level_public_json_shape_is_pinned() {
        let value =
            serde_json::to_value(probe_registry_v1().entries.remove(0)).expect("entry serializes");

        assert_eq!(
            value.get("level"),
            Some(&serde_json::json!({ "kind": "Always" }))
        );
        assert!(
            serde_json::from_value::<ProbeRegistryEntry>(serde_json::json!({
                "probe_id": 99,
                "source_selector": {
                    "kind": "ByValueRole",
                    "role": { "kind": "LogitVector" }
                },
                "level": "Always",
                "importance": { "kind": "Required" },
                "event_shape": {
                    "payload_layout": { "kind": "Empty" },
                    "max_payload_bytes": 0,
                    "stable_id": "checkpoint.empty"
                },
                "frequency_bound": { "kind": "PerPass", "max_events": 1 },
                "weight": 1,
                "evidence": { "kind": "Fixture", "reference": "probe.level", "hash": null }
            }))
            .is_err()
        );
    }

    #[test]
    fn probe_registry_rejects_empty_and_zero_weight() {
        assert!(matches!(
            ProbeRegistrySnapshot::new(Vec::new()),
            Err(ProbeRegistryError::EmptySnapshot)
        ));

        let mut entry = probe_registry_v1().entries.remove(0);
        entry.weight = 0;
        assert!(matches!(
            ProbeRegistrySnapshot::new(vec![entry]),
            Err(ProbeRegistryError::ZeroWeight { .. })
        ));
    }

    #[test]
    fn probe_registry_event_shapes_match_trace_layout_registry() {
        let probes = probe_registry_v1();
        let layouts = trace_event_layout_registry_v1();

        validate_probe_registry_event_shapes(&probes, &layouts)
            .expect("v1 probe shapes match the v1 trace layout registry");

        let mut mismatched = layouts.clone();
        mismatched.entries[0].shape.max_payload_bytes += 1;
        assert!(matches!(
            validate_probe_registry_event_shapes(&probes, &mismatched),
            Err(ProbeRegistryError::TraceEventLayoutMismatch { .. })
        ));

        let mut missing = layouts;
        missing.entries.remove(0);
        assert!(matches!(
            validate_probe_registry_event_shapes(&probes, &missing),
            Err(ProbeRegistryError::MissingTraceEventLayout { .. })
        ));
    }

    #[test]
    fn probe_registry_entry_per_selector_variant() {
        let snapshot = probe_registry_v1();

        assert!(snapshot.entries.iter().any(|entry| matches!(
            entry.source_selector,
            ProbeSourceSelector::ByAnchorCheckpoint { .. }
        )));
        assert!(snapshot.entries.iter().any(|entry| matches!(
            entry.source_selector,
            ProbeSourceSelector::ByInferOpTag { .. }
        )));
        assert!(snapshot.entries.iter().any(|entry| matches!(
            entry.source_selector,
            ProbeSourceSelector::ByEffectClass { .. }
        )));
        assert!(snapshot.entries.iter().any(|entry| matches!(
            entry.source_selector,
            ProbeSourceSelector::ByValueRole { .. }
        )));
    }

    #[test]
    fn probe_registry_versioning_story() {
        let snapshot = probe_registry_v1();
        let mut amended = snapshot.clone();
        amended.entries[0].evidence.reference.push_str(".amended");

        assert_eq!(
            probe_registry_hash(&snapshot).expect("hash computes"),
            probe_registry_hash(&snapshot).expect("hash recomputes")
        );
        assert_ne!(
            probe_registry_hash(&snapshot).expect("hash computes"),
            probe_registry_hash(&amended).expect("amended hash computes")
        );
    }

    #[test]
    fn probe_registry_v1_fixture_pins_canonical_json() {
        let snapshot = probe_registry_v1();
        let canonical = String::from_utf8(
            probe_registry_canonical_json_bytes(&snapshot).expect("canonical json computes"),
        )
        .expect("canonical json is utf-8");

        assert_eq!(
            canonical,
            include_str!("../fixtures/registries/probe_registry.v1.json").trim_end()
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
