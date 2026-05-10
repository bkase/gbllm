//! Workload manifest schema consumed by pipeline-entry validation.

use std::fmt;
use std::io;
use std::path::Path;

use gbf_foundation::{BlobRef, Hash256};
pub use gbf_foundation::{GoldenVectorId, WorkloadId};
use gbf_policy::model_profile::ModelSizeProfile;
use serde::{Deserialize, Serialize};

pub const WORKLOAD_MANIFEST_SCHEMA: &str = "gbf.workload_manifest.v1";
pub const V0_SUCCESS_ENVELOPE_GATE: &str = "v0_success_envelope";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifestRef {
    pub id: WorkloadId,
    pub manifest_hash: Hash256,
    pub locator: WorkloadLocator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WorkloadLocator {
    Path { path: String },
    Inline { blob: BlobRef },
    RegistryEntry { registry: RegistryId, key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifest {
    pub id: WorkloadId,
    pub schema_version: WorkloadSchemaVersion,
    pub self_hash: Hash256,
    pub golden_vectors: Vec<GoldenVectorRef>,
    #[serde(default)]
    pub future_fields: WorkloadFuturePlaceholder,
}

impl WorkloadManifest {
    pub fn from_toml_str(
        input: &str,
    ) -> Result<DenseBaselineWorkloadManifest, WorkloadManifestError> {
        DenseBaselineWorkloadManifest::from_toml_str(input)
    }

    pub fn from_toml_file(
        path: impl AsRef<Path>,
    ) -> Result<DenseBaselineWorkloadManifest, WorkloadManifestError> {
        DenseBaselineWorkloadManifest::from_toml_file(path)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadFuturePlaceholder {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSchemaVersion {
    pub epoch: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenVectorRef {
    pub id: GoldenVectorId,
    pub manifest_hash: Hash256,
}

/// Identifier scoped to workload registry namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistryId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenseBaselineWorkloadManifest {
    pub schema: String,
    pub schema_version: String,
    pub id: WorkloadId,
    pub class: WorkloadClass,
    pub model: WorkloadModelSelection,
    pub intent: WorkloadIntent,
    pub corpus: WorkloadCorpusRef,
    pub prompts: PromptSuite,
    pub observation: ObservationPolicy,
    pub execution: ExecutionMatrix,
    pub acceptance: AcceptanceMatrix,
    pub deferred_scope: Vec<String>,
}

impl DenseBaselineWorkloadManifest {
    pub fn from_toml_str(input: &str) -> Result<Self, WorkloadManifestError> {
        let manifest: Self = toml::from_str(input).map_err(WorkloadManifestError::Toml)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, WorkloadManifestError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| WorkloadManifestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    pub fn validate(&self) -> Result<(), WorkloadManifestError> {
        if self.schema != WORKLOAD_MANIFEST_SCHEMA {
            return Err(WorkloadManifestError::SchemaMismatch {
                expected: WORKLOAD_MANIFEST_SCHEMA,
                observed: self.schema.clone(),
            });
        }

        if !matches!(
            self.model.profile,
            ModelSizeProfile::Toy0 | ModelSizeProfile::Toy1
        ) {
            return Err(WorkloadManifestError::UnsupportedDenseProfile {
                profile: format!("{:?}", self.model.profile),
            });
        }

        if self.model.ffn_path != FfnPathSelection::Dense {
            return Err(WorkloadManifestError::NotDenseBaseline);
        }

        if !self.intent.dense_run_end_to_end {
            return Err(WorkloadManifestError::DenseIntentMissing);
        }

        if self.prompts.cases.is_empty() {
            return Err(WorkloadManifestError::EmptyPromptSuite);
        }

        for case in &self.prompts.cases {
            if case.prompt.is_empty() {
                return Err(WorkloadManifestError::EmptyPromptCase {
                    id: case.id.clone(),
                });
            }
        }

        if self.prompts.min_prompt_chars == 0
            || self.prompts.max_prompt_chars < self.prompts.min_prompt_chars
        {
            return Err(WorkloadManifestError::InvalidPromptCharBounds {
                min: self.prompts.min_prompt_chars,
                max: self.prompts.max_prompt_chars,
            });
        }

        if self.prompts.min_generated_chars == 0 {
            return Err(WorkloadManifestError::MissingGenerationTarget);
        }

        if !self.execution.denotational
            || !self.execution.artifact
            || !self.execution.schedule
            || !self.execution.harness
        {
            return Err(WorkloadManifestError::IncompleteEndToEndExecution);
        }

        if self.acceptance.conformance_gate.envelope.as_str() != V0_SUCCESS_ENVELOPE_GATE {
            return Err(WorkloadManifestError::MissingV0SuccessEnvelope {
                observed: self.acceptance.conformance_gate.envelope.clone(),
            });
        }

        Ok(())
    }
}

pub fn read_workload_manifest(
    path: impl AsRef<Path>,
) -> Result<DenseBaselineWorkloadManifest, WorkloadManifestError> {
    DenseBaselineWorkloadManifest::from_toml_file(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadClass {
    Conformance,
    Performance,
    Endurance,
    Persistence,
    Ui,
    Quality,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadModelSelection {
    pub profile: ModelSizeProfile,
    pub ffn_path: FfnPathSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FfnPathSelection {
    Dense,
    Moe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadIntent {
    pub dense_run_end_to_end: bool,
    pub run_goal: String,
    pub parity_report_owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadCorpusRef {
    pub manifest_path: String,
    pub split: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSuite {
    pub min_prompt_chars: u16,
    pub max_prompt_chars: u16,
    pub min_generated_chars: u16,
    pub seeds: Vec<u64>,
    pub cases: Vec<PromptCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCase {
    pub id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPolicy {
    pub checkpoints: CheckpointSelection,
    pub trace_level: TraceLevel,
    pub compare_domain: CompareDomain,
    pub determinism_requirement: DeterminismRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSelection {
    SemanticAndOperational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceLevel {
    Summary,
    Checkpoints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareDomain {
    TokenLogits,
    GeneratedBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismRequirement {
    SeededDecode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionMatrix {
    pub denotational: bool,
    pub artifact: bool,
    pub schedule: bool,
    pub harness: bool,
    pub hardware: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceMatrix {
    pub conformance_gate: ConformanceGate,
    pub experience_gate: ExperienceGate,
    pub quality_gate: QualityGate,
    pub deployability_gate: DeployabilityGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceGate {
    pub envelope: String,
    pub report_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperienceGate {
    pub max_repetition_collapse_run: u16,
    pub valid_charset_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityGate {
    pub baseline: String,
    pub metric: String,
    pub relation: GateRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateRelation {
    LessThan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployabilityGate {
    pub runtime_chrome_budget: String,
    pub emulator_token_smoke: bool,
}

#[derive(Debug)]
pub enum WorkloadManifestError {
    Io {
        path: String,
        source: io::Error,
    },
    Toml(toml::de::Error),
    SchemaMismatch {
        expected: &'static str,
        observed: String,
    },
    UnsupportedDenseProfile {
        profile: String,
    },
    NotDenseBaseline,
    DenseIntentMissing,
    EmptyPromptSuite,
    EmptyPromptCase {
        id: String,
    },
    InvalidPromptCharBounds {
        min: u16,
        max: u16,
    },
    MissingGenerationTarget,
    IncompleteEndToEndExecution,
    MissingV0SuccessEnvelope {
        observed: String,
    },
}

impl fmt::Display for WorkloadManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::SchemaMismatch { expected, observed } => {
                write!(
                    f,
                    "expected workload manifest schema {expected:?}, observed {observed:?}"
                )
            }
            Self::UnsupportedDenseProfile { profile } => {
                write!(
                    f,
                    "dense baseline supports Toy0 or Toy1, observed {profile}"
                )
            }
            Self::NotDenseBaseline => write!(f, "workload model selection is not dense-only"),
            Self::DenseIntentMissing => write!(f, "dense end-to-end intent is not asserted"),
            Self::EmptyPromptSuite => write!(f, "prompt suite must contain at least one case"),
            Self::EmptyPromptCase { id } => write!(f, "prompt case {id:?} is empty"),
            Self::InvalidPromptCharBounds { min, max } => {
                write!(f, "invalid prompt char bounds: min={min}, max={max}")
            }
            Self::MissingGenerationTarget => write!(f, "minimum generated chars must be non-zero"),
            Self::IncompleteEndToEndExecution => {
                write!(f, "dense baseline must exercise denotation through harness")
            }
            Self::MissingV0SuccessEnvelope { observed } => write!(
                f,
                "expected conformance gate {V0_SUCCESS_ENVELOPE_GATE:?}, observed {observed:?}"
            ),
        }
    }
}

impl std::error::Error for WorkloadManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::SchemaMismatch { .. }
            | Self::UnsupportedDenseProfile { .. }
            | Self::NotDenseBaseline
            | Self::DenseIntentMissing
            | Self::EmptyPromptSuite
            | Self::EmptyPromptCase { .. }
            | Self::InvalidPromptCharBounds { .. }
            | Self::MissingGenerationTarget
            | Self::IncompleteEndToEndExecution
            | Self::MissingV0SuccessEnvelope { .. } => None,
        }
    }
}
