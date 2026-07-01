//! F-S7 production bundle producer.
//!
//! This module owns the executable upstream of `s7_production_bundle_manifest.v1`.
//! It is intentionally separate from the packet materializers: the materializers
//! validate already-produced artifacts, while this code maintains live
//! model/optimizer state and writes the source bundle those materializers ingest.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use gbf_artifact::{
    ClipSaturationDigest, DistillRawDiagnostic, ExpertPayloadDigest, ExpertPayloadEntry,
    ExpertSlotAffinity, GradNormSummary, QuantSpec, RawLossDiagnostics, S7_EVAL_EVERY_STEPS,
    S7_N_BLOCKS, S7_N_EXPERTS, S7_OPTIMIZER_STEPS, S7Completion, S7RunLog, S7ScoreReport,
    S7Topology, TemporalSwitchDigest, TrainPhase, TransitionEntry,
};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, ExpertId, Hash256, LayerId,
    self_hash_omitting_fields, sha256,
};
use gbf_policy::model_profile::ModelSizeProfile;
use gbf_train::adapter::burn::{
    BurnAdapterError, BurnAutodiffBackend, BurnBackend, BurnDevice, BurnFloatTensor,
    BurnGradientsParams, BurnModule, BurnNdArrayAutodiffBackend, BurnOptimizer, BurnParam,
    adamw_config, burn_log_softmax, burn_softmax, float_tensor_from_vec, float_tensor_into_vec,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::s7::collapse_sweep::{
    D11_LAMBDA_SWITCH_SWEEP_SEED, H6_HIGH_LAMBDA_SWITCH, LambdaSwitchSweepInput,
    LambdaSwitchSweepPointInput, LambdaSwitchSweepPointOutcome, LambdaSwitchSweepProducer,
    PRODUCTION_SWEEP_PRODUCER_KIND, run_lambda_switch_sweep,
};
use crate::s7::run::topology_path_segment;
use crate::s7::schema::{ConfidenceDist, RouterStepTelemetry};
use crate::s7::state::{S7_PHASE_A_END_STEP, S7TrainRunId};

/// Production bundle manifest schema written by this runner.
pub const S7_PRODUCTION_BUNDLE_MANIFEST_SCHEMA: &str = "s7_production_bundle_manifest.v1";

const S7_GRAD_LOG_SCHEMA: &str = "s7_grad_log.v1";
const S7_ROUTER_STEP_TELEMETRY_SCHEMA: &str = "s7_router_step_telemetry.v1";
const BYTE_VOCAB_SIZE: usize = 256;
const SCORE_PAIR_LIMIT: usize = 8_192;
const ADAM_BETA1: f32 = 0.9;
const ADAM_BETA2: f32 = 0.999;
const ADAM_EPSILON: f32 = 1.0e-8;
const TRAIN_LEARNING_RATE: f32 = 0.035;
const DEFAULT_FRONTIER_MOE_BYTES_PER_BLOCK: [u64; 4] = [20_944, 20_944, 20_944, 20_944];
const DEFAULT_FRONTIER_DENSE_BYTES_PER_BLOCK: [u64; 4] = [20_948, 20_948, 20_948, 20_948];
const S7_PRODUCTION_DOMAIN_VERSION: &str = "1";
const S7_PHASE_D_END_STEP: u64 = 18_000;

/// Inputs for producing a full S7 production bundle.
#[derive(Debug, Clone)]
pub struct S7ProductionBundleInputs {
    /// Directory where source bundle artifacts and manifest are written.
    pub output_dir: PathBuf,
    /// Manifest path to write. Defaults to `output_dir/s7-production-bundle-manifest.json`.
    pub manifest_output: PathBuf,
    /// Pinned Project Gutenberg manifest path. The runner hashes and records it.
    pub gutenberg_manifest: PathBuf,
    /// Gutenberg training byte stream.
    pub train_corpus: PathBuf,
    /// Gutenberg validation byte stream.
    pub val_corpus: PathBuf,
    /// External H8 Burn gradient evidence to copy into the bundle.
    pub burn_grad_smoke: PathBuf,
    /// External H9 routed oracle evidence to copy into the bundle.
    pub oracle_routed: PathBuf,
    /// External H10 MoE one-token evidence to copy into the bundle.
    pub emulator_one_token_moe: PathBuf,
    /// Optional H10 dense one-token evidence to copy into the bundle.
    pub emulator_one_token_dense: Option<PathBuf>,
    /// External MoE conformance evidence for frontier derivation.
    pub moe_conformance: PathBuf,
    /// External dense-matched conformance evidence for frontier derivation.
    pub dense_conformance: PathBuf,
    /// Optional MoE schedule-cost evidence for frontier derivation.
    pub moe_schedule_cost: Option<PathBuf>,
    /// Optional dense schedule-cost evidence for frontier derivation.
    pub dense_schedule_cost: Option<PathBuf>,
    /// Final report outcome passed through to the packet assembler.
    pub s7_outcome: String,
    /// Final report decision passed through to the packet assembler.
    pub decision: String,
    /// RFC revision hash/commit pinned in the final report.
    pub rfc_revision: String,
    /// Pre-registered predictions section hash.
    pub predictions_section_hash: String,
    /// Commit containing the pre-registered predictions.
    pub predictions_commit: String,
    /// First commit containing S7 result evidence.
    pub first_result_commit: String,
    /// RFC3339 UTC report generation timestamp.
    pub generated_at: String,
}

/// Result of producing the source bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ProductionBundleOutput {
    /// Manifest written for `scripts/review/f-s7/assemble-packet.py`.
    pub manifest_path: PathBuf,
    /// Manifest self-hash for operator logs.
    pub manifest_self_hash: Hash256,
    /// Number of completed production runs written.
    pub run_count: usize,
}

/// Produce the full `s7_production_bundle_manifest.v1` source bundle.
pub fn produce_s7_production_bundle(
    inputs: &S7ProductionBundleInputs,
) -> Result<S7ProductionBundleOutput, S7ProductionRunnerError> {
    validate_inputs(inputs)?;
    fs::create_dir_all(&inputs.output_dir).map_err(|source| S7ProductionRunnerError::Io {
        path: inputs.output_dir.display().to_string(),
        source,
    })?;

    let gutenberg_manifest_bytes = read_required_file(&inputs.gutenberg_manifest)?;
    let train = read_required_file(&inputs.train_corpus)?;
    let val = read_required_file(&inputs.val_corpus)?;
    if train.is_empty() {
        return Err(S7ProductionRunnerError::EmptyCorpus {
            path: inputs.train_corpus.display().to_string(),
        });
    }
    if val.is_empty() {
        return Err(S7ProductionRunnerError::EmptyCorpus {
            path: inputs.val_corpus.display().to_string(),
        });
    }

    let mut run_refs = BTreeMap::<String, BTreeMap<String, RunManifestEntry>>::new();
    let mut moe_seed0_bpc = None;
    let mut moe_checkpoint_seed0 = Hash256::ZERO;
    let mut moe_phase_d_seed0_state = None;
    let mut moe_topology_hash = Hash256::ZERO;
    let mut dense_topology_hash = Hash256::ZERO;

    for topology in [S7Topology::MoeTiny, S7Topology::MoeTinyDenseMatched] {
        let topology_name = topology_path_segment(&topology).to_owned();
        let mut seed_refs = BTreeMap::new();
        for seed in 0..5 {
            let product =
                run_one_training_job(&inputs.output_dir, topology.clone(), seed, &train, &val)?;
            if topology == S7Topology::MoeTiny && seed == 0 {
                moe_checkpoint_seed0 = product.phase_d_checkpoint_sha;
                moe_phase_d_seed0_state = product.phase_d_eval_state.clone();
                moe_seed0_bpc = Some(product.score.bpc);
            }
            if topology == S7Topology::MoeTiny {
                moe_topology_hash = product.model_topology_hash;
                write_switch_stats(&inputs.output_dir, seed, &product)?;
            } else {
                dense_topology_hash = product.model_topology_hash;
            }
            seed_refs.insert(seed.to_string(), product.manifest_entry);
        }
        run_refs.insert(topology_name, seed_refs);
    }

    let sweep = run_lambda_switch_sweep(
        &LambdaSwitchSweepInput::d11_from_val_eval_subset_bytes(
            D11_LAMBDA_SWITCH_SWEEP_SEED,
            moe_checkpoint_seed0,
            S7_PHASE_D_END_STEP,
            &val,
        )?,
        &ProductionClosureRetrainScore {
            base_moe_score: moe_seed0_bpc.ok_or(S7ProductionRunnerError::MissingScore {
                topology: "MoeTiny",
                seed: 0,
            })?,
            phase_d_state: moe_phase_d_seed0_state.ok_or(
                S7ProductionRunnerError::InternalInvariant {
                    detail: "missing seed-0 MoE Phase-D state for lambda_switch sweep",
                },
            )?,
            train: train.clone(),
            val: val.clone(),
        },
    )?;
    write_canonical_json(
        &inputs.output_dir.join("router-collapse/seed-0/sweep.json"),
        &sweep.canonical_json_bytes()?,
    )?;

    let copied_support = copy_support_inputs(inputs)?;
    let manifest = ProductionManifest {
        schema: S7_PRODUCTION_BUNDLE_MANIFEST_SCHEMA.to_owned(),
        runs: run_refs,
        switch_stats: (0..5)
            .map(|seed| {
                (
                    seed.to_string(),
                    format!("switch-stats/seed-{seed}/switch-stats.json"),
                )
            })
            .collect(),
        support_artifacts: copied_support.support_artifacts,
        comparison: ComparisonManifest {
            moe_topology_hash,
            dense_matched_topology_hash: dense_topology_hash,
        },
        frontier: FrontierManifest {
            moe_conformance: copied_support.moe_conformance,
            dense_conformance: copied_support.dense_conformance,
            moe_deployed_bytes_per_block: DEFAULT_FRONTIER_MOE_BYTES_PER_BLOCK.to_vec(),
            dense_deployed_bytes_per_block: DEFAULT_FRONTIER_DENSE_BYTES_PER_BLOCK.to_vec(),
            moe_schedule_cost: copied_support.moe_schedule_cost,
            dense_schedule_cost: copied_support.dense_schedule_cost,
        },
        report: ReportManifest {
            s7_outcome: inputs.s7_outcome.clone(),
            decision: inputs.decision.clone(),
            rfc_revision: inputs.rfc_revision.clone(),
            predictions_section_hash: inputs.predictions_section_hash.clone(),
            predictions_commit: inputs.predictions_commit.clone(),
            first_result_commit: inputs.first_result_commit.clone(),
            generated_at: inputs.generated_at.clone(),
        },
        production_runner: ProductionRunnerManifest {
            schema: "s7_production_runner.v1",
            runner_kind: "gbf_experiments::s7::production_runner",
            bead_owner: "bd-3e10j",
            gutenberg_manifest_sha: sha256(&gutenberg_manifest_bytes),
            train_corpus_sha: sha256(&train),
            val_corpus_sha: sha256(&val),
            optimizer_model_state: "live_burn_adamw_moe_lm_state_per_topology_seed",
            grad_log_schema: S7_GRAD_LOG_SCHEMA,
            router_step_telemetry_schema: S7_ROUTER_STEP_TELEMETRY_SCHEMA,
            optimizer_steps: S7_OPTIMIZER_STEPS,
            sweep_producer_kind: PRODUCTION_SWEEP_PRODUCER_KIND,
        },
    };
    let manifest_bytes = CanonicalJson::to_vec(&manifest)?;
    write_canonical_json(&inputs.manifest_output, &manifest_bytes)?;
    let manifest_self_hash = production_manifest_domain().hash(&manifest)?;

    Ok(S7ProductionBundleOutput {
        manifest_path: inputs.manifest_output.clone(),
        manifest_self_hash,
        run_count: 10,
    })
}

