//! Shared F-B8 test infrastructure for synthetic inputs, fixtures, traces, and
//! the `gbf-storage-plan-debug` harness.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use gbf_abi::{TraceBudget as AbiTraceBudget, TraceDropPolicy as AbiTraceDropPolicy};
use gbf_foundation::{
    CompileProfileId, EvidenceRef, Hash256, LayerId, TargetProfileId, WorkloadId,
};
use gbf_policy::{
    CalibrationConfidenceRequirement, CompileKnobOverrides, CompileKnobPartialBounds,
    CompileKnobPartialValues, CompileKnobValues, CompileKnobs, CompileObjective,
    DEFAULT_COMPILE_PROFILE_ID, EffectiveConstraints, KnobLockSet, ObservabilityMode,
    ObservationKnob, ObservationProfileCaps, OverlayKnob, OverlayPromotion, PlacementKnob,
    PlacementProfile, PolicyProvenance, ProbeCollectionLevel, RangeCapsSpec, RangeKnob,
    ReductionPlanCeiling, RepairPolicy, RepairPolicyProfile, ResolvedCompilePolicy, RiskPolicy,
    RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob, RuntimeMode, ScheduleKnob,
    ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch, SramKnob,
    SramPageAggression, StorageKnob, StorageMaterialization, StoragePlanDiagnosticCode,
    StoragePlanDiagnosticProvenance, TraceBudget as PolicyTraceBudget,
    TraceDropPolicy as PolicyTraceDropPolicy, canonical_default_bounds_fixture,
};
use gbf_report::canonicalize;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::s1::quant_graph::{
    ClassifyHead, ClassifyHeadKind, DecodePlanId, DecodeSpec, DecodeSpecRecord, DeterminismClass,
    FfnActivationKind, FfnKindTag, FfnPlan, QuantFormat, QuantGraph, QuantGraphIdentity,
    ResidualCombinePolicy, ResidualPlan, SequenceSemanticsSpec, TensorId, quant_graph_self_hash,
};
use crate::s3::infer_ir::{
    GbInferIR, GbNode, InferIrIdentity, InferIrProvenance, InferOp, NodeId, QuantGraphEntityRef,
    SemanticAnchor, TokenIngressMode, TokenInput, TokenInputId, ValueDecl,
    ValueFormat as IrValueFormat, ValueId, ValueKind, ValueLayout, ValueProducerRef,
    infer_ir_self_hash,
};
use crate::s4::observation_plan::{
    AnchorAttachmentTable, ObservationPlan, ObservationPlanIdentity, ObservationProvenance,
    TraceBudgetProjection, observation_plan_self_hash,
};
use crate::s5::range_plan::{
    RangePlan, RangePlanIdentity, RangePlanProvenance, range_plan_self_hash,
};
use crate::storage_plan::cache::{StoragePlanCacheKey, StoragePlanCacheKeyInputs};
use crate::storage_plan::driver::{
    StoragePlanCoreDiagnosticDetail, StoragePlanCoreInput, StoragePlanCoreOutcome,
    StoragePlanCoreOutput, StoragePlanCoreResult, StoragePlanCoreValue, build_storage_plan_core,
};
use crate::storage_plan::emitter::emit_storage_plan_report;
use crate::storage_plan::invariants::{
    StoragePlanConsistencyContext, StoragePlanConsistencyView,
    validate_storage_plan_self_consistency,
};
use crate::storage_plan::lifetime::{LifetimeBound, LifetimeBoundSource, LifetimeBounds};
use crate::storage_plan::types::{
    AbstractLiveRange, AliasClassId, CommitAtomicityClass, CommitGroupDecl, CommitGroupId,
    CommitGroupReason, DurabilityClass, LifetimeClass, Materialization, NonEmptySortedSet,
    PersistKind, PersistPageDecl, PersistPageId, PersistSchemaPin, StorageBinding, StorageClass,
    StoragePlanInputIdentity, StoragePlanInputProduct, StoragePlanInputs, canonicalize_inputs,
    resolved_compile_policy_hash,
};
use crate::storage_plan::{
    AliasCandidateEdge, AliasIntent, PredicateEnv, PredicateValueFacts, QuantFormatId, ValueFormat,
    ValueRole,
};
use gbf_policy::ValidationCode;

pub mod synth {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct InputBuilder {
        fixture_id: String,
        expert_weights: usize,
        router_decision_value: bool,
        renorm_loop_scratch_tile_len: Option<u16>,
        observation_checkpoints: Vec<SemanticAnchorFixture>,
        transcript_capture_enabled: bool,
        promotion_level: RecomputePromotionLevel,
        determinism: DeterminismClass,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct SemanticAnchorFixture {
        pub id: Hash256,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    pub enum RecomputePromotionLevel {
        PreserveAll,
        RecomputePureValues,
        SpillColdValues,
    }

    impl From<RecomputePromotionLevel> for StorageMaterialization {
        fn from(value: RecomputePromotionLevel) -> Self {
            match value {
                RecomputePromotionLevel::PreserveAll => Self::PreserveAll,
                RecomputePromotionLevel::RecomputePureValues => Self::RecomputePureValues,
                RecomputePromotionLevel::SpillColdValues => Self::SpillColdValues,
            }
        }
    }

    impl Default for InputBuilder {
        fn default() -> Self {
            Self {
                fixture_id: "minimal_singleton".to_owned(),
                expert_weights: 0,
                router_decision_value: false,
                renorm_loop_scratch_tile_len: None,
                observation_checkpoints: Vec::new(),
                transcript_capture_enabled: false,
                promotion_level: RecomputePromotionLevel::PreserveAll,
                determinism: DeterminismClass::Deterministic,
            }
        }
    }

    impl InputBuilder {
        #[must_use]
        pub fn minimal() -> Self {
            Self::default()
        }

        #[must_use]
        pub fn with_expert_weights(n: usize) -> Self {
            Self::minimal().with_expert_weight_count(n)
        }

        #[must_use]
        pub fn with_expert_weight_count(mut self, n: usize) -> Self {
            self.expert_weights = n;
            self
        }

        #[must_use]
        pub fn with_router_decision_value(mut self) -> Self {
            self.router_decision_value = true;
            self
        }

        #[must_use]
        pub fn with_renorm_loop_scratch(mut self, tile_len: u16) -> Self {
            self.renorm_loop_scratch_tile_len = Some(tile_len);
            self
        }

        #[must_use]
        pub fn with_observation_checkpoint(mut self, at: SemanticAnchorFixture) -> Self {
            self.observation_checkpoints.push(at);
            self
        }

        #[must_use]
        pub fn with_transcript_capture(mut self, enabled: bool) -> Self {
            self.transcript_capture_enabled = enabled;
            self
        }

        #[must_use]
        pub fn with_promotion_level(mut self, level: RecomputePromotionLevel) -> Self {
            self.promotion_level = level;
            self
        }

        #[must_use]
        pub fn with_fixture_id(mut self, fixture_id: impl Into<String>) -> Self {
            self.fixture_id = fixture_id.into();
            self
        }

        #[must_use]
        pub fn with_determinism(mut self, determinism: DeterminismClass) -> Self {
            self.determinism = determinism;
            self
        }

        pub fn build(self) -> StoragePlanInputs {
            storage_plan_inputs(self)
        }

        pub fn build_core(self) -> StoragePlanCoreInput {
            storage_plan_core_input(self)
        }
    }

    #[must_use]
    pub fn tiny_tinystories_inputs() -> StoragePlanInputs {
        InputBuilder::minimal()
            .with_fixture_id("tiny_tinystories")
            .with_expert_weight_count(1)
            .with_transcript_capture(true)
            .build()
    }

    #[must_use]
    pub fn tiny_routed_ffn_inputs() -> StoragePlanInputs {
        InputBuilder::with_expert_weights(2)
            .with_fixture_id("tiny_routed_ffn")
            .with_router_decision_value()
            .with_renorm_loop_scratch(16)
            .with_determinism(DeterminismClass::BitExact)
            .build()
    }

    #[must_use]
    pub fn minimal_singleton_inputs() -> StoragePlanInputs {
        InputBuilder::minimal().build()
    }

    #[must_use]
    pub fn tiny_tinystories_core_input() -> StoragePlanCoreInput {
        InputBuilder::minimal()
            .with_fixture_id("tiny_tinystories")
            .with_expert_weight_count(1)
            .with_transcript_capture(true)
            .build_core()
    }

    #[must_use]
    pub fn tiny_routed_ffn_core_input() -> StoragePlanCoreInput {
        InputBuilder::with_expert_weights(2)
            .with_fixture_id("tiny_routed_ffn")
            .with_router_decision_value()
            .with_renorm_loop_scratch(16)
            .with_determinism(DeterminismClass::BitExact)
            .build_core()
    }

    #[must_use]
    pub fn minimal_singleton_core_input() -> StoragePlanCoreInput {
        InputBuilder::minimal().build_core()
    }

    fn storage_plan_inputs(builder: InputBuilder) -> StoragePlanInputs {
        let policy = resolved_policy(builder.promotion_level);
        let policy_hash = resolved_compile_policy_hash(&policy).expect("policy hashes");
        let quant_graph = quant_graph(builder.determinism);
        let quant_graph_hash = quant_graph_self_hash(&quant_graph).expect("quant graph hashes");
        let infer_ir = infer_ir(quant_graph_hash, builder.determinism);
        let infer_ir_hash = infer_ir_self_hash(&infer_ir).expect("infer ir hashes");
        let observation_plan =
            observation_plan(infer_ir_hash, quant_graph_hash, builder.determinism);
        let observation_plan_hash =
            observation_plan_self_hash(&observation_plan).expect("observation plan hashes");
        let range_plan = range_plan(infer_ir_hash, quant_graph_hash, builder.determinism);
        let range_plan_hash = range_plan_self_hash(&range_plan).expect("range plan hashes");
        let inputs = StoragePlanInputs {
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
        };
        canonicalize_inputs(&inputs).expect("synthetic storage inputs are canonical");
        inputs
    }

    fn storage_plan_core_input(builder: InputBuilder) -> StoragePlanCoreInput {
        let inputs = storage_plan_inputs(builder.clone());
        let identity = inputs.input_identity();
        let mut env = PredicateEnv::new()
            .with_recompute_promotion(builder.promotion_level.into())
            .with_wram_hot_per_value_eligibility_ceiling(32)
            .with_transcript_capture_enabled(builder.transcript_capture_enabled)
            .with_transcript_inline_ceiling(8);
        let mut values = vec![activation_value(1, 2, 3)];
        env = env.with_value(ValueId::new(1), sized_activation_facts(4));

        let mut next = 10_u32;
        for _ in 0..builder.expert_weights {
            let value = ValueId::new(next);
            values.push(StoragePlanCoreValue {
                value,
                materialization: Materialization::Materialize {
                    class: StorageClass::RomConst,
                    lifetime: LifetimeClass::Persistent,
                },
                live_range: live_range(next * 2, next * 2 + 1, LifetimeClass::Persistent),
                role: ValueRole::ExpertWeight,
                persist_kind: None,
                commit_group_reason: None,
            });
            env = env.with_value(value, expert_weight_facts(next));
            next += 1;
        }

        if builder.router_decision_value {
            let value = ValueId::new(next);
            values.push(StoragePlanCoreValue {
                value,
                materialization: Materialization::Materialize {
                    class: StorageClass::HramHot,
                    lifetime: LifetimeClass::Slice,
                },
                live_range: live_range(next * 2, next * 2 + 1, LifetimeClass::Slice),
                role: ValueRole::RouterDecision,
                persist_kind: None,
                commit_group_reason: None,
            });
            env = env
                .with_value(
                    value,
                    PredicateValueFacts::new(
                        ValueRole::RouterDecision,
                        ValueFormat::TokenIdDomain { vocab_size: 8 },
                    ),
                )
                .with_precomputed_hram_admitted_set(
                    crate::storage_plan::PrecomputedHramAdmittedSet {
                        admitted_values: BTreeSet::from([value]),
                        admission_order: vec![value],
                        cumulative_logical_size: 1,
                        allocatable_budget: 8,
                    },
                );
            next += 1;
        }

        let mut alias_edges = Vec::new();
        if builder.renorm_loop_scratch_tile_len.is_some() {
            let left = ValueId::new(next);
            let right = ValueId::new(next + 1);
            values.push(scratch_value(left, next * 2, next * 2 + 1));
            values.push(scratch_value(right, (next + 1) * 2, (next + 1) * 2 + 1));
            env = env
                .with_value(left, scratch_facts())
                .with_value(right, scratch_facts());
            alias_edges.push(AliasCandidateEdge {
                left,
                right,
                intent: AliasIntent::ScratchReuse,
            });
        }

        let topological_order = synthetic_core_topological_order(&inputs.infer_ir, &values);
        StoragePlanCoreInput {
            input_identity: identity,
            expected_input_hashes: inputs.recorded_hashes(),
            repair_policy: Default::default(),
            predicate_env: env,
            topological_order,
            values,
            alias_edges,
            alias_forced_recompute_values: BTreeSet::new(),
            fail_before_result: false,
        }
    }

    fn resolved_policy(promotion_level: RecomputePromotionLevel) -> ResolvedCompilePolicy {
        let bounds = canonical_default_bounds_fixture();
        let requested_runtime_modes = BTreeSet::from([RuntimeMode::Safe]);
        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID),
            objective: CompileObjective {
                service: None,
                max_cycles_per_token: Some(24_000),
                max_bank_switches_per_token: Some(8),
                max_sram_page_switches_per_token: Some(2),
                min_ui_headroom_pct: 10,
                max_rom_bytes: Some(2 * 1024 * 1024),
                risk: RiskPolicy {
                    cycle_quantile: 95,
                    switch_quantile: 99,
                    calibration_confidence_requirement:
                        CalibrationConfidenceRequirement::NoMinimumConfidence,
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            effective_constraints: EffectiveConstraints {
                target_caps: bounds.clone(),
                required_features: BTreeSet::new(),
                requested_runtime_modes: requested_runtime_modes.clone(),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Flexible,
            trace_budget: PolicyTraceBudget {
                max_events_per_slice: 8,
                max_bytes_per_frame: 512,
                drop_policy: PolicyTraceDropPolicy::HaltAndFault,
            },
            range_caps: RangeCapsSpec::default_v2(),
            observation_caps: ObservationProfileCaps::default_v2(),
            requested_runtime_modes,
            knobs: CompileKnobs {
                global: CompileKnobValues {
                    placement: PlacementKnob {
                        profile: PlacementProfile::Budgeted,
                    },
                    observation: ObservationKnob {
                        observability: ObservabilityMode::Flexible,
                        probe_level: ProbeCollectionLevel::Operational,
                    },
                    range: RangeKnob {
                        reduction_ceiling: ReductionPlanCeiling::Conservative,
                    },
                    storage: StorageKnob {
                        materialization: promotion_level.into(),
                    },
                    sram: SramKnob {
                        page_aggression: SramPageAggression::PackCold,
                    },
                    rom_window: RomWindowKnob {
                        kernel_residency_bias: RomKernelResidencyBias::PreferCommonBank,
                        kernel_duplication_bias: RomKernelDuplicationBias::Share,
                    },
                    overlay: OverlayKnob {
                        promotion: OverlayPromotion::TinyLuts,
                    },
                    schedule: ScheduleKnob {
                        tile_search: ScheduleTileSearch::Local,
                        slice_coarsening: ScheduleSliceCoarsening::Balanced,
                        resource_pressure: ScheduleResourcePressure::Balanced,
                    },
                },
                bounds,
                locks: KnobLockSet::default(),
                overrides: CompileKnobOverrides {
                    values: CompileKnobPartialValues::default(),
                    bounds: CompileKnobPartialBounds::default(),
                },
                provenance: Vec::new(),
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Default),
            provenance: PolicyProvenance {
                target_defaults: hash(0x01),
                profile_defaults: hash(0x02),
                compile_profile_spec_version: "2.0.0".to_owned(),
                hint_bundle_hash: None,
                compile_request_hash: hash(0x03),
                calibration_hash: None,
            },
        }
    }

    fn quant_graph(determinism: DeterminismClass) -> QuantGraph {
        QuantGraph {
            identity: QuantGraphIdentity {
                artifact_core_hash: hash(0x10),
                policy_resolution_self_hash: hash(0x11),
                artifact_validation_self_hash: hash(0x12),
                semantic_core_hash: hash(0x13),
                lowering_manifest_hash: hash(0x14),
                determinism,
                model_spec_summary: crate::s1::quant_graph::ModelSpecSummary {
                    n_layers: 1,
                    n_experts: BTreeMap::new(),
                    d_model: 8,
                    d_ff: 16,
                    vocab_size: 256,
                    ffn_kind: BTreeMap::from([(LayerId::new(0), FfnKindTag::Dense)]),
                },
            },
            tensors: Vec::new(),
            norm_plans: Vec::new(),
            layer_norms: BTreeMap::new(),
            routing_table: None,
            expert_sections: Vec::new(),
            ffn_plans: BTreeMap::from([(
                LayerId::new(0),
                FfnPlan {
                    layer: LayerId::new(0),
                    activation_kind: FfnActivationKind::Gelu,
                    intermediate_format: QuantFormat::Q8_8,
                },
            )]),
            decode_spec: DecodeSpecRecord {
                decode_plan_id: DecodePlanId::new(0),
                spec: DecodeSpec::Argmax,
                requires_rng: false,
            },
            sequence_semantics: SequenceSemanticsSpec::identity(),
            provenance: BTreeMap::new(),
            classify_head: ClassifyHead {
                kind: ClassifyHeadKind::Untied,
                weight: TensorId::new(0),
                bias: None,
                logit_format: QuantFormat::Q8_8,
            },
            residual_plan: ResidualPlan {
                activation_format: QuantFormat::Q8_8,
                combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
            },
        }
    }

    fn infer_ir(quant_graph_hash: Hash256, determinism: DeterminismClass) -> GbInferIR {
        let token_input_id = TokenInputId::new(0);
        let input = ValueId::new(0);
        let embedding = ValueId::new(1);
        let node = NodeId::new(0);
        let token_input = TokenInput::new(
            token_input_id,
            input,
            BTreeSet::from([TokenIngressMode::Prompt]),
        )
        .expect("token input fixture is valid");
        let values = vec![
            ValueDecl {
                value_id: input,
                kind: ValueKind::InputToken,
                format: IrValueFormat::TokenIdDomain { vocab_size: 256 },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: embedding,
                kind: ValueKind::EmbeddingOutput,
                format: IrValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                layout: ValueLayout::scalar(),
            },
        ];
        let nodes = vec![GbNode {
            node_id: node,
            op: InferOp::Embedding {
                token_input: token_input_id,
            },
            inputs: vec![input],
            effects_in: Vec::new(),
            outputs: vec![embedding],
            effects_out: Vec::new(),
            reduction_site: None,
        }];
        let provenance = InferIrProvenance {
            nodes: BTreeMap::from([(node, QuantGraphEntityRef::Embedding)]),
            values: BTreeMap::from([
                (
                    input,
                    ValueProducerRef::External {
                        token_input: token_input_id,
                    },
                ),
                (embedding, ValueProducerRef::Node { node }),
            ]),
            effects: BTreeMap::new(),
        };
        GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: quant_graph_hash,
                infer_ir_policy_projection_hash: hash(0x20),
                static_budget_self_hash: hash(0x21),
                requested_runtime_modes_hash: hash(0x22),
                determinism,
                topological_order_hash: hash(0x23),
            },
            vec![token_input],
            nodes,
            values,
            Vec::new(),
            provenance,
            BTreeMap::from([(node, SemanticAnchor::new(hash(0x24)))]),
        )
        .expect("infer_ir fixture is valid")
    }

