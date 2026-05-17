//! Trace event payload layouts selected by policy registries.

use std::error::Error;
use std::fmt;

use gbf_foundation::{EvidenceRef, Hash256};
use serde::{Deserialize, Deserializer, Serialize};

use crate::canonical::{canonical_json_bytes, domain_hash};

pub const ABI_TRACE_EVENT_PAYLOAD_BYTES: u16 = 16;
pub const TRACE_EVENT_LAYOUT_REGISTRY_VERSION: &str = "operational_probe_schema.v1";
pub const TRACE_EVENT_LAYOUT_REGISTRY_LOADED_EVENT: &str =
    "gbf_policy.trace_event_layout_registry.loaded";

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceEventTupleSpecId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceEventPayloadLayout {
    Empty,
    U8,
    U16,
    U32,
    Q8_8,
    Q16_16,
    TokenId,
    ExpertId,
    /// Reserved in the v1 sealed registry until tuple specs get their own owner bead.
    Tuple {
        spec: TraceEventTupleSpecId,
    },
}

impl TraceEventPayloadLayout {
    #[must_use]
    pub const fn fixed_payload_bytes(&self) -> Option<u16> {
        match self {
            Self::Empty => Some(0),
            Self::U8 => Some(1),
            Self::U16 | Self::Q8_8 | Self::TokenId | Self::ExpertId => Some(2),
            Self::U32 | Self::Q16_16 => Some(4),
            Self::Tuple { .. } => None,
        }
    }

    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::U8 => "U8",
            Self::U16 => "U16",
            Self::U32 => "U32",
            Self::Q8_8 => "Q8_8",
            Self::Q16_16 => "Q16_16",
            Self::TokenId => "TokenId",
            Self::ExpertId => "ExpertId",
            Self::Tuple { .. } => "Tuple",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEventShape {
    pub payload_layout: TraceEventPayloadLayout,
    pub max_payload_bytes: u16,
    pub stable_id: TraceEventTupleSpecId,
}

impl TraceEventShape {
    pub fn new(
        payload_layout: TraceEventPayloadLayout,
        max_payload_bytes: u16,
        stable_id: TraceEventTupleSpecId,
    ) -> Result<Self, TraceEventShapeError> {
        if max_payload_bytes > ABI_TRACE_EVENT_PAYLOAD_BYTES {
            return Err(TraceEventShapeError::MaxPayloadBytesExceedsAbiSlot {
                max_payload_bytes,
                abi_slot_bytes: ABI_TRACE_EVENT_PAYLOAD_BYTES,
            });
        }
        if let Some(expected_payload_bytes) = payload_layout.fixed_payload_bytes()
            && max_payload_bytes != expected_payload_bytes
        {
            return Err(TraceEventShapeError::FixedPayloadBytesMismatch {
                layout: payload_layout.kind_name(),
                max_payload_bytes,
                expected_payload_bytes,
            });
        }

        Ok(Self {
            payload_layout,
            max_payload_bytes,
            stable_id,
        })
    }
}

impl<'de> Deserialize<'de> for TraceEventShape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            payload_layout: TraceEventPayloadLayout,
            max_payload_bytes: u16,
            stable_id: TraceEventTupleSpecId,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.payload_layout, repr.max_payload_bytes, repr.stable_id)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEventShapeError {
    MaxPayloadBytesExceedsAbiSlot {
        max_payload_bytes: u16,
        abi_slot_bytes: u16,
    },
    FixedPayloadBytesMismatch {
        layout: &'static str,
        max_payload_bytes: u16,
        expected_payload_bytes: u16,
    },
}

impl fmt::Display for TraceEventShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxPayloadBytesExceedsAbiSlot {
                max_payload_bytes,
                abi_slot_bytes,
            } => write!(
                f,
                "trace event payload declares {max_payload_bytes} bytes but ABI slot holds {abi_slot_bytes}"
            ),
            Self::FixedPayloadBytesMismatch {
                layout,
                max_payload_bytes,
                expected_payload_bytes,
            } => write!(
                f,
                "trace event {layout} payload declares {max_payload_bytes} bytes but requires {expected_payload_bytes}"
            ),
        }
    }
}