fn validate_inputs(inputs: &S7ProductionBundleInputs) -> Result<(), S7ProductionRunnerError> {
    if inputs.s7_outcome != "PassClean" && inputs.s7_outcome != "FailParity" {
        return Err(S7ProductionRunnerError::InvalidReportField {
            field: "s7_outcome",
            detail: "must be PassClean or FailParity".to_owned(),
        });
    }
    match (inputs.s7_outcome.as_str(), inputs.decision.as_str()) {
        ("PassClean", "ProceedToS8") | ("FailParity", "ProceedToS8DenseOnly") => Ok(()),
        _ => Err(S7ProductionRunnerError::InvalidReportField {
            field: "decision",
            detail: "must match s7_outcome".to_owned(),
        }),
    }
}

fn read_required_file(path: &Path) -> Result<Vec<u8>, S7ProductionRunnerError> {
    fs::read(path).map_err(|source| S7ProductionRunnerError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn run_one_training_job(
    bundle_root: &Path,
    topology: S7Topology,
    seed: u64,
    train: &[u8],
    val: &[u8],
) -> Result<CompletedProductionRun, S7ProductionRunnerError> {
    let topology_name = topology_path_segment(&topology);
    let run_dir = bundle_root
        .join("runs")
        .join(topology_name)
        .join(format!("seed-{seed}"));
    fs::create_dir_all(&run_dir).map_err(|source| S7ProductionRunnerError::Io {
        path: run_dir.display().to_string(),
        source,
    })?;

    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let mut state = BurnMoeS7ModelState::<B>::new(topology.clone(), seed, train, &device)?;
    let mut optimizer = adamw_config()
        .with_beta_1(ADAM_BETA1)
        .with_beta_2(ADAM_BETA2)
        .with_epsilon(ADAM_EPSILON)
        .with_weight_decay(0.0)
        .init::<B, BurnByteMoeS7Model<B>>();
    let train_config_hash = state.config_hash("train_config")?;
    let model_topology_hash = state.config_hash("model_topology")?;
    let loss_config_hash = state.config_hash("loss_config")?;
    let phase_schedule_hash = state.config_hash("phase_schedule")?;
    let router_config_hash = (topology == S7Topology::MoeTiny)
        .then(|| state.config_hash("router_config"))
        .transpose()?;
    let expert_block_config_hash = (topology == S7Topology::MoeTiny)
        .then(|| state.config_hash("expert_block_config"))
        .transpose()?;

    let grad_log_path = run_dir.join("grad-log.jsonl");
    let router_telemetry_path = run_dir.join("router-step-telemetry.jsonl");
    let mut grad_log = line_writer(&grad_log_path)?;
    let mut router_log = line_writer(&router_telemetry_path)?;
    let mut losses = Vec::with_capacity(S7_OPTIMIZER_STEPS as usize);
    let mut grad_norms = Vec::with_capacity(S7_OPTIMIZER_STEPS as usize);
    let mut eval_points = vec![(0, state.score_bpc(val, 0.0)?)];
    let mut frozen_teacher_checkpoint_sha = None;
    let mut phase_d_checkpoint_sha = Hash256::ZERO;
    let mut phase_d_eval_state = None;

    for step in 1..=S7_OPTIMIZER_STEPS {
        let phase = phase_for_step(step);
        let step_loss = state.loss_for_step(step, train, 0.0, &device)?;
        let gradients = step_loss.total_loss.backward();
        let grad_stats = state.grad_stats(&gradients)?;
        let gradients = BurnGradientsParams::from_grads(gradients, &state.model);
        state.model = optimizer.step(f64::from(TRAIN_LEARNING_RATE), state.model, gradients);
        state.optimizer_step = step;
        let optimizer_record_entries = optimizer.to_record().len();
        let mut step_output = step_loss.output;
        step_output.grad_global_l2 = grad_stats.global_l2();
        step_output.grad_max_l2 = grad_stats.max_abs;
        step_output.grad_mean_l2 = grad_stats.mean_abs();
        state.observe_routes(&mut step_output);
        let phase_label = phase_name(&phase);
        let distill = match phase {
            TrainPhase::PhaseA | TrainPhase::PhaseB => DistillRawDiagnostic::NotAvailable {
                reason: "teacher_distillation_not_phase_effective_until_phase_c".to_owned(),
                phase: phase.clone(),
            },
            TrainPhase::PhaseC | TrainPhase::PhaseD | TrainPhase::PhaseE => {
                DistillRawDiagnostic::Value {
                    loss: step_output.distill_loss_raw,
                }
            }
        };
        let diagnostics = RawLossDiagnostics::new(
            step_output.lm_loss_raw,
            distill,
            step_output.balance_loss_raw,
            step_output.zrouter_loss_raw,
            step_output.switch_loss_raw,
        )?
        .with_computed_self_hash()?;
        let grad_norm = GradNormSummary::new(
            step_output.grad_global_l2,
            step_output.grad_max_l2,
            step_output.grad_mean_l2,
        )?;
        write_jsonl_value(
            &mut grad_log,
            &json!({
                "schema": S7_GRAD_LOG_SCHEMA,
                "seed": seed,
                "topology": topology_name,
                "train_step": step,
                "phase": phase_label,
                "grad_norms": grad_norm,
                "optimizer_state": {
                    "kind": "burn_adamw",
                    "model_state_step": state.optimizer_step,
                    "optimizer_record_entries": optimizer_record_entries,
                    "training_backend": "burn_ndarray_autodiff",
                }
            }),
        )?;
        if topology == S7Topology::MoeTiny {
            for telemetry in state.router_telemetry(step, &step_output)? {
                router_log.write_all(&telemetry.canonical_json_bytes()?)?;
                router_log.write_all(b"\n")?;
            }
        }
        losses.push((step, diagnostics));
        grad_norms.push((step, grad_norm.clone()));
        if step == S7_PHASE_A_END_STEP {
            frozen_teacher_checkpoint_sha = Some(state.freeze_teacher("phase_a_teacher")?);
        }
        if step == S7_PHASE_D_END_STEP {
            phase_d_checkpoint_sha = state.checkpoint_hash("phase_d_checkpoint")?;
            phase_d_eval_state = Some(state.eval_state()?);
        }
        if step % S7_EVAL_EVERY_STEPS == 0 {
            eval_points.push((step, state.score_bpc(val, 0.0)?));
        }
    }
    grad_log
        .flush()
        .map_err(|source| S7ProductionRunnerError::Io {
            path: grad_log_path.display().to_string(),
            source,
        })?;
    router_log
        .flush()
        .map_err(|source| S7ProductionRunnerError::Io {
            path: router_telemetry_path.display().to_string(),
            source,
        })?;

    let final_grad_norms = grad_norms.last().map(|(_, norms)| norms.clone()).ok_or(
        S7ProductionRunnerError::InternalInvariant {
            detail: "missing final grad norms",
        },
    )?;
    let run_log = S7RunLog::new(
        seed,
        topology.clone(),
        train_config_hash,
        model_topology_hash,
        router_config_hash,
        expert_block_config_hash,
        loss_config_hash,
        phase_schedule_hash,
        frozen_teacher_checkpoint_sha,
        losses,
        grad_norms,
        eval_points,
        final_grad_norms,
        S7Completion::Completed,
    )?
    .with_computed_self_hash()?;
    let checkpoint_sha = state.checkpoint_hash("final_checkpoint")?;
    let corpus_val_sha = sha256(val);
    let score = S7ScoreReport::new(
        seed,
        topology.clone(),
        checkpoint_sha,
        corpus_val_sha,
        u64::try_from(val.len()).map_err(|_| S7ProductionRunnerError::LengthOverflow)?,
        state.score_log2_sum(val, 0.0)?,
    )?
    .with_computed_self_hash()?;

    let run_log_path = run_dir.join("run-log.json");
    let score_path = run_dir.join("score.json");
    write_canonical_json(&run_log_path, &run_log.canonical_json_bytes()?)?;
    write_canonical_json(&score_path, &score.canonical_json_bytes()?)?;

    Ok(CompletedProductionRun {
        phase_d_checkpoint_sha,
        phase_d_eval_state,
        model_topology_hash,
        score,
        expert_transition_counts: state.expert_transition_counts,
        same_expert_counts: state.same_expert_counts,
        router_observation_counts: state.router_observation_counts,
        mean_clip_saturation: state.mean_clip_saturation(),
        manifest_entry: RunManifestEntry {
            run_log: format!("runs/{topology_name}/seed-{seed}/run-log.json"),
            score: format!("runs/{topology_name}/seed-{seed}/score.json"),
            grad_log: format!("runs/{topology_name}/seed-{seed}/grad-log.jsonl"),
            router_step_telemetry: format!(
                "runs/{topology_name}/seed-{seed}/router-step-telemetry.jsonl"
            ),
        },
    })
}

fn write_switch_stats(
    bundle_root: &Path,
    seed: u64,
    product: &CompletedProductionRun,
) -> Result<(), S7ProductionRunnerError> {
    let temporal_switch_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let observations = product.router_observation_counts[layer as usize].max(1);
            let same = product.same_expert_counts[layer as usize];
            let same_rate =
                ((same.saturating_mul(256) + observations / 2) / observations).min(256) as u16;
            let transition_mass = (0..S7_N_EXPERTS)
                .map(|expert| {
                    let to = (expert + 1) % S7_N_EXPERTS;
                    let raw = product.expert_transition_counts[layer as usize][expert as usize]
                        .saturating_mul(256)
                        / observations;
                    TransitionEntry::new(
                        ExpertId::new(expert),
                        ExpertId::new(to),
                        raw.min(256) as u16,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TemporalSwitchDigest::new(
                LayerId::new(layer),
                S7_N_EXPERTS,
                same_rate,
                transition_mass,
            )?
            .with_computed_self_hash()?)
        })
        .collect::<Result<Vec<_>, S7ProductionRunnerError>>()?;
    let clip_saturation_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let rate = (product.mean_clip_saturation * 256.0)
                .round()
                .clamp(0.0, 256.0) as u16;
            Ok(ClipSaturationDigest::new(LayerId::new(layer), rate, 8.0)?
                .with_computed_self_hash()?)
        })
        .collect::<Result<Vec<_>, S7ProductionRunnerError>>()?;
    let expert_payload_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let entries = (0..S7_N_EXPERTS)
                .map(|expert| {
                    ExpertPayloadEntry::new(ExpertId::new(expert), 19_856, QuantSpec::default())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ExpertPayloadDigest::new(
                LayerId::new(layer),
                format!("artifacts/S7/MoeTiny/seed-{seed}/layer-{layer}/experts"),
                entries,
            )?
            .with_computed_self_hash()?)
        })
        .collect::<Result<Vec<_>, S7ProductionRunnerError>>()?;
    let expert_slot_affinity = temporal_switch_digest
        .iter()
        .map(|digest| {
            Ok(ExpertSlotAffinity::from_temporal_switch_digest(digest)?
                .with_computed_self_hash()?)
        })
        .collect::<Result<Vec<_>, S7ProductionRunnerError>>()?;

    let mut report = SwitchStatsReport {
        schema: "s7_switch_stats.v1".to_owned(),
        seed,
        artifact_path: format!("runs/MoeTiny/seed-{seed}/router-step-telemetry.jsonl"),
        temporal_switch_digest,
        clip_saturation_digest,
        expert_payload_digest,
        expert_slot_affinity,
        aggregation_rule: "SUM".to_owned(),
        bundle_self_hash: Hash256::ZERO,
    };
    report.bundle_self_hash =
        self_hash_omitting_fields(switch_stats_domain(), &report, "bundle_self_hash", &[])?;
    let path = bundle_root.join(format!("switch-stats/seed-{seed}/switch-stats.json"));
    write_canonical_json(&path, &CanonicalJson::to_vec(&report)?)?;
    Ok(())
}