    fn observation_plan(
        infer_ir_hash: Hash256,
        quant_graph_hash: Hash256,
        determinism: DeterminismClass,
    ) -> ObservationPlan {
        ObservationPlan {
            identity: ObservationPlanIdentity {
                infer_ir_self_hash: infer_ir_hash,
                quant_graph_self_hash: quant_graph_hash,
                semantic_checkpoint_schema_hash: hash(0x30),
                observation_policy_projection_hash: hash(0x31),
                determinism,
                observability_mode: ObservabilityMode::Flexible,
                trace_budget: AbiTraceBudget::new(8, 512, AbiTraceDropPolicy::HaltAndFault)
                    .expect("trace budget is valid"),
                workload_id: WorkloadId::from("f_b8.synthetic"),
                probe_registry_hash: hash(0x32),
                metric_registry_hash: hash(0x33),
                trace_event_layout_registry_hash: hash(0x34),
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

    fn range_plan(
        infer_ir_hash: Hash256,
        quant_graph_hash: Hash256,
        determinism: DeterminismClass,
    ) -> RangePlan {
        RangePlan {
            identity: RangePlanIdentity {
                infer_ir_self_hash: infer_ir_hash,
                quant_graph_self_hash: quant_graph_hash,
                static_budget_self_hash: hash(0x40),
                range_policy_projection_hash: hash(0x41),
                determinism,
            },
            entries: Vec::new(),
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        }
    }

    fn activation_value(value: u32, def: u32, last: u32) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value: ValueId::new(value),
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            live_range: live_range(def, last, LifetimeClass::Slice),
            role: ValueRole::Activation,
            persist_kind: None,
            commit_group_reason: None,
        }
    }

    fn scratch_value(value: ValueId, def: u32, last: u32) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value,
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            live_range: live_range(def, last, LifetimeClass::Slice),
            role: ValueRole::Scratch,
            persist_kind: None,
            commit_group_reason: None,
        }
    }

    pub(super) fn live_range(def: u32, last: u32, lifetime: LifetimeClass) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: NodeId::new(def),
            first_use_node: Some(NodeId::new(last)),
            last_use_node: Some(NodeId::new(last)),
            lifetime_class: lifetime,
            checkpoint_stable: false,
        }
    }

    fn synthetic_core_topological_order(
        infer_ir: &GbInferIR,
        values: &[StoragePlanCoreValue],
    ) -> Vec<NodeId> {
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        for node in &infer_ir.nodes {
            if seen.insert(node.node_id) {
                order.push(node.node_id);
            }
        }
        let mut synthetic_only = BTreeSet::new();
        for value in values {
            synthetic_only.insert(value.live_range.def_node);
            if let Some(first_use) = value.live_range.first_use_node {
                synthetic_only.insert(first_use);
            }
            if let Some(last_use) = value.live_range.last_use_node {
                synthetic_only.insert(last_use);
            }
        }
        order.extend(synthetic_only.into_iter().filter(|node| seen.insert(*node)));
        order
    }

    fn sized_activation_facts(size: u32) -> PredicateValueFacts {
        let mut facts = PredicateValueFacts::new(
            ValueRole::Activation,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        );
        facts.logical_size = Some(size);
        facts
    }

    fn scratch_facts() -> PredicateValueFacts {
        let mut facts =
            PredicateValueFacts::new(ValueRole::Scratch, ValueFormat::IntAccum { width_bits: 16 });
        facts.logical_size = Some(2);
        facts
    }

    fn expert_weight_facts(tensor: u32) -> PredicateValueFacts {
        let mut facts = PredicateValueFacts::new(
            ValueRole::ExpertWeight,
            ValueFormat::ConstTensorRef {
                tensor_id: TensorId::new(tensor),
            },
        );
        facts.lifetime_estimate = Some(LifetimeClass::Persistent);
        facts
    }
}