impl Error for TraceEventShapeError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEventLayoutEntry {
    pub shape: TraceEventShape,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEventLayoutRegistrySnapshot {
    pub entries: Vec<TraceEventLayoutEntry>,
}

impl TraceEventLayoutRegistrySnapshot {
    pub fn new(
        mut entries: Vec<TraceEventLayoutEntry>,
    ) -> Result<Self, TraceEventLayoutRegistryError> {
        if entries.is_empty() {
            return Err(TraceEventLayoutRegistryError::EmptySnapshot);
        }

        entries.sort_by(|left, right| left.shape.stable_id.cmp(&right.shape.stable_id));

        for entry in &entries {
            if matches!(
                entry.shape.payload_layout,
                TraceEventPayloadLayout::Tuple { .. }
            ) {
                return Err(TraceEventLayoutRegistryError::TuplePayloadLayoutReserved {
                    stable_id: entry.shape.stable_id.clone(),
                });
            }
        }

        for pair in entries.windows(2) {
            if pair[0].shape.stable_id == pair[1].shape.stable_id {
                return Err(TraceEventLayoutRegistryError::DuplicateStableId {
                    stable_id: pair[0].shape.stable_id.clone(),
                });
            }
        }

        Ok(Self { entries })
    }
}

impl<'de> Deserialize<'de> for TraceEventLayoutRegistrySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            entries: Vec<TraceEventLayoutEntry>,
        }

        Self::new(Repr::deserialize(deserializer)?.entries).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEventLayoutRegistryError {
    EmptySnapshot,
    DuplicateStableId { stable_id: TraceEventTupleSpecId },
    TuplePayloadLayoutReserved { stable_id: TraceEventTupleSpecId },
}

impl fmt::Display for TraceEventLayoutRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySnapshot => {
                f.write_str("trace event layout registry must contain at least one entry")
            }
            Self::DuplicateStableId { stable_id } => {
                write!(f, "duplicate trace event layout stable id {}", stable_id.0)
            }
            Self::TuplePayloadLayoutReserved { stable_id } => write!(
                f,
                "trace event layout {} uses Tuple, which is reserved in v1",
                stable_id.0
            ),
        }
    }
}

impl Error for TraceEventLayoutRegistryError {}

pub fn trace_event_layout_registry_hash(
    snapshot: &TraceEventLayoutRegistrySnapshot,
) -> Result<Hash256, serde_json::Error> {
    domain_hash(
        "gbf-policy",
        "TraceEventLayoutRegistry",
        TRACE_EVENT_LAYOUT_REGISTRY_VERSION,
        snapshot,
    )
}

pub fn emit_trace_event_layout_registry_loaded(
    snapshot: &TraceEventLayoutRegistrySnapshot,
) -> Result<Hash256, serde_json::Error> {
    let hash = trace_event_layout_registry_hash(snapshot)?;
    let hash_hex = hash.to_hex();
    let entries = u32::try_from(snapshot.entries.len())
        .expect("trace event layout registry entry count fits telemetry u32");

    tracing::info!(
        event = TRACE_EVENT_LAYOUT_REGISTRY_LOADED_EVENT,
        entries = entries,
        hash = hash_hex.as_str(),
    );

    Ok(hash)
}

pub fn trace_event_layout_registry_canonical_json_bytes(
    snapshot: &TraceEventLayoutRegistrySnapshot,
) -> Result<Vec<u8>, serde_json::Error> {
    canonical_json_bytes(snapshot)
}

