//! S7 train-run helper surface.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gbf_artifact::{
    GradNormSummary, S7_N_BLOCKS, S7_OPTIMIZER_STEPS, S7Completion, S7RunLog, S7SchemaError,
    S7ScoreReport, S7Topology,
};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::s7::schema::{RouterStepTelemetry, RouterTelemetryError};
use crate::s7::state::{S7StateError, S7TeacherFreezeBoundary, S7TrainRunId, S7TrainRunState};

/// Result of the S7-local train-run helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7TrainAttempt {
    /// Identity of the attempted run.
    pub run_id: S7TrainRunId,
    /// Internal Phase A teacher-freeze boundary.
    pub phase_a_teacher: S7TeacherFreezeBoundary,
    /// Deterministic same-topology, same-seed teacher checkpoint hash for `s7_run_log.v1`.
    pub frozen_teacher_checkpoint_sha: Hash256,
}

/// Execute the S7-local train-run helper through the internal teacher-freeze boundary.
///
/// This helper intentionally stops after proving the Phase A freeze contract;
/// the full training-loop producer is owned by the end-to-end S7 run harness.
pub fn s7_train_run(topology: S7Topology, seed: u64) -> Result<S7TrainAttempt, S7StateError> {
    let mut state = S7TrainRunState::baseline_matched(topology, seed);
    let phase_a_teacher = state.freeze_teacher_at_phase_a_boundary()?;
    phase_a_teacher.emit_trace();

    Ok(S7TrainAttempt {
        run_id: state.run_id().clone(),
        frozen_teacher_checkpoint_sha: phase_a_teacher.teacher_checkpoint_sha,
        phase_a_teacher,
    })
}

/// Inputs for materializing one completed S7 production run into the closure packet layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7CompletedRunArtifactInputs {
    /// Packet/repository root where `experiments/S7/...` should be written.
    pub root: PathBuf,
    /// Topology expected for the run and score artifacts.
    pub topology: S7Topology,
    /// Seed expected for the run and score artifacts.
    pub seed: u64,
    /// Completed `s7_run_log.v1` produced by the real S7 training runner.
    pub run_log: PathBuf,
    /// `s7_score.v1` produced for the final checkpoint and Gutenberg validation bytes.
    pub score: PathBuf,
    /// Per-step `s7_grad_log.v1` JSONL produced by the training runner.
    pub grad_log: PathBuf,
    /// `s7_router_step_telemetry.v1` JSONL for MoE, or an empty file for dense matched runs.
    pub router_step_telemetry: PathBuf,
}

/// Canonical packet paths and hashes written for one completed S7 run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7MaterializedRunArtifacts {
    /// Canonical run-log packet path.
    pub run_log_path: PathBuf,
    /// Canonical score packet path.
    pub score_path: PathBuf,
    /// Canonical gradient-log packet path.
    pub grad_log_path: PathBuf,
    /// Canonical router-step telemetry packet path.
    pub router_step_telemetry_path: PathBuf,
    /// Verified `run_log_self_hash`.
    pub run_log_self_hash: Hash256,
    /// Verified `score_self_hash`.
    pub score_self_hash: Hash256,
}

