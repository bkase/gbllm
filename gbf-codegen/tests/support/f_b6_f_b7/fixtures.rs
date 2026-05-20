use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use gbf_abi::{
    CURRENT_ABI, CheckpointEntry, CompactCheckpointId, ProbeLevel, SemanticCheckpointId,
    SemanticCheckpointSchema, SemanticStratum,
};
use gbf_codegen::budget::{
    AccumulatorBound, AccumulatorDomain, BudgetDecisionSection, BudgetIdentitySection,
    BudgetPolicySection, BudgetProjectionSection, PerBankEntry, ReductionSiteProjection,
    RuntimeChromeBudgetSection, StaticBudgetReport, StaticPlacementModel,
    runtime_chrome_budget_section_hash, static_fit_interpretation_for_fits,
};
use gbf_codegen::s1::quant_graph::{DeterminismClass, QuantFormat};
use gbf_codegen::s3::infer_ir::{
    CanonicalProvenanceTuple, GbInferIR, GbInferIRProduct, GbNode, InferIrAuditParents,
    InferIrIdentity, InferIrProvenance, InferOp, InferOpTag, NodeId, QuantGraphEntityRef,
    SemanticAnchor, TokenIngressMode, TokenInput, TokenInputId, ValueDecl, ValueFormat, ValueId,
    ValueKind, ValueLayout, ValueProducerRef,
};
use gbf_codegen::s4::observation_plan::{
    CompareDomain, LockedObservationKnobs, ObservationPlanAuditParents, ObservationPlanInputs,
    ObservationPolicyProjection, SemanticCheckpointKind, TraceDemotionLevel,
    WorkloadObservationProjection, semantic_checkpoint_kind_to_id,
};
use gbf_codegen::s5::range_plan::{
    LockedRangeKnobs, RangePlanAuditParents, RangePlanInputs, RangePolicyProjection,
};
use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId, WorkloadId};
use gbf_policy::{
    BudgetSlotClass, DEFAULT_COMPILE_PROFILE_ID, MetricRegistrySnapshot, ObservabilityMode,
    ObservationProfileCaps, PlacementProfile, ProbeImportanceClass, ProbeRegistrySnapshot,
    RangeCapsSpec, ReductionPlanCeiling, ReductionSiteId, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeMemoryCapSection, RuntimeMode, RuntimeNucleusHash, TraceBudget as PolicyTraceBudget,
    TraceDropPolicy as PolicyTraceDropPolicy, TraceEventLayoutRegistrySnapshot, TraceProbeId,
    metric_registry_hash, metric_registry_v1, probe_registry_hash, probe_registry_v1,
    trace_event_layout_registry_hash, trace_event_layout_registry_v1,
};
use gbf_report::report_schemas::infer_ir_v1::{
    FixtureEquivalenceSkippedReason, FixtureEquivalenceTag,
};
use gbf_report::{ReportEnvelope, ReportOutcome, canonicalize, canonicalize_value};
use gbf_workload::manifest as workload_manifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn ck_id(kind: SemanticCheckpointKind) -> SemanticCheckpointId {
    semantic_checkpoint_kind_to_id(kind)
}

pub const fn policy_probe_id(value: u16) -> gbf_policy::diagnostics::TraceProbeId {
    gbf_policy::diagnostics::TraceProbeId(value)
}

pub const fn abi_probe_id(value: u16) -> gbf_abi::trace::TraceProbeId {
    gbf_abi::trace::TraceProbeId(value)
}

pub const fn to_abi(
    probe_id: gbf_policy::diagnostics::TraceProbeId,
) -> gbf_abi::trace::TraceProbeId {
    gbf_abi::trace::TraceProbeId(probe_id.0)
}

pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let value = serde_json::to_value(value).expect("fixture serializes");
    canonicalize_value(&value).expect("fixture canonicalizes")
}