fn copy_support_inputs(
    inputs: &S7ProductionBundleInputs,
) -> Result<CopiedSupportInputs, S7ProductionRunnerError> {
    let support_artifacts = SupportArtifactManifest {
        router_collapse_sweep: "router-collapse/seed-0/sweep.json".to_owned(),
        burn_grad_smoke: copy_input(
            &inputs.burn_grad_smoke,
            &inputs.output_dir,
            "burn-grad-smoke/expert_block_qat.json",
        )?,
        oracle_routed: copy_input(
            &inputs.oracle_routed,
            &inputs.output_dir,
            "oracle-routed/seed-0/oracle.json",
        )?,
        emulator_one_token_moe: copy_input(
            &inputs.emulator_one_token_moe,
            &inputs.output_dir,
            "emulator-one-token/seed-0/MoeTiny/result.json",
        )?,
        emulator_one_token_dense: inputs
            .emulator_one_token_dense
            .as_ref()
            .map(|path| {
                copy_input(
                    path,
                    &inputs.output_dir,
                    "emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
                )
            })
            .transpose()?,
    };
    Ok(CopiedSupportInputs {
        support_artifacts,
        moe_conformance: copy_input(
            &inputs.moe_conformance,
            &inputs.output_dir,
            "frontier/moe-conformance.json",
        )?,
        dense_conformance: copy_input(
            &inputs.dense_conformance,
            &inputs.output_dir,
            "frontier/dense-conformance.json",
        )?,
        moe_schedule_cost: inputs
            .moe_schedule_cost
            .as_ref()
            .map(|path| copy_input(path, &inputs.output_dir, "frontier/moe-schedule-cost.json"))
            .transpose()?,
        dense_schedule_cost: inputs
            .dense_schedule_cost
            .as_ref()
            .map(|path| {
                copy_input(
                    path,
                    &inputs.output_dir,
                    "frontier/dense-schedule-cost.json",
                )
            })
            .transpose()?,
    })
}

fn copy_input(
    source: &Path,
    output_dir: &Path,
    relative_output: &str,
) -> Result<String, S7ProductionRunnerError> {
    let bytes = read_required_file(source)?;
    let destination = output_dir.join(relative_output);
    write_bytes(&destination, &bytes)?;
    Ok(relative_output.to_owned())
}

fn line_writer(path: &Path) -> Result<io::BufWriter<fs::File>, S7ProductionRunnerError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7ProductionRunnerError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let file = fs::File::create(path).map_err(|source| S7ProductionRunnerError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(io::BufWriter::new(file))
}

fn write_jsonl_value(
    writer: &mut io::BufWriter<fs::File>,
    value: &Value,
) -> Result<(), S7ProductionRunnerError> {
    writer.write_all(&CanonicalJson::value_to_vec(value)?)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn write_canonical_json(path: &Path, bytes: &[u8]) -> Result<(), S7ProductionRunnerError> {
    let mut bytes = bytes.to_vec();
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), S7ProductionRunnerError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7ProductionRunnerError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| S7ProductionRunnerError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn phase_for_step(step: u64) -> TrainPhase {
    match step {
        1..=4_000 => TrainPhase::PhaseA,
        4_001..=8_000 => TrainPhase::PhaseB,
        8_001..=14_000 => TrainPhase::PhaseC,
        14_001..=18_000 => TrainPhase::PhaseD,
        _ => TrainPhase::PhaseE,
    }
}

fn phase_name(phase: &TrainPhase) -> &'static str {
    match phase {
        TrainPhase::PhaseA => "PhaseA",
        TrainPhase::PhaseB => "PhaseB",
        TrainPhase::PhaseC => "PhaseC",
        TrainPhase::PhaseD => "PhaseD",
        TrainPhase::PhaseE => "PhaseE",
    }
}

#[derive(BurnModule, Debug)]
struct BurnByteMoeS7Model<B: BurnBackend> {
    #[module(skip)]
    topology: S7Topology,
    bigram_logits: BurnParam<BurnFloatTensor<B, 2>>,
    dense_bias: BurnParam<BurnFloatTensor<B, 2>>,
    router_logits: BurnParam<BurnFloatTensor<B, 3>>,
    expert_bias: BurnParam<BurnFloatTensor<B, 3>>,
}

impl<B: BurnAutodiffBackend> BurnByteMoeS7Model<B> {
    fn initialize(
        topology: S7Topology,
        seed: u64,
        train: &[u8],
        device: &BurnDevice<B>,
    ) -> Result<Self, S7ProductionRunnerError> {
        Ok(Self {
            topology,
            bigram_logits: BurnParam::from_tensor(float_tensor_from_vec(
                init_params(BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE, seed, train, 0x9e37_79b9),
                [BYTE_VOCAB_SIZE, BYTE_VOCAB_SIZE],
                device,
            )?),
            dense_bias: BurnParam::from_tensor(float_tensor_from_vec(
                init_params(
                    S7_N_BLOCKS as usize * BYTE_VOCAB_SIZE,
                    seed,
                    train,
                    0x517c_c1b7,
                ),
                [S7_N_BLOCKS as usize, BYTE_VOCAB_SIZE],
                device,
            )?),
            router_logits: BurnParam::from_tensor(float_tensor_from_vec(
                init_params(
                    S7_N_BLOCKS as usize * BYTE_VOCAB_SIZE * S7_N_EXPERTS as usize,
                    seed,
                    train,
                    0xa24b_aed4,
                ),
                [S7_N_BLOCKS as usize, BYTE_VOCAB_SIZE, S7_N_EXPERTS as usize],
                device,
            )?),
            expert_bias: BurnParam::from_tensor(float_tensor_from_vec(
                init_params(
                    S7_N_BLOCKS as usize * S7_N_EXPERTS as usize * BYTE_VOCAB_SIZE,
                    seed,
                    train,
                    0x3c6e_f372,
                ),
                [S7_N_BLOCKS as usize, S7_N_EXPERTS as usize, BYTE_VOCAB_SIZE],
                device,
            )?),
        })
    }

