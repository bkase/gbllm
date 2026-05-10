//! Artifact hint-bundle schema.
//!
//! HintBundle leaf-value JSON is owned by `gbf_policy::compile::ConstraintValue`;
//! this module re-exports that policy type instead of defining a shadow schema.

use std::collections::BTreeMap;

use gbf_foundation::{Hash256, LayerId, TargetFamilyId, WorkloadId};
use gbf_policy::TraceProbeId;
pub use gbf_policy::compile::ConstraintValue;
use gbf_policy::compile::{CompileKnobId, CompileKnobPath, EvidenceRef, FieldPath};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::export_facts::ExportFacts;
use crate::lowerings::LoweringShardId;
use crate::preferences::CompilePreferences;
use crate::sequence::{SequenceExportFacts, SequenceSemanticsSpec};

pub const HINT_BUNDLE_HASH_DOMAIN_SEPARATOR: &[u8] =
    b"gbf:gbf-artifact:HintBundle:hint_bundle:1.0.0\0";

/// Top-level artifact hint bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintBundle {
    pub facts: ExportFacts,
    pub preferences: CompilePreferences,
    pub constraints: BuildConstraints,
}

impl HintBundle {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            facts: ExportFacts::new(
                Vec::new(),
                // The canonical empty bundle intentionally commits to the
                // smallest legal LinearState sequence fact. Changing this
                // choice changes the empty-bundle hash contract.
                SequenceExportFacts::for_spec(
                    SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
                ),
            ),
            preferences: CompilePreferences::empty(),
            constraints: BuildConstraints::empty(),
        }
    }

    #[must_use]
    pub fn compute_canonical_hash(&self) -> Hash256 {
        let canonical_json = self.canonical_json_bytes();
        let mut hasher = Sha256::new();
        hasher.update(HINT_BUNDLE_HASH_DOMAIN_SEPARATOR);
        hasher.update(canonical_json);
        Hash256::from_bytes(hasher.finalize().into())
    }

    #[must_use]
    fn canonical_json_bytes(&self) -> Vec<u8> {
        let value = serde_json::to_value(self).expect("hint bundle serializes to JSON value");
        let canonical = canonicalize_json_value(value);
        serde_json::to_vec(&canonical).expect("canonical hint bundle JSON serializes")
    }
}

/// Hard build constraints carried by a hint bundle.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConstraints {
    pub entries: Vec<BuildConstraintEntry>,
}

impl BuildConstraints {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConstraintEntry {
    pub provenance_id: TraceProbeId,
    pub knob: CompileKnobId,
    pub path: Option<CompileKnobPath>,
    pub value: ConstraintValue,
    pub evidence: Vec<EvidenceRef>,
    pub scope: EvidenceScope,
}

/// Stable diagnostic provenance for a scope-bearing fact or preference entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintScopeProvenance {
    pub provenance_id: TraceProbeId,
    pub field: FieldPath,
    pub scope: EvidenceScope,
}

/// Closed enum scoping a hint's applicability.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EvidenceScope {
    WholeArtifact,
    LayerScoped { layer: LayerId },
    TargetFamily { family: TargetFamilyId },
    WorkloadScoped { workload: WorkloadId },
    LoweringScoped { shard: LoweringShardId },
}

impl<'de> Deserialize<'de> for EvidenceScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        enum EvidenceScopeSerde {
            WholeArtifact {},
            LayerScoped { layer: LayerId },
            TargetFamily { family: TargetFamilyId },
            WorkloadScoped { workload: WorkloadId },
            LoweringScoped { shard: LoweringShardId },
        }

        Ok(match EvidenceScopeSerde::deserialize(deserializer)? {
            EvidenceScopeSerde::WholeArtifact {} => Self::WholeArtifact,
            EvidenceScopeSerde::LayerScoped { layer } => Self::LayerScoped { layer },
            EvidenceScopeSerde::TargetFamily { family } => Self::TargetFamily { family },
            EvidenceScopeSerde::WorkloadScoped { workload } => Self::WorkloadScoped { workload },
            EvidenceScopeSerde::LoweringScoped { shard } => Self::LoweringScoped { shard },
        })
    }
}

fn canonicalize_json_value(value: Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.into_iter().map(canonicalize_json_value).collect())
        }
        Value::Object(entries) => {
            let sorted: BTreeMap<_, _> = entries
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json_value(value)))
                .collect();
            Value::Object(sorted.into_iter().collect::<Map<_, _>>())
        }
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_uses_domain_separator() {
        let bundle = HintBundle::empty();
        let mut hasher = Sha256::new();
        hasher.update(HINT_BUNDLE_HASH_DOMAIN_SEPARATOR);
        hasher.update(bundle.canonical_json_bytes());
        let expected = Hash256::from_bytes(hasher.finalize().into());

        assert_eq!(bundle.compute_canonical_hash(), expected);
        assert_eq!(
            HINT_BUNDLE_HASH_DOMAIN_SEPARATOR,
            b"gbf:gbf-artifact:HintBundle:hint_bundle:1.0.0\0"
        );
    }
}