pub mod sc_violations {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct StoragePlanViolationFixture {
        pub fixture_id: String,
        pub expected_code: StoragePlanDiagnosticCode,
        pub rfc_section: String,
        pub provenance_schema: String,
        pub kind: StoragePlanViolationFixtureKind,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    pub enum StoragePlanViolationFixtureKind {
        ProductionBacked(Box<StoragePlanProductionFixture>),
        StubOnly { reason: String },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    pub enum StoragePlanProductionFixture {
        CoreDriver {
            core_input: StoragePlanCoreInput,
        },
        SelfConsistency {
            core_input: StoragePlanCoreInput,
            mutation: StoragePlanSelfConsistencyMutation,
        },
        SyntheticDiagnostic {
            core_input: StoragePlanCoreInput,
            code: StoragePlanDiagnosticCode,
        },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    pub enum StoragePlanSelfConsistencyMutation {
        BindingCoverageGap,
        BindingDoubleBind,
        RecomputeAliasNotIsolated,
        LifetimeTooShort,
        LifetimeTooLong,
        ForbiddenSpatialKey,
        PersistPageNotReferenced,
        CommitGroupEmpty,
        AliasClassMembershipMiss,
        AliasIntentMaterializationMismatch,
        InputHashMismatch { product: StoragePlanInputProduct },
    }

    impl StoragePlanViolationFixture {
        #[must_use]
        pub fn is_production_backed(&self) -> bool {
            matches!(
                &self.kind,
                StoragePlanViolationFixtureKind::ProductionBacked(fixture)
                    if matches!(
                        fixture.as_ref(),
                        StoragePlanProductionFixture::CoreDriver { .. }
                            | StoragePlanProductionFixture::SelfConsistency { .. }
                    )
            )
        }

        #[must_use]
        pub fn is_synthetic_diagnostic(&self) -> bool {
            matches!(
                &self.kind,
                StoragePlanViolationFixtureKind::ProductionBacked(fixture)
                    if matches!(fixture.as_ref(), StoragePlanProductionFixture::SyntheticDiagnostic { .. })
            )
        }

        #[must_use]
        pub fn is_stub_only(&self) -> bool {
            matches!(self.kind, StoragePlanViolationFixtureKind::StubOnly { .. })
        }

        pub fn run(&self) -> Result<StoragePlanCoreOutput, StubFixtureRunError> {
            match &self.kind {
                StoragePlanViolationFixtureKind::ProductionBacked(fixture) => {
                    match fixture.as_ref() {
                        StoragePlanProductionFixture::CoreDriver { core_input } => {
                            Ok(build_storage_plan_core(core_input))
                        }
                        StoragePlanProductionFixture::SelfConsistency {
                            core_input,
                            mutation,
                        } => Ok(self.run_self_consistency_mutation(core_input, *mutation)),
                        StoragePlanProductionFixture::SyntheticDiagnostic { core_input, code } => {
                            Ok(StoragePlanCoreOutput {
                                input_identity: core_input.input_identity.clone(),
                                outcome: StoragePlanCoreOutcome::Failed,
                                result: None,
                                summary: None,
                                diagnostics: vec![*code],
                                diagnostic_details: vec![diagnostic_detail_for_code(*code)],
                            })
                        }
                    }
                }
                StoragePlanViolationFixtureKind::StubOnly { reason } => Err(StubFixtureRunError {
                    fixture_id: self.fixture_id.clone(),
                    expected_code: self.expected_code,
                    reason: reason.clone(),
                }),
            }
        }

        fn run_self_consistency_mutation(
            &self,
            core_input: &StoragePlanCoreInput,
            mutation: StoragePlanSelfConsistencyMutation,
        ) -> StoragePlanCoreOutput {
            let output = build_storage_plan_core(core_input);
            let Some(mut result) = output.result else {
                return output;
            };
            let mut context = consistency_context_from_core_result(core_input, &result);
            let mut json_value = None;
            let mut binding_override = None;
            apply_self_consistency_mutation(
                mutation,
                &mut result,
                &mut context,
                &mut json_value,
                &mut binding_override,
            );
            let bindings: Vec<_> =
                binding_override.unwrap_or_else(|| result.bindings.values().cloned().collect());
            let diagnostics = validate_storage_plan_self_consistency(
                &context,
                StoragePlanConsistencyView {
                    input_identity: &core_input.input_identity,
                    bindings: &bindings,
                    alias_classes: &result.alias_classes,
                    persist_pages: &result.persist_pages,
                    commit_groups: &result.commit_groups,
                    json_value: json_value.as_ref(),
                },
            );
            failed_output_from_validation_diagnostics(
                core_input.input_identity.clone(),
                &diagnostics,
            )
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StubFixtureRunError {
        pub fixture_id: String,
        pub expected_code: StoragePlanDiagnosticCode,
        pub reason: String,
    }

    impl std::fmt::Display for StubFixtureRunError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{} is declaration-only for {}: {}",
                self.fixture_id,
                self.expected_code.as_str(),
                self.reason
            )
        }
    }

    impl std::error::Error for StubFixtureRunError {}

    fn apply_self_consistency_mutation(
        mutation: StoragePlanSelfConsistencyMutation,
        result: &mut StoragePlanCoreResult,
        context: &mut StoragePlanConsistencyContext,
        json_value: &mut Option<Value>,
        binding_override: &mut Option<Vec<StorageBinding>>,
    ) {
        match mutation {
            StoragePlanSelfConsistencyMutation::BindingCoverageGap => {
                let removed = result
                    .bindings
                    .keys()
                    .next()
                    .copied()
                    .expect("fixture has a binding");
                result.bindings.remove(&removed);
                result.alias_classes.clear();
            }
            StoragePlanSelfConsistencyMutation::BindingDoubleBind => {
                let binding = result
                    .bindings
                    .values()
                    .next()
                    .cloned()
                    .expect("fixture has a binding");
                let mut bindings = result.bindings.values().cloned().collect::<Vec<_>>();
                bindings.push(binding);
                *binding_override = Some(bindings);
            }
            StoragePlanSelfConsistencyMutation::RecomputeAliasNotIsolated => {
                let binding = result
                    .bindings
                    .get_mut(&ValueId::new(1))
                    .expect("fixture has value 1");
                binding.materialization = Materialization::Recompute;
            }
            StoragePlanSelfConsistencyMutation::LifetimeTooShort => {
                let value = ValueId::new(1);
                result
                    .bindings
                    .get_mut(&value)
                    .expect("fixture has value 1")
                    .materialization = Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                };
                context.lifetime_bounds.insert(
                    value,
                    bounds(LifetimeClass::Token, LifetimeClass::Persistent),
                );
            }
            StoragePlanSelfConsistencyMutation::LifetimeTooLong => {
                let value = ValueId::new(1);
                result
                    .bindings
                    .get_mut(&value)
                    .expect("fixture has value 1")
                    .materialization = Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Persistent,
                };
                context
                    .lifetime_bounds
                    .insert(value, bounds(LifetimeClass::Slice, LifetimeClass::Token));
            }
            StoragePlanSelfConsistencyMutation::ForbiddenSpatialKey => {
                *json_value = Some(serde_json::json!({ "byte_offset": 0 }));
            }
            StoragePlanSelfConsistencyMutation::PersistPageNotReferenced => {
                result.persist_pages.insert(
                    PersistPageId(99),
                    persist_page(PersistPageId(99), PersistKind::Trace),
                );
            }
            StoragePlanSelfConsistencyMutation::CommitGroupEmpty => {
                result.commit_groups.insert(
                    CommitGroupId(99),
                    CommitGroupDecl {
                        id: CommitGroupId(99),
                        members: NonEmptySortedSet::new([PersistPageId(99)])
                            .expect("fixture commit group is non-empty"),
                        kind_set: BTreeSet::from([PersistKind::Trace]),
                        atomicity: CommitAtomicityClass::AllOrNothing,
                    },
                );
            }
            StoragePlanSelfConsistencyMutation::AliasClassMembershipMiss => {
                result
                    .bindings
                    .get_mut(&ValueId::new(1))
                    .expect("fixture has value 1")
                    .alias_class = AliasClassId(99);
            }
            StoragePlanSelfConsistencyMutation::AliasIntentMaterializationMismatch => {
                let binding = result
                    .bindings
                    .get_mut(&ValueId::new(1))
                    .expect("fixture has value 1");
                binding.materialization = Materialization::Persist {
                    page: PersistPageId(1),
                    commit_group: CommitGroupId(1),
                };
                result.persist_pages.insert(
                    PersistPageId(1),
                    persist_page(PersistPageId(1), PersistKind::Trace),
                );
                result.commit_groups.insert(
                    CommitGroupId(1),
                    CommitGroupDecl {
                        id: CommitGroupId(1),
                        members: NonEmptySortedSet::new([PersistPageId(1)])
                            .expect("fixture commit group is non-empty"),
                        kind_set: BTreeSet::from([PersistKind::Trace]),
                        atomicity: CommitAtomicityClass::AllOrNothing,
                    },
                );
                context.lifetime_bounds.insert(
                    ValueId::new(1),
                    bounds(LifetimeClass::Persistent, LifetimeClass::Persistent),
                );
            }
            StoragePlanSelfConsistencyMutation::InputHashMismatch { product } => match product {
                StoragePlanInputProduct::QuantGraph => {
                    context.expected_input_hashes.quant_graph_hash = hash(0xfe);
                }
                StoragePlanInputProduct::InferIr => {
                    context.expected_input_hashes.infer_ir_hash = hash(0xfe);
                }
                StoragePlanInputProduct::ObservationPlan => {
                    context.expected_input_hashes.observation_plan_hash = hash(0xfe);
                }
                StoragePlanInputProduct::RangePlan => {
                    context.expected_input_hashes.range_plan_hash = hash(0xfe);
                }
                StoragePlanInputProduct::Policy => {
                    context.expected_input_hashes.policy_hash = hash(0xfe);
                }
            },
        }
    }

    /// SC1 / STORE-002: removes a produced value binding. Provenance schema:
    /// ValueProducer. RFC section: F-B8 self-consistency SC1.
    #[must_use]
    pub fn sc1_binding_coverage_gap() -> StoragePlanViolationFixture {
        self_consistency_fixture(
            "sc1_binding_coverage_gap",
            StoragePlanDiagnosticCode::StorageBindingCoverageGap,
            "SC1",
            synth::minimal_singleton_core_input(),
            StoragePlanSelfConsistencyMutation::BindingCoverageGap,
        )
    }

    /// SC5 / STORE-016: recompute binding is not isolated. Provenance schema:
    /// RecomputeAlias. RFC section: F-B8 self-consistency SC5.
    #[must_use]
    pub fn sc5_recompute_alias_violation() -> StoragePlanViolationFixture {
        self_consistency_fixture(
            "sc5_recompute_alias_violation",
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated,
            "SC5",
            alias_pair_core_input(),
            StoragePlanSelfConsistencyMutation::RecomputeAliasNotIsolated,
        )
    }

    /// SC10 / STORE-017 lower bound: selected lifetime is too short. Provenance
    /// schema: LifetimeAdmissibility. RFC section: F-B8 self-consistency SC10.
    #[must_use]
    pub fn sc10_lifetime_lower_bound() -> StoragePlanViolationFixture {
        self_consistency_fixture(
            "sc10_lifetime_lower_bound",
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            "SC10",
            synth::minimal_singleton_core_input(),
            StoragePlanSelfConsistencyMutation::LifetimeTooShort,
        )
    }

    /// SC10 / STORE-017 upper bound: selected lifetime exceeds admitted width.
    /// Provenance schema: LifetimeAdmissibility. RFC section: F-B8 SC10.
    #[must_use]
    pub fn sc10_lifetime_upper_bound() -> StoragePlanViolationFixture {
        self_consistency_fixture(
            "sc10_lifetime_upper_bound",
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            "SC10",
            synth::minimal_singleton_core_input(),
            StoragePlanSelfConsistencyMutation::LifetimeTooLong,
        )
    }

    /// SC11 / STORE-018: closed spatial surface key leak. Provenance schema:
    /// JsonPath. RFC section: F-B8 self-consistency SC11.
    #[must_use]
    pub fn sc11_forbidden_key_leak() -> StoragePlanViolationFixture {
        self_consistency_fixture(
            "sc11_forbidden_key_leak",
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak,
            "SC11",
            synth::minimal_singleton_core_input(),
            StoragePlanSelfConsistencyMutation::ForbiddenSpatialKey,
        )
    }

    /// SC12: envelope self-hash recursion regression. This maps to STORE-030
    /// when a recursive report body shape is emitted. Provenance schema:
    /// JsonPath. RFC section: F-B8 self-consistency SC12.
    #[must_use]
    pub fn sc12_envelope_recursion() -> StoragePlanViolationFixture {
        production_core_fixture(
            "sc12_envelope_recursion",
            StoragePlanDiagnosticCode::StorageReservedShapeEmitted,
            "SC12",
            reserved_shape_core_input(),
        )
    }

    #[must_use]
    pub fn store_031_mixed_intent_component() -> StoragePlanViolationFixture {
        production_core_fixture(
            "store_031_mixed_intent_component",
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent,
            "RC-31",
            mixed_intent_core_input(),
        )
    }

    #[must_use]
    pub fn store_032_pingpong_three_members() -> StoragePlanViolationFixture {
        production_core_fixture(
            "store_032_pingpong_three_members",
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
            "RC-32",
            pingpong_three_member_core_input(),
        )
    }

    #[must_use]
    pub fn store_033_forced_recompute_router() -> StoragePlanViolationFixture {
        production_core_fixture(
            "store_033_forced_recompute_router",
            StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed,
            "RC-33",
            forced_recompute_router_core_input(),
        )
    }

    #[must_use]
    pub fn store_034_policy_underflow() -> StoragePlanViolationFixture {
        synthetic_diagnostic_fixture(
            "store_034_policy_underflow",
            StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow,
            "RC-34",
        )
    }

    #[must_use]
    pub fn store_035_alias_fingerprint_collision() -> StoragePlanViolationFixture {
        synthetic_diagnostic_fixture(
            "store_035_alias_fingerprint_collision",
            StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision,
            "RC-35",
        )
    }

    #[must_use]
    pub fn all_store_violation_factories() -> Vec<StoragePlanViolationFixture> {
        StoragePlanDiagnosticCode::ALL
            .iter()
            .copied()
            .map(factory_for_code)
            .collect()
    }

    #[must_use]
    pub fn production_backed_violation_factories() -> Vec<StoragePlanViolationFixture> {
        all_store_violation_factories()
            .into_iter()
            .filter(StoragePlanViolationFixture::is_production_backed)
            .collect()
    }

    #[must_use]
    pub fn synthetic_diagnostic_violation_factories() -> Vec<StoragePlanViolationFixture> {
        all_store_violation_factories()
            .into_iter()
            .filter(StoragePlanViolationFixture::is_synthetic_diagnostic)
            .collect()
    }

    #[must_use]
    pub fn stub_only_violation_factories() -> Vec<StoragePlanViolationFixture> {
        all_store_violation_factories()
            .into_iter()
            .filter(StoragePlanViolationFixture::is_stub_only)
            .collect()
    }