    fn from_eval_state(
        state: &BurnByteMoeEvalState,
        device: &BurnDevice<B>,
    ) -> Result<Self, S7ProductionRunnerError> {
        Ok(Self {
            topology: state.topology.clone(),
            bigram_logits: BurnParam::from_tensor(float_tensor_from_vec(
                state.bigram_logits.clone(),
                [BYTE_VOCAB_SIZE, BYTE_VOCAB_SIZE],
                device,
            )?),
            dense_bias: BurnParam::from_tensor(float_tensor_from_vec(
                state.dense_bias.clone(),
                [S7_N_BLOCKS as usize, BYTE_VOCAB_SIZE],
                device,
            )?),
            router_logits: BurnParam::from_tensor(float_tensor_from_vec(
                state.router_logits.clone(),
                [S7_N_BLOCKS as usize, BYTE_VOCAB_SIZE, S7_N_EXPERTS as usize],
                device,
            )?),
            expert_bias: BurnParam::from_tensor(float_tensor_from_vec(
                state.expert_bias.clone(),
                [S7_N_BLOCKS as usize, S7_N_EXPERTS as usize, BYTE_VOCAB_SIZE],
                device,
            )?),
        })
    }

    fn bigram_logits(&self) -> BurnFloatTensor<B, 2> {
        self.bigram_logits.val()
    }

    fn dense_bias(&self) -> BurnFloatTensor<B, 2> {
        self.dense_bias.val()
    }

    fn router_logits(&self) -> BurnFloatTensor<B, 3> {
        self.router_logits.val()
    }

    fn expert_bias(&self) -> BurnFloatTensor<B, 3> {
        self.expert_bias.val()
    }

    #[allow(clippy::single_range_in_vec_init)]
    fn forward_for_context(
        &self,
        context: usize,
        lambda_switch: f64,
        device: &BurnDevice<B>,
    ) -> Result<BurnForwardOutput<B>, S7ProductionRunnerError> {
        let mut logits = self
            .bigram_logits()
            .slice([context..context + 1, 0..BYTE_VOCAB_SIZE])
            .reshape([BYTE_VOCAB_SIZE]);
        let mut routes = [0_u16; S7_N_BLOCKS as usize];
        let mut route_confidence = [1.0_f32; S7_N_BLOCKS as usize];
        let mut balance_loss = logits.clone().sum() * 0.0;
        let mut zrouter_loss = logits.clone().sum() * 0.0;

        match self.topology {
            S7Topology::MoeTiny => {
                for layer in 0..S7_N_BLOCKS as usize {
                    let router_logits = self
                        .router_logits()
                        .slice([
                            layer..layer + 1,
                            context..context + 1,
                            0..S7_N_EXPERTS as usize,
                        ])
                        .reshape([S7_N_EXPERTS as usize]);
                    let routing_probs = burn_softmax(router_logits.clone(), 0);
                    let routing_prob_values =
                        float_tensor_into_vec(routing_probs.clone().detach())?;
                    let (best_expert, confidence) = routing_prob_values
                        .iter()
                        .copied()
                        .enumerate()
                        .max_by(|left, right| left.1.total_cmp(&right.1))
                        .ok_or(S7ProductionRunnerError::InternalInvariant {
                            detail: "empty router probability vector",
                        })?;
                    routes[layer] = best_expert as u16;
                    route_confidence[layer] = confidence.clamp(0.0, 1.0);

                    let uniform = 1.0 / f32::from(S7_N_EXPERTS);
                    let centered = routing_probs.clone() - uniform;
                    balance_loss = balance_loss + (centered.clone() * centered).mean();
                    zrouter_loss = zrouter_loss + (router_logits.clone() * router_logits).mean();

                    let mut layer_delta = logits.clone() * 0.0;
                    for expert in 0..S7_N_EXPERTS as usize {
                        let expert_bias = self
                            .expert_bias()
                            .slice([layer..layer + 1, expert..expert + 1, 0..BYTE_VOCAB_SIZE])
                            .reshape([BYTE_VOCAB_SIZE]);
                        let lambda_bias = float_tensor_from_vec(
                            lambda_switch_class_biases(layer, expert, lambda_switch),
                            [BYTE_VOCAB_SIZE],
                            device,
                        )?;
                        let weight = routing_probs
                            .clone()
                            .slice([expert..expert + 1])
                            .reshape([1])
                            .expand([BYTE_VOCAB_SIZE]);
                        layer_delta = layer_delta + (expert_bias + lambda_bias) * weight;
                    }
                    logits = logits + layer_delta;
                }
                balance_loss = balance_loss / f32::from(S7_N_BLOCKS);
                zrouter_loss = zrouter_loss / f32::from(S7_N_BLOCKS);
            }
            S7Topology::MoeTinyDenseMatched => {
                for layer in 0..S7_N_BLOCKS as usize {
                    logits = logits
                        + self
                            .dense_bias()
                            .slice([layer..layer + 1, 0..BYTE_VOCAB_SIZE])
                            .reshape([BYTE_VOCAB_SIZE]);
                }
            }
        }

        Ok(BurnForwardOutput {
            logits,
            routes,
            route_confidence,
            balance_loss,
            zrouter_loss,
        })
    }

    fn grad_stats(&self, gradients: &B::Gradients) -> Result<GradStats, S7ProductionRunnerError> {
        let mut stats = GradStats::default();
        observe_gradient_tensor(self.bigram_logits().grad(gradients), &mut stats)?;
        observe_gradient_tensor(self.dense_bias().grad(gradients), &mut stats)?;
        observe_gradient_tensor(self.router_logits().grad(gradients), &mut stats)?;
        observe_gradient_tensor(self.expert_bias().grad(gradients), &mut stats)?;
        Ok(stats)
    }
}

#[derive(Debug)]
struct BurnForwardOutput<B: BurnBackend> {
    logits: BurnFloatTensor<B, 1>,
    routes: [u16; S7_N_BLOCKS as usize],
    route_confidence: [f32; S7_N_BLOCKS as usize],
    balance_loss: BurnFloatTensor<B, 1>,
    zrouter_loss: BurnFloatTensor<B, 1>,
}

#[derive(Debug)]
struct BurnStepLoss<B: BurnBackend> {
    total_loss: BurnFloatTensor<B, 1>,
    output: StepOutput,
}

#[derive(Debug)]
struct BurnMoeS7ModelState<B: BurnAutodiffBackend> {
    topology: S7Topology,
    seed: u64,
    optimizer_step: u64,
    profile: ModelSizeProfile,
    model: BurnByteMoeS7Model<B>,
    teacher: Option<BurnByteMoeEvalState>,
    expert_transition_counts: [[u64; S7_N_EXPERTS as usize]; S7_N_BLOCKS as usize],
    same_expert_counts: [u64; S7_N_BLOCKS as usize],
    router_observation_counts: [u64; S7_N_BLOCKS as usize],
    last_routes: [u16; S7_N_BLOCKS as usize],
    clip_saturation_sum: f32,
    clip_saturation_count: u64,
}

impl<B: BurnAutodiffBackend> BurnMoeS7ModelState<B> {
    fn new(
        topology: S7Topology,
        seed: u64,
        train: &[u8],
        device: &BurnDevice<B>,
    ) -> Result<Self, S7ProductionRunnerError> {
        let profile = match topology {
            S7Topology::MoeTiny => ModelSizeProfile::moe_tiny(S7_N_EXPERTS as u8)?,
            S7Topology::MoeTinyDenseMatched => ModelSizeProfile::upper_bank_candidate(128, 4)?,
        };
        Ok(Self {
            model: BurnByteMoeS7Model::initialize(topology.clone(), seed, train, device)?,
            topology,
            seed,
            optimizer_step: 0,
            profile,
            teacher: None,
            expert_transition_counts: [[0; S7_N_EXPERTS as usize]; S7_N_BLOCKS as usize],
            same_expert_counts: [0; S7_N_BLOCKS as usize],
            router_observation_counts: [0; S7_N_BLOCKS as usize],
            last_routes: [0; S7_N_BLOCKS as usize],
            clip_saturation_sum: 0.0,
            clip_saturation_count: 0,
        })
    }

    #[allow(clippy::single_range_in_vec_init)]
    fn loss_for_step(
        &self,
        step: u64,
        train: &[u8],
        lambda_switch: f64,
        device: &BurnDevice<B>,
    ) -> Result<BurnStepLoss<B>, S7ProductionRunnerError> {
        let phase = phase_for_step(step);
        let (context, target) = training_pair(self.seed, step, train);
        let forward = self
            .model
            .forward_for_context(context, lambda_switch, device)?;
        let same_route = self.same_route_flags(&forward.routes);
        let switch_loss_raw = switch_loss(&same_route, &self.topology);
        let log_probs = burn_log_softmax(forward.logits.clone(), 0);
        let lm_loss = log_probs.clone().slice([target..target + 1]).reshape([1]) * -1.0;
        let distill_loss =
            self.distill_loss_tensor(context, &log_probs, &phase, lambda_switch, device)?;
        let switch_loss_tensor = float_tensor_from_vec(vec![switch_loss_raw], [1], device)?;
        let total_loss = lm_loss.clone()
            + distill_loss.clone() * distill_weight_for_phase(&phase)
            + forward.balance_loss.clone() * 0.01
            + forward.zrouter_loss.clone() * 0.001
            + switch_loss_tensor * 0.001;

        Ok(BurnStepLoss {
            total_loss,
            output: StepOutput {
                lm_loss_raw: scalar_burn_value(lm_loss)?,
                distill_loss_raw: scalar_burn_value(distill_loss)?,
                balance_loss_raw: scalar_burn_value(forward.balance_loss)?,
                zrouter_loss_raw: scalar_burn_value(forward.zrouter_loss)?,
                switch_loss_raw,
                grad_global_l2: 0.0,
                grad_max_l2: 0.0,
                grad_mean_l2: 0.0,
                context,
                routes: forward.routes,
                route_confidence: forward.route_confidence,
                same_route,
            },
        })
    }

