//! Core Stage 6 StoragePlan types.

use gbf_foundation::{EvidenceRef, Hash256, SemVer, canonical_json::DomainHash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::s1::quant_graph::{DeterminismClass, QuantGraph};
use crate::s3::infer_ir::{GbInferIR, NodeId, ValueId};
use crate::s4::observation_plan::ObservationPlan;
use crate::s5::range_plan::RangePlan;
use gbf_policy::compile::ResolvedCompilePolicy;
use gbf_policy::{ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin};
use gbf_report::ReportSchemaId;

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum StorageClass {
    WramHot,
    HramHot,
    SramPaged,
    RomConst,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum LifetimeClass {
    Slice,
    ResumeWindow,
    Token,
    Session,
    Persistent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct DecisionRuleId(pub u32);

impl DecisionRuleId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PersistPageId(pub u32);

impl PersistPageId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CommitGroupId(pub u32);

impl CommitGroupId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct AliasClassId(pub u32);

impl AliasClassId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

fn deserialize_u32_id<'de, D>(deserializer: D, type_name: &'static str) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct IdVisitor {
        type_name: &'static str,
    }

    impl<'de> serde::de::Visitor<'de> for IdVisitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "{} as a u32 or string key", self.type_name)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u32::try_from(value)
                .map_err(|_| E::custom(format!("{} out of u32 range", self.type_name)))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            value
                .parse::<u32>()
                .map_err(|_| E::custom(format!("invalid {} string key", self.type_name)))
        }
    }

    deserializer.deserialize_any(IdVisitor { type_name })
}

impl<'de> Deserialize<'de> for DecisionRuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "DecisionRuleId").map(Self)
    }
}

impl<'de> Deserialize<'de> for PersistPageId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "PersistPageId").map(Self)
    }
}

impl<'de> Deserialize<'de> for CommitGroupId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "CommitGroupId").map(Self)
    }
}

impl<'de> Deserialize<'de> for AliasClassId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "AliasClassId").map(Self)
    }
}

pub type NonEmptySortedSet<T> = crate::s3::infer_ir::NonEmptySet<T>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AliasClassFingerprint(pub Hash256);

#[derive(Serialize)]
struct AliasClassFingerprintPayload {
    members: Vec<ValueId>,
    intent: AliasIntent,
}

impl AliasClassFingerprint {
    pub fn compute(
        members: &NonEmptySortedSet<ValueId>,
        intent: AliasIntent,
    ) -> Result<Self, gbf_foundation::CanonicalJsonError> {
        let sorted_members: Vec<ValueId> = members.iter().copied().collect();
        let payload = AliasClassFingerprintPayload {
            members: sorted_members,
            intent,
        };
        let domain = DomainHash::new("gbf-codegen", "AliasClassId", "v1", "1");
        let hash = domain.hash(&payload)?;
        Ok(Self(hash))
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum AliasIntent {
    NoAlias,
    ScratchReuse,
    PingPong,
    ResumeOverlap,
    PersistRotation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasClassError {
    NoAliasWithMultipleMembers,
    MultiMemberIntentWithSingleton,
    CanonicalJson(String),
}

impl std::fmt::Display for AliasClassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAliasWithMultipleMembers => {
                write!(f, "NoAlias intent paired with multi-member set")
            }
            Self::MultiMemberIntentWithSingleton => {
                write!(f, "multi-member intent paired with singleton set")
            }
            Self::CanonicalJson(err) => write!(f, "canonical JSON serialization error: {err}"),
        }
    }
}

impl std::error::Error for AliasClassError {}

#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
pub struct AliasClass {
    pub id: AliasClassId,
    pub fingerprint: AliasClassFingerprint,
    pub members: NonEmptySortedSet<ValueId>,
    pub intent: AliasIntent,
}

impl AliasClass {
    pub fn new(
        id: AliasClassId,
        members: NonEmptySortedSet<ValueId>,
        intent: AliasIntent,
    ) -> Result<Self, AliasClassError> {
        let member_count = members.len();
        if member_count == 1 {
            if intent != AliasIntent::NoAlias {
                return Err(AliasClassError::MultiMemberIntentWithSingleton);
            }
        } else {
            // member_count > 1
            if intent == AliasIntent::NoAlias {
                return Err(AliasClassError::NoAliasWithMultipleMembers);
            }
        }

        // Compute fingerprint!
        let fingerprint = AliasClassFingerprint::compute(&members, intent)
            .map_err(|e| AliasClassError::CanonicalJson(e.to_string()))?;

        Ok(Self {
            id,
            fingerprint,
            members,
            intent,
        })
    }

    #[must_use]
    pub const fn id(&self) -> AliasClassId {
        self.id
    }

    #[must_use]
    pub const fn fingerprint(&self) -> AliasClassFingerprint {
        self.fingerprint
    }

    #[must_use]
    pub const fn members(&self) -> &NonEmptySortedSet<ValueId> {
        &self.members
    }

    #[must_use]
    pub const fn intent(&self) -> AliasIntent {
        self.intent
    }
}