    fn factory_for_code(code: StoragePlanDiagnosticCode) -> StoragePlanViolationFixture {
        match code {
            StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => production_core_fixture(
                "store_001_no_admitting_decision_rule",
                code,
                "RC-001",
                fail_before_result_core_input(),
            ),
            StoragePlanDiagnosticCode::StorageBindingCoverageGap => sc1_binding_coverage_gap(),
            StoragePlanDiagnosticCode::StorageBindingDoubleBind => self_consistency_fixture(
                "store_003_binding_double_bind",
                code,
                "SC1",
                synth::minimal_singleton_core_input(),
                StoragePlanSelfConsistencyMutation::BindingDoubleBind,
            ),
            StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
                production_core_fixture(
                    "store_006_recompute_forbidden_for_observed_value",
                    code,
                    "DR-1b",
                    forced_recompute_observed_core_input(),
                )
            }
            StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
                production_core_fixture(
                    "store_008_persist_binding_kind_mismatch",
                    code,
                    "RC-008",
                    persist_binding_kind_mismatch_core_input(),
                )
            }
            StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => self_consistency_fixture(
                "store_009_persist_page_not_referenced",
                code,
                "SC7",
                synth::minimal_singleton_core_input(),
                StoragePlanSelfConsistencyMutation::PersistPageNotReferenced,
            ),
            StoragePlanDiagnosticCode::StorageCommitGroupEmpty => self_consistency_fixture(
                "store_010_commit_group_empty",
                code,
                "SC8",
                synth::minimal_singleton_core_input(),
                StoragePlanSelfConsistencyMutation::CommitGroupEmpty,
            ),
            StoragePlanDiagnosticCode::StorageCommitGroupKindMix => production_core_fixture(
                "store_011_commit_group_kind_mix",
                code,
                "RC-011",
                commit_group_kind_mix_core_input(),
            ),
            StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
                production_core_fixture(
                    "store_007_persist_sequence_state_unsupported_v1",
                    code,
                    "RC-007",
                    sequence_state_slot_core_input(),
                )
            }
            StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
                self_consistency_fixture(
                    "store_013_alias_intent_materialization_mismatch",
                    code,
                    "SC6",
                    alias_pair_core_input(),
                    StoragePlanSelfConsistencyMutation::AliasIntentMaterializationMismatch,
                )
            }
            StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
                production_core_fixture(
                    "store_014_alias_class_overlap_without_intent",
                    code,
                    "RC-014",
                    alias_overlap_core_input(),
                )
            }
            StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
                self_consistency_fixture(
                    "store_015_alias_class_membership_functional_violation",
                    code,
                    "SC4",
                    synth::minimal_singleton_core_input(),
                    StoragePlanSelfConsistencyMutation::AliasClassMembershipMiss,
                )
            }
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
                sc5_recompute_alias_violation()
            }
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
                sc10_lifetime_lower_bound()
            }
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak => sc11_forbidden_key_leak(),
            StoragePlanDiagnosticCode::StorageRangePlanHashMismatch => {
                input_hash_fixture(code, StoragePlanInputProduct::RangePlan)
            }
            StoragePlanDiagnosticCode::StorageInferIrHashMismatch => {
                input_hash_fixture(code, StoragePlanInputProduct::InferIr)
            }
            StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch => {
                input_hash_fixture(code, StoragePlanInputProduct::ObservationPlan)
            }
            StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch => {
                input_hash_fixture(code, StoragePlanInputProduct::QuantGraph)
            }
            StoragePlanDiagnosticCode::StoragePolicyHashMismatch => {
                input_hash_fixture(code, StoragePlanInputProduct::Policy)
            }
            StoragePlanDiagnosticCode::StorageReservedShapeEmitted => sc12_envelope_recursion(),
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
                store_031_mixed_intent_component()
            }
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
                store_032_pingpong_three_members()
            }
            StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
                store_033_forced_recompute_router()
            }
            StoragePlanDiagnosticCode::StorageRepairProposalIllegal => production_core_fixture(
                "store_027_repair_proposal_illegal",
                code,
                "RC-027",
                repair_bound_violation_core_input(),
            ),
            StoragePlanDiagnosticCode::StorageRomConstWriteViolation
            | StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation
            | StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix
            | StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules
            | StoragePlanDiagnosticCode::StorageIterationInputInvalid
            | StoragePlanDiagnosticCode::StorageOverlayLensViolation
            | StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown
            | StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch
            | StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow
            | StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
                synthetic_diagnostic_fixture(
                    format!("store_{:03}_{}", code.number(), code.name()),
                    code,
                    format!("RC-{:03}", code.number()),
                )
            }
        }
    }

    fn production_core_fixture(
        fixture_id: impl Into<String>,
        expected_code: StoragePlanDiagnosticCode,
        rfc_section: impl Into<String>,
        core_input: StoragePlanCoreInput,
    ) -> StoragePlanViolationFixture {
        StoragePlanViolationFixture {
            fixture_id: fixture_id.into(),
            expected_code,
            rfc_section: rfc_section.into(),
            provenance_schema: crate::storage_plan::storage_plan_provenance_schema(expected_code)
                .to_owned(),
            kind: StoragePlanViolationFixtureKind::ProductionBacked(Box::new(
                StoragePlanProductionFixture::CoreDriver { core_input },
            )),
        }
    }

    fn self_consistency_fixture(
        fixture_id: impl Into<String>,
        expected_code: StoragePlanDiagnosticCode,
        rfc_section: impl Into<String>,
        core_input: StoragePlanCoreInput,
        mutation: StoragePlanSelfConsistencyMutation,
    ) -> StoragePlanViolationFixture {
        StoragePlanViolationFixture {
            fixture_id: fixture_id.into(),
            expected_code,
            rfc_section: rfc_section.into(),
            provenance_schema: crate::storage_plan::storage_plan_provenance_schema(expected_code)
                .to_owned(),
            kind: StoragePlanViolationFixtureKind::ProductionBacked(Box::new(
                StoragePlanProductionFixture::SelfConsistency {
                    core_input,
                    mutation,
                },
            )),
        }
    }

    fn synthetic_diagnostic_fixture(
        fixture_id: impl Into<String>,
        expected_code: StoragePlanDiagnosticCode,
        rfc_section: impl Into<String>,
    ) -> StoragePlanViolationFixture {
        StoragePlanViolationFixture {
            fixture_id: fixture_id.into(),
            expected_code,
            rfc_section: rfc_section.into(),
            provenance_schema: crate::storage_plan::storage_plan_provenance_schema(expected_code)
                .to_owned(),
            kind: StoragePlanViolationFixtureKind::ProductionBacked(Box::new(
                StoragePlanProductionFixture::SyntheticDiagnostic {
                    core_input: synth::minimal_singleton_core_input(),
                    code: expected_code,
                },
            )),
        }
    }

    fn diagnostic_detail_for_code(
        code: StoragePlanDiagnosticCode,
    ) -> StoragePlanCoreDiagnosticDetail {
        StoragePlanCoreDiagnosticDetail {
            code,
            provenance: synthetic_provenance_for_code(code),
            evidence: vec![EvidenceRef {
                kind: "StoragePlanViolationFixture".to_owned(),
                reference: format!("{}.typed_provenance", code.as_str()),
                hash: Some(Hash256::ZERO),
            }],
        }
    }

    fn synthetic_provenance_for_code(
        code: StoragePlanDiagnosticCode,
    ) -> StoragePlanDiagnosticProvenance {
        match code {
            StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => {
                StoragePlanDiagnosticProvenance::ValueClassification {
                    value_id: 1,
                    producer_node: Some(1),
                    value_role: Some("Activation".to_owned()),
                    value_format: Some("QuantInt".to_owned()),
                }
            }
            StoragePlanDiagnosticCode::StorageBindingCoverageGap => {
                StoragePlanDiagnosticProvenance::ValueProducer {
                    value_id: 1,
                    producer_node: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageBindingDoubleBind => {
                StoragePlanDiagnosticProvenance::BindingSet {
                    value_id: 1,
                    binding_count: 2,
                }
            }
            StoragePlanDiagnosticCode::StorageRomConstWriteViolation => {
                StoragePlanDiagnosticProvenance::ProducerOp {
                    value_id: 1,
                    producer_node: 1,
                    op_tag: "MutableWrite".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => {
                StoragePlanDiagnosticProvenance::BudgetSet {
                    values: vec![1],
                    observed_bytes: 17,
                    budget_bytes: 16,
                }
            }
            StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
                StoragePlanDiagnosticProvenance::ObservationCheckpoint {
                    value_id: 1,
                    semantic_anchor: "checkpoint:1".to_owned(),
                    checkpoint_id: 1,
                }
            }
            StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
                StoragePlanDiagnosticProvenance::SequenceState {
                    value_id: 90,
                    state_slot_id: 0,
                    layer_id: 0,
                }
            }
            StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
                StoragePlanDiagnosticProvenance::PersistBinding {
                    value_id: 1,
                    persist_page_id: 1,
                    commit_group_id: 1,
                    persist_kind: "SequenceState".to_owned(),
                    expected: "matching persist binding kind".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => {
                StoragePlanDiagnosticProvenance::PersistPage {
                    invariant: "SC7".to_owned(),
                    persist_page_id: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageCommitGroupEmpty => {
                StoragePlanDiagnosticProvenance::CommitGroup {
                    invariant: "SC8".to_owned(),
                    commit_group_id: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageCommitGroupKindMix => {
                StoragePlanDiagnosticProvenance::CommitGroupKind {
                    commit_group_id: 1,
                    kinds: vec!["SequenceState".to_owned(), "Transcript".to_owned()],
                    allowed_table: "allowed_cross_kind_table_v1".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => {
                StoragePlanDiagnosticProvenance::CommitGroupDurability {
                    commit_group_id: 1,
                    durabilities: vec!["Critical".to_owned(), "Ephemeral".to_owned()],
                }
            }
            StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
                StoragePlanDiagnosticProvenance::AliasMaterialization {
                    alias_class_id: 1,
                    members: vec![1, 2],
                    intent: "ScratchReuse".to_owned(),
                    materializations: vec!["Recompute".to_owned()],
                }
            }
            StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
                StoragePlanDiagnosticProvenance::AliasOverlap {
                    alias_class_id: 1,
                    members: vec![1, 2],
                }
            }
            StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
                StoragePlanDiagnosticProvenance::AliasMembership {
                    invariant: "SC4".to_owned(),
                    value_id: 1,
                    alias_class_id: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
                StoragePlanDiagnosticProvenance::RecomputeAlias {
                    value_id: 1,
                    alias_class_id: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
                StoragePlanDiagnosticProvenance::LifetimeAdmissibility {
                    value_id: 1,
                    computed_lifetime: "Slice".to_owned(),
                    min_lifetime: "ResumeWindow".to_owned(),
                    max_lifetime: "Persistent".to_owned(),
                    source: "SC10".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak
            | StoragePlanDiagnosticCode::StorageReservedShapeEmitted => {
                StoragePlanDiagnosticProvenance::JsonPath {
                    json_path: "$.body.result.bindings[0].byte_offset".to_owned(),
                    field_or_tag: "byte_offset".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => {
                StoragePlanDiagnosticProvenance::RuleInstability {
                    rule_id: 1,
                    evidence: "decision_rule_set_hash drift".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageRangePlanHashMismatch
            | StoragePlanDiagnosticCode::StorageInferIrHashMismatch
            | StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch
            | StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch
            | StoragePlanDiagnosticCode::StoragePolicyHashMismatch => {
                StoragePlanDiagnosticProvenance::HashMismatch {
                    product: code.name().to_owned(),
                    recorded: Hash256::from_bytes([0x11; 32]),
                    computed: Hash256::from_bytes([0x22; 32]),
                }
            }
            StoragePlanDiagnosticCode::StorageIterationInputInvalid => {
                StoragePlanDiagnosticProvenance::Iteration {
                    iteration: 2,
                    ceiling: 1,
                }
            }
            StoragePlanDiagnosticCode::StorageOverlayLensViolation => {
                StoragePlanDiagnosticProvenance::OverlayLens {
                    value_id: 1,
                    materialization: "RomConst".to_owned(),
                    forced_override: false,
                }
            }
            StoragePlanDiagnosticCode::StorageRepairProposalIllegal => {
                StoragePlanDiagnosticProvenance::RepairProposal {
                    proposal_id: "repair-1".to_owned(),
                    delta: "PromoteRecomputeLevel".to_owned(),
                    locks_bounds: "within-bounds".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => {
                StoragePlanDiagnosticProvenance::EffectClass {
                    effect_id: 1,
                    effect_class: "Unknown".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => {
                StoragePlanDiagnosticProvenance::RoutingMismatch {
                    layer_id: 0,
                    expected_entry: "expert=0".to_owned(),
                }
            }
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
                StoragePlanDiagnosticProvenance::AliasMixedIntent {
                    members: vec![1, 2, 3],
                    edge_count: 2,
                    intents: vec!["ScratchReuse".to_owned(), "PingPong".to_owned()],
                }
            }
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
                StoragePlanDiagnosticProvenance::AliasCardinality {
                    alias_class_id: 1,
                    intent: "PingPong".to_owned(),
                    members: vec![1, 2, 3],
                }
            }
            StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
                StoragePlanDiagnosticProvenance::ForcedRecompute {
                    value_id: 1,
                    failed_predicates: vec!["ValueRoleOf".to_owned()],
                }
            }
            StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => {
                StoragePlanDiagnosticProvenance::PolicyBudget {
                    storage_class: "WramHot".to_owned(),
                    soft_bytes: 1,
                    reserved_bytes: 2,
                }
            }
            StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
                StoragePlanDiagnosticProvenance::FingerprintCollision {
                    first_payload_hash: Hash256::from_bytes([0xaa; 32]),
                    second_payload_hash: Hash256::from_bytes([0xbb; 32]),
                }
            }
        }
    }

    fn input_hash_fixture(
        code: StoragePlanDiagnosticCode,
        product: StoragePlanInputProduct,
    ) -> StoragePlanViolationFixture {
        self_consistency_fixture(
            format!("store_{:03}_{}", code.number(), code.name()),
            code,
            format!("RC-{:03}", code.number()),
            synth::minimal_singleton_core_input(),
            StoragePlanSelfConsistencyMutation::InputHashMismatch { product },
        )
    }

    fn fail_before_result_core_input() -> StoragePlanCoreInput {
        let mut input = synth::minimal_singleton_core_input();
        input.fail_before_result = true;
        input
    }

    fn sequence_state_slot_core_input() -> StoragePlanCoreInput {
        let mut input = synth::minimal_singleton_core_input();
        let sequence_state = ValueId::new(90);
        input.values.push(StoragePlanCoreValue {
            value: sequence_state,
            materialization: Materialization::Persist {
                page: PersistPageId(90),
                commit_group: CommitGroupId(90),
            },
            live_range: AbstractLiveRange {
                def_node: NodeId::new(190),
                first_use_node: Some(NodeId::new(190)),
                last_use_node: Some(NodeId::new(191)),
                lifetime_class: LifetimeClass::Persistent,
                checkpoint_stable: false,
            },
            role: ValueRole::SequenceStateSlot,
            persist_kind: None,
            commit_group_reason: None,
        });
        input
            .topological_order
            .extend([NodeId::new(190), NodeId::new(191)]);
        let env = std::mem::take(&mut input.predicate_env);
        input.predicate_env = env.with_value(
            sequence_state,
            PredicateValueFacts::new(
                ValueRole::SequenceStateSlot,
                ValueFormat::TokenIdDomain { vocab_size: 1 },
            )
            .with_sequence_state_slot(0, 0),
        );
        input
    }

    fn alias_pair_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                value(1, ValueRole::Scratch, materialize_hot()),
                value(2, ValueRole::Scratch, materialize_hot()),
            ],
            vec![edge(1, 2, AliasIntent::ScratchReuse)],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn alias_overlap_core_input() -> StoragePlanCoreInput {
        let mut left = value(1, ValueRole::Scratch, materialize_hot());
        left.live_range = synth::live_range(2, 5, LifetimeClass::Slice);
        let mut right = value(2, ValueRole::Scratch, materialize_hot());
        right.live_range = synth::live_range(3, 4, LifetimeClass::Slice);
        core_input(
            vec![left, right],
            vec![edge(1, 2, AliasIntent::ScratchReuse)],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn mixed_intent_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                value(1, ValueRole::Scratch, materialize_hot()),
                value(2, ValueRole::Scratch, materialize_hot()),
                value(3, ValueRole::Scratch, materialize_hot()),
            ],
            vec![
                edge(1, 2, AliasIntent::ScratchReuse),
                edge(2, 3, AliasIntent::PingPong),
            ],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn pingpong_three_member_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                value(1, ValueRole::Activation, materialize_hot()),
                value(2, ValueRole::Activation, materialize_hot()),
                value(3, ValueRole::Activation, materialize_hot()),
            ],
            vec![
                edge(1, 2, AliasIntent::PingPong),
                edge(2, 3, AliasIntent::PingPong),
            ],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn forced_recompute_router_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                value(1, ValueRole::Activation, materialize_hot()),
                value(2, ValueRole::RouterDecision, materialize_hot()),
            ],
            vec![],
            BTreeSet::from([ValueId::new(2)]),
            |env| env,
        )
    }

    fn forced_recompute_observed_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                value(1, ValueRole::Activation, materialize_hot()),
                value(2, ValueRole::Activation, materialize_hot()),
            ],
            vec![],
            BTreeSet::from([ValueId::new(2)]),
            |env| env.with_observed_checkpoint_backing_value(ValueId::new(2)),
        )
    }

    fn repair_bound_violation_core_input() -> StoragePlanCoreInput {
        let mut input = core_input(
            vec![value(1, ValueRole::Activation, materialize_hot())],
            vec![],
            BTreeSet::new(),
            |env| {
                env.with_recompute_cycle_ceiling(8)
                    .with_recompute_cost_estimate(ValueId::new(1), 5)
            },
        );
        input.repair_policy = crate::storage_plan::StoragePlanRepairPolicy {
            soft_pressure_threshold_bytes: Some(1),
            recompute_promotion: StorageMaterialization::PreserveAll,
            max_recompute_promotion: StorageMaterialization::PreserveAll,
            storage_recompute_promotion_locked: false,
        };
        input
    }

    fn persist_binding_kind_mismatch_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![persist_value(
                1,
                PersistPageId(1),
                CommitGroupId(1),
                PersistKind::Trace,
                CommitGroupReason::SequenceStateWithTranscript,
            )],
            vec![],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn commit_group_kind_mix_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                persist_value(
                    1,
                    PersistPageId(1),
                    CommitGroupId(1),
                    PersistKind::Trace,
                    CommitGroupReason::Independent,
                ),
                persist_value(
                    2,
                    PersistPageId(2),
                    CommitGroupId(1),
                    PersistKind::Harness,
                    CommitGroupReason::Independent,
                ),
            ],
            vec![],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn reserved_shape_core_input() -> StoragePlanCoreInput {
        core_input(
            vec![
                persist_value(
                    1,
                    PersistPageId(1),
                    CommitGroupId(1),
                    PersistKind::SequenceState,
                    CommitGroupReason::ContinuationWithSequenceState,
                ),
                persist_value(
                    2,
                    PersistPageId(2),
                    CommitGroupId(1),
                    PersistKind::Continuation,
                    CommitGroupReason::ContinuationWithSequenceState,
                ),
            ],
            vec![],
            BTreeSet::new(),
            |env| env,
        )
    }

    fn core_input(
        values: Vec<StoragePlanCoreValue>,
        alias_edges: Vec<AliasCandidateEdge>,
        alias_forced_recompute_values: BTreeSet<ValueId>,
        adjust_env: impl FnOnce(PredicateEnv) -> PredicateEnv,
    ) -> StoragePlanCoreInput {
        let mut env = PredicateEnv::new().with_wram_hot_per_value_eligibility_ceiling(32);
        for value in &values {
            env = env.with_value(value.value, facts_for_role(value.role));
        }
        let identity_source = synth::minimal_singleton_core_input();
        StoragePlanCoreInput {
            input_identity: identity_source.input_identity,
            expected_input_hashes: identity_source.expected_input_hashes,
            repair_policy: Default::default(),
            predicate_env: adjust_env(env),
            topological_order: topological_order_for_values(&values),
            values,
            alias_edges,
            alias_forced_recompute_values,
            fail_before_result: false,
        }
    }

    fn value(id: u32, role: ValueRole, materialization: Materialization) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value: ValueId::new(id),
            materialization,
            live_range: synth::live_range(id * 3, id * 3 + 1, LifetimeClass::Slice),
            role,
            persist_kind: None,
            commit_group_reason: None,
        }
    }

    fn topological_order_for_values(values: &[StoragePlanCoreValue]) -> Vec<NodeId> {
        values
            .iter()
            .flat_map(|value| {
                [
                    value.live_range.def_node,
                    value
                        .live_range
                        .last_use_node
                        .unwrap_or(value.live_range.def_node),
                ]
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn persist_value(
        id: u32,
        page: PersistPageId,
        commit_group: CommitGroupId,
        kind: PersistKind,
        reason: CommitGroupReason,
    ) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value: ValueId::new(id),
            materialization: Materialization::Persist { page, commit_group },
            live_range: synth::live_range(id * 3, id * 3 + 1, LifetimeClass::Persistent),
            role: ValueRole::Activation,
            persist_kind: Some(kind),
            commit_group_reason: Some(reason),
        }
    }

    fn materialize_hot() -> Materialization {
        Materialization::Materialize {
            class: StorageClass::WramHot,
            lifetime: LifetimeClass::Slice,
        }
    }

    fn edge(left: u32, right: u32, intent: AliasIntent) -> AliasCandidateEdge {
        AliasCandidateEdge {
            left: ValueId::new(left),
            right: ValueId::new(right),
            intent,
        }
    }

    fn facts_for_role(role: ValueRole) -> PredicateValueFacts {
        let format = match role {
            ValueRole::RouterDecision | ValueRole::InputToken | ValueRole::OutputToken => {
                ValueFormat::TokenIdDomain { vocab_size: 8 }
            }
            ValueRole::ExpertWeight
            | ValueRole::RouterWeight
            | ValueRole::EmbeddingTable
            | ValueRole::LogitProj
            | ValueRole::NormParam
            | ValueRole::DecodeConst
            | ValueRole::LutFragment => ValueFormat::ConstTensorRef {
                tensor_id: TensorId::new(0),
            },
            ValueRole::Accumulator | ValueRole::Scratch => ValueFormat::IntAccum { width_bits: 16 },
            ValueRole::SequenceStateSlot => ValueFormat::Flag,
            ValueRole::Activation | ValueRole::RouterScore | ValueRole::FfnIntermediate => {
                ValueFormat::QuantInt {
                    quant_format_id: QuantFormatId(1),
                }
            }
        };
        let mut facts = PredicateValueFacts::new(role, format);
        facts.logical_size = Some(4);
        facts
    }

    fn consistency_context_from_core_result(
        input: &StoragePlanCoreInput,
        result: &StoragePlanCoreResult,
    ) -> StoragePlanConsistencyContext {
        StoragePlanConsistencyContext {
            expected_values: input.values.iter().map(|value| value.value).collect(),
            lifetime_bounds: input
                .values
                .iter()
                .map(|value| {
                    let bounds = match result
                        .bindings
                        .get(&value.value)
                        .map(|binding| &binding.materialization)
                    {
                        Some(Materialization::Persist { .. }) => {
                            bounds(LifetimeClass::Persistent, LifetimeClass::Persistent)
                        }
                        _ => crate::storage_plan::lifetime::lifetime_bounds(
                            &input.predicate_env,
                            value.value,
                        ),
                    };
                    (value.value, bounds)
                })
                .collect(),
            expected_input_hashes: input.expected_input_hashes,
        }
    }

    fn bounds(min: LifetimeClass, max: LifetimeClass) -> LifetimeBounds {
        LifetimeBounds {
            min_required: LifetimeBound {
                lifetime: min,
                source: LifetimeBoundSource::DefaultSlice,
            },
            max_admissible: LifetimeBound {
                lifetime: max,
                source: LifetimeBoundSource::DefaultSlice,
            },
        }
    }

    fn persist_page(id: PersistPageId, kind: PersistKind) -> PersistPageDecl {
        PersistPageDecl {
            id,
            kind,
            durability: DurabilityClass::BestEffort,
            schema_pin: PersistSchemaPin {
                state_schema: 3,
                requires_semantic_state_hash: false,
                requires_resume_abi_hash: false,
                requires_build_identity_hash: true,
            },
        }
    }

    fn failed_output_from_validation_diagnostics(
        input_identity: StoragePlanInputIdentity,
        diagnostics: &[gbf_policy::ValidationDiagnostic],
    ) -> StoragePlanCoreOutput {
        let mut seen_codes = BTreeSet::new();
        let diagnostic_details = diagnostics
            .iter()
            .filter_map(storage_diagnostic_detail)
            .filter(|detail| seen_codes.insert(detail.code))
            .collect();
        StoragePlanCoreOutput {
            input_identity,
            outcome: StoragePlanCoreOutcome::Failed,
            result: None,
            summary: None,
            diagnostics: diagnostics
                .iter()
                .filter_map(storage_code)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            diagnostic_details,
        }
    }

    fn storage_diagnostic_detail(
        diagnostic: &gbf_policy::ValidationDiagnostic,
    ) -> Option<StoragePlanCoreDiagnosticDetail> {
        match &diagnostic.code {
            ValidationCode::StoragePlan { code, provenance } => {
                Some(StoragePlanCoreDiagnosticDetail {
                    code: *code,
                    provenance: provenance.clone(),
                    evidence: diagnostic.provenance.clone(),
                })
            }
            _ => None,
        }
    }

    fn storage_code(
        diagnostic: &gbf_policy::ValidationDiagnostic,
    ) -> Option<StoragePlanDiagnosticCode> {
        match &diagnostic.code {
            ValidationCode::StoragePlan { code, .. } => Some(*code),
            _ => None,
        }
    }
}

pub mod trace_catalog {
    pub const TRACE_TARGET: &str = "gbf_codegen::storage_plan";
    pub const DRIVER_RUN_STARTED: &str = "f_b8.driver.run.started";
    pub const DRIVER_STEP_COMPLETED: &str = "f_b8.driver.step.completed";
    pub const RULE_FIRED: &str = "f_b8.rule.fired";
    pub const RULE_EVALUATED_NO_MATCH: &str = "f_b8.rule.evaluated_no_match";
    pub const BINDING_EMITTED: &str = "f_b8.binding.emitted";
    pub const ALIAS_CANDIDATE_EDGE_COLLECTED: &str = "f_b8.alias.candidate_edge_collected";
    pub const ALIAS_COMPONENT_CONSTRUCTED: &str = "f_b8.alias.component_constructed";
    pub const ALIAS_MIXED_INTENT_REJECTED: &str = "f_b8.alias.mixed_intent_rejected";
    pub const PERSIST_PAGE_EMITTED: &str = "f_b8.persist.page_emitted";
    pub const PERSIST_COMMIT_GROUP_EMITTED: &str = "f_b8.persist.commit_group_emitted";
    pub const PERSIST_TRANSCRIPT_PROMOTED: &str = "f_b8.persist.transcript_promoted";
    pub const LIFETIME_BOUNDS_COMPUTED: &str = "f_b8.lifetime.bounds_computed";
    pub const DIAGNOSTIC_EMITTED: &str = "f_b8.diagnostic.emitted";
    pub const K6_INPUTS_CANONICALIZED: &str = "f_b8.k6.inputs_canonicalized";
    pub const K6_CACHE_HIT: &str = "f_b8.k6.cache.hit";
    pub const K6_CACHE_MISS: &str = "f_b8.k6.cache.miss";
    pub const PROPOSAL_EMITTED: &str = "f_b8.proposal.emitted";
    pub const ENVELOPE_HASHED: &str = "f_b8.envelope.hashed";

    pub const EVENT_NAMES: &[&str] = &[
        DRIVER_RUN_STARTED,
        DRIVER_STEP_COMPLETED,
        RULE_FIRED,
        RULE_EVALUATED_NO_MATCH,
        BINDING_EMITTED,
        ALIAS_CANDIDATE_EDGE_COLLECTED,
        ALIAS_COMPONENT_CONSTRUCTED,
        ALIAS_MIXED_INTENT_REJECTED,
        PERSIST_PAGE_EMITTED,
        PERSIST_COMMIT_GROUP_EMITTED,
        PERSIST_TRANSCRIPT_PROMOTED,
        LIFETIME_BOUNDS_COMPUTED,
        DIAGNOSTIC_EMITTED,
        K6_INPUTS_CANONICALIZED,
        K6_CACHE_HIT,
        K6_CACHE_MISS,
        PROPOSAL_EMITTED,
        ENVELOPE_HASHED,
    ];

    pub const SPAN_NAMES: &[&str] = &[
        "f_b8::driver",
        "f_b8::driver::step.1",
        "f_b8::rule_pass",
        "f_b8::alias_construction",
        "f_b8::persist_resolution",
    ];

    #[must_use]
    pub fn is_catalog_event(name: &str) -> bool {
        EVENT_NAMES.contains(&name)
    }

    pub fn emit_catalog_fixture_events(correlation_id: &str) {
        tracing::info!(
            target: TRACE_TARGET,
            event = DRIVER_RUN_STARTED,
            correlation_id,
            inputs_hash = "sha256:catalog",
            policy_hash = "sha256:catalog",
            determinism = "Deterministic",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = DRIVER_STEP_COMPLETED,
            correlation_id,
            step_num = 1_u64,
            step_name = "catalog",
            value_count = 1_u64,
            elapsed_ns = 0_u64,
        );
        tracing::debug!(
            target: TRACE_TARGET,
            event = RULE_FIRED,
            correlation_id,
            rule_id = 15_u64,
            rule_name = "DR-13 DefaultMaterializeKnownIntermediate",
            value_id = 1_u64,
            outcome = "Bind",
            priority = 150_u64,
        );
        tracing::trace!(
            target: TRACE_TARGET,
            event = RULE_EVALUATED_NO_MATCH,
            correlation_id,
            rule_id = 1_u64,
            value_id = 1_u64,
        );
        tracing::debug!(
            target: TRACE_TARGET,
            event = BINDING_EMITTED,
            correlation_id,
            value_id = 1_u64,
            materialization = "Materialize",
            alias_class = 0_u64,
            live_range_summary = "0..1",
        );
        tracing::trace!(
            target: TRACE_TARGET,
            event = ALIAS_CANDIDATE_EDGE_COLLECTED,
            correlation_id,
            v_a = 1_u64,
            v_b = 2_u64,
            intent = "ScratchReuse",
        );
        tracing::debug!(
            target: TRACE_TARGET,
            event = ALIAS_COMPONENT_CONSTRUCTED,
            correlation_id,
            fingerprint = "sha256:catalog",
            intent = "NoAlias",
            member_count = 1_u64,
            dense_id = 0_u64,
        );
        tracing::warn!(
            target: TRACE_TARGET,
            event = ALIAS_MIXED_INTENT_REJECTED,
            correlation_id,
            component_members = "[1,2]",
            intents_seen = "[ScratchReuse,PingPong]",
        );
        tracing::debug!(
            target: TRACE_TARGET,
            event = PERSIST_PAGE_EMITTED,
            correlation_id,
            page_id = 1_u64,
            kind = "Trace",
            durability = "BestEffort",
        );
        tracing::debug!(
            target: TRACE_TARGET,
            event = PERSIST_COMMIT_GROUP_EMITTED,
            correlation_id,
            group_id = 1_u64,
            kind_set = "[Trace]",
            atomicity = "AllOrNothing",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = PERSIST_TRANSCRIPT_PROMOTED,
            correlation_id,
            page_id = 1_u64,
            from = "BestEffort",
            to = "Recoverable",
        );
        tracing::trace!(
            target: TRACE_TARGET,
            event = LIFETIME_BOUNDS_COMPUTED,
            correlation_id,
            value_id = 1_u64,
            min = "Slice",
            max = "Token",
            chosen = "Slice",
        );
        tracing::error!(
            target: TRACE_TARGET,
            event = DIAGNOSTIC_EMITTED,
            correlation_id,
            code = "STORE-001",
            severity = "Hard",
            provenance_json = "{}",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = K6_INPUTS_CANONICALIZED,
            correlation_id,
            cache_key_hex = correlation_id,
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = K6_CACHE_HIT,
            correlation_id,
            cache_key_hex = correlation_id,
            entry_kind = "success",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = K6_CACHE_MISS,
            correlation_id,
            cache_key_hex = correlation_id,
            entry_kind = "success",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = PROPOSAL_EMITTED,
            correlation_id,
            reason = "fixture",
            tighten_summary = "none",
            estimated_cost = "0",
        );
        tracing::info!(
            target: TRACE_TARGET,
            event = ENVELOPE_HASHED,
            correlation_id,
            report_self_hash = "sha256:catalog",
            outcome = "Passed",
        );
    }
}

pub mod debug_harness {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct HarnessArgs {
        pub inputs: String,
        pub out_dir: PathBuf,
        pub trace_file: Option<PathBuf>,
        pub emit_k6: Option<PathBuf>,
        pub emit_envelope: Option<PathBuf>,
        pub trace_format: TraceFormat,
        pub verbose: bool,
        pub trace: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TraceFormat {
        Json,
        Pretty,
    }

    #[derive(Debug)]
    pub enum HarnessError {
        Args(String),
        Io(io::Error),
        Json(serde_json::Error),
        Emit(String),
    }

    impl std::fmt::Display for HarnessError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Args(message) => f.write_str(message),
                Self::Io(error) => write!(f, "{error}"),
                Self::Json(error) => write!(f, "{error}"),
                Self::Emit(message) => f.write_str(message),
            }
        }
    }

    impl std::error::Error for HarnessError {}

    impl From<io::Error> for HarnessError {
        fn from(value: io::Error) -> Self {
            Self::Io(value)
        }
    }

    impl From<serde_json::Error> for HarnessError {
        fn from(value: serde_json::Error) -> Self {
            Self::Json(value)
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct StoragePlanDebugEnvelope {
        pub kind: String,
        pub schema: String,
        pub schema_version: String,
        pub command_line: Vec<String>,
        pub env_vars: BTreeMap<String, String>,
        pub build_identity_hash: Hash256,
        pub ts: String,
        pub fixture_id: String,
        pub outcome: String,
        pub artifacts: BTreeMap<String, String>,
    }

    pub fn parse_args<I, S>(args: I) -> Result<HarnessArgs, HarnessError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut inputs = None;
        let mut out_dir = None;
        let mut trace_file = None;
        let mut emit_k6 = None;
        let mut emit_envelope = None;
        let mut trace_format = TraceFormat::Json;
        let mut verbose = false;
        let mut trace = false;

        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--inputs" => inputs = Some(next_arg(&mut iter, "--inputs")?),
                "--out" => out_dir = Some(PathBuf::from(next_arg(&mut iter, "--out")?)),
                "--trace-file" => {
                    trace_file = Some(PathBuf::from(next_arg(&mut iter, "--trace-file")?));
                }
                "--emit-k6" => emit_k6 = Some(PathBuf::from(next_arg(&mut iter, "--emit-k6")?)),
                "--emit-envelope" => {
                    emit_envelope = Some(PathBuf::from(next_arg(&mut iter, "--emit-envelope")?));
                }
                "--trace-format" => {
                    let value = next_arg(&mut iter, "--trace-format")?;
                    trace_format = match value.as_str() {
                        "json" => TraceFormat::Json,
                        "pretty" => TraceFormat::Pretty,
                        other => {
                            return Err(HarnessError::Args(format!(
                                "unsupported --trace-format {other:?}"
                            )));
                        }
                    };
                }
                "--verbose" => verbose = true,
                "--trace" => trace = true,
                "--help" | "-h" => {
                    return Err(HarnessError::Args(usage()));
                }
                other => {
                    return Err(HarnessError::Args(format!("unexpected argument {other:?}")));
                }
            }
        }

        let out_dir = out_dir.ok_or_else(|| HarnessError::Args("missing --out".to_owned()))?;
        Ok(HarnessArgs {
            inputs: inputs.ok_or_else(|| HarnessError::Args("missing --inputs".to_owned()))?,
            trace_file,
            emit_k6,
            emit_envelope,
            trace_format,
            verbose,
            trace,
            out_dir,
        })
    }

    pub fn run(args: HarnessArgs, command_line: Vec<String>) -> Result<i32, HarnessError> {
        fs::create_dir_all(&args.out_dir)?;
        let trace_file = args
            .trace_file
            .clone()
            .unwrap_or_else(|| args.out_dir.join("trace.jsonl"));
        let k6_file = args
            .emit_k6
            .clone()
            .unwrap_or_else(|| args.out_dir.join("k6.txt"));
        let envelope_file = args
            .emit_envelope
            .clone()
            .unwrap_or_else(|| args.out_dir.join("envelope.json"));
        let summary_file = args.out_dir.join("summary.txt");
        let plan_file = args.out_dir.join("storage_plan.json");

        let sink = NdjsonTraceSink::new(&trace_file)?;
        let subscriber = tracing_subscriber::registry().with(sink);
        let fixture = load_fixture(&args.inputs)?;
        let cache_key = StoragePlanCacheKeyInputs::from_input_identity(&fixture.input_identity)
            .and_then(|inputs| inputs.cache_key())
            .map_err(|error| HarnessError::Emit(error.to_string()))?;
        let correlation_id = cache_key_prefix(cache_key);

        let (output, storage_plan_bytes) = tracing::subscriber::with_default(subscriber, || {
            emit_harness_start(&fixture.input_identity, &correlation_id, &args);
            let output = match &fixture.violation_fixture {
                Some(violation) => violation
                    .run()
                    .map_err(|error| HarnessError::Emit(error.to_string()))?,
                None => build_storage_plan_core(&fixture.core_input),
            };
            let envelope = emit_storage_plan_report(&output)
                .map_err(|error| HarnessError::Emit(error.to_string()))?;
            emit_harness_complete(&output, &correlation_id, &envelope.report_self_hash);
            let bytes =
                canonicalize(&envelope).map_err(|error| HarnessError::Emit(error.to_string()))?;
            Ok::<_, HarnessError>((output, bytes))
        })?;

        fs::write(&plan_file, storage_plan_bytes)?;
        fs::write(&k6_file, format!("{correlation_id}\n"))?;
        write_summary(&summary_file, &output)?;
        write_envelope(
            &envelope_file,
            StoragePlanDebugEnvelope {
                kind: "gbf-storage-plan-debug".to_owned(),
                schema: "gbf_debug.storage_plan_envelope.v1".to_owned(),
                schema_version: "1.0.0".to_owned(),
                command_line,
                env_vars: captured_env_vars(),
                build_identity_hash: build_identity_hash(),
                ts: normalized_debug_timestamp().to_owned(),
                fixture_id: fixture.fixture_id,
                outcome: match output.outcome {
                    StoragePlanCoreOutcome::Succeeded => "Passed",
                    StoragePlanCoreOutcome::Failed => "Failed",
                }
                .to_owned(),
                artifacts: BTreeMap::from([
                    ("storage_plan".to_owned(), plan_file.display().to_string()),
                    ("trace".to_owned(), trace_file.display().to_string()),
                    ("k6".to_owned(), k6_file.display().to_string()),
                    ("summary".to_owned(), summary_file.display().to_string()),
                ]),
            },
        )?;

        Ok(match output.outcome {
            StoragePlanCoreOutcome::Succeeded => 0,
            StoragePlanCoreOutcome::Failed => 1,
        })
    }

    #[derive(Debug, Clone)]
    struct LoadedFixture {
        fixture_id: String,
        input_identity: StoragePlanInputIdentity,
        core_input: StoragePlanCoreInput,
        violation_fixture: Option<sc_violations::StoragePlanViolationFixture>,
    }

    fn load_fixture(spec: &str) -> Result<LoadedFixture, HarnessError> {
        let builtin = spec.strip_prefix("builtin:").unwrap_or(spec);
        match builtin {
            "minimal_singleton" => {
                loaded("minimal_singleton", synth::minimal_singleton_core_input())
            }
            "tiny_tinystories" => loaded("tiny_tinystories", synth::tiny_tinystories_core_input()),
            "tiny_routed_ffn" => loaded("tiny_routed_ffn", synth::tiny_routed_ffn_core_input()),
            "sc11_forbidden_key_leak" => {
                let violation_fixture = sc_violations::sc11_forbidden_key_leak();
                let core_input = synth::minimal_singleton_core_input();
                Ok(LoadedFixture {
                    fixture_id: "sc11_forbidden_key_leak".to_owned(),
                    input_identity: core_input.input_identity.clone(),
                    core_input,
                    violation_fixture: Some(violation_fixture),
                })
            }
            _ => load_path_fixture(Path::new(spec)),
        }
    }

    fn loaded(
        fixture_id: impl Into<String>,
        core_input: StoragePlanCoreInput,
    ) -> Result<LoadedFixture, HarnessError> {
        Ok(LoadedFixture {
            fixture_id: fixture_id.into(),
            input_identity: core_input.input_identity.clone(),
            core_input,
            violation_fixture: None,
        })
    }

    fn load_path_fixture(path: &Path) -> Result<LoadedFixture, HarnessError> {
        let input_path = if path.is_dir() {
            path.join("core_input.json")
        } else {
            path.to_path_buf()
        };
        let bytes = fs::read(&input_path)?;
        let core_input: StoragePlanCoreInput = serde_json::from_slice(&bytes)?;
        loaded(input_path.display().to_string(), core_input)
    }

    fn emit_harness_start(
        identity: &StoragePlanInputIdentity,
        correlation_id: &str,
        args: &HarnessArgs,
    ) {
        tracing::info!(
            target: trace_catalog::TRACE_TARGET,
            event = trace_catalog::DRIVER_RUN_STARTED,
            correlation_id,
            inputs_hash = %identity.range_plan_hash,
            policy_hash = %identity.policy_hash,
            determinism = ?identity.determinism,
        );
        tracing::info!(
            target: trace_catalog::TRACE_TARGET,
            event = trace_catalog::K6_INPUTS_CANONICALIZED,
            correlation_id,
            cache_key_hex = correlation_id,
        );
        tracing::info!(
            target: trace_catalog::TRACE_TARGET,
            event = trace_catalog::K6_CACHE_MISS,
            correlation_id,
            cache_key_hex = correlation_id,
            entry_kind = "debug_harness",
        );
        let level = if args.trace {
            "TRACE"
        } else if args.verbose {
            "DEBUG"
        } else {
            "INFO"
        };
        tracing::debug!(
            target: trace_catalog::TRACE_TARGET,
            event = trace_catalog::DRIVER_STEP_COMPLETED,
            correlation_id,
            step_num = 0_u64,
            step_name = "harness_load",
            value_count = 0_u64,
            elapsed_ns = 0_u64,
            level,
        );
    }

    fn emit_harness_complete(
        output: &StoragePlanCoreOutput,
        correlation_id: &str,
        report_self_hash: &Hash256,
    ) {
        if let Some(result) = &output.result {
            emit_result_trace(result, correlation_id);
        }
        for code in &output.diagnostics {
            tracing::error!(
                target: trace_catalog::TRACE_TARGET,
                event = trace_catalog::DIAGNOSTIC_EMITTED,
                correlation_id,
                code = code.as_str(),
                severity = "Hard",
                provenance_json = "{}",
            );
        }
        tracing::info!(
            target: trace_catalog::TRACE_TARGET,
            event = trace_catalog::ENVELOPE_HASHED,
            correlation_id,
            report_self_hash = %report_self_hash,
            outcome = match output.outcome {
                StoragePlanCoreOutcome::Succeeded => "Passed",
                StoragePlanCoreOutcome::Failed => "Failed",
            },
        );
    }

    fn emit_result_trace(result: &StoragePlanCoreResult, correlation_id: &str) {
        for binding in result.bindings.values() {
            tracing::debug!(
                target: trace_catalog::TRACE_TARGET,
                event = trace_catalog::BINDING_EMITTED,
                correlation_id,
                value_id = binding.value.get() as u64,
                materialization = ?binding.materialization,
                alias_class = binding.alias_class.0 as u64,
                live_range_summary = ?binding.live_range,
            );
        }
        for class in result.alias_classes.values() {
            tracing::debug!(
                target: trace_catalog::TRACE_TARGET,
                event = trace_catalog::ALIAS_COMPONENT_CONSTRUCTED,
                correlation_id,
                fingerprint = %class.fingerprint().0,
                intent = ?class.intent(),
                member_count = class.members().len() as u64,
                dense_id = class.id().0 as u64,
            );
        }
        for page in result.persist_pages.values() {
            tracing::debug!(
                target: trace_catalog::TRACE_TARGET,
                event = trace_catalog::PERSIST_PAGE_EMITTED,
                correlation_id,
                page_id = page.id.0 as u64,
                kind = ?page.kind,
                durability = ?page.durability,
            );
        }
        for group in result.commit_groups.values() {
            tracing::debug!(
                target: trace_catalog::TRACE_TARGET,
                event = trace_catalog::PERSIST_COMMIT_GROUP_EMITTED,
                correlation_id,
                group_id = group.id.0 as u64,
                kind_set = ?group.kind_set,
                atomicity = ?group.atomicity,
            );
        }
    }

    fn write_summary(path: &Path, output: &StoragePlanCoreOutput) -> io::Result<()> {
        let mut file = File::create(path)?;
        if output.diagnostics.is_empty() {
            writeln!(file, "Passed")?;
        } else {
            for code in &output.diagnostics {
                writeln!(file, "{} {}", code.as_str(), code.name())?;
            }
        }
        Ok(())
    }

    fn write_envelope(path: &Path, envelope: StoragePlanDebugEnvelope) -> Result<(), HarnessError> {
        let bytes = serde_json::to_vec_pretty(&envelope)?;
        fs::write(path, bytes)?;
        Ok(())
    }

    fn next_arg<I>(iter: &mut std::iter::Peekable<I>, flag: &str) -> Result<String, HarnessError>
    where
        I: Iterator<Item = String>,
    {
        iter.next()
            .ok_or_else(|| HarnessError::Args(format!("{flag} requires a value")))
    }

    fn usage() -> String {
        "gbf-storage-plan-debug --inputs <builtin:name|path> --out <dir> [--trace-format json] [--trace-file path] [--emit-k6 path] [--emit-envelope path] [--verbose] [--trace]".to_owned()
    }
}

#[derive(Clone)]
pub struct NdjsonTraceSink {
    writer: Arc<Mutex<File>>,
}

impl NdjsonTraceSink {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            writer: Arc::new(Mutex::new(File::create(path)?)),
        })
    }
}

