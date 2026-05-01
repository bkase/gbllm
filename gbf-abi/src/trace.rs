//! Trace event layout, probe metadata, and trace budget types.

use core::fmt;
#[cfg(test)]
use core::mem::{align_of, size_of};

#[cfg(test)]
use memoffset::offset_of;
use serde::{Deserialize, Serialize};

use crate::checkpoint::CompactCheckpointId;
use crate::interrupt::SliceId;
#[cfg(feature = "host")]
use crate::version::{AbiVersion, BuildIdentityBlock};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TraceProbeId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeLevel {
    Always,
    OnError,
    Verbose,
}

impl ProbeLevel {
    pub const ALL: [Self; 3] = [Self::Always, Self::OnError, Self::Verbose];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum ProbeBudgetClass {
    PerSlice,
    PerFrame,
    PerSession,
}

impl ProbeBudgetClass {
    pub const ALL: [Self; 3] = [Self::PerSlice, Self::PerFrame, Self::PerSession];
}

/// Fixed 32-byte trace event slot.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub seq: u32,
    pub timestamp_m_cycles: u32,
    pub slice: SliceId,
    pub probe: TraceProbeId,
    pub checkpoint: CompactCheckpointId,
    pub data: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBudget {
    pub max_events_per_slice: u16,
    pub max_bytes_per_frame: u16,
    pub drop_policy: TraceDropPolicy,
}

impl TraceBudget {
    pub fn new(
        max_events_per_slice: u16,
        max_bytes_per_frame: u16,
        drop_policy: TraceDropPolicy,
    ) -> Result<Self, TraceBudgetError> {
        match (max_events_per_slice == 0, max_bytes_per_frame == 0) {
            (true, false) => Err(TraceBudgetError::ZeroEventsWithNonzeroBytes),
            (false, true) => Err(TraceBudgetError::NonzeroEventsWithZeroBytes),
            _ => Ok(Self {
                max_events_per_slice,
                max_bytes_per_frame,
                drop_policy,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBudgetError {
    ZeroEventsWithNonzeroBytes,
    NonzeroEventsWithZeroBytes,
}

impl fmt::Display for TraceBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroEventsWithNonzeroBytes => {
                f.write_str("trace budget cannot allow bytes when events are zero")
            }
            Self::NonzeroEventsWithZeroBytes => {
                f.write_str("trace budget cannot allow events when bytes are zero")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TraceBudgetError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndFault,
}

impl TraceDropPolicy {
    pub const ALL: [Self; 3] = [Self::DropOldest, Self::DropNewest, Self::HaltAndFault];
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceProbeRegistry {
    pub abi_version: AbiVersion,
    pub build_hash: [u8; 32],
    pub probes: alloc::vec::Vec<TraceProbeEntry>,
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceProbeEntry {
    pub probe: TraceProbeId,
    pub level: ProbeLevel,
    pub budget_class: ProbeBudgetClass,
    pub payload_schema_tag: alloc::borrow::Cow<'static, str>,
}

#[cfg(feature = "host")]
impl TraceProbeRegistry {
    fn validate_unique_probe_ids(&self) -> Result<(), TraceProbeRegistryError> {
        let mut seen = alloc::collections::BTreeSet::new();
        for entry in &self.probes {
            if !seen.insert(entry.probe) {
                return Err(TraceProbeRegistryError::DuplicateProbe { probe: entry.probe });
            }
        }

        Ok(())
    }

    pub fn validate_against_identity(
        &self,
        identity: &BuildIdentityBlock,
    ) -> Result<(), TraceProbeRegistryError> {
        self.validate_unique_probe_ids()?;
        if self.abi_version != identity.abi {
            return Err(TraceProbeRegistryError::AbiVersionMismatch {
                expected: identity.abi,
                observed: self.abi_version,
            });
        }
        if self.build_hash != identity.build_hash {
            return Err(TraceProbeRegistryError::BuildHashMismatch {
                expected: identity.build_hash,
                observed: self.build_hash,
            });
        }

        Ok(())
    }
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceProbeRegistryError {
    DuplicateProbe {
        probe: TraceProbeId,
    },
    BuildHashMismatch {
        expected: [u8; 32],
        observed: [u8; 32],
    },
    AbiVersionMismatch {
        expected: AbiVersion,
        observed: AbiVersion,
    },
}

#[cfg(feature = "host")]
impl fmt::Display for TraceProbeRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateProbe { probe } => write!(f, "duplicate trace probe id {}", probe.0),
            Self::BuildHashMismatch { .. } => f.write_str("trace registry build hash mismatch"),
            Self::AbiVersionMismatch { expected, observed } => write!(
                f,
                "trace registry ABI mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

#[cfg(all(feature = "host", feature = "std"))]
impl std::error::Error for TraceProbeRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_layout() {
        assert_eq!(size_of::<TraceEvent>(), 32);
        assert_eq!(align_of::<TraceEvent>(), 4);
        assert_eq!(offset_of!(TraceEvent, seq), 0);
        assert_eq!(offset_of!(TraceEvent, timestamp_m_cycles), 4);
        assert_eq!(offset_of!(TraceEvent, slice), 8);
        assert_eq!(offset_of!(TraceEvent, probe), 12);
        assert_eq!(offset_of!(TraceEvent, checkpoint), 14);
        assert_eq!(offset_of!(TraceEvent, data), 16);
    }

    #[test]
    fn probe_level_exhaustive() {
        assert_eq!(ProbeLevel::ALL.len(), 3);
        assert!(ProbeLevel::ALL.contains(&ProbeLevel::Always));
        assert!(ProbeLevel::ALL.contains(&ProbeLevel::OnError));
        assert!(ProbeLevel::ALL.contains(&ProbeLevel::Verbose));
    }

    #[test]
    fn probe_budget_class_exhaustive() {
        assert_eq!(ProbeBudgetClass::ALL.len(), 3);
        assert!(ProbeBudgetClass::ALL.contains(&ProbeBudgetClass::PerSlice));
        assert!(ProbeBudgetClass::ALL.contains(&ProbeBudgetClass::PerFrame));
        assert!(ProbeBudgetClass::ALL.contains(&ProbeBudgetClass::PerSession));
    }

    #[test]
    fn drop_policy_exhaustive() {
        assert_eq!(TraceDropPolicy::ALL.len(), 3);
        assert!(TraceDropPolicy::ALL.contains(&TraceDropPolicy::DropOldest));
        assert!(TraceDropPolicy::ALL.contains(&TraceDropPolicy::DropNewest));
        assert!(TraceDropPolicy::ALL.contains(&TraceDropPolicy::HaltAndFault));
    }

    #[test]
    fn serde_round_trip() {
        let event = TraceEvent {
            seq: u32::MAX,
            timestamp_m_cycles: 17,
            slice: SliceId(3),
            probe: TraceProbeId(4),
            checkpoint: CompactCheckpointId(5),
            data: [9; 16],
        };

        let encoded = serde_json::to_string(&event).expect("event serializes");
        let decoded: TraceEvent = serde_json::from_str(&encoded).expect("event deserializes");

        assert_eq!(decoded, event);

        let budget = TraceBudget::new(7, 224, TraceDropPolicy::HaltAndFault).expect("budget");
        let encoded = serde_json::to_string(&budget).expect("budget serializes");
        let decoded: TraceBudget = serde_json::from_str(&encoded).expect("budget deserializes");
        assert_eq!(decoded, budget);

        for level in ProbeLevel::ALL {
            let encoded = serde_json::to_string(&level).expect("level serializes");
            let decoded: ProbeLevel = serde_json::from_str(&encoded).expect("level deserializes");
            assert_eq!(decoded, level);
        }

        for budget_class in ProbeBudgetClass::ALL {
            let encoded = serde_json::to_string(&budget_class).expect("class serializes");
            let decoded: ProbeBudgetClass =
                serde_json::from_str(&encoded).expect("class deserializes");
            assert_eq!(decoded, budget_class);
        }

        for policy in TraceDropPolicy::ALL {
            let encoded = serde_json::to_string(&policy).expect("policy serializes");
            let decoded: TraceDropPolicy =
                serde_json::from_str(&encoded).expect("policy deserializes");
            assert_eq!(decoded, policy);
        }
    }

    #[test]
    fn trace_budget_constructor_rejects_inconsistent() {
        assert_eq!(
            TraceBudget::new(0, 1, TraceDropPolicy::DropOldest),
            Err(TraceBudgetError::ZeroEventsWithNonzeroBytes)
        );
        assert_eq!(
            TraceBudget::new(1, 0, TraceDropPolicy::DropOldest),
            Err(TraceBudgetError::NonzeroEventsWithZeroBytes)
        );
    }

    #[test]
    fn trace_budget_constructor_accepts_zero_zero() {
        assert_eq!(
            TraceBudget::new(0, 0, TraceDropPolicy::DropNewest),
            Ok(TraceBudget {
                max_events_per_slice: 0,
                max_bytes_per_frame: 0,
                drop_policy: TraceDropPolicy::DropNewest
            })
        );
    }

    #[test]
    fn seq_modulo_documented() {
        let event = TraceEvent {
            seq: u32::MAX,
            timestamp_m_cycles: u32::MAX,
            slice: SliceId(1),
            probe: TraceProbeId(2),
            checkpoint: CompactCheckpointId(3),
            data: [0; 16],
        };

        assert_eq!(event.seq.wrapping_add(1), 0);
        assert_eq!(event.timestamp_m_cycles.wrapping_add(1), 0);
    }

    #[test]
    #[cfg(feature = "host")]
    fn trace_probe_registry_rejects_duplicate_probe() {
        let registry = TraceProbeRegistry {
            abi_version: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            probes: alloc::vec![
                TraceProbeEntry {
                    probe: TraceProbeId(1),
                    level: ProbeLevel::Always,
                    budget_class: ProbeBudgetClass::PerSlice,
                    payload_schema_tag: alloc::borrow::Cow::Borrowed("a"),
                },
                TraceProbeEntry {
                    probe: TraceProbeId(1),
                    level: ProbeLevel::OnError,
                    budget_class: ProbeBudgetClass::PerFrame,
                    payload_schema_tag: alloc::borrow::Cow::Borrowed("b"),
                },
            ],
        };
        let identity = BuildIdentityBlock::new(crate::version::BuildIdentityArgs {
            abi: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            artifact_core_hash: [2; 32],
            runtime_nucleus_hash: [3; 32],
            compile_request_hash: [4; 32],
            timestamp_unix: 0,
            continuation_tail_bytes: 0,
            semantic_schema_version: 1,
        });

        assert_eq!(
            registry.validate_against_identity(&identity),
            Err(TraceProbeRegistryError::DuplicateProbe {
                probe: TraceProbeId(1)
            })
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn trace_probe_registry_identity_validation() {
        let registry = TraceProbeRegistry {
            abi_version: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            probes: alloc::vec![TraceProbeEntry {
                probe: TraceProbeId(1),
                level: ProbeLevel::Always,
                budget_class: ProbeBudgetClass::PerSlice,
                payload_schema_tag: alloc::borrow::Cow::Borrowed("slice.start"),
            }],
        };
        let mut identity_args = crate::version::BuildIdentityArgs {
            abi: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            artifact_core_hash: [2; 32],
            runtime_nucleus_hash: [3; 32],
            compile_request_hash: [4; 32],
            timestamp_unix: 0,
            continuation_tail_bytes: 0,
            semantic_schema_version: 1,
        };
        let identity = BuildIdentityBlock::new(identity_args);
        registry
            .validate_against_identity(&identity)
            .expect("registry matches identity");

        identity_args.build_hash = [9; 32];
        let mismatched = BuildIdentityBlock::new(identity_args);
        assert!(matches!(
            registry.validate_against_identity(&mismatched),
            Err(TraceProbeRegistryError::BuildHashMismatch { .. })
        ));
    }

    #[test]
    #[cfg(feature = "host")]
    fn trace_probe_registry_serde_round_trip() {
        let registry = TraceProbeRegistry {
            abi_version: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            probes: alloc::vec![TraceProbeEntry {
                probe: TraceProbeId(1),
                level: ProbeLevel::Always,
                budget_class: ProbeBudgetClass::PerSlice,
                payload_schema_tag: alloc::borrow::Cow::Borrowed("slice.start"),
            }],
        };
        let encoded = serde_json::to_string(&registry).expect("registry serializes");
        let decoded: TraceProbeRegistry =
            serde_json::from_str(&encoded).expect("registry deserializes");

        assert_eq!(decoded, registry);
    }
}