impl<'de> Deserialize<'de> for AliasClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AliasClassRaw {
            id: AliasClassId,
            fingerprint: AliasClassFingerprint,
            members: NonEmptySortedSet<ValueId>,
            intent: AliasIntent,
        }

        let raw = AliasClassRaw::deserialize(deserializer)?;
        let class = Self::new(raw.id, raw.members, raw.intent).map_err(serde::de::Error::custom)?;

        // Also verify fingerprint matches the computed fingerprint!
        if class.fingerprint != raw.fingerprint {
            return Err(serde::de::Error::custom(format!(
                "Fingerprint mismatch: expected {:?}, observed {:?}",
                class.fingerprint, raw.fingerprint
            )));
        }

        Ok(class)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum ValueSelector {
    Value(ValueId),
    AliasClass(AliasClassFingerprint),
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum PersistKind {
    SequenceState,
    Continuation,
    Transcript,
    Harness,
    Trace,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum DurabilityClass {
    Critical,
    Recoverable,
    BestEffort,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum CommitAtomicityClass {
    /// All members must commit; failure of any member rolls all back.
    AllOrNothing,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Materialization {
    Recompute {},
    Materialize {
        class: StorageClass,
        lifetime: LifetimeClass,
    },
    Persist {
        page: PersistPageId,
        commit_group: CommitGroupId,
    },
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct AbstractLiveRange {
    /// Producer node in GbInferIR topological order.
    pub def_node: NodeId,
    /// First use node in GbInferIR topological order, if any.
    pub first_use_node: Option<NodeId>,
    /// Last use node in GbInferIR topological order, if any.
    pub last_use_node: Option<NodeId>,
    /// Conservative lifetime class derived from the def-use interval
    /// and ObservationPlan constraints.
    pub lifetime_class: LifetimeClass,
    /// True when an ObservationPlan semantic checkpoint requires the
    /// value to be inspectable at a stable point.
    pub checkpoint_stable: bool,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BindingJustification {
    /// Default decision rule fired for this value's role/format.
    DecisionRule { rule_id: DecisionRuleId },
    /// Override applied via CompileKnobOverrides.forced_recompute.
    ForcedRecompute {},
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct StorageBinding {
    pub value: ValueId,
    pub materialization: Materialization,
    pub alias_class: AliasClassId,
    pub live_range: AbstractLiveRange,
    pub justification: BindingJustification,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct PersistSchemaPin {
    /// state_schema field of PersistHeader (planv0.md line 2147).
    pub state_schema: u16,
    /// Whether semantic_state_hash is required to be the artifact's
    /// canonical semantic state hash (true for SequenceState pages
    /// in v1; false for Transcript/Harness/Trace).
    pub requires_semantic_state_hash: bool,
    /// Whether resume_abi_hash is required (true for Continuation;
    /// false otherwise).
    pub requires_resume_abi_hash: bool,
    /// Whether build_identity_hash is required (true for Harness/Trace;
    /// false otherwise).
    pub requires_build_identity_hash: bool,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct PersistPageDecl {
    pub id: PersistPageId,
    pub kind: PersistKind,
    pub durability: DurabilityClass,
    pub schema_pin: PersistSchemaPin,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct CommitGroupDecl {
    pub id: CommitGroupId,
    pub members: crate::s3::infer_ir::NonEmptySet<PersistPageId>,
    pub kind_set: BTreeSet<PersistKind>,
    /// Constrains which PersistGroupCommit headers are runtime-legal.
    pub atomicity: CommitAtomicityClass,
}

/// Invariant SC11 check: validates that the serialized JSON string of the storage plan
/// contains no object keys or enum tags from the ForbiddenStage6SpatialSurface list.
pub fn check_sc11_no_forbidden_keys(json_str: &str) -> Result<(), String> {
    let forbidden = vec![
        ["byte", "offset"].join("_"),
        ["byte", "alignment"].join("_"),
        ["byte", "address"].join("_"),
        ["concrete", "bank"].join("_"),
        ["rom", "bank"].join("_"),
        ["sram", "bank"].join("_"),
        ["slice", "id"].join("_"),
        ["lease", "id"].join("_"),
        ["overlay", "region"].join("_"),
        ["overlay", "install"].join("_"),
        ["page", "byte", "address"].join("_"),
        ["kernel", "residency"].join("_"),
        ["sram", "page", "family", "id"].join("_"),
        ["sram", "working", "set", "id"].join("_"),
        ["Resource", "Vector"].concat(),
        ["Sched", "Slice"].concat(),
        ["Residency", "Epoch"].concat(),
        ["Overlay", "Id"].concat(),
        ["Overlay", "Install"].concat(),
        ["Kernel", "Residency"].concat(),
        ["Bank", "Class"].concat(),
        ["Rom", "Bank"].concat(),
        ["Sram", "Bank"].concat(),
        ["Resi", "dency"].concat(),
    ];

    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {e}"))?;

    fn walk(val: &serde_json::Value, forbidden: &[String]) -> Result<(), String> {
        match val {
            serde_json::Value::Object(map) => {
                for (key, v) in map {
                    if forbidden.iter().any(|f| f == key) {
                        return Err(format!("Forbidden spatial key found: {key}"));
                    }
                    if let Some(s) = v.as_str().filter(|s| forbidden.iter().any(|f| f == s)) {
                        return Err(format!("Forbidden spatial enum tag/value found: {s}"));
                    }
                    walk(v, forbidden)?;
                }
            }
            serde_json::Value::Array(arr) => {
                for v in arr {
                    walk(v, forbidden)?;
                }
            }
            serde_json::Value::String(s) if forbidden.iter().any(|f| f == s) => {
                return Err(format!("Forbidden spatial enum tag/value found: {s}"));
            }
            _ => {}
        }
        Ok(())
    }

    walk(&value, &forbidden)
}

#[derive(Clone, Debug)]
pub struct StoragePlanInputs {
    pub policy: ResolvedCompilePolicy,
    pub policy_hash: Hash256,
    pub quant_graph: QuantGraph,
    pub quant_graph_hash: Hash256,
    pub infer_ir: GbInferIR,
    pub infer_ir_hash: Hash256,
    pub observation_plan: ObservationPlan,
    pub observation_plan_hash: Hash256,
    pub range_plan: RangePlan,
    pub range_plan_hash: Hash256,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct StoragePlanInputIdentity {
    pub quant_graph_hash: Hash256,
    pub infer_ir_hash: Hash256,
    pub observation_plan_hash: Hash256,
    pub range_plan_hash: Hash256,
    pub policy_hash: Hash256,
    pub determinism: DeterminismClass,
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
}

impl StoragePlanInputIdentity {
    #[must_use]
    pub fn from_inputs(inputs: &StoragePlanInputs) -> Self {
        Self {
            quant_graph_hash: inputs.quant_graph_hash,
            infer_ir_hash: inputs.infer_ir_hash,
            observation_plan_hash: inputs.observation_plan_hash,
            range_plan_hash: inputs.range_plan_hash,
            policy_hash: inputs.policy_hash,
            determinism: inputs.quant_graph.identity.determinism,
            schema: ReportSchemaId::new("storage_plan.v1"),
            schema_version: SemVer::new(1, 0, 0),
        }
    }
}

#[allow(clippy::result_large_err)]
pub fn canonicalize_inputs(inputs: &StoragePlanInputs) -> Result<(), ValidationDiagnostic> {
    // 1. STORE-020: RangePlan hash
    let range_plan_computed = crate::s5::range_plan::range_plan_self_hash(&inputs.range_plan)
        .expect("RangePlan self hash should compute");
    if range_plan_computed != inputs.range_plan_hash {
        return Err(ValidationDiagnostic::hard(
            ValidationOrigin::StoragePlanConstruction,
            ValidationCode::StorageRangePlanHashMismatch,
            ValidationDetail::HashMismatch {
                expected: inputs.range_plan_hash,
                observed: range_plan_computed,
            },
            vec![
                EvidenceRef {
                    kind: "recorded".to_owned(),
                    reference: "recorded".to_owned(),
                    hash: Some(inputs.range_plan_hash),
                },
                EvidenceRef {
                    kind: "observed".to_owned(),
                    reference: "observed".to_owned(),
                    hash: Some(range_plan_computed),
                },
            ],
        ));
    }

    // 2. STORE-021: GbInferIR hash
    let infer_ir_computed = crate::s3::infer_ir::infer_ir_self_hash(&inputs.infer_ir)
        .expect("GbInferIR self hash should compute");
    if infer_ir_computed != inputs.infer_ir_hash {
        return Err(ValidationDiagnostic::hard(
            ValidationOrigin::StoragePlanConstruction,
            ValidationCode::StorageInferIrHashMismatch,
            ValidationDetail::HashMismatch {
                expected: inputs.infer_ir_hash,
                observed: infer_ir_computed,
            },
            vec![
                EvidenceRef {
                    kind: "recorded".to_owned(),
                    reference: "recorded".to_owned(),
                    hash: Some(inputs.infer_ir_hash),
                },
                EvidenceRef {
                    kind: "observed".to_owned(),
                    reference: "observed".to_owned(),
                    hash: Some(infer_ir_computed),
                },
            ],
        ));
    }

    // 3. STORE-022: ObservationPlan hash
    let observation_plan_computed =
        crate::s4::observation_plan::observation_plan_self_hash(&inputs.observation_plan)
            .expect("ObservationPlan self hash should compute");
    if observation_plan_computed != inputs.observation_plan_hash {
        return Err(ValidationDiagnostic::hard(
            ValidationOrigin::StoragePlanConstruction,
            ValidationCode::StorageObservationPlanHashMismatch,
            ValidationDetail::HashMismatch {
                expected: inputs.observation_plan_hash,
                observed: observation_plan_computed,
            },
            vec![
                EvidenceRef {
                    kind: "recorded".to_owned(),
                    reference: "recorded".to_owned(),
                    hash: Some(inputs.observation_plan_hash),
                },
                EvidenceRef {
                    kind: "observed".to_owned(),
                    reference: "observed".to_owned(),
                    hash: Some(observation_plan_computed),
                },
            ],
        ));
    }

    // 4. STORE-023: QuantGraph hash
    let quant_graph_computed = crate::s1::quant_graph::quant_graph_self_hash(&inputs.quant_graph)
        .expect("QuantGraph self hash should compute");
    if quant_graph_computed != inputs.quant_graph_hash {
        return Err(ValidationDiagnostic::hard(
            ValidationOrigin::StoragePlanConstruction,
            ValidationCode::StorageQuantGraphHashMismatch,
            ValidationDetail::HashMismatch {
                expected: inputs.quant_graph_hash,
                observed: quant_graph_computed,
            },
            vec![
                EvidenceRef {
                    kind: "recorded".to_owned(),
                    reference: "recorded".to_owned(),
                    hash: Some(inputs.quant_graph_hash),
                },
                EvidenceRef {
                    kind: "observed".to_owned(),
                    reference: "observed".to_owned(),
                    hash: Some(quant_graph_computed),
                },
            ],
        ));
    }

    // 5. STORE-024: ResolvedCompilePolicy hash
    let policy_computed = inputs.policy.canonical_hash();
    if policy_computed != inputs.policy_hash {
        return Err(ValidationDiagnostic::hard(
            ValidationOrigin::StoragePlanConstruction,
            ValidationCode::StoragePolicyHashMismatch,
            ValidationDetail::HashMismatch {
                expected: inputs.policy_hash,
                observed: policy_computed,
            },
            vec![
                EvidenceRef {
                    kind: "recorded".to_owned(),
                    reference: "recorded".to_owned(),
                    hash: Some(inputs.policy_hash),
                },
                EvidenceRef {
                    kind: "observed".to_owned(),
                    reference: "observed".to_owned(),
                    hash: Some(policy_computed),
                },
            ],
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_lifetime_class_ordering() {
        assert!(LifetimeClass::Slice < LifetimeClass::ResumeWindow);
        assert!(LifetimeClass::ResumeWindow < LifetimeClass::Token);
        assert!(LifetimeClass::Token < LifetimeClass::Session);
        assert!(LifetimeClass::Session < LifetimeClass::Persistent);
    }

    #[test]
    fn test_sc11_validation() {
        // A valid JSON representing storage plan elements
        let valid_json = r#"
        {
            "class": "WramHot",
            "lifetime": "Slice",
            "page": 1,
            "commit_group": 2
        }
        "#;
        assert!(check_sc11_no_forbidden_keys(valid_json).is_ok());

        // Forbidden keys should trigger an error
        let forbidden_key_json = r#"
        {
            "concrete_bank": 1,
            "class": "WramHot"
        }
        "#;
        assert!(check_sc11_no_forbidden_keys(forbidden_key_json).is_err());

        // Forbidden values/enum tags should trigger an error
        let forbidden_val_json = r#"
        {
            "some_key": "RomBank"
        }
        "#;
        assert!(check_sc11_no_forbidden_keys(forbidden_val_json).is_err());
    }

    #[test]
    fn test_newtypes_serde() {
        let rule_id = DecisionRuleId::new(42);
        let serialized = serde_json::to_string(&rule_id).unwrap();
        assert_eq!(serialized, "42");
        let deserialized: DecisionRuleId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, rule_id);

        // String representation (as Map key)
        let map_serialized = r#"{"42": "some_value"}"#;
        let map: BTreeMap<DecisionRuleId, String> = serde_json::from_str(map_serialized).unwrap();
        assert_eq!(map.get(&rule_id).unwrap(), "some_value");
    }

    #[test]
    fn test_materialization_serde() {
        let mat = Materialization::Materialize {
            class: StorageClass::WramHot,
            lifetime: LifetimeClass::Slice,
        };
        let serialized = serde_json::to_string(&mat).unwrap();
        let deserialized: Materialization = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, mat);
        assert!(check_sc11_no_forbidden_keys(&serialized).is_ok());
    }

    #[test]
    fn test_alias_class_invariants_and_fingerprints() {
        // Construct NonEmptySortedSet with 1 member
        let val1 = ValueId::new(10);
        let mut members1 = BTreeSet::new();
        members1.insert(val1);
        let set1 = NonEmptySortedSet::new(members1).unwrap();

        // 1 member with MultiMember intent (e.g. ScratchReuse) should be rejected
        let class_err = AliasClass::new(
            AliasClassId::new(1),
            set1.clone(),
            AliasIntent::ScratchReuse,
        );
        assert_eq!(
            class_err,
            Err(AliasClassError::MultiMemberIntentWithSingleton)
        );

        // 1 member with NoAlias should succeed
        let class1 = AliasClass::new(AliasClassId::new(1), set1, AliasIntent::NoAlias).unwrap();
        assert_eq!(class1.intent(), AliasIntent::NoAlias);

        // Construct NonEmptySortedSet with multiple members
        let val2 = ValueId::new(20);
        let mut members2 = BTreeSet::new();
        members2.insert(val1);
        members2.insert(val2);
        let set2 = NonEmptySortedSet::new(members2).unwrap();

        // Multiple members with NoAlias should be rejected
        let class_err2 = AliasClass::new(AliasClassId::new(2), set2.clone(), AliasIntent::NoAlias);
        assert_eq!(class_err2, Err(AliasClassError::NoAliasWithMultipleMembers));

        // Multiple members with PingPong should succeed
        let class2 =
            AliasClass::new(AliasClassId::new(2), set2.clone(), AliasIntent::PingPong).unwrap();
        assert_eq!(class2.intent(), AliasIntent::PingPong);

        // Equal AliasClass values produce equal fingerprints
        let class2_same =
            AliasClass::new(AliasClassId::new(2), set2.clone(), AliasIntent::PingPong).unwrap();
        assert_eq!(class2.fingerprint(), class2_same.fingerprint());

        // Different intent produces different fingerprint
        let class2_diff = AliasClass::new(
            AliasClassId::new(2),
            set2.clone(),
            AliasIntent::ScratchReuse,
        )
        .unwrap();
        assert_ne!(class2.fingerprint(), class2_diff.fingerprint());

        // Fingerprint is order-insensitive on the members set (canonical JSON sorts set first)
        let mut members2_rev = BTreeSet::new();
        members2_rev.insert(val2);
        members2_rev.insert(val1);
        let set2_rev = NonEmptySortedSet::new(members2_rev).unwrap();
        let class2_rev =
            AliasClass::new(AliasClassId::new(2), set2_rev, AliasIntent::PingPong).unwrap();
        assert_eq!(class2.fingerprint(), class2_rev.fingerprint());
    }

    #[test]
    fn test_alias_class_serde() {
        let val1 = ValueId::new(10);
        let val2 = ValueId::new(20);
        let mut members = BTreeSet::new();
        members.insert(val1);
        members.insert(val2);
        let set = NonEmptySortedSet::new(members).unwrap();

        let class = AliasClass::new(AliasClassId::new(5), set, AliasIntent::PingPong).unwrap();
        let serialized = serde_json::to_string(&class).unwrap();
        let deserialized: AliasClass = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, class);

        // A tampered fingerprint should fail deserialization
        let tampered_json = serialized.replace(
            &class.fingerprint().0.to_string(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        );
        let deserialized_tampered: Result<AliasClass, _> = serde_json::from_str(&tampered_json);
        assert!(deserialized_tampered.is_err());
    }

    fn mock_policy_fixture() -> ResolvedCompilePolicy {
        use gbf_foundation::{CompileProfileId, FieldPath, TargetProfileId};
        use gbf_policy::{
            CalibrationConfidenceClass, CalibrationConfidenceRequirement, CompileKnobId,
            CompileKnobOverrides, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues,
            CompileKnobs, CompileObjective, CompilerFeature, ConstraintOperation,
            ConstraintProvenance, EffectiveConstraints, KnobLockSet, ObservabilityMode,
            ObservationKnob, ObservationProfileCaps, OverlayKnob, OverlayPromotion, PlacementKnob,
            PlacementProfile, PolicyProvenance, PolicySource, ProbeCollectionLevel, RangeCapsSpec,
            RangeKnob, ReductionPlanCeiling, RepairPolicy, RepairPolicyProfile, RiskPolicy,
            RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob, RuntimeMode,
            ScheduleKnob, ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
            ServiceLevelObjective, SramKnob, SramPageAggression, StorageKnob,
            StorageMaterialization, TraceBudget, TraceDropPolicy, canonical_default_bounds_fixture,
        };

        let values = CompileKnobValues {
            placement: PlacementKnob {
                profile: PlacementProfile::StrictOnePerBank,
            },
            observation: ObservationKnob {
                observability: ObservabilityMode::Invariant,
                probe_level: ProbeCollectionLevel::Operational,
            },
            range: RangeKnob {
                reduction_ceiling: ReductionPlanCeiling::Conservative,
            },
            storage: StorageKnob {
                materialization: StorageMaterialization::RecomputePureValues,
            },
            sram: SramKnob {
                page_aggression: SramPageAggression::PackCold,
            },
            rom_window: RomWindowKnob {
                kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            },
            overlay: OverlayKnob {
                promotion: OverlayPromotion::TinyLuts,
            },
            schedule: ScheduleKnob {
                tile_search: ScheduleTileSearch::Local,
                slice_coarsening: ScheduleSliceCoarsening::Balanced,
                resource_pressure: ScheduleResourcePressure::Balanced,
            },
        };

        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: CompileObjective {
                service: Some(ServiceLevelObjective {
                    max_first_token_cycles_p95: Some(3_000),
                    max_checkpoint_gap_cycles_p95: None,
                    max_resume_latency_cycles_p95: Some(1_000),
                    max_ui_jitter_frames_p99: Some(1),
                }),
                max_cycles_per_token: Some(8_000),
                max_bank_switches_per_token: Some(5),
                max_sram_page_switches_per_token: Some(1),
                min_ui_headroom_pct: 9,
                max_rom_bytes: Some(512 * 1024),
                risk: RiskPolicy {
                    cycle_quantile: 95,
                    switch_quantile: 99,
                    calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                        class: CalibrationConfidenceClass::Weak,
                    },
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 4,
                max_bytes_per_frame: 128,
                drop_policy: TraceDropPolicy::HaltAndFault,
            },
            range_caps: RangeCapsSpec::default_v2(),
            observation_caps: ObservationProfileCaps::default_v2(),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: CompileKnobs {
                global: values,
                bounds: canonical_default_bounds_fixture(),
                locks: KnobLockSet::default(),
                overrides: CompileKnobOverrides::default(),
                provenance: vec![CompileKnobProvenanceEntry {
                    path: CompileKnobPath {
                        knob: CompileKnobId::Placement,
                        selector: None,
                        field: Some(FieldPath::from("global.profile")),
                    },
                    chain: vec![ConstraintProvenance {
                        source: PolicySource::ProfileDefault,
                        operation: ConstraintOperation::SeedDefault,
                        evidence: vec![EvidenceRef {
                            kind: "ProfileFile".to_owned(),
                            reference: "Bringup.toml".to_owned(),
                            hash: Some(hash(8)),
                        }],
                    }],
                }],
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: PolicyProvenance {
                target_defaults: hash(6),
                profile_defaults: hash(9),
                compile_profile_spec_version: "2.0.0".to_owned(),
                hint_bundle_hash: Some(hash(4)),
                compile_request_hash: hash(5),
                calibration_hash: Some(hash(7)),
            },
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn mock_quant_graph() -> QuantGraph {
        use crate::s1::quant_graph::{
            ClassifyHead, ClassifyHeadKind, DecodePlanId, DecodeSpec, DecodeSpecRecord,
            ModelSpecSummary, QuantFormat, QuantGraph, QuantGraphIdentity, ResidualCombinePolicy,
            ResidualPlan, TensorId,
        };

        QuantGraph {
            identity: QuantGraphIdentity {
                artifact_core_hash: hash(0x02),
                policy_resolution_self_hash: hash(0x0c),
                artifact_validation_self_hash: hash(0x0b),
                semantic_core_hash: hash(0x02),
                lowering_manifest_hash: hash(0x05),
                determinism: DeterminismClass::BitExact,
                model_spec_summary: ModelSpecSummary {
                    n_layers: 0,
                    n_experts: BTreeMap::new(),
                    d_model: 64,
                    d_ff: 128,
                    vocab_size: 16,
                    ffn_kind: BTreeMap::new(),
                },
            },
            tensors: Vec::new(),
            norm_plans: Vec::new(),
            layer_norms: BTreeMap::new(),
            routing_table: None,
            expert_sections: Vec::new(),
            ffn_plans: BTreeMap::new(),
            decode_spec: DecodeSpecRecord {
                decode_plan_id: DecodePlanId::new(0),
                spec: DecodeSpec::Argmax,
                requires_rng: false,
            },
            sequence_semantics: crate::s1::quant_graph::SequenceSemanticsSpec::identity(),
            provenance: BTreeMap::new(),
            classify_head: ClassifyHead {
                kind: ClassifyHeadKind::Tied,
                weight: TensorId::new(1),
                bias: None,
                logit_format: QuantFormat::I8,
            },
            residual_plan: ResidualPlan {
                activation_format: QuantFormat::I8,
                combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
            },
        }
    }

    fn mock_infer_ir() -> GbInferIR {
        use crate::s1::quant_graph::QuantFormat;
        use crate::s3::infer_ir::{
            GbInferIR, GbNode, InferIrIdentity, InferIrProvenance, InferOp, QuantGraphEntityRef,
            SemanticAnchor, TokenIngressMode, TokenInput, TokenInputId, ValueAxis, ValueDecl,
            ValueFormat, ValueKind, ValueLayout, ValueProducerRef,
        };

        let token_input = TokenInput::new(
            TokenInputId::new(0),
            ValueId::new(0),
            BTreeSet::from([TokenIngressMode::Prompt]),
        )
        .expect("token input builds");
        let node = GbNode {
            node_id: NodeId::new(0),
            op: InferOp::Embedding {
                token_input: TokenInputId::new(0),
            },
            inputs: vec![ValueId::new(0)],
            effects_in: Vec::new(),
            outputs: vec![ValueId::new(1)],
            effects_out: Vec::new(),
            reduction_site: None,
        };
        let mut provenance = InferIrProvenance::default();
        provenance.nodes.insert(
            NodeId::new(0),
            QuantGraphEntityRef::TokenInput {
                token_input: TokenInputId::new(0),
            },
        );
        provenance.values.insert(
            ValueId::new(0),
            ValueProducerRef::External {
                token_input: TokenInputId::new(0),
            },
        );
        provenance.values.insert(
            ValueId::new(1),
            ValueProducerRef::Node {
                node: NodeId::new(0),
            },
        );
        let anchors = BTreeMap::from([(NodeId::new(0), SemanticAnchor::new(hash(0x76)))]);

        GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: hash(0x71),
                infer_ir_policy_projection_hash: hash(0x72),
                static_budget_self_hash: hash(0x73),
                requested_runtime_modes_hash: hash(0x77),
                determinism: DeterminismClass::BitExact,
                topological_order_hash: hash(0x78),
            },
            vec![token_input],
            vec![node],
            vec![
                ValueDecl {
                    value_id: ValueId::new(0),
                    kind: ValueKind::InputToken,
                    format: ValueFormat::TokenIdDomain { vocab_size: 16 },
                    layout: ValueLayout::scalar(),
                },
                ValueDecl {
                    value_id: ValueId::new(1),
                    kind: ValueKind::EmbeddingOutput,
                    format: ValueFormat::Quant {
                        format: QuantFormat::I8,
                    },
                    layout: ValueLayout {
                        shape: vec![ValueAxis::Model],
                    },
                },
            ],
            Vec::new(),
            provenance,
            anchors,
        )
        .expect("infer ir builds")
    }

    fn mock_observation_plan() -> ObservationPlan {
        use crate::s4::observation_plan::{
            AnchorAttachmentTable, ObservationPlan, ObservationPlanIdentity, ObservationProvenance,
            TraceBudgetProjection,
        };
        use gbf_policy::ObservabilityMode;
        use gbf_workload::WorkloadId;

        ObservationPlan {
            identity: ObservationPlanIdentity {
                infer_ir_self_hash: hash(0x81),
                quant_graph_self_hash: hash(0x82),
                semantic_checkpoint_schema_hash: hash(0x83),
                observation_policy_projection_hash: hash(0x84),
                determinism: DeterminismClass::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: gbf_abi::trace::TraceBudget {
                    max_events_per_slice: 8,
                    max_bytes_per_frame: 128,
                    drop_policy: gbf_abi::trace::TraceDropPolicy::DropOldest,
                },
                workload_id: WorkloadId::from("stage4.cache.fixture"),
                probe_registry_hash: hash(0x86),
                metric_registry_hash: hash(0x87),
                trace_event_layout_registry_hash: hash(0x88),
            },
            semantic: Vec::new(),
            probes: Vec::new(),
            metrics: Vec::new(),
            anchor_table: AnchorAttachmentTable {
                semantic: BTreeMap::new(),
                probes: BTreeMap::new(),
                metrics: BTreeMap::new(),
            },
            provenance: ObservationProvenance {
                semantic_provenance: BTreeMap::new(),
                probe_provenance: BTreeMap::new(),
                metric_provenance: BTreeMap::new(),
            },
            trace_budget_projection: TraceBudgetProjection {
                projected_max_events_per_slice: 0,
                projected_max_bytes_per_frame: 0,
                fits_declared_budget: true,
            },
        }
    }

    fn mock_range_plan() -> RangePlan {
        use crate::s5::range_plan::{RangePlan, RangePlanIdentity, RangePlanProvenance};

        RangePlan {
            identity: RangePlanIdentity {
                infer_ir_self_hash: hash(0x91),
                quant_graph_self_hash: hash(0x92),
                static_budget_self_hash: hash(0x93),
                range_policy_projection_hash: hash(0x94),
                determinism: DeterminismClass::BitExact,
            },
            entries: Vec::new(),
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        }
    }

    fn make_valid_inputs() -> StoragePlanInputs {
        let policy = mock_policy_fixture();
        let quant_graph = mock_quant_graph();
        let infer_ir = mock_infer_ir();
        let observation_plan = mock_observation_plan();
        let range_plan = mock_range_plan();

        let policy_hash = policy.canonical_hash();
        let quant_graph_hash = crate::s1::quant_graph::quant_graph_self_hash(&quant_graph).unwrap();
        let infer_ir_hash = crate::s3::infer_ir::infer_ir_self_hash(&infer_ir).unwrap();
        let observation_plan_hash =
            crate::s4::observation_plan::observation_plan_self_hash(&observation_plan).unwrap();
        let range_plan_hash = crate::s5::range_plan::range_plan_self_hash(&range_plan).unwrap();

        StoragePlanInputs {
            policy,
            policy_hash,
            quant_graph,
            quant_graph_hash,
            infer_ir,
            infer_ir_hash,
            observation_plan,
            observation_plan_hash,
            range_plan,
            range_plan_hash,
        }
    }

    #[test]
    fn test_storage_plan_inputs_canonicalize_happy_path() {
        let inputs = make_valid_inputs();
        assert!(canonicalize_inputs(&inputs).is_ok());
    }

    #[test]
    fn test_storage_plan_inputs_mismatches() {
        let inputs = make_valid_inputs();

        // 1. perturbed policy_hash
        {
            let mut bad_inputs = inputs.clone();
            bad_inputs.policy_hash = hash(0x99);
            let err = canonicalize_inputs(&bad_inputs).unwrap_err();
            assert_eq!(err.code, ValidationCode::StoragePolicyHashMismatch);
            assert_eq!(err.severity, gbf_policy::DiagnosticSeverity::Hard);
            assert_eq!(err.origin, ValidationOrigin::StoragePlanConstruction);
            if let ValidationDetail::HashMismatch { expected, observed } = err.detail {
                assert_eq!(expected, hash(0x99));
                assert_eq!(observed, inputs.policy_hash);
            } else {
                panic!("Expected HashMismatch detail");
            }
            assert_eq!(err.provenance.len(), 2);
            assert_eq!(err.provenance[0].kind, "recorded");
            assert_eq!(err.provenance[1].kind, "observed");
        }

        // 2. perturbed quant_graph_hash
        {
            let mut bad_inputs = inputs.clone();
            bad_inputs.quant_graph_hash = hash(0x99);
            let err = canonicalize_inputs(&bad_inputs).unwrap_err();
            assert_eq!(err.code, ValidationCode::StorageQuantGraphHashMismatch);
            assert_eq!(err.severity, gbf_policy::DiagnosticSeverity::Hard);
            assert_eq!(err.origin, ValidationOrigin::StoragePlanConstruction);
            if let ValidationDetail::HashMismatch { expected, observed } = err.detail {
                assert_eq!(expected, hash(0x99));
                assert_eq!(observed, inputs.quant_graph_hash);
            } else {
                panic!("Expected HashMismatch detail");
            }
        }

        // 3. perturbed infer_ir_hash
        {
            let mut bad_inputs = inputs.clone();
            bad_inputs.infer_ir_hash = hash(0x99);
            let err = canonicalize_inputs(&bad_inputs).unwrap_err();
            assert_eq!(err.code, ValidationCode::StorageInferIrHashMismatch);
            assert_eq!(err.severity, gbf_policy::DiagnosticSeverity::Hard);
            assert_eq!(err.origin, ValidationOrigin::StoragePlanConstruction);
            if let ValidationDetail::HashMismatch { expected, observed } = err.detail {
                assert_eq!(expected, hash(0x99));
                assert_eq!(observed, inputs.infer_ir_hash);
            } else {
                panic!("Expected HashMismatch detail");
            }
        }

        // 4. perturbed observation_plan_hash
        {
            let mut bad_inputs = inputs.clone();
            bad_inputs.observation_plan_hash = hash(0x99);
            let err = canonicalize_inputs(&bad_inputs).unwrap_err();
            assert_eq!(err.code, ValidationCode::StorageObservationPlanHashMismatch);
            assert_eq!(err.severity, gbf_policy::DiagnosticSeverity::Hard);
            assert_eq!(err.origin, ValidationOrigin::StoragePlanConstruction);
            if let ValidationDetail::HashMismatch { expected, observed } = err.detail {
                assert_eq!(expected, hash(0x99));
                assert_eq!(observed, inputs.observation_plan_hash);
            } else {
                panic!("Expected HashMismatch detail");
            }
        }

        // 5. perturbed range_plan_hash
        {
            let mut bad_inputs = inputs.clone();
            bad_inputs.range_plan_hash = hash(0x99);
            let err = canonicalize_inputs(&bad_inputs).unwrap_err();
            assert_eq!(err.code, ValidationCode::StorageRangePlanHashMismatch);
            assert_eq!(err.severity, gbf_policy::DiagnosticSeverity::Hard);
            assert_eq!(err.origin, ValidationOrigin::StoragePlanConstruction);
            if let ValidationDetail::HashMismatch { expected, observed } = err.detail {
                assert_eq!(expected, hash(0x99));
                assert_eq!(observed, inputs.range_plan_hash);
            } else {
                panic!("Expected HashMismatch detail");
            }
        }
    }

    #[test]
    fn test_identity_no_output_hash() {
        let inputs = make_valid_inputs();
        let identity = StoragePlanInputIdentity::from_inputs(&inputs);
        let serialized = serde_json::to_value(&identity).unwrap();
        assert!(
            serialized
                .as_object()
                .unwrap()
                .get("report_self_hash")
                .is_none()
        );
        assert!(serialized.as_object().unwrap().get("self_hash").is_none());
    }

    #[test]
    fn test_identity_canonical_json_stable() {
        let inputs = make_valid_inputs();
        let identity = StoragePlanInputIdentity::from_inputs(&inputs);
        let canonical_bytes =
            gbf_foundation::canonical_json::CanonicalJson::to_vec(&identity).unwrap();
        let json_str = String::from_utf8(canonical_bytes).unwrap();

        // Assert sorting and specific fields existence/types
        assert!(json_str.starts_with('{'));
        assert!(json_str.ends_with('}'));

        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["schema"].as_str().unwrap(), "storage_plan.v1");
        assert_eq!(parsed["schema_version"]["major"].as_u64().unwrap(), 1);
        assert_eq!(parsed["schema_version"]["minor"].as_u64().unwrap(), 0);
        assert_eq!(parsed["schema_version"]["patch"].as_u64().unwrap(), 0);
        assert_eq!(parsed["determinism"]["kind"].as_str().unwrap(), "BitExact");
    }

    #[test]
    fn test_identity_schema_pins() {
        let inputs = make_valid_inputs();
        let identity = StoragePlanInputIdentity::from_inputs(&inputs);
        assert_eq!(identity.schema, ReportSchemaId::new("storage_plan.v1"));
        assert_eq!(identity.schema_version, SemVer::new(1, 0, 0));
    }

    #[test]
    fn test_binding_justification_serde() {
        let rule = BindingJustification::DecisionRule {
            rule_id: DecisionRuleId::new(123),
        };
        let serialized_rule = serde_json::to_string(&rule).unwrap();
        assert!(serialized_rule.contains(r#""kind":"DecisionRule""#));
        assert!(serialized_rule.contains(r#""rule_id":123"#));

        let deserialized_rule: BindingJustification =
            serde_json::from_str(&serialized_rule).unwrap();
        assert_eq!(deserialized_rule, rule);

        let recompute = BindingJustification::ForcedRecompute {};
        let serialized_recompute = serde_json::to_string(&recompute).unwrap();
        assert!(serialized_recompute.contains(r#""kind":"ForcedRecompute""#));

        let deserialized_recompute: BindingJustification =
            serde_json::from_str(&serialized_recompute).unwrap();
        assert_eq!(deserialized_recompute, recompute);

        // deny_unknown_fields check
        let bad_json = r#"{"kind":"ForcedRecompute","unknown_field":true}"#;
        assert!(serde_json::from_str::<BindingJustification>(bad_json).is_err());
    }

    #[test]
    fn test_materialization_internal_serde() {
        let mat = Materialization::Recompute {};
        let serialized_mat = serde_json::to_string(&mat).unwrap();
        assert!(serialized_mat.contains(r#""kind":"Recompute""#));

        let deserialized_mat: Materialization = serde_json::from_str(&serialized_mat).unwrap();
        assert_eq!(deserialized_mat, mat);

        // deny_unknown_fields check
        let bad_json = r#"{"kind":"Recompute","extra":1}"#;
        assert!(serde_json::from_str::<Materialization>(bad_json).is_err());
    }

    #[test]
    fn test_storage_binding_field_count() {
        let binding = StorageBinding {
            value: ValueId::new(1),
            materialization: Materialization::Recompute {},
            alias_class: AliasClassId::new(2),
            live_range: AbstractLiveRange {
                def_node: NodeId::new(0),
                first_use_node: None,
                last_use_node: None,
                lifetime_class: LifetimeClass::Slice,
                checkpoint_stable: false,
            },
            justification: BindingJustification::ForcedRecompute {},
        };
        let val = serde_json::to_value(&binding).unwrap();
        let obj = val.as_object().unwrap();
        assert_eq!(obj.len(), 5);
        assert!(obj.contains_key("value"));
        assert!(obj.contains_key("materialization"));
        assert!(obj.contains_key("alias_class"));
        assert!(obj.contains_key("live_range"));
        assert!(obj.contains_key("justification"));
    }
}