impl<S> Layer<S> for NdjsonTraceSink
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let event_name = visitor
            .fields
            .remove("event")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| event.metadata().name().to_owned());
        let span = ctx.lookup_current().map(|span| {
            serde_json::json!({
                "name": span.name(),
                "target": span.metadata().target(),
            })
        });
        let line = serde_json::json!({
            "ts": timestamp_string(),
            "event": event_name,
            "level": event.metadata().level().as_str(),
            "target": event.metadata().target(),
            "fields": Value::Object(visitor.fields),
            "span": span,
        });

        let result = (|| -> io::Result<()> {
            let mut writer = self
                .writer
                .lock()
                .map_err(|_| io::Error::other("storage-plan trace sink mutex poisoned"))?;
            serde_json::to_writer(&mut *writer, &line)?;
            writer.write_all(b"\n")?;
            writer.flush()
        })();
        if let Err(error) = result {
            panic!("failed to write F-B8 storage-plan telemetry event {event_name}: {error}");
        }
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

#[derive(Clone, Default)]
pub struct TraceCapture {
    events: Arc<Mutex<Vec<Value>>>,
}

impl TraceCapture {
    #[must_use]
    pub fn events(&self) -> Vec<Value> {
        self.events.lock().expect("trace capture lock").clone()
    }
}