/// Validate and materialize one completed S7 run into the packet layout expected by closure gates.
///
/// This function is intentionally an artifact landing pad rather than a
/// training loop. It refuses fixture-shaped/diverged/incomplete artifacts and
/// re-emits canonical JSON/JSONL so the eventual production runner has a
/// strict, repeatable way to populate `experiments/S7/runs/...` and
/// `experiments/S7/scores/...`.
pub fn materialize_completed_run_artifacts(
    inputs: &S7CompletedRunArtifactInputs,
) -> Result<S7MaterializedRunArtifacts, S7RunMaterializeError> {
    let run_log: S7RunLog = read_json_file(&inputs.run_log)?;
    validate_completed_run_log(&run_log, inputs)?;
    let score: S7ScoreReport = read_json_file(&inputs.score)?;
    validate_score(&score, inputs)?;

    let grad_log_records = read_grad_log(&inputs.grad_log, &run_log)?;
    let telemetry_records = read_router_step_telemetry(&inputs.router_step_telemetry, inputs)?;

    let topology_path = topology_path_segment(&inputs.topology);
    let run_dir = inputs
        .root
        .join("experiments/S7/runs")
        .join(topology_path)
        .join(format!("seed-{}", inputs.seed));
    let score_dir = inputs
        .root
        .join("experiments/S7/scores")
        .join(topology_path)
        .join(format!("seed-{}", inputs.seed));
    let run_log_path = run_dir.join("run-log.json");
    let grad_log_path = run_dir.join("grad-log.jsonl");
    let router_step_telemetry_path = run_dir.join("router-step-telemetry.jsonl");
    let score_path = score_dir.join("score.json");

    write_canonical_json(&run_log_path, &run_log.canonical_json_bytes()?)?;
    write_canonical_json(&score_path, &score.canonical_json_bytes()?)?;
    write_jsonl_values(&grad_log_path, &grad_log_records)?;
    write_router_telemetry_jsonl(&router_step_telemetry_path, &telemetry_records)?;

    Ok(S7MaterializedRunArtifacts {
        run_log_path,
        score_path,
        grad_log_path,
        router_step_telemetry_path,
        run_log_self_hash: run_log.run_log_self_hash,
        score_self_hash: score.score_self_hash,
    })
}