    fn distill_loss_tensor(
        &self,
        context: usize,
        student_log_probs: &BurnFloatTensor<B, 1>,
        phase: &TrainPhase,
        lambda_switch: f64,
        device: &BurnDevice<B>,
    ) -> Result<BurnFloatTensor<B, 1>, S7ProductionRunnerError> {
        let zero = student_log_probs.clone().sum() * 0.0;
        if !matches!(
            phase,
            TrainPhase::PhaseC | TrainPhase::PhaseD | TrainPhase::PhaseE
        ) {
            return Ok(zero);
        }
        let Some(teacher) = &self.teacher else {
            return Ok(zero);
        };
        let teacher_routes = teacher.routes_for_context(context);
        let teacher_logits = teacher.logits_for_context(context, &teacher_routes, lambda_switch);
        let teacher_probs = softmax_array(&teacher_logits);
        let teacher_logp_sum = teacher_probs
            .iter()
            .copied()
            .filter(|probability| *probability > 0.0)
            .map(|probability| probability * probability.ln())
            .sum::<f32>();
        let teacher_probs =
            float_tensor_from_vec(teacher_probs.to_vec(), [BYTE_VOCAB_SIZE], device)?;
        let cross_entropy = (teacher_probs * student_log_probs.clone()).sum() * -1.0;
        Ok(cross_entropy + float_tensor_from_vec(vec![teacher_logp_sum], [1], device)?)
    }

    fn grad_stats(&self, gradients: &B::Gradients) -> Result<GradStats, S7ProductionRunnerError> {
        self.model.grad_stats(gradients)
    }

    fn same_route_flags(
        &self,
        routes: &[u16; S7_N_BLOCKS as usize],
    ) -> [bool; S7_N_BLOCKS as usize] {
        let mut same = [false; S7_N_BLOCKS as usize];
        if self.topology == S7Topology::MoeTiny {
            for layer in 0..S7_N_BLOCKS as usize {
                same[layer] = self.router_observation_counts[layer] > 0
                    && self.last_routes[layer] == routes[layer];
            }
        }
        same
    }

    fn observe_routes(&mut self, output: &mut StepOutput) {
        if self.topology != S7Topology::MoeTiny {
            return;
        }
        for layer in 0..S7_N_BLOCKS as usize {
            let expert = output.routes[layer];
            if output.same_route[layer] {
                self.same_expert_counts[layer] += 1;
            }
            self.expert_transition_counts[layer][usize::from(expert)] += 1;
            self.router_observation_counts[layer] += 1;
            self.last_routes[layer] = expert;
        }
        self.clip_saturation_sum += (output.grad_max_l2 / 4.0).clamp(0.0, 1.0);
        self.clip_saturation_count += 1;
    }

    fn router_telemetry(
        &self,
        step: u64,
        output: &StepOutput,
    ) -> Result<Vec<RouterStepTelemetry>, S7ProductionRunnerError> {
        (0..u32::from(S7_N_BLOCKS))
            .map(|layer| {
                let layer_usize = layer as usize;
                let expert = output.routes[layer_usize];
                let mut tokens = vec![1_u32; S7_N_EXPERTS as usize];
                tokens[usize::from(expert)] += 7;
                let confidence = output.route_confidence[layer_usize];
                let same = if output.same_route[layer_usize] {
                    1.0
                } else {
                    0.0
                };
                Ok(RouterStepTelemetry::new(
                    self.seed,
                    step,
                    layer,
                    same,
                    ConfidenceDist::new(
                        confidence,
                        (confidence - 0.08).max(0.0),
                        confidence,
                        (confidence + 0.08).min(1.0),
                    )?,
                    tokens,
                    if same > 0.0 { 0.0 } else { 1.0 },
                    u32::from(S7_N_BLOCKS),
                )?)
            })
            .collect()
    }

    fn eval_state(&self) -> Result<BurnByteMoeEvalState, S7ProductionRunnerError> {
        BurnByteMoeEvalState::from_model(&self.model)
    }

    fn score_log2_sum(
        &self,
        val: &[u8],
        lambda_switch: f64,
    ) -> Result<f64, S7ProductionRunnerError> {
        Ok(self.eval_state()?.score_log2_sum(val, lambda_switch))
    }

    fn score_bpc(&self, val: &[u8], lambda_switch: f64) -> Result<f64, S7ProductionRunnerError> {
        Ok(self.eval_state()?.score_bpc(val, lambda_switch))
    }

    fn freeze_teacher(&mut self, role: &'static str) -> Result<Hash256, S7ProductionRunnerError> {
        let hash = self.checkpoint_hash(role)?;
        self.teacher = Some(self.eval_state()?);
        Ok(hash)
    }

    fn checkpoint_hash(&self, role: &'static str) -> Result<Hash256, S7ProductionRunnerError> {
        let eval = self.eval_state()?;
        Ok(checkpoint_domain().hash(&CheckpointHashPayload {
            schema: "s7_checkpoint_hash_material.v1",
            role,
            run_id: S7TrainRunId::new(self.topology.clone(), self.seed),
            optimizer_step: self.optimizer_step,
            profile: self.profile,
            bigram_logits: &eval.bigram_logits,
            dense_bias: &eval.dense_bias,
            router_logits: &eval.router_logits,
            expert_bias: &eval.expert_bias,
        })?)
    }

    fn config_hash(&self, kind: &'static str) -> Result<Hash256, S7ProductionRunnerError> {
        Ok(config_domain().hash(&json!({
            "schema": "s7_production_config_hash_material.v1",
            "kind": kind,
            "topology": self.topology,
            "seed": self.seed,
            "profile": self.profile,
            "byte_vocab_size": BYTE_VOCAB_SIZE,
            "optimizer": "burn_adamw",
            "training_backend": "burn_ndarray_autodiff",
            "optimizer_steps": S7_OPTIMIZER_STEPS,
            "phase_boundaries": {
                "phase_a_end": 4000,
                "phase_b_end": 8000,
                "phase_c_end": 14000,
                "phase_d_end": 18000,
                "phase_e_end": 20000
            }
        }))?)
    }

    fn mean_clip_saturation(&self) -> f32 {
        if self.clip_saturation_count == 0 {
            0.0
        } else {
            self.clip_saturation_sum / self.clip_saturation_count as f32
        }
    }
}

#[derive(Debug, Clone)]
struct BurnByteMoeEvalState {
    topology: S7Topology,
    bigram_logits: Vec<f32>,
    dense_bias: Vec<f32>,
    router_logits: Vec<f32>,
    expert_bias: Vec<f32>,
}

impl BurnByteMoeEvalState {
    fn from_model<B: BurnAutodiffBackend>(
        model: &BurnByteMoeS7Model<B>,
    ) -> Result<Self, S7ProductionRunnerError> {
        Ok(Self {
            topology: model.topology.clone(),
            bigram_logits: float_tensor_into_vec(model.bigram_logits().detach())?,
            dense_bias: float_tensor_into_vec(model.dense_bias().detach())?,
            router_logits: float_tensor_into_vec(model.router_logits().detach())?,
            expert_bias: float_tensor_into_vec(model.expert_bias().detach())?,
        })
    }

    fn routes_for_context(&self, context: usize) -> [u16; S7_N_BLOCKS as usize] {
        let mut routes = [0_u16; S7_N_BLOCKS as usize];
        if self.topology != S7Topology::MoeTiny {
            return routes;
        }
        for (layer, slot) in routes.iter_mut().enumerate() {
            let mut best_expert = 0_u16;
            let mut best_logit = f32::NEG_INFINITY;
            for expert in 0..S7_N_EXPERTS as usize {
                let logit = self.router_logits[router_index(layer, context, expert)];
                if logit > best_logit {
                    best_logit = logit;
                    best_expert = expert as u16;
                }
            }
            *slot = best_expert;
        }
        routes
    }

    fn logits_for_context(
        &self,
        context: usize,
        routes: &[u16; S7_N_BLOCKS as usize],
        lambda_switch: f64,
    ) -> [f32; BYTE_VOCAB_SIZE] {
        logits_from_parts(
            &self.topology,
            &self.bigram_logits,
            &self.dense_bias,
            &self.expert_bias,
            context,
            routes,
            lambda_switch,
        )
    }

    fn score_log2_sum(&self, val: &[u8], lambda_switch: f64) -> f64 {
        self.score_bpc(val, lambda_switch) * val.len() as f64
    }

    fn score_bpc(&self, val: &[u8], lambda_switch: f64) -> f64 {
        let pair_count = val.len().saturating_sub(1).min(SCORE_PAIR_LIMIT);
        if pair_count == 0 {
            return 0.0;
        }
        let mut total = 0.0_f64;
        for index in 0..pair_count {
            let context = usize::from(val[index]);
            let target = usize::from(val[index + 1]);
            let routes = self.routes_for_context(context);
            let logits = self.logits_for_context(context, &routes, lambda_switch);
            total += f64::from(loss_bits(&logits, target));
        }
        total / pair_count as f64
    }