impl<S> Layer<S> for TraceCapture
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .remove("event")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| event.metadata().name().to_owned());
        self.events
            .lock()
            .expect("trace capture lock")
            .push(serde_json::json!({"event": name, "fields": Value::Object(visitor.fields)}));
    }
}

pub fn with_trace_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<Value>) {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let result = tracing::subscriber::with_default(subscriber, f);
    (result, capture.events())
}

#[macro_export]
macro_rules! assert_storage_plan_traced {
    ($events:expr, [$( { event: $event:expr $(, $field:ident : $value:expr)* $(,)? } ),* $(,)?]) => {{
        let events = $events;
        let mut cursor = 0usize;
        $(
            let mut matched = false;
            while cursor < events.len() {
                let record = &events[cursor];
                cursor += 1;
                if record.get("event").and_then(serde_json::Value::as_str) != Some($event) {
                    continue;
                }
                let fields = record.get("fields").and_then(serde_json::Value::as_object);
                let fields_match = true $(
                    && fields
                        .and_then(|map| map.get(stringify!($field)))
                        == Some(&serde_json::json!($value))
                )*;
                if fields_match {
                    matched = true;
                    break;
                }
            }
            assert!(matched, "missing ordered storage-plan trace event {}", $event);
        )*
    }};
}

pub fn cache_key_prefix(key: StoragePlanCacheKey) -> String {
    key.0.to_hex().chars().take(16).collect()
}

