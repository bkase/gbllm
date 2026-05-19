//! K6 cache-key material and semantic rule manifests for Stage 6.

use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256, SemVer};
use gbf_policy::ValidationDiagnostic;
use gbf_report::ReportSchemaId;
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::DeterminismClass;
use crate::storage_plan::rules::{DecisionRuleSetManifest, decision_rule_set_manifest};
use crate::storage_plan::types::{
    CommitGroupReason, DurabilityClass, PersistKind, PersistSchemaPin, STORAGE_PLAN_SCHEMA_ID,
    STORAGE_PLAN_SCHEMA_VERSION, StoragePlanInputIdentity, StoragePlanInputs,
};

pub const STORAGE_PLAN_CACHE_KEY_SCHEMA: &str = "storage_plan.cache_key.v1";
pub const PERSIST_COMPAT_MANIFEST_SCHEMA: &str = "storage_plan.persist_compat.v1";
pub const ALIAS_RULE_SET_MANIFEST_SCHEMA: &str = "storage_plan.alias_rule_set.v1";
pub const STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION: &str = "F-B8-v2";

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StoragePlanCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCacheKeyInputs {
    pub quant_graph_hash: Hash256,
    pub infer_ir_hash: Hash256,
    pub observation_plan_hash: Hash256,
    pub range_plan_hash: Hash256,
    pub policy_hash: Hash256,
    pub determinism: DeterminismClass,
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub decision_rule_set_hash: Hash256,
    pub persist_compat_hash: Hash256,
    pub alias_rule_set_hash: Hash256,
}

impl StoragePlanCacheKeyInputs {
    pub fn from_input_identity(
        identity: &StoragePlanInputIdentity,
    ) -> Result<Self, CanonicalJsonError> {
        Ok(Self {
            quant_graph_hash: identity.quant_graph_hash,
            infer_ir_hash: identity.infer_ir_hash,
            observation_plan_hash: identity.observation_plan_hash,
            range_plan_hash: identity.range_plan_hash,
            policy_hash: identity.policy_hash,
            determinism: identity.determinism,
            schema: identity.schema.clone(),
            schema_version: identity.schema_version,
            decision_rule_set_hash: decision_rule_set_hash()?,
            persist_compat_hash: persist_compat_manifest_hash()?,
            alias_rule_set_hash: alias_rule_set_manifest_hash()?,
        })
    }

    pub fn from_inputs(inputs: &StoragePlanInputs) -> Result<Self, CanonicalJsonError> {
        Self::from_input_identity(&inputs.input_identity())
    }

    pub fn cache_key(&self) -> Result<StoragePlanCacheKey, CanonicalJsonError> {
        storage_plan_cache_key(self)
    }
}

pub fn storage_plan_cache_key_inputs(
    identity: &StoragePlanInputIdentity,
) -> Result<StoragePlanCacheKeyInputs, CanonicalJsonError> {
    StoragePlanCacheKeyInputs::from_input_identity(identity)
}

pub fn storage_plan_cache_key(
    inputs: &StoragePlanCacheKeyInputs,
) -> Result<StoragePlanCacheKey, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "StageCacheKey",
        STORAGE_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(inputs)
    .map(StoragePlanCacheKey)
}

pub fn decision_rule_set_hash() -> Result<Hash256, CanonicalJsonError> {
    decision_rule_set_hash_for(&decision_rule_set_manifest())
}

