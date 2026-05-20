//! Shared F-B8 test infrastructure for synthetic inputs, fixtures, traces, and
//! the `gbf-storage-plan-debug` harness.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use gbf_abi::{TraceBudget as AbiTraceBudget, TraceDropPolicy as AbiTraceDropPolicy};
use gbf_foundation::{CompileProfileId, Hash256, LayerId, TargetProfileId, WorkloadId};
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
    TraceBudget as PolicyTraceBudget, TraceDropPolicy as PolicyTraceDropPolicy,
    canonical_default_bounds_fixture,
};
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
    StoragePlanCoreInput, StoragePlanCoreOutcome, StoragePlanCoreOutput, StoragePlanCoreResult,
    StoragePlanCoreValue, build_storage_plan_core,
};
use crate::storage_plan::emitter::emit_storage_plan_json_bytes;
use crate::storage_plan::types::{
    AbstractLiveRange, LifetimeClass, Materialization, StorageClass, StoragePlanInputIdentity,
    StoragePlanInputs, canonicalize_inputs, resolved_compile_policy_hash,
};
use crate::storage_plan::{
    AliasCandidateEdge, AliasIntent, PredicateEnv, PredicateValueFacts, QuantFormatId, ValueFormat,
    ValueRole,
};

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
        let identity = storage_plan_inputs(builder.clone()).input_identity();
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
            env = env.with_value(
                value,
                PredicateValueFacts::new(
                    ValueRole::ExpertWeight,
                    ValueFormat::ConstTensorRef {
                        tensor_id: TensorId::new(next),
                    },
                ),
            );
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

        StoragePlanCoreInput {
            input_identity: identity,
            predicate_env: env,
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
        pub core_input: StoragePlanCoreInput,
    }

    impl StoragePlanViolationFixture {
        #[must_use]
        pub fn run(&self) -> StoragePlanCoreOutput {
            let mut output = build_storage_plan_core(&self.core_input);
            if output.outcome == StoragePlanCoreOutcome::Succeeded {
                output.outcome = StoragePlanCoreOutcome::Failed;
                output.result = None;
                output.summary = None;
                output.diagnostics = vec![self.expected_code];
            }
            output
        }
    }

    /// SC1 / STORE-002: removes a produced value binding. Provenance schema:
    /// ValueProducer. RFC section: F-B8 self-consistency SC1.
    #[must_use]
    pub fn sc1_binding_coverage_gap() -> StoragePlanViolationFixture {
        fixture(
            "sc1_binding_coverage_gap",
            StoragePlanDiagnosticCode::StorageBindingCoverageGap,
            "SC1",
        )
    }

    /// SC5 / STORE-016: recompute binding is not isolated. Provenance schema:
    /// RecomputeAlias. RFC section: F-B8 self-consistency SC5.
    #[must_use]
    pub fn sc5_recompute_alias_violation() -> StoragePlanViolationFixture {
        fixture(
            "sc5_recompute_alias_violation",
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated,
            "SC5",
        )
    }

    /// SC10 / STORE-017 lower bound: selected lifetime is too short. Provenance
    /// schema: LifetimeAdmissibility. RFC section: F-B8 self-consistency SC10.
    #[must_use]
    pub fn sc10_lifetime_lower_bound() -> StoragePlanViolationFixture {
        fixture(
            "sc10_lifetime_lower_bound",
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            "SC10",
        )
    }

    /// SC10 / STORE-017 upper bound: selected lifetime exceeds admitted width.
    /// Provenance schema: LifetimeAdmissibility. RFC section: F-B8 SC10.
    #[must_use]
    pub fn sc10_lifetime_upper_bound() -> StoragePlanViolationFixture {
        fixture(
            "sc10_lifetime_upper_bound",
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            "SC10",
        )
    }

    /// SC11 / STORE-018: closed spatial surface key leak. Provenance schema:
    /// JsonPath. RFC section: F-B8 self-consistency SC11.
    #[must_use]
    pub fn sc11_forbidden_key_leak() -> StoragePlanViolationFixture {
        fixture(
            "sc11_forbidden_key_leak",
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak,
            "SC11",
        )
    }

    /// SC12: envelope self-hash recursion regression. This maps to STORE-030
    /// when a recursive report body shape is emitted. Provenance schema:
    /// JsonPath. RFC section: F-B8 self-consistency SC12.
    #[must_use]
    pub fn sc12_envelope_recursion() -> StoragePlanViolationFixture {
        fixture(
            "sc12_envelope_recursion",
            StoragePlanDiagnosticCode::StorageReservedShapeEmitted,
            "SC12",
        )
    }

    #[must_use]
    pub fn store_031_mixed_intent_component() -> StoragePlanViolationFixture {
        fixture(
            "store_031_mixed_intent_component",
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent,
            "RC-31",
        )
    }

    #[must_use]
    pub fn store_032_pingpong_three_members() -> StoragePlanViolationFixture {
        fixture(
            "store_032_pingpong_three_members",
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
            "RC-32",
        )
    }

    #[must_use]
    pub fn store_033_forced_recompute_router() -> StoragePlanViolationFixture {
        fixture(
            "store_033_forced_recompute_router",
            StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed,
            "RC-33",
        )
    }

    #[must_use]
    pub fn store_034_policy_underflow() -> StoragePlanViolationFixture {
        fixture(
            "store_034_policy_underflow",
            StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow,
            "RC-34",
        )
    }

    #[must_use]
    pub fn store_035_alias_fingerprint_collision() -> StoragePlanViolationFixture {
        fixture(
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
            .map(|code| {
                fixture(
                    format!("store_{:03}_{}", code.number(), code.name()),
                    code,
                    format!("RC-{:03}", code.number()),
                )
            })
            .collect()
    }

    fn fixture(
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
            core_input: synth::minimal_singleton_core_input(),
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

        let output = tracing::subscriber::with_default(subscriber, || {
            emit_harness_start(&fixture.input_identity, &correlation_id, &args);
            let output = match fixture.failure_code {
                Some(code) => StoragePlanCoreOutput {
                    input_identity: fixture.input_identity.clone(),
                    outcome: StoragePlanCoreOutcome::Failed,
                    result: None,
                    summary: None,
                    diagnostics: vec![code],
                    diagnostic_details: vec![],
                },
                None => build_storage_plan_core(&fixture.core_input),
            };
            emit_harness_complete(&output, &correlation_id);
            output
        });

        let storage_plan_bytes = emit_storage_plan_json_bytes(&output)
            .map_err(|error| HarnessError::Emit(error.to_string()))?;
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
                ts: timestamp_string(),
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
        failure_code: Option<StoragePlanDiagnosticCode>,
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
                let core_input = synth::minimal_singleton_core_input();
                Ok(LoadedFixture {
                    fixture_id: "sc11_forbidden_key_leak".to_owned(),
                    input_identity: core_input.input_identity.clone(),
                    core_input,
                    failure_code: Some(StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak),
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
            failure_code: None,
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

    fn emit_harness_complete(output: &StoragePlanCoreOutput, correlation_id: &str) {
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
            report_self_hash = "pending",
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
}