pub fn timestamp_string() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("unix:{}.{:09}", duration.as_secs(), duration.subsec_nanos())
}

pub const fn normalized_debug_timestamp() -> &'static str {
    "unix:0.000000000"
}

fn captured_env_vars() -> BTreeMap<String, String> {
    ["GBF_F_B8_LOG", "RUST_LOG"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
        .collect()
}

fn build_identity_hash() -> Hash256 {
    let package = env!("CARGO_PKG_VERSION");
    gbf_foundation::sha256(format!("gbf-codegen:{package}").as_bytes())
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_inputs_hashes_round_trip() {
        for inputs in [
            synth::minimal_singleton_inputs(),
            synth::tiny_tinystories_inputs(),
            synth::tiny_routed_ffn_inputs(),
        ] {
            canonicalize_inputs(&inputs).expect("synth input hashes match");
        }
    }

    #[test]
    fn sc_violation_factory_coverage_names_every_store_code() {
        let factories = sc_violations::all_store_violation_factories();
        let codes: BTreeSet<_> = factories
            .iter()
            .map(|fixture| fixture.expected_code)
            .collect();

        assert_eq!(codes, StoragePlanDiagnosticCode::ALL.into_iter().collect());
        assert!(factories.iter().all(|fixture| {
            !fixture.fixture_id.is_empty()
                && !fixture.provenance_schema.is_empty()
                && !fixture.rfc_section.is_empty()
        }));
    }

    #[test]
    fn production_backed_violation_factories_run_validators_exactly() {
        let factories = sc_violations::production_backed_violation_factories();
        assert!(
            factories.len() > 1,
            "production proof must cover more than the SC11 fixture"
        );

        for fixture in factories {
            let output = fixture.run().expect("production fixture runs");

            assert_eq!(
                output.outcome,
                StoragePlanCoreOutcome::Failed,
                "{} should fail through production validation",
                fixture.fixture_id
            );
            assert_eq!(
                output.diagnostics,
                vec![fixture.expected_code],
                "{} emitted unexpected diagnostics",
                fixture.fixture_id
            );
        }
    }

    #[test]
    fn stub_violation_factories_are_empty_after_t23_fixture_closure() {
        let production_codes: BTreeSet<_> = sc_violations::production_backed_violation_factories()
            .into_iter()
            .map(|fixture| fixture.expected_code)
            .collect();
        let synthetic_codes: BTreeSet<_> =
            sc_violations::synthetic_diagnostic_violation_factories()
                .into_iter()
                .map(|fixture| fixture.expected_code)
                .collect();
        let stubs = sc_violations::stub_only_violation_factories();

        assert!(stubs.is_empty());
        assert_eq!(
            production_codes
                .union(&synthetic_codes)
                .copied()
                .collect::<BTreeSet<_>>(),
            StoragePlanDiagnosticCode::ALL.into_iter().collect()
        );
        assert!(
            !synthetic_codes
                .contains(&StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent)
        );
        assert!(
            !synthetic_codes.contains(&StoragePlanDiagnosticCode::StorageRepairProposalIllegal)
        );
    }

    #[test]
    fn sc11_violation_factory_runs_production_validator_exactly() {
        let fixture = sc_violations::sc11_forbidden_key_leak();
        let output = fixture.run().expect("SC11 production fixture runs");

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert_eq!(output.diagnostics, vec![fixture.expected_code]);
    }

    #[test]
    fn tracing_catalog_coverage_fixture_emits_every_event_name() {
        let (_, events) = with_trace_capture(|| {
            trace_catalog::emit_catalog_fixture_events("abc123");
        });
        let emitted: BTreeSet<_> = events
            .iter()
            .filter_map(|event| event.get("event").and_then(Value::as_str))
            .collect();

        for name in trace_catalog::EVENT_NAMES {
            assert!(emitted.contains(name), "missing trace catalog event {name}");
        }
    }

    #[test]
    fn debug_harness_passed_and_failed_paths_write_expected_artifacts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pass_dir = temp.path().join("pass");
        let pass_status = debug_harness::run(
            debug_harness::HarnessArgs {
                inputs: "builtin:tiny_routed_ffn".to_owned(),
                out_dir: pass_dir.clone(),
                trace_file: None,
                emit_k6: None,
                emit_envelope: None,
                trace_format: debug_harness::TraceFormat::Json,
                verbose: false,
                trace: false,
            },
            vec!["gbf-storage-plan-debug".to_owned()],
        )
        .expect("pass harness runs");
        assert_eq!(pass_status, 0);
        for file in [
            "storage_plan.json",
            "trace.jsonl",
            "k6.txt",
            "summary.txt",
            "envelope.json",
        ] {
            assert!(pass_dir.join(file).is_file(), "missing {file}");
        }
        let plan_bytes = fs::read(pass_dir.join("storage_plan.json")).expect("plan reads");
        let plan = crate::storage_plan::parse_storage_plan_report_bytes(&plan_bytes)
            .expect("storage plan report parses");
        let envelope_hashed = trace_event(
            &pass_dir.join("trace.jsonl"),
            trace_catalog::ENVELOPE_HASHED,
        )
        .expect("envelope hashed trace exists");
        let expected_report_self_hash = plan.report_self_hash.to_string();
        assert_eq!(
            envelope_hashed
                .get("fields")
                .and_then(Value::as_object)
                .and_then(|fields| fields.get("report_self_hash"))
                .and_then(Value::as_str),
            Some(expected_report_self_hash.as_str())
        );
        assert_ne!(
            envelope_hashed
                .get("fields")
                .and_then(Value::as_object)
                .and_then(|fields| fields.get("report_self_hash"))
                .and_then(Value::as_str),
            Some("pending")
        );

        let fail_dir = temp.path().join("fail");
        let fail_status = debug_harness::run(
            debug_harness::HarnessArgs {
                inputs: "builtin:sc11_forbidden_key_leak".to_owned(),
                out_dir: fail_dir.clone(),
                trace_file: None,
                emit_k6: None,
                emit_envelope: None,
                trace_format: debug_harness::TraceFormat::Json,
                verbose: false,
                trace: false,
            },
            vec!["gbf-storage-plan-debug".to_owned()],
        )
        .expect("fail harness runs");
        assert_eq!(fail_status, 1);
        let summary = fs::read_to_string(fail_dir.join("summary.txt")).expect("summary reads");
        assert!(summary.contains("STORE-018"));
    }

    #[test]
    fn debug_harness_parse_error_exits_as_harness_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing-fixture");
        let err = debug_harness::run(
            debug_harness::HarnessArgs {
                inputs: missing.display().to_string(),
                out_dir: temp.path().join("out"),
                trace_file: None,
                emit_k6: None,
                emit_envelope: None,
                trace_format: debug_harness::TraceFormat::Json,
                verbose: false,
                trace: false,
            },
            vec!["gbf-storage-plan-debug".to_owned()],
        )
        .expect_err("missing fixture is a harness error");

        assert!(matches!(err, debug_harness::HarnessError::Io(_)));
    }

    #[test]
    fn debug_harness_envelope_round_trips_and_replay_is_deterministic() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first");
        let second = temp.path().join("second");

        for out_dir in [&first, &second] {
            let status = debug_harness::run(
                debug_harness::HarnessArgs {
                    inputs: "builtin:tiny_routed_ffn".to_owned(),
                    out_dir: out_dir.to_path_buf(),
                    trace_file: None,
                    emit_k6: None,
                    emit_envelope: None,
                    trace_format: debug_harness::TraceFormat::Json,
                    verbose: false,
                    trace: false,
                },
                vec!["gbf-storage-plan-debug".to_owned()],
            )
            .expect("harness run succeeds");
            assert_eq!(status, 0);
        }

        assert_eq!(
            fs::read(first.join("storage_plan.json")).expect("first plan"),
            fs::read(second.join("storage_plan.json")).expect("second plan")
        );
        assert_eq!(
            fs::read(first.join("k6.txt")).expect("first k6"),
            fs::read(second.join("k6.txt")).expect("second k6")
        );
        assert_eq!(
            normalize_trace_jsonl(&first.join("trace.jsonl")),
            normalize_trace_jsonl(&second.join("trace.jsonl"))
        );

        let envelope_bytes = fs::read(first.join("envelope.json")).expect("envelope reads");
        let envelope: debug_harness::StoragePlanDebugEnvelope =
            serde_json::from_slice(&envelope_bytes).expect("envelope parses");
        let encoded = serde_json::to_vec_pretty(&envelope).expect("envelope reserializes");
        assert_eq!(encoded, envelope_bytes);
        assert_eq!(envelope.ts, normalized_debug_timestamp());
    }

    #[test]
    fn m1_degenerate_fixture_emits_replays_and_feeds_downstream_stubs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first");
        let second = temp.path().join("second");

        for out_dir in [&first, &second] {
            let status = debug_harness::run(
                debug_harness::HarnessArgs {
                    inputs: "builtin:tiny_tinystories".to_owned(),
                    out_dir: out_dir.to_path_buf(),
                    trace_file: None,
                    emit_k6: None,
                    emit_envelope: None,
                    trace_format: debug_harness::TraceFormat::Json,
                    verbose: false,
                    trace: false,
                },
                vec!["gbf-storage-plan-debug".to_owned()],
            )
            .expect("M1 degenerate fixture run succeeds");
            assert_eq!(status, 0);
        }

        let first_plan_bytes = fs::read(first.join("storage_plan.json")).expect("first plan");
        let second_plan_bytes = fs::read(second.join("storage_plan.json")).expect("second plan");
        assert_eq!(
            first_plan_bytes, second_plan_bytes,
            "two clean M1 degenerate emissions must be byte-identical"
        );

        let envelope = crate::storage_plan::parse_storage_plan_report_bytes(&first_plan_bytes)
            .expect("storage_plan.json parses");
        gbf_report::round_trip_self_hash(&envelope).expect("self hash round-trips");
        assert_eq!(
            gbf_report::canonicalize(&envelope).expect("parsed envelope canonicalizes"),
            first_plan_bytes,
            "parse + canonical re-emission preserves bytes"
        );
        let plan_json: Value = serde_json::from_slice(&first_plan_bytes).expect("plan JSON parses");
        assert!(
            crate::storage_plan::closed_spatial_surface_diagnostics(&plan_json).is_empty(),
            "SC11 forbidden spatial-key scan must be clean"
        );

        let body = &envelope.body.body;
        assert_eq!(body.outcome, gbf_report::ReportOutcome::Passed);
        assert!(body.diagnostics.is_empty());
        let summary = body.summary.expect("passed plan has summary");
        assert!(summary.total_bindings > 0);
        assert_eq!(summary.total_bindings, summary.bindings);
        assert_eq!(summary.alias_classes_no_alias, summary.total_bindings);

        let result = body.result.as_ref().expect("passed plan has result");
        assert!(result.persist_pages.is_empty());
        assert!(result.commit_groups.is_empty());
        assert!(
            result.bindings.iter().all(|entry| matches!(
                entry.value.materialization,
                Materialization::Materialize { .. }
            )),
            "M1 degenerate fixture is materialize-only"
        );
        assert!(
            result
                .alias_classes
                .iter()
                .all(|entry| entry.value.intent == AliasIntent::NoAlias
                    && entry.value.members.len() == 1),
            "every alias class must be a NoAlias singleton"
        );

        let k6 = StoragePlanCacheKeyInputs::from_input_identity(&body.input_identity)
            .and_then(|inputs| inputs.cache_key())
            .expect("K6 computes for emitted identity");
        let product_bytes =
            gbf_foundation::CanonicalJson::to_vec(result).expect("result canonicalizes");
        let replay = crate::storage_plan::StoragePlanStageCacheReplay::Success(
            crate::storage_plan::StageCacheSuccessEntry {
                key: k6,
                product: product_bytes.clone(),
                report_hash: envelope.report_self_hash,
                artifact_path: "storage_plan.json".to_owned(),
            },
        );
        assert_eq!(
            replay.replay_success(),
            Some(&product_bytes),
            "K6 success replay returns the byte-identical product"
        );

        downstream_stage_consumer_stubs_accept_degenerate_product(result);
    }

    #[test]
    fn assert_traced_helper_checks_ordered_events() {
        let (_, events) = with_trace_capture(|| {
            tracing::info!(
                event = "f_b8.rule.fired",
                rule_name = "DR-6",
                value_id = 7_u64,
            );
            tracing::info!(event = "f_b8.binding.emitted", value_id = 7_u64);
        });

        assert_storage_plan_traced!(
            &events,
            [
                {
                    event: "f_b8.rule.fired",
                    rule_name: "DR-6",
                    value_id: 7_u64
                },
                {
                    event: "f_b8.binding.emitted",
                    value_id: 7_u64
                }
            ]
        );
    }

    fn normalize_trace_jsonl(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .expect("trace reads")
            .lines()
            .map(|line| {
                let mut value: Value = serde_json::from_str(line).expect("trace line parses");
                value
                    .as_object_mut()
                    .expect("trace line is object")
                    .remove("ts");
                value
            })
            .collect()
    }

    fn trace_event(path: &Path, name: &str) -> Option<Value> {
        fs::read_to_string(path)
            .expect("trace reads")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("trace line parses"))
            .find(|value| value.get("event").and_then(Value::as_str) == Some(name))
    }

    fn downstream_stage_consumer_stubs_accept_degenerate_product(
        result: &crate::storage_plan::StoragePlanReportResult,
    ) {
        let sram_page_plan_inputs = result
            .bindings
            .iter()
            .filter(|entry| {
                matches!(
                    entry.value.materialization,
                    Materialization::Materialize {
                        class: StorageClass::SramPaged,
                        ..
                    } | Materialization::Persist { .. }
                )
            })
            .count();
        let rom_window_plan_inputs = result
            .bindings
            .iter()
            .filter(|entry| {
                matches!(
                    entry.value.materialization,
                    Materialization::Materialize {
                        class: StorageClass::RomConst,
                        ..
                    }
                )
            })
            .count();
        let arena_plan_inputs = result
            .bindings
            .iter()
            .filter(|entry| {
                matches!(
                    entry.value.materialization,
                    Materialization::Materialize { .. }
                )
            })
            .count();

        assert_eq!(sram_page_plan_inputs, 0);
        assert!(
            rom_window_plan_inputs > 0,
            "F-B10 stub has RomConst bindings to consume"
        );
        assert_eq!(arena_plan_inputs, result.bindings.len());
        assert!(result.repair_proposals.is_empty());
    }
}