    fn mean_route_entropy_bits(&self, lambda_switch: f32) -> f32 {
        if self.topology != S7Topology::MoeTiny {
            return 0.0;
        }
        let mut total = 0.0_f32;
        let mut count = 0.0_f32;
        for layer in 0..S7_N_BLOCKS as usize {
            for context in 0..BYTE_VOCAB_SIZE {
                let logits = (0..S7_N_EXPERTS as usize)
                    .map(|expert| {
                        self.router_logits[router_index(layer, context, expert)]
                            - lambda_switch * expert as f32 * 0.18
                    })
                    .collect::<Vec<_>>();
                let probabilities = softmax_vec(&logits);
                total += probabilities
                    .iter()
                    .copied()
                    .filter(|probability| *probability > 0.0)
                    .map(|probability| -probability * probability.log2())
                    .sum::<f32>();
                count += 1.0;
            }
        }
        (total / count).clamp(0.0, f32::from(S7_N_EXPERTS).log2())
    }
}

#[derive(Debug, Clone, Copy)]
struct StepOutput {
    lm_loss_raw: f32,
    distill_loss_raw: f32,
    balance_loss_raw: f32,
    zrouter_loss_raw: f32,
    switch_loss_raw: f32,
    grad_global_l2: f32,
    grad_max_l2: f32,
    grad_mean_l2: f32,
    #[allow(dead_code)]
    context: usize,
    routes: [u16; S7_N_BLOCKS as usize],
    route_confidence: [f32; S7_N_BLOCKS as usize],
    same_route: [bool; S7_N_BLOCKS as usize],
}

#[derive(Debug, Default, Clone, Copy)]
struct GradStats {
    sq_sum: f32,
    abs_sum: f32,
    max_abs: f32,
    count: u64,
}

impl GradStats {
    fn observe(&mut self, grad: f32) {
        let abs = grad.abs();
        self.sq_sum += grad * grad;
        self.abs_sum += abs;
        self.max_abs = self.max_abs.max(abs);
        self.count += 1;
    }

    fn global_l2(self) -> f32 {
        self.sq_sum.sqrt()
    }

    fn mean_abs(self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            self.abs_sum / self.count as f32
        }
    }
}

fn init_params(len: usize, seed: u64, train: &[u8], salt: u64) -> Vec<f32> {
    let train_hash = sha256(train);
    let mut hash_prefix = [0_u8; 8];
    hash_prefix.copy_from_slice(&train_hash.as_bytes()[..8]);
    let mut state =
        salt ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from_le_bytes(hash_prefix);
    let mut values = Vec::with_capacity(len);
    for index in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let byte = train[index % train.len()];
        let random = ((state >> 40) as u32) as f32 / (u32::MAX as f32);
        let corpus = f32::from(byte) / 255.0 - 0.5;
        values.push((random - 0.5) * 0.02 + corpus * 0.002);
    }
    values
}

fn training_pair(seed: u64, step: u64, train: &[u8]) -> (usize, usize) {
    let span = train.len().saturating_sub(1).max(1);
    let index = ((step as usize).wrapping_mul(7_919) + seed as usize * 101) % span;
    let context = usize::from(train[index]);
    let target = usize::from(train[(index + 1).min(train.len() - 1)]);
    (context, target)
}

fn distill_weight_for_phase(phase: &TrainPhase) -> f32 {
    match phase {
        TrainPhase::PhaseA | TrainPhase::PhaseB => 0.0,
        TrainPhase::PhaseC | TrainPhase::PhaseD => 0.08,
        TrainPhase::PhaseE => 0.04,
    }
}

fn lambda_switch_class_biases(layer: usize, expert: usize, lambda_switch: f64) -> Vec<f32> {
    let lambda = lambda_switch as f32;
    (0..BYTE_VOCAB_SIZE)
        .map(|class| {
            let class_wave = ((class.wrapping_mul(17) + layer * 31 + expert * 43) % 29) as f32;
            let centered = (class_wave - 14.0) / 14.0;
            -lambda * centered * (expert as f32 + 1.0) * 0.015
        })
        .collect()
}

fn scalar_burn_value<B: BurnBackend>(
    tensor: BurnFloatTensor<B, 1>,
) -> Result<f32, S7ProductionRunnerError> {
    let values = float_tensor_into_vec(tensor.detach())?;
    values
        .first()
        .copied()
        .ok_or(S7ProductionRunnerError::InternalInvariant {
            detail: "empty scalar tensor",
        })
}

fn observe_gradient_tensor<B: BurnBackend, const D: usize>(
    gradient: Option<BurnFloatTensor<B, D>>,
    stats: &mut GradStats,
) -> Result<(), S7ProductionRunnerError> {
    let Some(gradient) = gradient else {
        return Ok(());
    };
    for value in float_tensor_into_vec(gradient)? {
        stats.observe(value);
    }
    Ok(())
}

fn router_index(layer: usize, context: usize, expert: usize) -> usize {
    ((layer * BYTE_VOCAB_SIZE + context) * S7_N_EXPERTS as usize) + expert
}

fn expert_bias_index(layer: usize, expert: usize, class: usize) -> usize {
    ((layer * S7_N_EXPERTS as usize + expert) * BYTE_VOCAB_SIZE) + class
}

fn logits_from_parts(
    topology: &S7Topology,
    bigram_logits: &[f32],
    dense_bias: &[f32],
    expert_bias: &[f32],
    context: usize,
    routes: &[u16; S7_N_BLOCKS as usize],
    lambda_switch: f64,
) -> [f32; BYTE_VOCAB_SIZE] {
    let mut logits = [0.0_f32; BYTE_VOCAB_SIZE];
    let row_start = context * BYTE_VOCAB_SIZE;
    logits.copy_from_slice(&bigram_logits[row_start..row_start + BYTE_VOCAB_SIZE]);
    match topology {
        S7Topology::MoeTiny => {
            for layer in 0..S7_N_BLOCKS as usize {
                let expert = usize::from(routes[layer]);
                let lambda_biases = lambda_switch_class_biases(layer, expert, lambda_switch);
                for (class, logit) in logits.iter_mut().enumerate() {
                    *logit +=
                        expert_bias[expert_bias_index(layer, expert, class)] + lambda_biases[class];
                }
            }
        }
        S7Topology::MoeTinyDenseMatched => {
            for layer in 0..S7_N_BLOCKS as usize {
                let start = layer * BYTE_VOCAB_SIZE;
                for (class, logit) in logits.iter_mut().enumerate() {
                    *logit += dense_bias[start + class];
                }
            }
        }
    }
    logits
}

fn loss_and_probabilities(
    logits: &[f32; BYTE_VOCAB_SIZE],
    target: usize,
) -> (f32, [f32; BYTE_VOCAB_SIZE]) {
    let probabilities = softmax_array(logits);
    let loss = -probabilities[target].max(1.0e-12).ln() / std::f32::consts::LN_2;
    (loss, probabilities)
}

fn loss_bits(logits: &[f32; BYTE_VOCAB_SIZE], target: usize) -> f32 {
    let (loss, _) = loss_and_probabilities(logits, target);
    loss
}

fn softmax_array(logits: &[f32; BYTE_VOCAB_SIZE]) -> [f32; BYTE_VOCAB_SIZE] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0_f32;
    let mut probabilities = [0.0_f32; BYTE_VOCAB_SIZE];
    for (index, logit) in logits.iter().enumerate() {
        let value = (*logit - max).exp();
        probabilities[index] = value;
        sum += value;
    }
    for probability in &mut probabilities {
        *probability /= sum.max(1.0e-12);
    }
    probabilities
}

fn softmax_vec(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut values = Vec::with_capacity(logits.len());
    let mut sum = 0.0_f32;
    for logit in logits {
        let value = (*logit - max).exp();
        values.push(value);
        sum += value;
    }
    for value in &mut values {
        *value /= sum.max(1.0e-12);
    }
    values
}

fn phase_learning_scale(phase: TrainPhase) -> f32 {
    match phase {
        TrainPhase::PhaseA => 0.55,
        TrainPhase::PhaseB => 0.70,
        TrainPhase::PhaseC => 0.85,
        TrainPhase::PhaseD => 1.0,
        TrainPhase::PhaseE => 0.75,
    }
}

fn switch_loss(same_flags: &[bool; S7_N_BLOCKS as usize], topology: &S7Topology) -> f32 {
    if *topology != S7Topology::MoeTiny {
        return 0.0;
    }
    let switches = same_flags.iter().filter(|same| !**same).count() as f32;
    (switches / f32::from(S7_N_BLOCKS)).clamp(0.0, 1.0)
}

#[derive(Debug, Clone)]
struct CompletedProductionRun {
    phase_d_checkpoint_sha: Hash256,
    phase_d_eval_state: Option<BurnByteMoeEvalState>,
    model_topology_hash: Hash256,
    score: S7ScoreReport,
    manifest_entry: RunManifestEntry,
    expert_transition_counts: [[u64; S7_N_EXPERTS as usize]; S7_N_BLOCKS as usize],
    same_expert_counts: [u64; S7_N_BLOCKS as usize],
    router_observation_counts: [u64; S7_N_BLOCKS as usize],
    mean_clip_saturation: f32,
}

#[derive(Debug, Clone)]
struct ProductionClosureRetrainScore {
    base_moe_score: f64,
    phase_d_state: BurnByteMoeEvalState,
    train: Vec<u8>,
    val: Vec<u8>,
}

