//! Core public type surface for Stage 6 `StoragePlan`.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use crate::s1::quant_graph::{DeterminismClass, QuantGraph, quant_graph_self_hash};
use crate::s3::infer_ir::{GbInferIR, NodeId, ValueId, infer_ir_self_hash};
use crate::s4::observation_plan::{ObservationPlan, observation_plan_self_hash};
use crate::s5::range_plan::{RangePlan, range_plan_self_hash};
use crate::storage_plan::predicates::{ValueFormat, ValueRole};
use gbf_foundation::{CanonicalJsonError, DomainHash, EvidenceRef, Hash256, SemVer};
use gbf_policy::{ResolvedCompilePolicy, StoragePlanDiagnosticCode};
use gbf_report::ReportSchemaId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const STORAGE_PLAN_SCHEMA_ID: &str = "storage_plan.v1";
pub const STORAGE_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

impl StoragePlanInputs {
    #[must_use]
    pub const fn recorded_hashes(&self) -> StoragePlanInputHashes {
        StoragePlanInputHashes {
            quant_graph_hash: self.quant_graph_hash,
            infer_ir_hash: self.infer_ir_hash,
            observation_plan_hash: self.observation_plan_hash,
            range_plan_hash: self.range_plan_hash,
            policy_hash: self.policy_hash,
        }
    }