pub fn trace_event_layout_registry_v1() -> TraceEventLayoutRegistrySnapshot {
    TraceEventLayoutRegistrySnapshot::new(vec![
        TraceEventLayoutEntry {
            shape: trace_shape("checkpoint.empty", TraceEventPayloadLayout::Empty, 0),
            evidence: registry_evidence("trace_event_layout/checkpoint.empty"),
        },
        TraceEventLayoutEntry {
            shape: trace_shape("effect.fault_u32", TraceEventPayloadLayout::U32, 4),
            evidence: registry_evidence("trace_event_layout/effect.fault_u32"),
        },
        TraceEventLayoutEntry {
            shape: trace_shape("op.counter_u16", TraceEventPayloadLayout::U16, 2),
            evidence: registry_evidence("trace_event_layout/op.counter_u16"),
        },
        TraceEventLayoutEntry {
            shape: trace_shape("value.q8_8", TraceEventPayloadLayout::Q8_8, 2),
            evidence: registry_evidence("trace_event_layout/value.q8_8"),
        },
    ])
    .expect("v1 trace event layout registry is unique, ABI-bounded, and v1-feasible")
}

pub fn load_trace_event_layout_registry_v1() -> TraceEventLayoutRegistrySnapshot {
    let snapshot = trace_event_layout_registry_v1();
    emit_trace_event_layout_registry_loaded(&snapshot)
        .expect("v1 trace event layout registry telemetry emits");
    snapshot
}

pub(crate) fn trace_shape(
    stable_id: &'static str,
    payload_layout: TraceEventPayloadLayout,
    max_payload_bytes: u16,
) -> TraceEventShape {
    TraceEventShape::new(
        payload_layout,
        max_payload_bytes,
        TraceEventTupleSpecId(stable_id.to_owned()),
    )
    .expect("static v1 trace event shape is ABI-bounded")
}