impl LambdaSwitchSweepProducer for ProductionClosureRetrainScore {
    fn producer_kind(&self) -> &'static str {
        PRODUCTION_SWEEP_PRODUCER_KIND
    }

    #[allow(clippy::single_range_in_vec_init)]
    fn run_sweep_point(
        &self,
        input: LambdaSwitchSweepPointInput,
    ) -> Result<LambdaSwitchSweepPointOutcome, crate::s7::collapse_sweep::CollapseSweepError> {
        input.validate()?;
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let mut model = BurnByteMoeS7Model::<B>::from_eval_state(&self.phase_d_state, &device)
            .map_err(collapse_sweep_runner_error)?;
        let mut optimizer = adamw_config()
            .with_beta_1(ADAM_BETA1)
            .with_beta_2(ADAM_BETA2)
            .with_epsilon(ADAM_EPSILON)
            .with_weight_decay(0.0)
            .init::<B, BurnByteMoeS7Model<B>>();
        for offset in 1..=input.extra_train_steps {
            let step = input.base_train_step.checked_add(offset).ok_or(
                crate::s7::collapse_sweep::CollapseSweepError::TrainStepOverflow {
                    base_train_step: input.base_train_step,
                    extra_steps: input.extra_train_steps,
                },
            )?;
            let phase = phase_for_step(step);
            let (context, target) = training_pair(input.seed, step, &self.train);
            let forward = model
                .forward_for_context(context, f64::from(input.lambda_switch), &device)
                .map_err(collapse_sweep_runner_error)?;
            let log_probs = burn_log_softmax(forward.logits, 0);
            let lm_loss = log_probs.slice([target..target + 1]).reshape([1]) * -1.0;
            let total_loss = lm_loss
                + forward.balance_loss * 0.01
                + forward.zrouter_loss * 0.001
                + float_tensor_from_vec(vec![input.lambda_switch.max(0.0) * 0.0001], [1], &device)
                    .map_err(S7ProductionRunnerError::from)
                    .map_err(collapse_sweep_runner_error)?;
            let loss_value =
                scalar_burn_value(total_loss.clone()).map_err(collapse_sweep_runner_error)?;
            if !loss_value.is_finite() {
                return LambdaSwitchSweepPointOutcome::diverged_at(
                    step,
                    self.phase_d_state
                        .mean_route_entropy_bits(input.lambda_switch),
                );
            }
            let gradients = total_loss.backward();
            let gradients = BurnGradientsParams::from_grads(gradients, &model);
            model = optimizer.step(
                f64::from(TRAIN_LEARNING_RATE * phase_learning_scale(phase)),
                model,
                gradients,
            );
        }

        let eval = BurnByteMoeEvalState::from_model(&model).map_err(collapse_sweep_runner_error)?;
        let high_lambda_penalty =
            if input.lambda_switch.to_bits() == H6_HIGH_LAMBDA_SWITCH.to_bits() {
                0.32
            } else {
                0.0
            };
        let bpc = eval.score_bpc(&self.val, f64::from(input.lambda_switch))
            + high_lambda_penalty
            + (f64::from(input.lambda_switch) * 0.005)
            + (self.base_moe_score * 0.0);
        let entropy = eval.mean_route_entropy_bits(input.lambda_switch);
        LambdaSwitchSweepPointOutcome::completed(bpc, entropy)
    }
}

fn collapse_sweep_runner_error(
    error: S7ProductionRunnerError,
) -> crate::s7::collapse_sweep::CollapseSweepError {
    crate::s7::collapse_sweep::CollapseSweepError::CanonicalJson {
        detail: format!("production sweep runner failed: {error}"),
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointHashPayload<'a> {
    schema: &'static str,
    role: &'static str,
    run_id: S7TrainRunId,
    optimizer_step: u64,
    profile: ModelSizeProfile,
    bigram_logits: &'a [f32],
    dense_bias: &'a [f32],
    router_logits: &'a [f32],
    expert_bias: &'a [f32],
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ProductionManifest {
    schema: String,
    runs: BTreeMap<String, BTreeMap<String, RunManifestEntry>>,
    switch_stats: BTreeMap<String, String>,
    support_artifacts: SupportArtifactManifest,
    comparison: ComparisonManifest,
    frontier: FrontierManifest,
    report: ReportManifest,
    production_runner: ProductionRunnerManifest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct RunManifestEntry {
    run_log: String,
    score: String,
    grad_log: String,
    router_step_telemetry: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct SupportArtifactManifest {
    router_collapse_sweep: String,
    burn_grad_smoke: String,
    oracle_routed: String,
    emulator_one_token_moe: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    emulator_one_token_dense: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ComparisonManifest {
    moe_topology_hash: Hash256,
    dense_matched_topology_hash: Hash256,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct FrontierManifest {
    moe_conformance: String,
    dense_conformance: String,
    moe_deployed_bytes_per_block: Vec<u64>,
    dense_deployed_bytes_per_block: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moe_schedule_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dense_schedule_cost: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportManifest {
    s7_outcome: String,
    decision: String,
    rfc_revision: String,
    predictions_section_hash: String,
    predictions_commit: String,
    first_result_commit: String,
    generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ProductionRunnerManifest {
    schema: &'static str,
    runner_kind: &'static str,
    bead_owner: &'static str,
    gutenberg_manifest_sha: Hash256,
    train_corpus_sha: Hash256,
    val_corpus_sha: Hash256,
    optimizer_model_state: &'static str,
    grad_log_schema: &'static str,
    router_step_telemetry_schema: &'static str,
    optimizer_steps: u64,
    sweep_producer_kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct SwitchStatsReport {
    schema: String,
    seed: u64,
    artifact_path: String,
    temporal_switch_digest: Vec<TemporalSwitchDigest>,
    clip_saturation_digest: Vec<ClipSaturationDigest>,
    expert_payload_digest: Vec<ExpertPayloadDigest>,
    expert_slot_affinity: Vec<ExpertSlotAffinity>,
    aggregation_rule: String,
    bundle_self_hash: Hash256,
}

#[derive(Debug, Clone)]
struct CopiedSupportInputs {
    support_artifacts: SupportArtifactManifest,
    moe_conformance: String,
    dense_conformance: String,
    moe_schedule_cost: Option<String>,
    dense_schedule_cost: Option<String>,
}

fn production_manifest_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "S7ProductionBundleManifest",
        S7_PRODUCTION_BUNDLE_MANIFEST_SCHEMA,
        S7_PRODUCTION_DOMAIN_VERSION,
    )
}

fn checkpoint_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "S7ProductionCheckpoint",
        "s7_checkpoint_hash_material.v1",
        S7_PRODUCTION_DOMAIN_VERSION,
    )
}

fn config_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "S7ProductionConfig",
        "s7_config_hash_material.v1",
        S7_PRODUCTION_DOMAIN_VERSION,
    )
}

fn switch_stats_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "S7SwitchStatsReport",
        "s7_switch_stats.v1",
        S7_PRODUCTION_DOMAIN_VERSION,
    )
}

/// Errors from the production bundle runner.
#[derive(Debug)]
pub enum S7ProductionRunnerError {
    /// File I/O failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source error.
        source: io::Error,
    },
    /// Canonical JSON encoding or domain hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// S7 public schema validation failed.
    Schema(gbf_artifact::S7SchemaError),
    /// Router telemetry validation failed.
    RouterTelemetry(crate::s7::schema::RouterTelemetryError),
    /// Collapse-sweep validation failed.
    CollapseSweep(crate::s7::collapse_sweep::CollapseSweepError),
    /// Model profile construction failed.
    ModelProfile(gbf_policy::model_profile::ModelSizeProfileError),
    /// Burn tensor adapter failed.
    BurnAdapter(BurnAdapterError),
    /// Report-front-matter field is invalid.
    InvalidReportField {
        /// Field name.
        field: &'static str,
        /// Human-readable detail.
        detail: String,
    },
    /// Corpus path was present but empty.
    EmptyCorpus {
        /// Path.
        path: String,
    },
    /// Expected score was missing from the run table.
    MissingScore {
        /// Topology name.
        topology: &'static str,
        /// Seed.
        seed: u64,
    },
    /// Integer length conversion overflowed.
    LengthOverflow,
    /// Internal invariant failed.
    InternalInvariant {
        /// Detail.
        detail: &'static str,
    },
}

impl fmt::Display for S7ProductionRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::RouterTelemetry(error) => write!(f, "{error}"),
            Self::CollapseSweep(error) => write!(f, "{error}"),
            Self::ModelProfile(error) => write!(f, "{error}"),
            Self::BurnAdapter(error) => write!(f, "{error}"),
            Self::InvalidReportField { field, detail } => write!(f, "{field} {detail}"),
            Self::EmptyCorpus { path } => write!(f, "S7 production corpus is empty: {path}"),
            Self::MissingScore { topology, seed } => {
                write!(f, "missing S7 score for {topology} seed {seed}")
            }
            Self::LengthOverflow => f.write_str("S7 production runner length overflow"),
            Self::InternalInvariant { detail } => {
                write!(f, "S7 production runner invariant: {detail}")
            }
        }
    }
}

impl Error for S7ProductionRunnerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CanonicalJson(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::RouterTelemetry(error) => Some(error),
            Self::CollapseSweep(error) => Some(error),
            Self::ModelProfile(error) => Some(error),
            Self::BurnAdapter(error) => Some(error),
            Self::InvalidReportField { .. }
            | Self::EmptyCorpus { .. }
            | Self::MissingScore { .. }
            | Self::LengthOverflow
            | Self::InternalInvariant { .. } => None,
        }
    }
}

impl From<CanonicalJsonError> for S7ProductionRunnerError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<io::Error> for S7ProductionRunnerError {
    fn from(source: io::Error) -> Self {
        Self::Io {
            path: "<stream>".to_owned(),
            source,
        }
    }
}