fn write_canonical_json<T: Serialize>(dir: &Path, file_name: &str, value: &T) -> PathBuf {
    fs::create_dir_all(dir).expect("fixture directory is created");
    let path = dir.join(file_name);
    fs::write(&path, canonical_json_bytes(value)).expect("fixture json is written");
    path
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn evidence(kind: &str, reference: &str) -> gbf_foundation::EvidenceRef {
    gbf_foundation::EvidenceRef {
        kind: kind.to_owned(),
        reference: reference.to_owned(),
        hash: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GbInferIRFixture {
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_policy_projection_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub requested_runtime_modes_hash: Hash256,
    pub determinism: DeterminismClass,
}

impl GbInferIRFixture {
    pub fn dense_default() -> Self {
        Self {
            quant_graph_self_hash: hash(0x21),
            infer_ir_policy_projection_hash: hash(0x22),
            static_budget_self_hash: hash(0x23),
            requested_runtime_modes_hash: hash(0x24),
            determinism: DeterminismClass::BitExact,
        }
    }

    pub fn build(&self) -> GbInferIR {
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
                format: ValueFormat::TokenIdDomain { vocab_size: 256 },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: embedding,
                kind: ValueKind::EmbeddingOutput,
                format: ValueFormat::Quant {
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
        let anchor = SemanticAnchor::new(hash(0x25));
        let anchors = BTreeMap::from([(node, anchor)]);

        GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: self.quant_graph_self_hash,
                infer_ir_policy_projection_hash: self.infer_ir_policy_projection_hash,
                static_budget_self_hash: self.static_budget_self_hash,
                requested_runtime_modes_hash: self.requested_runtime_modes_hash,
                determinism: self.determinism,
                topological_order_hash: hash(0x26),
            },
            vec![token_input],
            nodes,
            values,
            Vec::new(),
            provenance,
            anchors,
        )
        .expect("infer_ir fixture is valid")
    }

    pub fn build_product(&self) -> GbInferIRProduct {
        GbInferIRProduct::new(
            self.build(),
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x27),
                compile_request_hash: hash(0x28),
            },
            BTreeSet::from([RuntimeMode::Safe]),
            FixtureEquivalenceTag::Skipped {
                reason: FixtureEquivalenceSkippedReason::FeatureFlagDisabled,
            },
        )
        .expect("infer_ir product fixture is valid")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationPlanInputsFixture {
    inputs: ObservationPlanInputs,
}

impl ObservationPlanInputsFixture {
    #[allow(dead_code)]
    pub fn dense_default() -> Self {
        let infer_ir_product = GbInferIRFixture::dense_default().build_product();
        let static_budget_self_hash = infer_ir_product.infer_ir.identity.static_budget_self_hash;
        let semantic_checkpoint_schema = semantic_checkpoint_schema();
        let probe_registry = ProbeRegistrySnapshotFixture::default().build();
        let metric_registry = MetricRegistrySnapshotFixture::default().build();
        let trace_event_layout_registry =
            TraceEventLayoutRegistrySnapshotFixture::default().build();

        Self {
            inputs: ObservationPlanInputs {
                infer_ir_self_hash: infer_ir_product.infer_ir_self_hash,
                quant_graph_self_hash: infer_ir_product.infer_ir.identity.quant_graph_self_hash,
                infer_ir_product,
                semantic_checkpoint_schema,
                semantic_checkpoint_schema_hash: hash(0x31),
                artifact_declared_semantic_checkpoint_schema_hash: hash(0x31),
                probe_registry_hash: probe_registry_hash(&probe_registry)
                    .expect("probe registry hashes"),
                probe_registry,
                metric_registry_hash: metric_registry_hash(&metric_registry)
                    .expect("metric registry hashes"),
                metric_registry,
                trace_event_layout_registry_hash: trace_event_layout_registry_hash(
                    &trace_event_layout_registry,
                )
                .expect("trace layout registry hashes"),
                trace_event_layout_registry,
                op_policy_projection: ObservationPolicyProjection {
                    profile_id: CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID),
                    profile_observation_caps: ObservationProfileCaps::default_v2(),
                    determinism_class: DeterminismClass::BitExact,
                    observability_mode: ObservabilityMode::Flexible,
                    trace_budget: PolicyTraceBudget {
                        max_events_per_slice: 8,
                        max_bytes_per_frame: 512,
                        drop_policy: PolicyTraceDropPolicy::HaltAndFault,
                    },
                    trace_demotion: TraceDemotionLevel::None,
                    optional_probe_floor: ProbeImportanceClass::BestEffort,
                    workload_observation: default_workload_observation("dense_default"),
                    disabled_optional_probes: BTreeSet::new(),
                },
                audit_parents: ObservationPlanAuditParents {
                    policy_resolution_self_hash: hash(0x32),
                    compile_request_hash: hash(0x33),
                    static_budget_self_hash,
                    artifact_aux_hash: hash(0x35),
                    locked_observation_knobs: LockedObservationKnobs {
                        trace_demotion_locked: false,
                        optional_probe_floor_locked: false,
                        probe_selection_locked: false,
                    },
                },
            },
        }
    }

    #[allow(dead_code)]
    pub fn with_workload(mut self, workload: WorkloadObservationProjection) -> Self {
        self.inputs.op_policy_projection.workload_observation = workload;
        self
    }

    #[allow(dead_code)]
    pub fn with_observability_mode(mut self, mode: ObservabilityMode) -> Self {
        self.inputs.op_policy_projection.observability_mode = mode;
        self
    }

    #[allow(dead_code)]
    pub fn with_disabled_probe(mut self, probe: TraceProbeId) -> Self {
        self.inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(probe);
        self
    }

    #[allow(dead_code)]
    pub fn with_probe_floor(mut self, floor: ProbeImportanceClass) -> Self {
        self.inputs.op_policy_projection.optional_probe_floor = floor;
        self
    }

    pub fn build(self) -> ObservationPlanInputs {
        self.inputs
    }

    #[allow(dead_code)]
    pub fn expect_emit(self, dir: &Path) -> PathBuf {
        self.write_to(dir)
    }

    pub fn write_to(self, dir: &Path) -> PathBuf {
        write_canonical_json(dir, "inputs.json", &self.inputs)
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_json_bytes(&self.inputs)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangePlanInputsFixture {
    inputs: RangePlanInputs,
}

impl RangePlanInputsFixture {
    pub fn chunked_i16() -> Self {
        let infer_ir_product = GbInferIRFixture::dense_default().build_product();
        let static_budget_report =
            StaticBudgetReductionSiteFactsFixture::single_i16().build_report();
        let static_budget_self_hash = static_budget_report.static_budget_self_hash;

        Self {
            inputs: RangePlanInputs {
                infer_ir_self_hash: infer_ir_product.infer_ir_self_hash,
                quant_graph_self_hash: infer_ir_product.infer_ir.identity.quant_graph_self_hash,
                infer_ir_product,
                static_budget_report,
                static_budget_self_hash,
                range_policy_projection: RangePolicyProjection {
                    profile_id: CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID),
                    range_caps: RangeCapsSpec::default_v2(),
                    reduction_ceiling: ReductionPlanCeiling::Conservative,
                    reduction_ceiling_overrides: BTreeMap::new(),
                    determinism_class: DeterminismClass::BitExact,
                },
                audit_parents: RangePlanAuditParents {
                    policy_resolution_self_hash: hash(0x41),
                    compile_request_hash: hash(0x42),
                    artifact_aux_hash: hash(0x43),
                    locked_range_knobs: LockedRangeKnobs {
                        reduction_ceiling_locked: false,
                    },
                },
            },
        }
    }

    pub fn build(self) -> RangePlanInputs {
        self.inputs
    }

    #[allow(dead_code)]
    pub fn expect_emit(self, dir: &Path) -> PathBuf {
        self.write_to(dir)
    }

    pub fn write_to(self, dir: &Path) -> PathBuf {
        write_canonical_json(dir, "inputs.json", &self.inputs)
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_json_bytes(&self.inputs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticBudgetReductionSiteFactsFixture {
    sites: Vec<ReductionSiteProjection>,
}

impl StaticBudgetReductionSiteFactsFixture {
    pub fn single_i16() -> Self {
        Self {
            sites: vec![ReductionSiteProjection {
                site: ReductionSiteId("dense.matmul.0".to_owned()),
                layer: Some(gbf_foundation::LayerId::new(0)),
                expert: None,
                term_count: 4,
                input_max_abs_q: 32,
                weight_max_abs_q: 16,
                bias_max_abs_q: Some(3),
                accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            }],
        }
    }

    pub fn build_facts(&self) -> Vec<ReductionSiteProjection> {
        self.sites.clone()
    }

    pub fn build_report(&self) -> StaticBudgetReport {
        let runtime_budget = RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID),
            runtime_nucleus_hash: RuntimeNucleusHash::real(hash(0x50)),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x51),
            },
            wram_reserved: 128,
            sram_reserved: 512,
        };
        let runtime_budget_section = RuntimeChromeBudgetSection::from(&runtime_budget);
        let mut projections = BudgetProjectionSection::default();
        projections.routing_model.kind = "synthetic-f-b6-f-b7".to_owned();
        projections.per_bank_occupancy = vec![PerBankEntry {
            slot: BudgetSlotId::new(0),
            class: BudgetSlotClass::CommonBank,
            usable_bytes: 16 * 1024,
            reserved_slack: 128,
            effective_cap_bytes: i64::from(16 * 1024 - 128),
            assigned_bytes: 0,
            residual_bytes: 16 * 1024 - 128,
            assigned_components: Vec::new(),
            placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
        }];
        projections.accumulator_maxima = self
            .sites
            .iter()
            .map(|site| {
                let product = u64::from(site.input_max_abs_q) * u64::from(site.weight_max_abs_q);
                let bias = u64::from(site.bias_max_abs_q.unwrap_or(0));
                AccumulatorBound {
                    site: site.site.clone(),
                    projected_max_abs: product * u64::from(site.term_count) + bias,
                    i16_safe: true,
                    i32_safe: true,
                }
            })
            .collect();
        let placement_profile = PlacementProfile::Budgeted;
        let body = gbf_codegen::budget::StaticBudgetReportBody {
            identity: BudgetIdentitySection {
                artifact_core_hash: hash(0x52),
                quant_graph_hash: hash(0x21),
                policy_resolution_self_hash: hash(0x53),
                runtime_chrome_budget_hash: Some(
                    runtime_chrome_budget_section_hash(&runtime_budget_section)
                        .expect("runtime budget hashes"),
                ),
                target_profile_hash: hash(0x54),
            },
            policy: BudgetPolicySection {
                placement_profile,
                objective_hash: hash(0x55),
            },
            runtime_chrome_budget: Some(runtime_budget_section),
            projections,
            decision: BudgetDecisionSection {
                fits: true,
                interpretation: static_fit_interpretation_for_fits(true),
                placement_model: StaticPlacementModel::for_profile(placement_profile),
                failures: Vec::new(),
            },
            diagnostics: Vec::new(),
        };
        let report = ReportEnvelope::new(ReportOutcome::Passed, body)
            .expect("static budget envelope validates")
            .with_computed_self_hash()
            .expect("static budget self hash computes");
        let canonical_bytes = canonicalize(&report).expect("static budget canonicalizes");
        StaticBudgetReport {
            static_budget_self_hash: report.report_self_hash,
            static_budget_canonical_bytes_hash: Hash256::from_bytes(
                Sha256::digest(&canonical_bytes).into(),
            ),
            report,
            reduction_site_facts: self.build_facts(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeRegistrySnapshotFixture;

impl ProbeRegistrySnapshotFixture {
    pub fn build(&self) -> ProbeRegistrySnapshot {
        probe_registry_v1()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricRegistrySnapshotFixture;

impl MetricRegistrySnapshotFixture {
    pub fn build(&self) -> MetricRegistrySnapshot {
        metric_registry_v1()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEventLayoutRegistrySnapshotFixture;

impl TraceEventLayoutRegistrySnapshotFixture {
    pub fn build(&self) -> TraceEventLayoutRegistrySnapshot {
        trace_event_layout_registry_v1()
    }
}

fn semantic_checkpoint_schema() -> SemanticCheckpointSchema {
    SemanticCheckpointSchema {
        schema_version: 1,
        abi_version: CURRENT_ABI,
        build_hash: [0x61; 32],
        compile_request_hash: [0x62; 32],
        checkpoints: vec![CheckpointEntry {
            semantic: ck_id(SemanticCheckpointKind::PostEmbedding {
                layer: gbf_foundation::LayerId::new(0),
            }),
            compact: CompactCheckpointId(1),
            stratum: SemanticStratum::Denotation,
            source_op: Some(Cow::Borrowed("embedding")),
        }],
    }
}

pub fn default_workload_observation(slug: &str) -> WorkloadObservationProjection {
    let compare_domain_workload = workload_manifest::CompareDomain::TokenLogits;
    let determinism_requirement = workload_manifest::DeterminismRequirement::SeededDecode;
    WorkloadObservationProjection {
        workload_id: WorkloadId::from(format!("f_b6_f_b7.{slug}")),
        checkpoint_selection: workload_manifest::CheckpointSelection::SemanticAndOperational,
        trace_level: workload_manifest::TraceLevel::Checkpoints,
        compare_domain_workload,
        compare_domain_policy: CompareDomain::from(compare_domain_workload),
        determinism_requirement,
        determinism_class_v1: DeterminismClass::from(determinism_requirement),
    }
}

pub fn build_stage4_inputs_for(ir: &GbInferIRFixture) -> ObservationPlanInputs {
    let mut fixture = ObservationPlanInputsFixture::dense_default();
    let product = ir.build_product();
    fixture.inputs.quant_graph_self_hash = product.infer_ir.identity.quant_graph_self_hash;
    fixture.inputs.infer_ir_self_hash = product.infer_ir_self_hash;
    fixture.inputs.infer_ir_product = product;
    fixture.build()
}

pub fn build_stage5_inputs_for(
    ir: &GbInferIRFixture,
    static_budget_report: &StaticBudgetReport,
) -> RangePlanInputs {
    let mut fixture = RangePlanInputsFixture::chunked_i16();
    let product = ir.build_product();
    fixture.inputs.quant_graph_self_hash = product.infer_ir.identity.quant_graph_self_hash;
    fixture.inputs.infer_ir_self_hash = product.infer_ir_self_hash;
    fixture.inputs.infer_ir_product = product;
    fixture.inputs.static_budget_self_hash = static_budget_report.static_budget_self_hash;
    fixture.inputs.static_budget_report = static_budget_report.clone();
    fixture.build()
}

#[allow(dead_code)]
fn _assert_probe_bridge_shape() {
    let policy = policy_probe_id(7);
    let abi = abi_probe_id(7);
    assert_eq!(to_abi(policy), abi);
    let _ = ProbeLevel::Always;
    let _ = CanonicalProvenanceTuple::new(InferOpTag::Embedding, 0);
    let _ = evidence("fixture", "f_b6_f_b7");
}