    #[must_use]
    pub fn input_identity(&self) -> StoragePlanInputIdentity {
        StoragePlanInputIdentity::new(self.recorded_hashes(), self.range_plan.identity.determinism)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanInputHashes {
    pub quant_graph_hash: Hash256,
    pub infer_ir_hash: Hash256,
    pub observation_plan_hash: Hash256,
    pub range_plan_hash: Hash256,
    pub policy_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub fn new(hashes: StoragePlanInputHashes, determinism: DeterminismClass) -> Self {
        Self {
            quant_graph_hash: hashes.quant_graph_hash,
            infer_ir_hash: hashes.infer_ir_hash,
            observation_plan_hash: hashes.observation_plan_hash,
            range_plan_hash: hashes.range_plan_hash,
            policy_hash: hashes.policy_hash,
            determinism,
            schema: ReportSchemaId::from(STORAGE_PLAN_SCHEMA_ID),
            schema_version: STORAGE_PLAN_SCHEMA_VERSION,
        }
    }

    #[must_use]
    pub fn hash_for_product(&self, product: StoragePlanInputProduct) -> Hash256 {
        match product {
            StoragePlanInputProduct::QuantGraph => self.quant_graph_hash,
            StoragePlanInputProduct::InferIr => self.infer_ir_hash,
            StoragePlanInputProduct::ObservationPlan => self.observation_plan_hash,
            StoragePlanInputProduct::RangePlan => self.range_plan_hash,
            StoragePlanInputProduct::Policy => self.policy_hash,
        }
    }
}

impl StoragePlanInputHashes {
    #[must_use]
    pub fn hash_for_product(&self, product: StoragePlanInputProduct) -> Hash256 {
        match product {
            StoragePlanInputProduct::QuantGraph => self.quant_graph_hash,
            StoragePlanInputProduct::InferIr => self.infer_ir_hash,
            StoragePlanInputProduct::ObservationPlan => self.observation_plan_hash,
            StoragePlanInputProduct::RangePlan => self.range_plan_hash,
            StoragePlanInputProduct::Policy => self.policy_hash,
        }
    }
}

pub fn canonicalize_inputs(inputs: &StoragePlanInputs) -> Result<(), StoragePlanInputDiagnostic> {
    let computed = StoragePlanInputHashes {
        quant_graph_hash: quant_graph_self_hash(&inputs.quant_graph).map_err(|error| {
            StoragePlanInputDiagnostic::hash_compute(
                StoragePlanInputProduct::QuantGraph,
                error.to_string(),
            )
        })?,
        infer_ir_hash: infer_ir_self_hash(&inputs.infer_ir).map_err(|error| {
            StoragePlanInputDiagnostic::hash_compute(
                StoragePlanInputProduct::InferIr,
                error.to_string(),
            )
        })?,
        observation_plan_hash: observation_plan_self_hash(&inputs.observation_plan).map_err(
            |error| {
                StoragePlanInputDiagnostic::hash_compute(
                    StoragePlanInputProduct::ObservationPlan,
                    error.to_string(),
                )
            },
        )?,
        range_plan_hash: range_plan_self_hash(&inputs.range_plan).map_err(|error| {
            StoragePlanInputDiagnostic::hash_compute(
                StoragePlanInputProduct::RangePlan,
                error.to_string(),
            )
        })?,
        policy_hash: resolved_compile_policy_hash(&inputs.policy).map_err(|error| {
            StoragePlanInputDiagnostic::hash_compute(
                StoragePlanInputProduct::Policy,
                error.to_string(),
            )
        })?,
    };

    validate_storage_plan_input_hashes(&inputs.recorded_hashes(), &computed)
}

pub fn validate_storage_plan_input_hashes(
    recorded: &StoragePlanInputHashes,
    computed: &StoragePlanInputHashes,
) -> Result<(), StoragePlanInputDiagnostic> {
    for spec in STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS {
        check_recorded_hash(
            spec.input_code,
            spec.product,
            recorded.hash_for_product(spec.product),
            computed.hash_for_product(spec.product),
        )?;
    }

    Ok(())
}

/// Stage-6-local policy hash for the F-B8 input identity contract.
///
/// RFC F-B8 names this field `policy_hash = policy.canonical_hash`, but the
/// current policy crate exposes resolved policies rather than a single
/// canonical policy product hash owner. Until that owner exists in gbf-policy,
/// this domain-separated hash is the explicit Stage 6 contract and the only
/// bridge point callers should use.
pub fn resolved_compile_policy_hash(
    policy: &ResolvedCompilePolicy,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "ResolvedCompilePolicy",
        STORAGE_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(policy)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanInputDiagnostic {
    HashMismatch {
        code: StoragePlanInputDiagnosticCode,
        product: StoragePlanInputProduct,
        recorded: Hash256,
        computed: Hash256,
    },
    HashComputeFailed {
        product: StoragePlanInputProduct,
        message: String,
    },
}

impl StoragePlanInputDiagnostic {
    fn hash_mismatch(
        code: StoragePlanInputDiagnosticCode,
        product: StoragePlanInputProduct,
        recorded: Hash256,
        computed: Hash256,
    ) -> Self {
        Self::HashMismatch {
            code,
            product,
            recorded,
            computed,
        }
    }

    fn hash_compute(product: StoragePlanInputProduct, message: String) -> Self {
        Self::HashComputeFailed { product, message }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanInputDiagnosticCode {
    Store020RangePlan,
    Store021InferIr,
    Store022ObservationPlan,
    Store023QuantGraph,
    Store024Policy,
}

impl StoragePlanInputDiagnosticCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Store020RangePlan => "STORE-020",
            Self::Store021InferIr => "STORE-021",
            Self::Store022ObservationPlan => "STORE-022",
            Self::Store023QuantGraph => "STORE-023",
            Self::Store024Policy => "STORE-024",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanInputProduct {
    QuantGraph,
    InferIr,
    ObservationPlan,
    RangePlan,
    Policy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoragePlanInputHashMismatchSpec {
    pub input_code: StoragePlanInputDiagnosticCode,
    pub storage_code: StoragePlanDiagnosticCode,
    pub product: StoragePlanInputProduct,
    pub identity_field: &'static str,
}

pub const STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS: [StoragePlanInputHashMismatchSpec; 5] = [
    StoragePlanInputHashMismatchSpec {
        input_code: StoragePlanInputDiagnosticCode::Store020RangePlan,
        storage_code: StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
        product: StoragePlanInputProduct::RangePlan,
        identity_field: "range_plan",
    },
    StoragePlanInputHashMismatchSpec {
        input_code: StoragePlanInputDiagnosticCode::Store021InferIr,
        storage_code: StoragePlanDiagnosticCode::StorageInferIrHashMismatch,
        product: StoragePlanInputProduct::InferIr,
        identity_field: "infer_ir",
    },
    StoragePlanInputHashMismatchSpec {
        input_code: StoragePlanInputDiagnosticCode::Store022ObservationPlan,
        storage_code: StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch,
        product: StoragePlanInputProduct::ObservationPlan,
        identity_field: "observation_plan",
    },
    StoragePlanInputHashMismatchSpec {
        input_code: StoragePlanInputDiagnosticCode::Store023QuantGraph,
        storage_code: StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch,
        product: StoragePlanInputProduct::QuantGraph,
        identity_field: "quant_graph",
    },
    StoragePlanInputHashMismatchSpec {
        input_code: StoragePlanInputDiagnosticCode::Store024Policy,
        storage_code: StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
        product: StoragePlanInputProduct::Policy,
        identity_field: "policy",
    },
];

fn check_recorded_hash(
    code: StoragePlanInputDiagnosticCode,
    product: StoragePlanInputProduct,
    recorded: Hash256,
    computed: Hash256,
) -> Result<(), StoragePlanInputDiagnostic> {
    if recorded == computed {
        Ok(())
    } else {
        Err(StoragePlanInputDiagnostic::hash_mismatch(
            code, product, recorded, computed,
        ))
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum StorageClass {
    WramHot,
    HramHot,
    SramPaged,
    RomConst,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum LifetimeClass {
    Slice,
    ResumeWindow,
    Token,
    Session,
    Persistent,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum Materialization {
    Recompute,
    Materialize {
        class: StorageClass,
        lifetime: LifetimeClass,
    },
    Persist {
        page: PersistPageId,
        commit_group: CommitGroupId,
    },
}

/// Abstract topological live interval for a value in GbInferIR node order.
///
/// This is the evidence alias planning uses to prove non-conflict between
/// values that share a lifetime class. It is deliberately abstract: concrete
/// placement remains owned by later planning stages.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct AbstractLiveRange {
    /// Node that defines the value.
    pub def_node: NodeId,
    /// First node that observes the value, or `None` for a definition with no
    /// observed use in this product.
    pub first_use_node: Option<NodeId>,
    /// Last node that observes the value, or `None` when no observed use exists.
    pub last_use_node: Option<NodeId>,
    /// Minimum semantic duration required by the value.
    pub lifetime_class: LifetimeClass,
    /// Whether this interval may survive a checkpoint boundary unchanged.
    pub checkpoint_stable: bool,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct StorageBinding {
    pub value: ValueId,
    pub materialization: Materialization,
    pub alias_class: AliasClassId,
    pub live_range: AbstractLiveRange,
    pub justification: BindingJustification,
}

/// Reason a storage binding exists in the plan.
///
/// F-B8 v1 keeps this closed to real decision rules plus the explicit
/// forced-recompute path. Refinement proposals are recorded in policy
/// provenance, not re-emitted through this enum.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum BindingJustification {
    /// The binding was admitted by a named deterministic decision rule.
    DecisionRule(DecisionRuleId),
    /// The policy forced this value through the first-class recompute path.
    ForcedRecompute,
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct AliasClassId(pub u32);

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct AliasClassFingerprint(pub Hash256);

impl AliasClassFingerprint {
    const DOMAIN: DomainHash<'static> =
        DomainHash::new("gbf-codegen", "AliasClassId", "v1", "1.0.0");

    pub fn for_members(
        members: &NonEmptySortedSet<ValueId>,
        intent: AliasIntent,
    ) -> Result<Self, CanonicalJsonError> {
        let payload = AliasClassFingerprintPayload {
            members: members.iter().copied().collect(),
            intent,
        };

        Self::DOMAIN.hash(&payload).map(Self)
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

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize)]
pub struct AliasClass {
    id: AliasClassId,
    fingerprint: AliasClassFingerprint,
    members: NonEmptySortedSet<ValueId>,
    intent: AliasIntent,
}

impl AliasClass {
    pub fn new(
        id: AliasClassId,
        members: NonEmptySortedSet<ValueId>,
        intent: AliasIntent,
    ) -> Result<Self, AliasClassError> {
        validate_alias_intent(members.len(), intent)?;
        let fingerprint = AliasClassFingerprint::for_members(&members, intent)
            .map_err(AliasClassError::Fingerprint)?;

        Ok(Self {
            id,
            fingerprint,
            members,
            intent,
        })
    }

    pub fn from_members<I>(
        id: AliasClassId,
        members: I,
        intent: AliasIntent,
    ) -> Result<Self, AliasClassError>
    where
        I: IntoIterator<Item = ValueId>,
    {
        let members = NonEmptySortedSet::new(members)
            .map_err(|NonEmptySortedSetError| AliasClassError::EmptyMembers)?;
        Self::new(id, members, intent)
    }

    #[must_use]
    pub fn id(&self) -> &AliasClassId {
        &self.id
    }

    #[must_use]
    pub fn fingerprint(&self) -> AliasClassFingerprint {
        self.fingerprint
    }

    #[must_use]
    pub fn members(&self) -> &NonEmptySortedSet<ValueId> {
        &self.members
    }

    #[must_use]
    pub fn intent(&self) -> AliasIntent {
        self.intent
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum ValueSelector {
    Value(ValueId),
    AliasClass(AliasClassFingerprint),
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct NonEmptySortedSet<T> {
    items: BTreeSet<T>,
}

impl<T> NonEmptySortedSet<T>
where
    T: Ord,
{
    pub fn new<I>(items: I) -> Result<Self, NonEmptySortedSetError>
    where
        I: IntoIterator<Item = T>,
    {
        Self::from_btree_set(items.into_iter().collect())
    }

    pub fn from_btree_set(items: BTreeSet<T>) -> Result<Self, NonEmptySortedSetError> {
        if items.is_empty() {
            return Err(NonEmptySortedSetError);
        }

        Ok(Self { items })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_singleton(&self) -> bool {
        self.items.len() == 1
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.items.iter()
    }

    #[must_use]
    pub fn as_btree_set(&self) -> &BTreeSet<T> {
        &self.items
    }
}

impl<T> Serialize for NonEmptySortedSet<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.items.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for NonEmptySortedSet<T>
where
    T: Ord + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let items = BTreeSet::<T>::deserialize(deserializer)?;
        Self::from_btree_set(items).map_err(serde::de::Error::custom)
    }
}

impl<'a, T> IntoIterator for &'a NonEmptySortedSet<T> {
    type Item = &'a T;
    type IntoIter = std::collections::btree_set::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct NonEmptySortedSetError;

impl fmt::Display for NonEmptySortedSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NonEmptySortedSet must contain at least one item")
    }
}

impl Error for NonEmptySortedSetError {}

#[derive(Debug)]
pub enum AliasClassError {
    EmptyMembers,
    SingletonMustBeNoAlias { intent: AliasIntent },
    NoAliasRequiresSingleton { member_count: usize },
    Fingerprint(CanonicalJsonError),
}

impl fmt::Display for AliasClassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMembers => f.write_str("AliasClass must contain at least one member"),
            Self::SingletonMustBeNoAlias { intent } => {
                write!(f, "singleton AliasClass must use NoAlias, got {intent:?}")
            }
            Self::NoAliasRequiresSingleton { member_count } => {
                write!(
                    f,
                    "NoAlias AliasClass must be a singleton, got {member_count} members"
                )
            }
            Self::Fingerprint(error) => write!(f, "alias-class fingerprint failed: {error}"),
        }
    }
}

impl Error for AliasClassError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Fingerprint(error) => Some(error),
            Self::EmptyMembers
            | Self::SingletonMustBeNoAlias { .. }
            | Self::NoAliasRequiresSingleton { .. } => None,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct DecisionRuleId(pub u32);

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct PersistPageId(pub u32);

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct CommitGroupId(pub u32);

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
#[serde(deny_unknown_fields)]
pub struct PersistSchemaPin {
    pub state_schema: u16,
    pub requires_semantic_state_hash: bool,
    pub requires_resume_abi_hash: bool,
    pub requires_build_identity_hash: bool,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistPageDecl {
    pub id: PersistPageId,
    pub kind: PersistKind,
    pub durability: DurabilityClass,
    pub schema_pin: PersistSchemaPin,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitGroupDecl {
    pub id: CommitGroupId,
    pub members: NonEmptySortedSet<PersistPageId>,
    pub kind_set: BTreeSet<PersistKind>,
    pub atomicity: CommitAtomicityClass,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum CommitAtomicityClass {
    AllOrNothing,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub enum CommitGroupReason {
    PerSequenceStateSlot,
    SequenceStateWithTranscript,
    ContinuationWithSequenceState,
    ContinuationOnly,
    Independent,
    OrderedRecoverable,
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
pub struct AdmittingPredicateId(pub u32);

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageProvenance {
    pub bindings: BTreeMap<ValueId, BindingProvenance>,
    pub alias_classes: BTreeMap<AliasClassId, AliasClassProvenance>,
    pub persist_pages: BTreeMap<PersistPageId, PersistPageProvenance>,
    pub commit_groups: BTreeMap<CommitGroupId, CommitGroupProvenance>,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingProvenance {
    pub admitting_predicate: AdmittingPredicateId,
    pub decision_rule: DecisionRuleId,
    pub policy_refinement_applied: bool,
    #[serde(deserialize_with = "deserialize_sorted_evidence")]
    pub evidence: Vec<EvidenceRef>,
    pub op_output_role: Option<ValueRole>,
    pub op_output_format: Option<ValueFormat>,
}

impl BindingProvenance {
    #[must_use]
    pub fn new(
        admitting_predicate: AdmittingPredicateId,
        decision_rule: DecisionRuleId,
        policy_refinement_applied: bool,
        mut evidence: Vec<EvidenceRef>,
        op_output_role: Option<ValueRole>,
        op_output_format: Option<ValueFormat>,
    ) -> Self {
        evidence.sort();
        Self {
            admitting_predicate,
            decision_rule,
            policy_refinement_applied,
            evidence,
            op_output_role,
            op_output_format,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasClassProvenance {
    pub admitting_intent: AliasIntent,
    #[serde(deserialize_with = "deserialize_sorted_evidence")]
    pub evidence: Vec<EvidenceRef>,
}

impl AliasClassProvenance {
    #[must_use]
    pub fn new(admitting_intent: AliasIntent, mut evidence: Vec<EvidenceRef>) -> Self {
        evidence.sort();
        Self {
            admitting_intent,
            evidence,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistPageProvenance {
    pub source: PersistPageSource,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PersistPageSource {
    SequenceStateSlot { layer: u16, slot: u32 },
    Continuation,
    Transcript { family: u32 },
    Harness { family: u32 },
    Trace { family: u32 },
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitGroupProvenance {
    pub reason: CommitGroupReason,
    #[serde(deserialize_with = "deserialize_sorted_evidence")]
    pub evidence: Vec<EvidenceRef>,
}

impl CommitGroupProvenance {
    #[must_use]
    pub fn new(reason: CommitGroupReason, mut evidence: Vec<EvidenceRef>) -> Self {
        evidence.sort();
        Self { reason, evidence }
    }
}

fn deserialize_sorted_evidence<'de, D>(deserializer: D) -> Result<Vec<EvidenceRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut evidence = Vec::<EvidenceRef>::deserialize(deserializer)?;
    evidence.sort();
    Ok(evidence)
}

#[derive(Serialize)]
struct AliasClassFingerprintPayload {
    members: Vec<ValueId>,
    intent: AliasIntent,
}

fn validate_alias_intent(member_count: usize, intent: AliasIntent) -> Result<(), AliasClassError> {
    match (member_count, intent) {
        (1, AliasIntent::NoAlias) => Ok(()),
        (1, intent) => Err(AliasClassError::SingletonMustBeNoAlias { intent }),
        (_, AliasIntent::NoAlias) => {
            Err(AliasClassError::NoAliasRequiresSingleton { member_count })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::CanonicalJson;

    #[test]
    fn lifetime_class_order_matches_rfc() {
        assert!(LifetimeClass::Slice < LifetimeClass::ResumeWindow);
        assert!(LifetimeClass::ResumeWindow < LifetimeClass::Token);
        assert!(LifetimeClass::Token < LifetimeClass::Session);
        assert!(LifetimeClass::Session < LifetimeClass::Persistent);
    }

    #[test]
    fn storage_binding_carries_exact_t1_fields() {
        let live_range = AbstractLiveRange {
            def_node: NodeId::new(1),
            first_use_node: Some(NodeId::new(2)),
            last_use_node: Some(NodeId::new(3)),
            lifetime_class: LifetimeClass::Token,
            checkpoint_stable: true,
        };
        let binding = StorageBinding {
            value: ValueId::new(4),
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Token,
            },
            alias_class: AliasClassId(5),
            live_range: live_range.clone(),
            justification: BindingJustification::DecisionRule(DecisionRuleId(6)),
        };

        let StorageBinding {
            value,
            materialization,
            alias_class,
            live_range: binding_live_range,
            justification,
        } = binding;

        assert_eq!(value, ValueId::new(4));
        assert_eq!(
            materialization,
            Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Token,
            }
        );
        assert_eq!(alias_class, AliasClassId(5));
        assert_eq!(binding_live_range, live_range);
        assert_eq!(
            justification,
            BindingJustification::DecisionRule(DecisionRuleId(6))
        );
    }

    #[test]
    fn binding_justification_has_exact_t1_variants() {
        fn variant_index(justification: BindingJustification) -> u8 {
            match justification {
                BindingJustification::DecisionRule(_) => 0,
                BindingJustification::ForcedRecompute => 1,
            }
        }

        let variants = [
            BindingJustification::DecisionRule(DecisionRuleId(7)),
            BindingJustification::ForcedRecompute,
        ];

        assert_eq!(variants.len(), 2);
        assert_eq!(variant_index(variants[0].clone()), 0);
        assert_eq!(variant_index(variants[1].clone()), 1);
    }

    #[test]
    fn alias_fingerprint_is_order_insensitive_on_members() {
        let ascending = NonEmptySortedSet::new([ValueId::new(1), ValueId::new(2)])
            .expect("members are non-empty");
        let descending = NonEmptySortedSet::new([ValueId::new(2), ValueId::new(1)])
            .expect("members are non-empty");

        let first = AliasClassFingerprint::for_members(&ascending, AliasIntent::ScratchReuse)
            .expect("fingerprint computes");
        let second = AliasClassFingerprint::for_members(&descending, AliasIntent::ScratchReuse)
            .expect("fingerprint computes");

        assert_eq!(first, second);
    }

    #[test]
    fn alias_class_fingerprint_tracks_members_and_intent() {
        let first = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(1), ValueId::new(2)],
            AliasIntent::ScratchReuse,
        )
        .expect("alias class constructs");
        let second = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(2), ValueId::new(1)],
            AliasIntent::ScratchReuse,
        )
        .expect("alias class constructs");
        let different_intent = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(1), ValueId::new(2)],
            AliasIntent::PingPong,
        )
        .expect("alias class constructs");

        assert_eq!(first, second);
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.fingerprint(), different_intent.fingerprint());
    }

    #[test]
    fn alias_class_rejects_noalias_multi_member() {
        let error = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(1), ValueId::new(2)],
            AliasIntent::NoAlias,
        )
        .expect_err("NoAlias must stay singleton-only");

        assert!(matches!(
            error,
            AliasClassError::NoAliasRequiresSingleton { member_count: 2 }
        ));
    }

    #[test]
    fn alias_class_rejects_non_noalias_singleton() {
        let error = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(1)],
            AliasIntent::ScratchReuse,
        )
        .expect_err("singletons must be NoAlias");

        assert!(matches!(
            error,
            AliasClassError::SingletonMustBeNoAlias {
                intent: AliasIntent::ScratchReuse
            }
        ));
    }

    #[test]
    fn value_selector_uses_fingerprint_for_alias_class_selectors() {
        let members = NonEmptySortedSet::new([ValueId::new(1), ValueId::new(2)])
            .expect("members are non-empty");
        let fingerprint = AliasClassFingerprint::for_members(&members, AliasIntent::ScratchReuse)
            .expect("fingerprint computes");
        let selector = ValueSelector::AliasClass(fingerprint);

        match selector {
            ValueSelector::Value(value) => panic!("unexpected value selector: {value:?}"),
            ValueSelector::AliasClass(selected) => assert_eq!(selected, fingerprint),
        }
    }

    #[test]
    fn input_identity_round_trips_without_output_hash_field() {
        let identity = StoragePlanInputIdentity::new(input_hashes(), DeterminismClass::BitExact);
        let bytes = CanonicalJson::to_vec(&identity).expect("identity canonicalizes");
        let decoded: StoragePlanInputIdentity =
            serde_json::from_slice(&bytes).expect("identity decodes");
        let value = serde_json::to_value(&identity).expect("identity serializes");
        let object = value.as_object().expect("identity is a JSON object");

        assert_eq!(decoded, identity);
        assert_eq!(
            identity.schema,
            ReportSchemaId::from(STORAGE_PLAN_SCHEMA_ID)
        );
        assert_eq!(identity.schema_version, STORAGE_PLAN_SCHEMA_VERSION);
        assert!(!object.contains_key("report_self_hash"));
        assert!(!object.keys().any(|key| key.contains("report_self_hash")));
    }

    #[test]
    fn input_identity_is_computable_before_storage_plan_body_exists() {
        let hashes = input_hashes();
        let identity = StoragePlanInputIdentity::new(hashes, DeterminismClass::Deterministic);

        assert_eq!(identity.quant_graph_hash, hashes.quant_graph_hash);
        assert_eq!(identity.infer_ir_hash, hashes.infer_ir_hash);
        assert_eq!(identity.observation_plan_hash, hashes.observation_plan_hash);
        assert_eq!(identity.range_plan_hash, hashes.range_plan_hash);
        assert_eq!(identity.policy_hash, hashes.policy_hash);
        assert_eq!(identity.determinism, DeterminismClass::Deterministic);
    }

    #[test]
    fn canonicalize_input_hashes_reports_store_020_to_024() {
        let computed = input_hashes();
        let cases = [
            (
                StoragePlanInputDiagnosticCode::Store020RangePlan,
                StoragePlanInputProduct::RangePlan,
                "STORE-020",
                with_range_plan_hash(computed, hash(0x90)),
                hash(0x90),
                computed.range_plan_hash,
            ),
            (
                StoragePlanInputDiagnosticCode::Store021InferIr,
                StoragePlanInputProduct::InferIr,
                "STORE-021",
                with_infer_ir_hash(computed, hash(0x91)),
                hash(0x91),
                computed.infer_ir_hash,
            ),
            (
                StoragePlanInputDiagnosticCode::Store022ObservationPlan,
                StoragePlanInputProduct::ObservationPlan,
                "STORE-022",
                with_observation_plan_hash(computed, hash(0x92)),
                hash(0x92),
                computed.observation_plan_hash,
            ),
            (
                StoragePlanInputDiagnosticCode::Store023QuantGraph,
                StoragePlanInputProduct::QuantGraph,
                "STORE-023",
                with_quant_graph_hash(computed, hash(0x93)),
                hash(0x93),
                computed.quant_graph_hash,
            ),
            (
                StoragePlanInputDiagnosticCode::Store024Policy,
                StoragePlanInputProduct::Policy,
                "STORE-024",
                with_policy_hash(computed, hash(0x94)),
                hash(0x94),
                computed.policy_hash,
            ),
        ];

        for (code, product, code_text, recorded, expected_recorded, expected_computed) in cases {
            let error = validate_storage_plan_input_hashes(&recorded, &computed)
                .expect_err("mismatched input hash must fail");

            match error {
                StoragePlanInputDiagnostic::HashMismatch {
                    code: observed_code,
                    product: observed_product,
                    recorded,
                    computed,
                } => {
                    assert_eq!(observed_code, code);
                    assert_eq!(observed_code.as_str(), code_text);
                    assert_eq!(observed_product, product);
                    assert_eq!(recorded, expected_recorded);
                    assert_eq!(computed, expected_computed);
                }
                StoragePlanInputDiagnostic::HashComputeFailed { .. } => {
                    panic!("hash mismatch test should not compute product hashes")
                }
            }
        }
    }

    #[test]
    fn canonicalize_input_hashes_accepts_matching_hashes() {
        let hashes = input_hashes();
        validate_storage_plan_input_hashes(&hashes, &hashes).expect("matching hashes canonicalize");
    }

    #[test]
    fn store_020_to_024_mapping_bridges_input_precheck_and_invariants() {
        let pairs: Vec<_> = STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS
            .iter()
            .map(|spec| (spec.input_code.as_str(), spec.storage_code.as_str()))
            .collect();

        assert_eq!(
            pairs,
            vec![
                ("STORE-020", "STORE-020"),
                ("STORE-021", "STORE-021"),
                ("STORE-022", "STORE-022"),
                ("STORE-023", "STORE-023"),
                ("STORE-024", "STORE-024"),
            ]
        );
    }

    #[test]
    fn policy_hash_contract_is_stage6_local_until_policy_owner_exists() {
        let policy = crate::storage_plan_test_infra::synth::minimal_singleton_inputs().policy;
        let expected = DomainHash::new(
            "gbf-codegen",
            "ResolvedCompilePolicy",
            STORAGE_PLAN_SCHEMA_ID,
            "1.0.0",
        )
        .hash(&policy)
        .expect("policy hashes with Stage 6 domain");

        assert_eq!(
            resolved_compile_policy_hash(&policy).expect("policy hash computes"),
            expected
        );
    }

    fn input_hashes() -> StoragePlanInputHashes {
        StoragePlanInputHashes {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
        }
    }

    fn with_quant_graph_hash(
        mut hashes: StoragePlanInputHashes,
        value: Hash256,
    ) -> StoragePlanInputHashes {
        hashes.quant_graph_hash = value;
        hashes
    }

    fn with_infer_ir_hash(
        mut hashes: StoragePlanInputHashes,
        value: Hash256,
    ) -> StoragePlanInputHashes {
        hashes.infer_ir_hash = value;
        hashes
    }

    fn with_observation_plan_hash(
        mut hashes: StoragePlanInputHashes,
        value: Hash256,
    ) -> StoragePlanInputHashes {
        hashes.observation_plan_hash = value;
        hashes
    }

    fn with_range_plan_hash(
        mut hashes: StoragePlanInputHashes,
        value: Hash256,
    ) -> StoragePlanInputHashes {
        hashes.range_plan_hash = value;
        hashes
    }

    fn with_policy_hash(
        mut hashes: StoragePlanInputHashes,
        value: Hash256,
    ) -> StoragePlanInputHashes {
        hashes.policy_hash = value;
        hashes
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