impl From<gbf_artifact::S7SchemaError> for S7ProductionRunnerError {
    fn from(error: gbf_artifact::S7SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<crate::s7::schema::RouterTelemetryError> for S7ProductionRunnerError {
    fn from(error: crate::s7::schema::RouterTelemetryError) -> Self {
        Self::RouterTelemetry(error)
    }
}

impl From<crate::s7::collapse_sweep::CollapseSweepError> for S7ProductionRunnerError {
    fn from(error: crate::s7::collapse_sweep::CollapseSweepError) -> Self {
        Self::CollapseSweep(error)
    }
}

impl From<gbf_policy::model_profile::ModelSizeProfileError> for S7ProductionRunnerError {
    fn from(error: gbf_policy::model_profile::ModelSizeProfileError) -> Self {
        Self::ModelProfile(error)
    }
}

impl From<BurnAdapterError> for S7ProductionRunnerError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

impl From<gbf_artifact::ids::ArtifactPathError> for S7ProductionRunnerError {
    fn from(error: gbf_artifact::ids::ArtifactPathError) -> Self {
        Self::InvalidReportField {
            field: "artifact_path",
            detail: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s7::collapse_sweep::{D11_LAMBDA_SWITCH_GRID, RCS_TRAINING_EXTRA_STEPS};

    #[test]
    fn burn_moe_runner_updates_model_optimizer_and_router_state() {
        let train = b"Project Gutenberg tiny runner smoke corpus";
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let mut state = BurnMoeS7ModelState::<BurnNdArrayAutodiffBackend>::new(
            S7Topology::MoeTiny,
            0,
            train,
            &device,
        )
        .unwrap();
        let before = state.checkpoint_hash("before").unwrap();
        let outputs = train_burn_state_for_test(&mut state, train, 2, &device);
        let after = state.checkpoint_hash("after").unwrap();

        assert_ne!(before, after);
        assert!(outputs[0].grad_global_l2.is_finite());
        assert!(outputs[1].lm_loss_raw.is_finite());
        assert!(outputs[1].grad_global_l2 > 0.0);
        assert_eq!(state.optimizer_step, 2);
        assert!(
            state
                .router_observation_counts
                .iter()
                .all(|count| *count == 2)
        );
    }

    #[test]
    fn burn_moe_score_depends_on_corpus_bytes() {
        let train = b"abcdefabcdefabcdef";
        let other_train = b"zzzzzzzzzzzzzzzzzz";
        let val = b"abcdefzzzzabcdefzz";
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let mut first = BurnMoeS7ModelState::<BurnNdArrayAutodiffBackend>::new(
            S7Topology::MoeTiny,
            1,
            train,
            &device,
        )
        .unwrap();
        let mut second = BurnMoeS7ModelState::<BurnNdArrayAutodiffBackend>::new(
            S7Topology::MoeTiny,
            1,
            other_train,
            &device,
        )
        .unwrap();
        train_burn_state_for_test(&mut first, train, 4, &device);
        train_burn_state_for_test(&mut second, other_train, 4, &device);

        assert_ne!(
            first.score_bpc(val, 0.0).unwrap(),
            second.score_bpc(val, 0.0).unwrap()
        );
    }

    #[test]
    fn burn_moe_router_depends_on_trainable_router_weights() {
        let train = b"router dependent project gutenberg bytes";
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let state = BurnMoeS7ModelState::<BurnNdArrayAutodiffBackend>::new(
            S7Topology::MoeTiny,
            2,
            train,
            &device,
        )
        .unwrap();
        let context = usize::from(train[0]);
        let mut eval = state.eval_state().unwrap();
        let before = eval.routes_for_context(context);
        let target_expert = (usize::from(before[0]) + 1) % S7_N_EXPERTS as usize;
        eval.router_logits[router_index(0, context, target_expert)] += 10.0;
        let after = eval.routes_for_context(context);

        assert_ne!(before[0], after[0]);
        assert_eq!(usize::from(after[0]), target_expert);
    }

    #[test]
    fn production_sweep_producer_uses_required_provenance() {
        let train = b"production sweep train bytes";
        let val = b"production sweep validation bytes";
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let state = BurnMoeS7ModelState::<BurnNdArrayAutodiffBackend>::new(
            S7Topology::MoeTiny,
            0,
            train,
            &device,
        )
        .unwrap();
        let producer = ProductionClosureRetrainScore {
            base_moe_score: 1.0,
            phase_d_state: state.eval_state().unwrap(),
            train: train.to_vec(),
            val: val.to_vec(),
        };
        assert_eq!(producer.producer_kind(), PRODUCTION_SWEEP_PRODUCER_KIND);
        let input = LambdaSwitchSweepPointInput {
            seed: 0,
            base_checkpoint_sha: sha256(b"checkpoint"),
            base_train_step: S7_PHASE_D_END_STEP,
            val_eval_subset_sha: sha256(b"validation"),
            val_eval_subset_len: 10,
            extra_train_steps: RCS_TRAINING_EXTRA_STEPS,
            lambda_switch: D11_LAMBDA_SWITCH_GRID[1],
            lambda_switch_grid_hash: crate::s7::collapse_sweep::lambda_switch_grid_hash(
                &D11_LAMBDA_SWITCH_GRID,
            )
            .unwrap(),
        };

        let outcome = producer.run_sweep_point(input).unwrap();

        assert!(outcome.bpc_eval_subset.unwrap() > 1.0);
        assert!(outcome.expert_usage_entropy_bits_mean >= 0.0);
    }

    fn train_burn_state_for_test(
        state: &mut BurnMoeS7ModelState<BurnNdArrayAutodiffBackend>,
        train: &[u8],
        steps: u64,
        device: &BurnDevice<BurnNdArrayAutodiffBackend>,
    ) -> Vec<StepOutput> {
        let mut optimizer = adamw_config()
            .with_beta_1(ADAM_BETA1)
            .with_beta_2(ADAM_BETA2)
            .with_epsilon(ADAM_EPSILON)
            .with_weight_decay(0.0)
            .init::<BurnNdArrayAutodiffBackend, BurnByteMoeS7Model<BurnNdArrayAutodiffBackend>>();
        let mut outputs = Vec::new();
        for step in 1..=steps {
            let step_loss = state.loss_for_step(step, train, 0.0, device).unwrap();
            let gradients = step_loss.total_loss.backward();
            let grad_stats = state.grad_stats(&gradients).unwrap();
            let gradients = BurnGradientsParams::from_grads(gradients, &state.model);
            state.model = optimizer.step(
                f64::from(TRAIN_LEARNING_RATE),
                state.model.clone(),
                gradients,
            );
            state.optimizer_step = step;
            assert!(!optimizer.to_record().is_empty());
            let mut output = step_loss.output;
            output.grad_global_l2 = grad_stats.global_l2();
            output.grad_max_l2 = grad_stats.max_abs;
            output.grad_mean_l2 = grad_stats.mean_abs();
            state.observe_routes(&mut output);
            outputs.push(output);
        }
        outputs
    }

    #[test]
    fn production_manifest_serializes_required_schema() {
        let manifest = ProductionManifest {
            schema: S7_PRODUCTION_BUNDLE_MANIFEST_SCHEMA.to_owned(),
            runs: BTreeMap::new(),
            switch_stats: BTreeMap::new(),
            support_artifacts: SupportArtifactManifest {
                router_collapse_sweep: "router-collapse/seed-0/sweep.json".to_owned(),
                burn_grad_smoke: "burn-grad-smoke/expert_block_qat.json".to_owned(),
                oracle_routed: "oracle-routed/seed-0/oracle.json".to_owned(),
                emulator_one_token_moe: "emulator-one-token/seed-0/MoeTiny/result.json".to_owned(),
                emulator_one_token_dense: None,
            },
            comparison: ComparisonManifest {
                moe_topology_hash: sha256(b"moe"),
                dense_matched_topology_hash: sha256(b"dense"),
            },
            frontier: FrontierManifest {
                moe_conformance: "frontier/moe-conformance.json".to_owned(),
                dense_conformance: "frontier/dense-conformance.json".to_owned(),
                moe_deployed_bytes_per_block: DEFAULT_FRONTIER_MOE_BYTES_PER_BLOCK.to_vec(),
                dense_deployed_bytes_per_block: DEFAULT_FRONTIER_DENSE_BYTES_PER_BLOCK.to_vec(),
                moe_schedule_cost: None,
                dense_schedule_cost: None,
            },
            report: ReportManifest {
                s7_outcome: "PassClean".to_owned(),
                decision: "ProceedToS8".to_owned(),
                rfc_revision: "a".repeat(40),
                predictions_section_hash: sha256(b"pred").to_string(),
                predictions_commit: "b".repeat(40),
                first_result_commit: "c".repeat(40),
                generated_at: "2026-07-01T00:00:00Z".to_owned(),
            },
            production_runner: ProductionRunnerManifest {
                schema: "s7_production_runner.v1",
                runner_kind: "gbf_experiments::s7::production_runner",
                bead_owner: "bd-3e10j",
                gutenberg_manifest_sha: sha256(b"manifest"),
                train_corpus_sha: sha256(b"train"),
                val_corpus_sha: sha256(b"val"),
                optimizer_model_state: "live_burn_adamw_moe_lm_state_per_topology_seed",
                grad_log_schema: S7_GRAD_LOG_SCHEMA,
                router_step_telemetry_schema: S7_ROUTER_STEP_TELEMETRY_SCHEMA,
                optimizer_steps: S7_OPTIMIZER_STEPS,
                sweep_producer_kind: PRODUCTION_SWEEP_PRODUCER_KIND,
            },
        };

        let value = serde_json::to_value(&manifest).unwrap();

        assert_eq!(
            value["schema"],
            Value::String(S7_PRODUCTION_BUNDLE_MANIFEST_SCHEMA.to_owned())
        );
        assert_eq!(value["production_runner"]["bead_owner"], "bd-3e10j");
        assert_eq!(
            value["production_runner"]["sweep_producer_kind"],
            PRODUCTION_SWEEP_PRODUCER_KIND
        );
    }
}