pub fn decision_rule_set_hash_for(
    manifest: &DecisionRuleSetManifest,
) -> Result<Hash256, CanonicalJsonError> {
    manifest_hash(
        "DecisionRuleSetManifest",
        "storage_plan.decision_rule_set.v1",
        manifest,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistCompatManifest {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub rfc_revision: String,
    pub per_kind: Vec<PersistKindCompatEntry>,
    pub allowed_cross_kind_sets: Vec<PersistAllowedKindSet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistKindCompatEntry {
    pub kind: PersistKind,
    pub source_semantics: String,
    pub allowed_reasons: Vec<CommitGroupReason>,
    pub schema_pin: PersistSchemaPin,
    pub durability_semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistAllowedKindSet {
    pub reason: CommitGroupReason,
    pub kinds: Vec<PersistKind>,
    pub rfc_revision: String,
}

pub fn persist_compat_manifest() -> PersistCompatManifest {
    PersistCompatManifest {
        schema: ReportSchemaId::from(PERSIST_COMPAT_MANIFEST_SCHEMA),
        schema_version: STORAGE_PLAN_SCHEMA_VERSION,
        rfc_revision: STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION.to_owned(),
        per_kind: vec![
            PersistKindCompatEntry {
                kind: PersistKind::SequenceState,
                source_semantics: "SequenceStateSlot".to_owned(),
                allowed_reasons: vec![
                    CommitGroupReason::PerSequenceStateSlot,
                    CommitGroupReason::SequenceStateWithTranscript,
                    CommitGroupReason::ContinuationWithSequenceState,
                ],
                schema_pin: PersistSchemaPin {
                    state_schema: 1,
                    requires_semantic_state_hash: true,
                    requires_resume_abi_hash: false,
                    requires_build_identity_hash: false,
                },
                durability_semantics: format!("{:?}", DurabilityClass::Critical),
            },
            PersistKindCompatEntry {
                kind: PersistKind::Continuation,
                source_semantics: "ContinuationLiveSet".to_owned(),
                allowed_reasons: vec![
                    CommitGroupReason::ContinuationWithSequenceState,
                    CommitGroupReason::ContinuationOnly,
                ],
                schema_pin: PersistSchemaPin {
                    state_schema: 2,
                    requires_semantic_state_hash: false,
                    requires_resume_abi_hash: true,
                    requires_build_identity_hash: false,
                },
                durability_semantics: format!("{:?}", DurabilityClass::Recoverable),
            },
            PersistKindCompatEntry {
                kind: PersistKind::Transcript,
                source_semantics: "OutputTokenCapture".to_owned(),
                allowed_reasons: vec![
                    CommitGroupReason::Independent,
                    CommitGroupReason::SequenceStateWithTranscript,
                ],
                schema_pin: data_page_schema_pin(),
                durability_semantics: "BestEffort unless grouped with sequence state".to_owned(),
            },
            PersistKindCompatEntry {
                kind: PersistKind::Harness,
                source_semantics: "HarnessIngressEgress".to_owned(),
                allowed_reasons: vec![CommitGroupReason::Independent],
                schema_pin: data_page_schema_pin(),
                durability_semantics: format!("{:?}", DurabilityClass::BestEffort),
            },
            PersistKindCompatEntry {
                kind: PersistKind::Trace,
                source_semantics: "TraceCapture".to_owned(),
                allowed_reasons: vec![CommitGroupReason::Independent],
                schema_pin: data_page_schema_pin(),
                durability_semantics: format!("{:?}", DurabilityClass::BestEffort),
            },
        ],
        allowed_cross_kind_sets: vec![
            allowed_kind_set(
                CommitGroupReason::PerSequenceStateSlot,
                vec![PersistKind::SequenceState],
            ),
            allowed_kind_set(
                CommitGroupReason::SequenceStateWithTranscript,
                vec![PersistKind::SequenceState, PersistKind::Transcript],
            ),
            allowed_kind_set(
                CommitGroupReason::ContinuationWithSequenceState,
                vec![PersistKind::Continuation, PersistKind::SequenceState],
            ),
            allowed_kind_set(
                CommitGroupReason::ContinuationOnly,
                vec![PersistKind::Continuation],
            ),
            allowed_kind_set(
                CommitGroupReason::Independent,
                vec![PersistKind::Transcript],
            ),
            allowed_kind_set(CommitGroupReason::Independent, vec![PersistKind::Harness]),
            allowed_kind_set(CommitGroupReason::Independent, vec![PersistKind::Trace]),
        ],
    }
}

pub fn persist_compat_manifest_hash() -> Result<Hash256, CanonicalJsonError> {
    persist_compat_manifest_hash_for(&persist_compat_manifest())
}

pub fn persist_compat_manifest_hash_for(
    manifest: &PersistCompatManifest,
) -> Result<Hash256, CanonicalJsonError> {
    manifest_hash(
        "PersistCompatManifest",
        PERSIST_COMPAT_MANIFEST_SCHEMA,
        manifest,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasRuleSetManifest {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub rfc_revision: String,
    pub pair_predicates: Vec<AliasPairPredicateManifestEntry>,
    pub cardinality_constraints: Vec<AliasCardinalityConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasPairPredicateManifestEntry {
    pub id: String,
    pub intent: String,
    pub predicate_semantics: String,
    pub rfc_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasCardinalityConstraint {
    pub intent: String,
    pub min_members: u32,
    pub max_members: Option<u32>,
    pub rfc_revision: String,
}

pub fn alias_rule_set_manifest() -> AliasRuleSetManifest {
    AliasRuleSetManifest {
        schema: ReportSchemaId::from(ALIAS_RULE_SET_MANIFEST_SCHEMA),
        schema_version: STORAGE_PLAN_SCHEMA_VERSION,
        rfc_revision: STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION.to_owned(),
        pair_predicates: vec![
            alias_pair(
                "PP-ScratchReuse",
                "ScratchReuse",
                "Disjoint hot scratch or accumulator",
            ),
            alias_pair(
                "PP-PingPong",
                "PingPong",
                "Typed alternating activation pair",
            ),
            alias_pair(
                "PP-ResumeOverlap",
                "ResumeOverlap",
                "Resume boundary overlap",
            ),
            alias_pair(
                "PP-PersistRotation",
                "PersistRotation",
                "Persist page rotation pair",
            ),
        ],
        cardinality_constraints: vec![
            alias_cardinality("NoAlias", 1, Some(1)),
            alias_cardinality("ScratchReuse", 2, None),
            alias_cardinality("PingPong", 2, Some(2)),
            alias_cardinality("ResumeOverlap", 2, None),
            alias_cardinality("PersistRotation", 2, Some(2)),
        ],
    }
}

pub fn alias_rule_set_manifest_hash() -> Result<Hash256, CanonicalJsonError> {
    alias_rule_set_manifest_hash_for(&alias_rule_set_manifest())
}

pub fn alias_rule_set_manifest_hash_for(
    manifest: &AliasRuleSetManifest,
) -> Result<Hash256, CanonicalJsonError> {
    manifest_hash(
        "AliasRuleSetManifest",
        ALIAS_RULE_SET_MANIFEST_SCHEMA,
        manifest,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageCacheSuccessEntry<P> {
    pub key: StoragePlanCacheKey,
    pub product: P,
    pub report_hash: Hash256,
    pub artifact_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageCacheFailureMemo {
    pub key: StoragePlanCacheKey,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub report_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanStageCacheReplay<P> {
    Success(StageCacheSuccessEntry<P>),
    FailureMemo(StageCacheFailureMemo),
    Miss { key: StoragePlanCacheKey },
}

impl<P> StoragePlanStageCacheReplay<P> {
    #[must_use]
    pub const fn is_miss(&self) -> bool {
        matches!(self, Self::Miss { .. })
    }

    #[must_use]
    pub fn replay_success(&self) -> Option<&P> {
        match self {
            Self::Success(entry) => Some(&entry.product),
            Self::FailureMemo(_) | Self::Miss { .. } => None,
        }
    }

    #[must_use]
    pub fn replay_failure(&self) -> Option<&[ValidationDiagnostic]> {
        match self {
            Self::FailureMemo(memo) => Some(&memo.diagnostics),
            Self::Success(_) | Self::Miss { .. } => None,
        }
    }
}

pub fn failure_memo_diagnostic_bytes(
    memo: &StageCacheFailureMemo,
) -> Result<Vec<u8>, CanonicalJsonError> {
    CanonicalJson::to_vec(&memo.diagnostics)
}

fn data_page_schema_pin() -> PersistSchemaPin {
    PersistSchemaPin {
        state_schema: 3,
        requires_semantic_state_hash: false,
        requires_resume_abi_hash: false,
        requires_build_identity_hash: true,
    }
}

fn allowed_kind_set(reason: CommitGroupReason, kinds: Vec<PersistKind>) -> PersistAllowedKindSet {
    PersistAllowedKindSet {
        reason,
        kinds,
        rfc_revision: STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION.to_owned(),
    }
}

fn alias_pair(
    id: &str,
    intent: &str,
    predicate_semantics: &str,
) -> AliasPairPredicateManifestEntry {
    AliasPairPredicateManifestEntry {
        id: id.to_owned(),
        intent: intent.to_owned(),
        predicate_semantics: predicate_semantics.to_owned(),
        rfc_revision: STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION.to_owned(),
    }
}

fn alias_cardinality(
    intent: &str,
    min_members: u32,
    max_members: Option<u32>,
) -> AliasCardinalityConstraint {
    AliasCardinalityConstraint {
        intent: intent.to_owned(),
        min_members,
        max_members,
        rfc_revision: STORAGE_PLAN_RULE_MANIFEST_RFC_REVISION.to_owned(),
    }
}

fn manifest_hash<T>(
    report_type: &'static str,
    schema: &'static str,
    manifest: &T,
) -> Result<Hash256, CanonicalJsonError>
where
    T: Serialize,
{
    DomainHash::new("gbf-codegen", report_type, schema, "1.0.0").hash(manifest)
}

#[cfg(test)]
mod tests {
    use gbf_foundation::Hash256;
    use gbf_policy::{StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance, ValidationCode};

    use super::*;
    use crate::storage_plan::diagnostics::storage_plan_diagnostic;

    #[test]
    fn manifest_revision_change_updates_decision_rule_set_hash() {
        let mut manifest = decision_rule_set_manifest();
        let first = decision_rule_set_hash_for(&manifest).expect("first hash");
        manifest.rules[0].rfc_revision.push_str("+amended");
        let second = decision_rule_set_hash_for(&manifest).expect("second hash");

        assert_ne!(first, second);
        assert_eq!(first, decision_rule_set_hash().expect("default hash"));
    }

    #[test]
    fn k6_is_computable_without_output_report_hash() {
        let identity = identity();
        let inputs = StoragePlanCacheKeyInputs::from_input_identity(&identity)
            .expect("cache inputs compute");
        let key = inputs.cache_key().expect("cache key computes");
        let json = serde_json::to_value(&inputs).expect("cache inputs serialize");
        let _from_storage_inputs: fn(
            &StoragePlanInputs,
        )
            -> Result<StoragePlanCacheKeyInputs, CanonicalJsonError> =
            StoragePlanCacheKeyInputs::from_inputs;

        assert_ne!(key.0, Hash256::ZERO);
        assert!(!json_has_key(&json, "report_self_hash"));
        assert_eq!(inputs.schema, ReportSchemaId::from(STORAGE_PLAN_SCHEMA_ID));
    }

    #[test]
    fn manifest_hashes_are_semantic_and_deterministic() {
        let first =
            StoragePlanCacheKeyInputs::from_input_identity(&identity()).expect("first inputs");
        let second =
            StoragePlanCacheKeyInputs::from_input_identity(&identity()).expect("second inputs");

        assert_eq!(first, second);
        assert_eq!(
            persist_compat_manifest().allowed_cross_kind_sets.len(),
            7,
            "CG-Wf-3 entries are part of the semantic manifest"
        );
        assert_eq!(alias_rule_set_manifest().pair_predicates.len(), 4);
    }

    #[test]
    fn stage_cache_success_hit_and_miss_replay() {
        let key = StoragePlanCacheKeyInputs::from_input_identity(&identity())
            .expect("inputs")
            .cache_key()
            .expect("key");
        let success = StoragePlanStageCacheReplay::Success(StageCacheSuccessEntry {
            key,
            product: "storage-plan-product".to_owned(),
            report_hash: hash(9),
            artifact_path: "storage_plan.json".to_owned(),
        });
        let miss = StoragePlanStageCacheReplay::<String>::Miss { key };

        assert_eq!(
            success.replay_success(),
            Some(&"storage-plan-product".to_owned())
        );
        assert!(miss.is_miss());
        assert!(miss.replay_success().is_none());
    }

    #[test]
    fn stage_cache_failure_memo_replays_byte_identical_diagnostics() {
        let key = StoragePlanCacheKeyInputs::from_input_identity(&identity())
            .expect("inputs")
            .cache_key()
            .expect("key");
        let diagnostics = vec![
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                StoragePlanDiagnosticProvenance::ValueProducer {
                    value_id: 7,
                    producer_node: 0,
                },
                vec![],
            )
            .expect("diagnostic"),
        ];
        let memo = StageCacheFailureMemo {
            key,
            diagnostics,
            report_hash: hash(10),
        };
        let replay = StoragePlanStageCacheReplay::<String>::FailureMemo(memo.clone());

        assert!(matches!(
            replay.replay_failure().and_then(|items| items.first()),
            Some(diagnostic)
                if matches!(
                    diagnostic.code,
                    ValidationCode::StoragePlan {
                        code: StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                        ..
                    }
                )
        ));
        assert_eq!(
            failure_memo_diagnostic_bytes(&memo).expect("first bytes"),
            failure_memo_diagnostic_bytes(&memo).expect("second bytes")
        );
    }

    fn identity() -> StoragePlanInputIdentity {
        StoragePlanInputIdentity {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
            determinism: DeterminismClass::Deterministic,
            schema: ReportSchemaId::from(STORAGE_PLAN_SCHEMA_ID),
            schema_version: STORAGE_PLAN_SCHEMA_VERSION,
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn json_has_key(value: &serde_json::Value, key: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .any(|(field, nested)| field == key || json_has_key(nested, key)),
            serde_json::Value::Array(values) => {
                values.iter().any(|nested| json_has_key(nested, key))
            }
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => false,
        }
    }
}