/// Return the canonical packet path segment for an S7 topology.
#[must_use]
pub const fn topology_path_segment(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

fn validate_completed_run_log(
    run_log: &S7RunLog,
    inputs: &S7CompletedRunArtifactInputs,
) -> Result<(), S7RunMaterializeError> {
    if run_log.seed != inputs.seed {
        return Err(S7RunMaterializeError::IdentityMismatch {
            artifact: "run-log",
            field: "seed",
            expected: inputs.seed.to_string(),
            observed: run_log.seed.to_string(),
        });
    }
    if run_log.topology != inputs.topology {
        return Err(S7RunMaterializeError::IdentityMismatch {
            artifact: "run-log",
            field: "topology",
            expected: topology_path_segment(&inputs.topology).to_owned(),
            observed: topology_path_segment(&run_log.topology).to_owned(),
        });
    }
    if run_log.completion != S7Completion::Completed {
        return Err(S7RunMaterializeError::IncompleteRun {
            observed: format!("{:?}", run_log.completion),
        });
    }
    if run_log.losses.len() != S7_OPTIMIZER_STEPS as usize {
        return Err(S7RunMaterializeError::InvalidCompletedRunLength {
            field: "losses",
            observed: run_log.losses.len(),
            expected: S7_OPTIMIZER_STEPS as usize,
        });
    }
    reject_zero_hash("run-log", "train_config_hash", run_log.train_config_hash)?;
    reject_zero_hash(
        "run-log",
        "model_topology_hash",
        run_log.model_topology_hash,
    )?;
    reject_zero_hash("run-log", "loss_config_hash", run_log.loss_config_hash)?;
    reject_zero_hash(
        "run-log",
        "phase_schedule_hash",
        run_log.phase_schedule_hash,
    )?;
    reject_zero_hash(
        "run-log",
        "frozen_teacher_checkpoint_sha",
        run_log
            .frozen_teacher_checkpoint_sha
            .ok_or(S7RunMaterializeError::MissingHash {
                artifact: "run-log",
                field: "frozen_teacher_checkpoint_sha",
            })?,
    )?;
    match inputs.topology {
        S7Topology::MoeTiny => {
            reject_zero_hash(
                "run-log",
                "router_config_hash",
                run_log
                    .router_config_hash
                    .ok_or(S7RunMaterializeError::MissingHash {
                        artifact: "run-log",
                        field: "router_config_hash",
                    })?,
            )?;
            reject_zero_hash(
                "run-log",
                "expert_block_config_hash",
                run_log
                    .expert_block_config_hash
                    .ok_or(S7RunMaterializeError::MissingHash {
                        artifact: "run-log",
                        field: "expert_block_config_hash",
                    })?,
            )?;
        }
        S7Topology::MoeTinyDenseMatched => {
            if run_log.router_config_hash.is_some() || run_log.expert_block_config_hash.is_some() {
                return Err(S7RunMaterializeError::DenseRunHasRouterHashes);
            }
        }
    }
    for (step, diagnostics) in &run_log.losses {
        let expected = diagnostics.computed_self_hash()?;
        if diagnostics.diagnostics_self_hash != expected {
            return Err(S7RunMaterializeError::SelfHashMismatch {
                artifact: "run-log.losses",
                field: "diagnostics_self_hash",
                step: Some(*step),
                expected,
                observed: diagnostics.diagnostics_self_hash,
            });
        }
    }
    let expected = run_log.computed_self_hash()?;
    if run_log.run_log_self_hash != expected {
        return Err(S7RunMaterializeError::SelfHashMismatch {
            artifact: "run-log",
            field: "run_log_self_hash",
            step: None,
            expected,
            observed: run_log.run_log_self_hash,
        });
    }
    Ok(())
}

fn validate_score(
    score: &S7ScoreReport,
    inputs: &S7CompletedRunArtifactInputs,
) -> Result<(), S7RunMaterializeError> {
    if score.seed != inputs.seed {
        return Err(S7RunMaterializeError::IdentityMismatch {
            artifact: "score",
            field: "seed",
            expected: inputs.seed.to_string(),
            observed: score.seed.to_string(),
        });
    }
    if score.topology != inputs.topology {
        return Err(S7RunMaterializeError::IdentityMismatch {
            artifact: "score",
            field: "topology",
            expected: topology_path_segment(&inputs.topology).to_owned(),
            observed: topology_path_segment(&score.topology).to_owned(),
        });
    }
    reject_zero_hash("score", "checkpoint_sha", score.checkpoint_sha)?;
    reject_zero_hash("score", "corpus_val_sha", score.corpus_val_sha)?;
    let expected = score.computed_self_hash()?;
    if score.score_self_hash != expected {
        return Err(S7RunMaterializeError::SelfHashMismatch {
            artifact: "score",
            field: "score_self_hash",
            step: None,
            expected,
            observed: score.score_self_hash,
        });
    }
    Ok(())
}

fn read_grad_log(path: &Path, run_log: &S7RunLog) -> Result<Vec<Value>, S7RunMaterializeError> {
    let records = read_jsonl_file(path, "grad log")?;
    if records.len() != run_log.grad_norms.len() {
        return Err(S7RunMaterializeError::JsonlLengthMismatch {
            label: "grad log",
            observed: records.len(),
            expected: run_log.grad_norms.len(),
        });
    }
    let expected_by_step = run_log
        .grad_norms
        .iter()
        .map(|(step, summary)| Ok((*step, serde_json::to_value(summary)?)))
        .collect::<Result<BTreeMap<_, _>, serde_json::Error>>()?;
    for (index, record) in records.iter().enumerate() {
        let location = JsonlLocation {
            label: "grad log",
            line: index + 1,
        };
        require_json_field(
            record,
            location,
            "schema",
            Value::String("s7_grad_log.v1".to_owned()),
        )?;
        require_json_field(record, location, "seed", Value::from(run_log.seed))?;
        let train_step = positive_u64_field(record, location, "train_step")?;
        let expected_step = u64::try_from(index + 1)
            .map_err(|_| S7RunMaterializeError::LengthOverflow { label: "grad log" })?;
        if train_step != expected_step {
            return Err(S7RunMaterializeError::JsonlFieldMismatch {
                label: "grad log",
                line: index + 1,
                field: "train_step",
                expected: expected_step.to_string(),
                observed: train_step.to_string(),
            });
        }
        let grad_norms =
            record
                .get("grad_norms")
                .ok_or(S7RunMaterializeError::MissingJsonlField {
                    label: "grad log",
                    line: index + 1,
                    field: "grad_norms",
                })?;
        let expected =
            expected_by_step
                .get(&train_step)
                .ok_or(S7RunMaterializeError::JsonlFieldMismatch {
                    label: "grad log",
                    line: index + 1,
                    field: "train_step",
                    expected: "step from run-log grad_norms".to_owned(),
                    observed: train_step.to_string(),
                })?;
        if grad_norms != expected {
            return Err(S7RunMaterializeError::JsonlFieldMismatch {
                label: "grad log",
                line: index + 1,
                field: "grad_norms",
                expected: expected.to_string(),
                observed: grad_norms.to_string(),
            });
        }
        let _: GradNormSummary = serde_json::from_value(grad_norms.clone())?;
    }
    Ok(records)
}

fn read_router_step_telemetry(
    path: &Path,
    inputs: &S7CompletedRunArtifactInputs,
) -> Result<Vec<RouterStepTelemetry>, S7RunMaterializeError> {
    let records = read_jsonl_file(path, "router-step telemetry")?;
    match inputs.topology {
        S7Topology::MoeTinyDenseMatched => {
            if !records.is_empty() {
                return Err(S7RunMaterializeError::DenseTelemetryNotEmpty {
                    observed: records.len(),
                });
            }
            Ok(Vec::new())
        }
        S7Topology::MoeTiny => {
            if records.is_empty() {
                return Err(S7RunMaterializeError::MissingMoeTelemetry);
            }
            let mut layers_by_step: BTreeMap<u64, BTreeSet<u32>> = BTreeMap::new();
            let mut telemetry_records = Vec::with_capacity(records.len());
            for (index, record) in records.into_iter().enumerate() {
                let telemetry = unwrap_router_step_telemetry(record, index + 1)?;
                if telemetry.seed != inputs.seed {
                    return Err(S7RunMaterializeError::JsonlFieldMismatch {
                        label: "router-step telemetry",
                        line: index + 1,
                        field: "seed",
                        expected: inputs.seed.to_string(),
                        observed: telemetry.seed.to_string(),
                    });
                }
                telemetry.verify_self_hash()?;
                layers_by_step
                    .entry(telemetry.train_step)
                    .or_default()
                    .insert(telemetry.layer_id);
                telemetry_records.push(telemetry);
            }
            let expected_layers = (0..u32::from(S7_N_BLOCKS)).collect::<BTreeSet<_>>();
            for (train_step, layers) in layers_by_step {
                if layers != expected_layers {
                    return Err(S7RunMaterializeError::TelemetryLayerCoverage {
                        train_step,
                        observed: layers.into_iter().collect(),
                    });
                }
            }
            Ok(telemetry_records)
        }
    }
}

fn unwrap_router_step_telemetry(
    record: Value,
    line: usize,
) -> Result<RouterStepTelemetry, S7RunMaterializeError> {
    let Some(object) = record.as_object() else {
        return Err(S7RunMaterializeError::JsonlRecordNotObject {
            label: "router-step telemetry",
            line,
        });
    };
    if let Some(payload_text) = object.get("telemetry_canonical_json") {
        let Some(payload_text) = payload_text.as_str() else {
            return Err(S7RunMaterializeError::JsonlFieldMismatch {
                label: "router-step telemetry",
                line,
                field: "telemetry_canonical_json",
                expected: "JSON string".to_owned(),
                observed: payload_text.to_string(),
            });
        };
        if object.get("event_name").and_then(Value::as_str) != Some("s7.router.step") {
            return Err(S7RunMaterializeError::JsonlFieldMismatch {
                label: "router-step telemetry",
                line,
                field: "event_name",
                expected: "s7.router.step".to_owned(),
                observed: object
                    .get("event_name")
                    .map_or_else(|| "missing".to_owned(), ToString::to_string),
            });
        }
        let telemetry: RouterStepTelemetry = serde_json::from_str(payload_text)?;
        if let Some(flat_hash) = object.get("telemetry_self_hash")
            && flat_hash != &Value::String(telemetry.telemetry_self_hash.to_string())
        {
            return Err(S7RunMaterializeError::JsonlFieldMismatch {
                label: "router-step telemetry",
                line,
                field: "telemetry_self_hash",
                expected: telemetry.telemetry_self_hash.to_string(),
                observed: flat_hash.to_string(),
            });
        }
        Ok(telemetry)
    } else {
        Ok(serde_json::from_value(record)?)
    }
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, S7RunMaterializeError> {
    let bytes = fs::read(path).map_err(|source| S7RunMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| S7RunMaterializeError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn read_jsonl_file(path: &Path, label: &'static str) -> Result<Vec<Value>, S7RunMaterializeError> {
    let text = fs::read_to_string(path).map_err(|source| S7RunMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut records = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(S7RunMaterializeError::BlankJsonlLine {
                label,
                line: line_index + 1,
            });
        }
        let record = serde_json::from_str::<Value>(line).map_err(|source| {
            S7RunMaterializeError::JsonLine {
                label,
                line: line_index + 1,
                source,
            }
        })?;
        if !record.is_object() {
            return Err(S7RunMaterializeError::JsonlRecordNotObject {
                label,
                line: line_index + 1,
            });
        }
        records.push(record);
    }
    Ok(records)
}

fn write_canonical_json(path: &Path, bytes: &[u8]) -> Result<(), S7RunMaterializeError> {
    let mut bytes = bytes.to_vec();
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

fn write_jsonl_values(path: &Path, records: &[Value]) -> Result<(), S7RunMaterializeError> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(CanonicalJson::value_to_vec(record)?);
        bytes.push(b'\n');
    }
    write_bytes(path, &bytes)
}

fn write_router_telemetry_jsonl(
    path: &Path,
    records: &[RouterStepTelemetry],
) -> Result<(), S7RunMaterializeError> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(record.canonical_json_bytes()?);
        bytes.push(b'\n');
    }
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), S7RunMaterializeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7RunMaterializeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| S7RunMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn reject_zero_hash(
    artifact: &'static str,
    field: &'static str,
    hash: Hash256,
) -> Result<(), S7RunMaterializeError> {
    if hash == Hash256::ZERO {
        Err(S7RunMaterializeError::ZeroHash { artifact, field })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct JsonlLocation {
    label: &'static str,
    line: usize,
}

fn require_json_field(
    record: &Value,
    location: JsonlLocation,
    field: &'static str,
    expected: Value,
) -> Result<(), S7RunMaterializeError> {
    let observed = record
        .get(field)
        .ok_or(S7RunMaterializeError::MissingJsonlField {
            label: location.label,
            line: location.line,
            field,
        })?;
    if observed != &expected {
        return Err(S7RunMaterializeError::JsonlFieldMismatch {
            label: location.label,
            line: location.line,
            field,
            expected: expected.to_string(),
            observed: observed.to_string(),
        });
    }
    Ok(())
}

fn positive_u64_field(
    record: &Value,
    location: JsonlLocation,
    field: &'static str,
) -> Result<u64, S7RunMaterializeError> {
    let observed = record
        .get(field)
        .ok_or(S7RunMaterializeError::MissingJsonlField {
            label: location.label,
            line: location.line,
            field,
        })?;
    let Some(value) = observed.as_u64().filter(|value| *value > 0) else {
        return Err(S7RunMaterializeError::JsonlFieldMismatch {
            label: location.label,
            line: location.line,
            field,
            expected: "positive integer".to_owned(),
            observed: observed.to_string(),
        });
    };
    Ok(value)
}

/// Errors raised while materializing S7 completed-run artifacts.
#[derive(Debug)]
pub enum S7RunMaterializeError {
    /// Filesystem I/O failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source error.
        source: io::Error,
    },
    /// JSON decoding failed.
    Json {
        /// Path being decoded.
        path: String,
        /// Source error.
        source: serde_json::Error,
    },
    /// JSONL decoding failed.
    JsonLine {
        /// Log label.
        label: &'static str,
        /// One-based line number.
        line: usize,
        /// Source error.
        source: serde_json::Error,
    },
    /// Artifact schema validation failed.
    ArtifactSchema(S7SchemaError),
    /// Router telemetry validation failed.
    RouterTelemetry(RouterTelemetryError),
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// A run was not completed, so it is not a closure-candidate run product.
    IncompleteRun {
        /// Observed completion tag.
        observed: String,
    },
    /// Completed-run series length was not the RFC-pinned production length.
    InvalidCompletedRunLength {
        /// Field with the bad length.
        field: &'static str,
        /// Observed length.
        observed: usize,
        /// Expected length.
        expected: usize,
    },
    /// Artifact identity did not match the requested seed/topology.
    IdentityMismatch {
        /// Artifact label.
        artifact: &'static str,
        /// Field label.
        field: &'static str,
        /// Expected value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// A required hash field was missing.
    MissingHash {
        /// Artifact label.
        artifact: &'static str,
        /// Field label.
        field: &'static str,
    },
    /// A production lineage hash used the all-zero test sentinel.
    ZeroHash {
        /// Artifact label.
        artifact: &'static str,
        /// Field label.
        field: &'static str,
    },
    /// A dense matched run carried router/expert hashes.
    DenseRunHasRouterHashes,
    /// A self-hash did not match the artifact payload.
    SelfHashMismatch {
        /// Artifact label.
        artifact: &'static str,
        /// Self-hash field.
        field: &'static str,
        /// Optional train step.
        step: Option<u64>,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// A JSONL log had the wrong record count.
    JsonlLengthMismatch {
        /// Log label.
        label: &'static str,
        /// Observed count.
        observed: usize,
        /// Expected count.
        expected: usize,
    },
    /// A JSONL line was blank.
    BlankJsonlLine {
        /// Log label.
        label: &'static str,
        /// One-based line number.
        line: usize,
    },
    /// A JSONL record was not an object.
    JsonlRecordNotObject {
        /// Log label.
        label: &'static str,
        /// One-based line number.
        line: usize,
    },
    /// A required JSONL field was missing.
    MissingJsonlField {
        /// Log label.
        label: &'static str,
        /// One-based line number.
        line: usize,
        /// Field label.
        field: &'static str,
    },
    /// A JSONL field had an unexpected value.
    JsonlFieldMismatch {
        /// Log label.
        label: &'static str,
        /// One-based line number.
        line: usize,
        /// Field label.
        field: &'static str,
        /// Expected value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// Dense matched router telemetry was not empty.
    DenseTelemetryNotEmpty {
        /// Observed record count.
        observed: usize,
    },
    /// MoE router telemetry was missing.
    MissingMoeTelemetry,
    /// MoE telemetry did not cover every layer for a sampled step.
    TelemetryLayerCoverage {
        /// Training step.
        train_step: u64,
        /// Observed layer ids.
        observed: Vec<u32>,
    },
    /// A length conversion overflowed.
    LengthOverflow {
        /// Log label.
        label: &'static str,
    },
}

impl fmt::Display for S7RunMaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: invalid JSON: {source}"),
            Self::JsonLine {
                label,
                line,
                source,
            } => write!(f, "{label} line {line}: invalid JSON: {source}"),
            Self::ArtifactSchema(error) => write!(f, "{error}"),
            Self::RouterTelemetry(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::IncompleteRun { observed } => {
                write!(
                    f,
                    "S7 materialize-run requires completed run-log, observed {observed}"
                )
            }
            Self::InvalidCompletedRunLength {
                field,
                observed,
                expected,
            } => write!(
                f,
                "S7 completed run-log {field} length must be {expected}, observed {observed}"
            ),
            Self::IdentityMismatch {
                artifact,
                field,
                expected,
                observed,
            } => write!(
                f,
                "S7 {artifact} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::MissingHash { artifact, field } => {
                write!(f, "S7 {artifact} missing required {field}")
            }
            Self::ZeroHash { artifact, field } => {
                write!(
                    f,
                    "S7 {artifact} {field} must not use the all-zero test sentinel"
                )
            }
            Self::DenseRunHasRouterHashes => {
                f.write_str("S7 dense matched run-log must keep router/expert hashes null")
            }
            Self::SelfHashMismatch {
                artifact,
                field,
                step,
                expected,
                observed,
            } => {
                if let Some(step) = step {
                    write!(
                        f,
                        "S7 {artifact} step {step} {field} mismatch: expected {expected}, observed {observed}"
                    )
                } else {
                    write!(
                        f,
                        "S7 {artifact} {field} mismatch: expected {expected}, observed {observed}"
                    )
                }
            }
            Self::JsonlLengthMismatch {
                label,
                observed,
                expected,
            } => write!(
                f,
                "S7 {label} must contain {expected} records, observed {observed}"
            ),
            Self::BlankJsonlLine { label, line } => {
                write!(f, "S7 {label} line {line} must not be blank")
            }
            Self::JsonlRecordNotObject { label, line } => {
                write!(f, "S7 {label} line {line} must be a JSON object")
            }
            Self::MissingJsonlField { label, line, field } => {
                write!(f, "S7 {label} line {line} missing {field}")
            }
            Self::JsonlFieldMismatch {
                label,
                line,
                field,
                expected,
                observed,
            } => write!(
                f,
                "S7 {label} line {line} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::DenseTelemetryNotEmpty { observed } => write!(
                f,
                "S7 dense matched router-step telemetry must be empty, observed {observed} records"
            ),
            Self::MissingMoeTelemetry => {
                f.write_str("S7 MoE router-step telemetry must contain at least one record")
            }
            Self::TelemetryLayerCoverage {
                train_step,
                observed,
            } => write!(
                f,
                "S7 MoE router-step telemetry must cover layers 0..3 for train_step {train_step}; observed {observed:?}"
            ),
            Self::LengthOverflow { label } => write!(f, "S7 {label} length overflowed"),
        }
    }
}

impl std::error::Error for S7RunMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } | Self::JsonLine { source, .. } => Some(source),
            Self::ArtifactSchema(error) => Some(error),
            Self::RouterTelemetry(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::IncompleteRun { .. }
            | Self::InvalidCompletedRunLength { .. }
            | Self::IdentityMismatch { .. }
            | Self::MissingHash { .. }
            | Self::ZeroHash { .. }
            | Self::DenseRunHasRouterHashes
            | Self::SelfHashMismatch { .. }
            | Self::JsonlLengthMismatch { .. }
            | Self::BlankJsonlLine { .. }
            | Self::JsonlRecordNotObject { .. }
            | Self::MissingJsonlField { .. }
            | Self::JsonlFieldMismatch { .. }
            | Self::DenseTelemetryNotEmpty { .. }
            | Self::MissingMoeTelemetry
            | Self::TelemetryLayerCoverage { .. }
            | Self::LengthOverflow { .. } => None,
        }
    }
}

impl From<S7SchemaError> for S7RunMaterializeError {
    fn from(error: S7SchemaError) -> Self {
        Self::ArtifactSchema(error)
    }
}

impl From<RouterTelemetryError> for S7RunMaterializeError {
    fn from(error: RouterTelemetryError) -> Self {
        Self::RouterTelemetry(error)
    }
}

impl From<CanonicalJsonError> for S7RunMaterializeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<serde_json::Error> for S7RunMaterializeError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json {
            path: "<in-memory>".to_owned(),
            source,
        }
    }
}