fn registry_evidence(reference: &'static str) -> EvidenceRef {
    EvidenceRef {
        kind: "F-B6-F-B7".to_owned(),
        reference: reference.to_owned(),
        hash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    #[test]
    fn trace_event_layout_registry_snapshot_serde_round_trip() {
        let snapshot = trace_event_layout_registry_v1();
        let encoded = serde_json::to_string(&snapshot).expect("snapshot serializes");
        let decoded: TraceEventLayoutRegistrySnapshot =
            serde_json::from_str(&encoded).expect("snapshot deserializes");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn trace_event_layout_registry_snapshot_domain_hash_deterministic() {
        let snapshot = trace_event_layout_registry_v1();

        assert_eq!(
            trace_event_layout_registry_hash(&snapshot).expect("hash computes"),
            trace_event_layout_registry_hash(&snapshot).expect("hash recomputes")
        );
        assert_eq!(
            trace_event_layout_registry_canonical_json_bytes(&snapshot)
                .expect("canonical json computes"),
            trace_event_layout_registry_canonical_json_bytes(&snapshot)
                .expect("canonical json recomputes")
        );
    }

    #[test]
    fn trace_event_layout_registry_loaded_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::callsite::rebuild_interest_cache();
        let snapshot = tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            load_trace_event_layout_registry_v1()
        });
        tracing::callsite::rebuild_interest_cache();
        let hash = trace_event_layout_registry_hash(&snapshot)
            .expect("hash computes")
            .to_hex();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str)
                    == Some(TRACE_EVENT_LAYOUT_REGISTRY_LOADED_EVENT)
            })
            .expect("trace event layout registry loaded event is captured");

        assert_eq!(event.fields.get("entries").map(String::as_str), Some("4"));
        assert_eq!(
            event.fields.get("hash").map(String::as_str),
            Some(hash.as_str())
        );
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn trace_event_shape_payload_bytes_within_abi_slot() {
        assert!(
            TraceEventShape::new(
                TraceEventPayloadLayout::Tuple {
                    spec: TraceEventTupleSpecId("too_large".to_owned()),
                },
                ABI_TRACE_EVENT_PAYLOAD_BYTES + 1,
                TraceEventTupleSpecId("too_large".to_owned()),
            )
            .is_err()
        );

        let value = serde_json::json!({
            "payload_layout": { "kind": "Tuple", "spec": "too_large" },
            "max_payload_bytes": ABI_TRACE_EVENT_PAYLOAD_BYTES + 1,
            "stable_id": "too_large"
        });
        assert!(serde_json::from_value::<TraceEventShape>(value).is_err());
    }

    #[test]
    fn trace_event_shape_fixed_layout_byte_sizes_are_enforced() {
        let cases = [
            (TraceEventPayloadLayout::Empty, 0),
            (TraceEventPayloadLayout::U8, 1),
            (TraceEventPayloadLayout::U16, 2),
            (TraceEventPayloadLayout::U32, 4),
            (TraceEventPayloadLayout::Q8_8, 2),
            (TraceEventPayloadLayout::Q16_16, 4),
            (TraceEventPayloadLayout::TokenId, 2),
            (TraceEventPayloadLayout::ExpertId, 2),
        ];

        for (layout, bytes) in cases {
            TraceEventShape::new(
                layout.clone(),
                bytes,
                TraceEventTupleSpecId(format!("shape.{}", layout.kind_name())),
            )
            .expect("fixed layout accepts exact byte count");

            assert!(matches!(
                TraceEventShape::new(
                    layout.clone(),
                    bytes + 1,
                    TraceEventTupleSpecId(format!("shape.{}.bad", layout.kind_name())),
                ),
                Err(TraceEventShapeError::FixedPayloadBytesMismatch { .. })
            ));
        }
    }

    #[test]
    fn trace_event_layout_registry_rejects_empty_and_reserved_tuple_layout() {
        assert!(matches!(
            TraceEventLayoutRegistrySnapshot::new(Vec::new()),
            Err(TraceEventLayoutRegistryError::EmptySnapshot)
        ));

        let stable_id = TraceEventTupleSpecId("tuple.reserved".to_owned());
        let shape = TraceEventShape::new(
            TraceEventPayloadLayout::Tuple {
                spec: stable_id.clone(),
            },
            8,
            stable_id,
        )
        .expect("tuple shapes are schema-reserved but ABI-bounded");
        assert!(matches!(
            TraceEventLayoutRegistrySnapshot::new(vec![TraceEventLayoutEntry {
                shape,
                evidence: registry_evidence("trace_event_layout/tuple.reserved"),
            }]),
            Err(TraceEventLayoutRegistryError::TuplePayloadLayoutReserved { .. })
        ));
    }

    #[test]
    fn trace_event_layout_registry_accepts_all_non_tuple_layout_variants() {
        let cases = [
            (TraceEventPayloadLayout::Empty, 0),
            (TraceEventPayloadLayout::U8, 1),
            (TraceEventPayloadLayout::U16, 2),
            (TraceEventPayloadLayout::U32, 4),
            (TraceEventPayloadLayout::Q8_8, 2),
            (TraceEventPayloadLayout::Q16_16, 4),
            (TraceEventPayloadLayout::TokenId, 2),
            (TraceEventPayloadLayout::ExpertId, 2),
        ];

        let entries = cases
            .into_iter()
            .map(|(layout, bytes)| {
                let stable_id = format!("layout.{}", layout.kind_name());
                TraceEventLayoutEntry {
                    shape: TraceEventShape::new(
                        layout,
                        bytes,
                        TraceEventTupleSpecId(stable_id.clone()),
                    )
                    .expect("non-tuple layout has exact v1 payload bytes"),
                    evidence: EvidenceRef {
                        kind: "Fixture".to_owned(),
                        reference: stable_id,
                        hash: None,
                    },
                }
            })
            .collect();

        TraceEventLayoutRegistrySnapshot::new(entries)
            .expect("every non-tuple layout variant is seeded or schema-covered in v1");
    }

    #[test]
    fn trace_event_layout_registry_v1_fixture_pins_canonical_json() {
        let snapshot = trace_event_layout_registry_v1();
        let canonical = String::from_utf8(
            trace_event_layout_registry_canonical_json_bytes(&snapshot)
                .expect("canonical json computes"),
        )
        .expect("canonical json is utf-8");

        assert_eq!(
            canonical,
            include_str!("../fixtures/registries/trace_event_layout_registry.v1.json").trim_end()
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
