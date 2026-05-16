//! Workload manifest schema consumed by pipeline-entry validation.

use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::path::Path;

use gbf_artifact::{DecodeMode, DecodeSpec, LexicalError, TextCharSeq};
use gbf_foundation::{BlobRef, CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
pub use gbf_foundation::{GoldenVectorId, GoldenVectorRef, WorkloadId};
use gbf_policy::model_profile::ModelSizeProfile;
use serde::{Deserialize, Deserializer, Serialize};

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

pub const V0_SUCCESS_WORKLOAD_SCHEMA: &str = "workload_manifest.v1";
pub const V0_SUCCESS_WORKLOAD_SCHEMA_VERSION: &str = "1";
pub const V0_SUCCESS_WORKLOAD_ID: &str = "v0_success";
pub const V0_SUCCESS_PROMPT_COUNT: usize = 8;
pub const V0_SUCCESS_AGREEMENT_SUBSET_SIZE: usize = 3;
pub const V0_SUCCESS_PROMPT_MIN_CHARS: usize = 64;
pub const V0_SUCCESS_PROMPT_MAX_CHARS: usize = 128;
pub const V0_SUCCESS_EXPECTED_MIN_GEN: u32 = 128;
pub const V0_SUCCESS_EXPECTED_MAX_REPEAT: u32 = 8;
pub const V0_SUCCESS_HELD_OUT_CHAPTER_SHA: &str =
    "sha256:7ab0b4bcf5da7a28d0cfc4c15b579159bf4b922b5080a914ec00986ae80c6653";
pub const V0_SUCCESS_WORKLOAD_LOADED_EVENT: &str = "s3::workload::loaded";
pub const V0_SUCCESS_WORKLOAD_LOG_TARGET: &str = "gbf_workload::manifest";

/// S3 v0_success workload manifest.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifest_v0 {
    #[serde(deserialize_with = "deserialize_v0_success_schema")]
    pub schema: String,
    pub id: WorkloadId,
    pub class: WorkloadClass,
    pub prompts: Vec<PromptCase>,
    pub seeds: Vec<u64>,
    pub session: SessionProfile_S3,
    pub observation: ObservationPolicy_S3,
    pub execution: ExecutionMatrix_S3,
    pub acceptance: AcceptanceMatrix_S3,
    pub workload_self_hash: Hash256,
}

impl WorkloadManifest_v0 {
    pub fn from_toml_str(input: &str) -> Result<Self, WorkloadError> {
        let manifest: Self = toml::from_str(input).map_err(WorkloadError::Toml)?;
        manifest.validate()?;
        manifest.emit_loaded_event()?;
        Ok(manifest)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, WorkloadError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| WorkloadError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    pub fn validate(&self) -> Result<(), WorkloadError> {
        self.validate_invariants()?;
        self.verify_self_hash()
    }

    pub fn validate_invariants(&self) -> Result<(), WorkloadError> {
        if self.schema != V0_SUCCESS_WORKLOAD_SCHEMA {
            return Err(WorkloadError::SchemaMismatch {
                expected: V0_SUCCESS_WORKLOAD_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        if self.id.as_str() != V0_SUCCESS_WORKLOAD_ID {
            return Err(WorkloadError::IdMismatch {
                expected: V0_SUCCESS_WORKLOAD_ID,
                observed: self.id.to_string(),
            });
        }
        if self.class != WorkloadClass::Conformance {
            return Err(WorkloadError::ClassMismatch {
                observed: self.class,
            });
        }
        if self.prompts.len() != V0_SUCCESS_PROMPT_COUNT {
            return Err(WorkloadError::PromptArityMismatch {
                expected: V0_SUCCESS_PROMPT_COUNT,
                observed: self.prompts.len(),
            });
        }
        if self.seeds != [0, 1, 2, 3, 4] {
            return Err(WorkloadError::SeedMatrixMismatch {
                expected: vec![0, 1, 2, 3, 4],
                observed: self.seeds.clone(),
            });
        }
        if self.session != SessionProfile_S3::pinned() {
            return Err(WorkloadError::SessionMatrixMismatch);
        }
        if self.observation != ObservationPolicy_S3::pinned() {
            return Err(WorkloadError::ObservationPolicyMismatch);
        }
        if self.execution != ExecutionMatrix_S3::pinned() {
            return Err(WorkloadError::ExecutionMatrixMismatch);
        }
        if self.acceptance != AcceptanceMatrix_S3::pinned() {
            return Err(WorkloadError::AcceptanceMatrixMismatch);
        }

        let expected_chapter_sha = v0_success_held_out_chapter_sha();
        for case in &self.prompts {
            case.validate(expected_chapter_sha)?;
        }

        Ok(())
    }

    pub fn verify_self_hash(&self) -> Result<(), WorkloadError> {
        let expected = self.compute_self_hash()?;
        if expected == self.workload_self_hash {
            Ok(())
        } else {
            Err(WorkloadError::WorkloadSelfHashMismatch {
                expected,
                observed: self.workload_self_hash,
            })
        }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkloadError> {
        CanonicalJson::to_vec(self).map_err(WorkloadError::CanonicalJson)
    }

    pub fn compute_self_hash(&self) -> Result<Hash256, WorkloadError> {
        let mut value = serde_json::to_value(self).map_err(WorkloadError::SerdeJson)?;
        value
            .as_object_mut()
            .ok_or(WorkloadError::ExpectedObjectForSelfHash)?
            .remove("workload_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(WorkloadError::CanonicalJson)?;
        Self::domain()
            .hash_canonical_bytes(&canonical)
            .map_err(WorkloadError::CanonicalJson)
    }

    pub fn to_toml_string(&self) -> Result<String, WorkloadError> {
        self.validate_invariants()?;
        Ok(format_v0_success_toml(self))
    }

    #[must_use]
    pub fn agreement_subset(&self) -> &[PromptCase] {
        &self.prompts[..V0_SUCCESS_AGREEMENT_SUBSET_SIZE]
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-workload",
            "WorkloadManifest_v0",
            V0_SUCCESS_WORKLOAD_SCHEMA,
            V0_SUCCESS_WORKLOAD_SCHEMA_VERSION,
        )
    }

    fn emit_loaded_event(&self) -> Result<(), WorkloadError> {
        let observation_policy_hash = self.observation.compute_hash()?;
        tracing::info!(
            target: V0_SUCCESS_WORKLOAD_LOG_TARGET,
            event_name = V0_SUCCESS_WORKLOAD_LOADED_EVENT,
            id = %self.id,
            prompt_count = self.prompts.len() as u64,
            agreement_subset_size = V0_SUCCESS_AGREEMENT_SUBSET_SIZE as u64,
            observation_policy_hash = %observation_policy_hash,
            workload_self_hash = %self.workload_self_hash,
        );
        Ok(())
    }
}

pub fn read_v0_success_workload_manifest(
    path: impl AsRef<Path>,
) -> Result<WorkloadManifest_v0, WorkloadError> {
    WorkloadManifest_v0::from_toml_file(path)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PromptId(pub String);

impl PromptId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for PromptId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for PromptId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for PromptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCase {
    pub id: PromptId,
    pub prompt_chars: TextCharSeq,
    pub held_out_chapter_sha: Hash256,
    pub expected_min_gen: u32,
    pub expected_max_repeat: u32,
    pub decode_mode: DecodeMode,
    pub rng_spec: RngSpec,
}

impl PromptCase {
    pub fn new(
        id: impl Into<PromptId>,
        prompt_chars: Vec<u8>,
        held_out_chapter_sha: Hash256,
    ) -> Result<Self, WorkloadError> {
        Ok(Self {
            id: id.into(),
            prompt_chars: TextCharSeq::new(prompt_chars).map_err(WorkloadError::Lexical)?,
            held_out_chapter_sha,
            expected_min_gen: V0_SUCCESS_EXPECTED_MIN_GEN,
            expected_max_repeat: V0_SUCCESS_EXPECTED_MAX_REPEAT,
            decode_mode: DecodeMode::Argmax,
            rng_spec: RngSpec::NoRng,
        })
    }

    fn validate(&self, expected_chapter_sha: Hash256) -> Result<(), WorkloadError> {
        let len = self.prompt_chars.len();
        if !(V0_SUCCESS_PROMPT_MIN_CHARS..=V0_SUCCESS_PROMPT_MAX_CHARS).contains(&len) {
            return Err(WorkloadError::PromptLengthOutOfBounds {
                id: self.id.clone(),
                len,
                min: V0_SUCCESS_PROMPT_MIN_CHARS,
                max: V0_SUCCESS_PROMPT_MAX_CHARS,
            });
        }
        if self.held_out_chapter_sha != expected_chapter_sha {
            return Err(WorkloadError::ChapterShaMismatch {
                id: self.id.clone(),
                expected: expected_chapter_sha,
                observed: self.held_out_chapter_sha,
            });
        }
        if self.expected_min_gen < V0_SUCCESS_EXPECTED_MIN_GEN {
            return Err(WorkloadError::PromptGenerationTargetTooSmall {
                id: self.id.clone(),
                expected_min: V0_SUCCESS_EXPECTED_MIN_GEN,
                observed: self.expected_min_gen,
            });
        }
        if self.expected_max_repeat > V0_SUCCESS_EXPECTED_MAX_REPEAT {
            return Err(WorkloadError::PromptRepeatLimitTooLarge {
                id: self.id.clone(),
                expected_max: V0_SUCCESS_EXPECTED_MAX_REPEAT,
                observed: self.expected_max_repeat,
            });
        }
        if self.decode_mode != DecodeMode::Argmax {
            return Err(WorkloadError::PromptDecodeModeMismatch {
                id: self.id.clone(),
                observed: self.decode_mode,
            });
        }
        if self.rng_spec != RngSpec::NoRng {
            return Err(WorkloadError::PromptRngSpecMismatch {
                id: self.id.clone(),
                observed: self.rng_spec,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RngSpec {
    NoRng,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionProfile_S3 {
    pub decode: DecodeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode_transforms: Option<DecodeTransforms>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_policy: Option<TranscriptPolicy>,
}

impl SessionProfile_S3 {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            decode: DecodeSpec::argmax(),
            decode_transforms: None,
            transcript_policy: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeTransforms {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptPolicy {}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPolicy_S3 {
    pub checkpoints: Vec<ObservationCheckpoint>,
    pub trace_level: S3TraceLevel,
    pub compare_domain: S3CompareDomain,
    pub determinism_requirement: S3DeterminismRequirement,
    pub agreement_trace: AgreementTrace,
    pub checkpoint_roles: CheckpointRoles_S3,
}

impl ObservationPolicy_S3 {
    #[must_use]
    pub fn pinned() -> Self {
        Self {
            checkpoints: vec![
                ObservationCheckpoint::PostEmbedding,
                ObservationCheckpoint::PostLogits,
                ObservationCheckpoint::PostDecode,
            ],
            trace_level: S3TraceLevel::Standard,
            compare_domain: S3CompareDomain::LogitsF32CanonicalReduction,
            determinism_requirement: S3DeterminismRequirement::BitExact,
            agreement_trace: AgreementTrace {
                generated_steps: 16,
                stop_on_eos: false,
            },
            checkpoint_roles: CheckpointRoles_S3::pinned(),
        }
    }

    pub fn compute_hash(&self) -> Result<Hash256, WorkloadError> {
        Self::domain()
            .hash(self)
            .map_err(WorkloadError::CanonicalJson)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-workload",
            "ObservationPolicy_S3",
            "observation_policy.s3",
            V0_SUCCESS_WORKLOAD_SCHEMA_VERSION,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCheckpoint {
    PostEmbedding,
    PostLogits,
    PostDecode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S3TraceLevel {
    Standard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S3CompareDomain {
    LogitsF32CanonicalReduction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S3DeterminismRequirement {
    BitExact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementTrace {
    pub generated_steps: u32,
    pub stop_on_eos: bool,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRoles_S3 {
    pub post_embedding: CheckpointRole,
    pub post_logits: CheckpointRole,
    pub post_decode: CheckpointRole,
}

impl CheckpointRoles_S3 {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            post_embedding: CheckpointRole::ObservationOnly,
            post_logits: CheckpointRole::AgreementGated,
            post_decode: CheckpointRole::AgreementGated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointRole {
    ObservationOnly,
    AgreementGated,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionMatrix_S3 {
    pub denotational: bool,
    pub artifact: bool,
    pub schedule: bool,
    pub harness: bool,
    pub hardware: bool,
}

impl ExecutionMatrix_S3 {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            denotational: true,
            artifact: true,
            schedule: false,
            harness: false,
            hardware: false,
        }
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceMatrix_S3 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_phase_a_vs_bundle: Option<EnvelopeGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_phase_d_vs_artifact: Option<EnvelopeGate>,
    pub bundle_vs_artifact: AcceptanceDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_vs_schedule: Option<EnvelopeGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_vs_runtime: Option<EnvelopeGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<AcceptanceDisposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<AcceptanceDisposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<AcceptanceDisposition>,
}

impl AcceptanceMatrix_S3 {
    #[must_use]
    pub fn pinned() -> Self {
        Self {
            live_phase_a_vs_bundle: Some(EnvelopeGate {
                max_per_token_logit_abs_diff: 4.0e-6,
                numeric_mode: EnvelopeNumericMode::Fp32,
                argmax_token_must_match: true,
            }),
            live_phase_d_vs_artifact: Some(EnvelopeGate {
                max_per_token_logit_abs_diff: 0.0,
                numeric_mode: EnvelopeNumericMode::Bitwise,
                argmax_token_must_match: true,
            }),
            bundle_vs_artifact: AcceptanceDisposition::ReportOnly,
            artifact_vs_schedule: None,
            schedule_vs_runtime: None,
            performance: None,
            experience: None,
            recovery: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeGate {
    pub max_per_token_logit_abs_diff: f64,
    pub numeric_mode: EnvelopeNumericMode,
    pub argmax_token_must_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeNumericMode {
    Fp32,
    Bitwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceDisposition {
    ReportOnly,
}

#[derive(Debug)]
pub enum WorkloadError {
    Io {
        path: String,
        source: io::Error,
    },
    Toml(toml::de::Error),
    TomlSerialize(toml::ser::Error),
    SerdeJson(serde_json::Error),
    CanonicalJson(CanonicalJsonError),
    ExpectedObjectForSelfHash,
    SchemaMismatch {
        expected: &'static str,
        observed: String,
    },
    IdMismatch {
        expected: &'static str,
        observed: String,
    },
    ClassMismatch {
        observed: WorkloadClass,
    },
    SeedMatrixMismatch {
        expected: Vec<u64>,
        observed: Vec<u64>,
    },
    SessionMatrixMismatch,
    ObservationPolicyMismatch,
    ExecutionMatrixMismatch,
    AcceptanceMatrixMismatch,
    PromptArityMismatch {
        expected: usize,
        observed: usize,
    },
    PromptLengthOutOfBounds {
        id: PromptId,
        len: usize,
        min: usize,
        max: usize,
    },
    ChapterShaMismatch {
        id: PromptId,
        expected: Hash256,
        observed: Hash256,
    },
    PromptGenerationTargetTooSmall {
        id: PromptId,
        expected_min: u32,
        observed: u32,
    },
    PromptRepeatLimitTooLarge {
        id: PromptId,
        expected_max: u32,
        observed: u32,
    },
    PromptDecodeModeMismatch {
        id: PromptId,
        observed: DecodeMode,
    },
    PromptRngSpecMismatch {
        id: PromptId,
        observed: RngSpec,
    },
    WorkloadSelfHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    Lexical(LexicalError),
}

impl fmt::Display for WorkloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::TomlSerialize(error) => write!(f, "{error}"),
            Self::SerdeJson(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::ExpectedObjectForSelfHash => {
                write!(f, "workload self hash requires a top-level object")
            }
            Self::SchemaMismatch { expected, observed } => {
                write!(f, "expected schema {expected:?}, observed {observed:?}")
            }
            Self::IdMismatch { expected, observed } => {
                write!(
                    f,
                    "expected workload id {expected:?}, observed {observed:?}"
                )
            }
            Self::ClassMismatch { observed } => {
                write!(
                    f,
                    "expected conformance workload class, observed {observed:?}"
                )
            }
            Self::SeedMatrixMismatch { expected, observed } => {
                write!(f, "expected seeds {expected:?}, observed {observed:?}")
            }
            Self::SessionMatrixMismatch => write!(f, "session profile does not match S3 policy"),
            Self::ObservationPolicyMismatch => {
                write!(f, "observation policy does not match RFC S3 policy")
            }
            Self::ExecutionMatrixMismatch => write!(f, "execution matrix does not match S3 policy"),
            Self::AcceptanceMatrixMismatch => {
                write!(f, "acceptance matrix does not match S3 policy")
            }
            Self::PromptArityMismatch { expected, observed } => {
                write!(f, "expected {expected} prompts, observed {observed}")
            }
            Self::PromptLengthOutOfBounds { id, len, min, max } => {
                write!(
                    f,
                    "prompt {id} length {len} is outside inclusive bounds {min}..={max}"
                )
            }
            Self::ChapterShaMismatch {
                id,
                expected,
                observed,
            } => write!(
                f,
                "prompt {id} held-out chapter sha mismatch: expected {expected}, observed {observed}"
            ),
            Self::PromptGenerationTargetTooSmall {
                id,
                expected_min,
                observed,
            } => write!(
                f,
                "prompt {id} expected_min_gen {observed} is below {expected_min}"
            ),
            Self::PromptRepeatLimitTooLarge {
                id,
                expected_max,
                observed,
            } => write!(
                f,
                "prompt {id} expected_max_repeat {observed} exceeds {expected_max}"
            ),
            Self::PromptDecodeModeMismatch { id, observed } => {
                write!(f, "prompt {id} decode mode {observed:?} is not argmax")
            }
            Self::PromptRngSpecMismatch { id, observed } => {
                write!(f, "prompt {id} rng spec {observed:?} is not no_rng")
            }
            Self::WorkloadSelfHashMismatch { expected, observed } => {
                write!(
                    f,
                    "workload_self_hash mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::Lexical(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WorkloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::TomlSerialize(error) => Some(error),
            Self::SerdeJson(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Lexical(error) => Some(error),
            Self::ExpectedObjectForSelfHash
            | Self::SchemaMismatch { .. }
            | Self::IdMismatch { .. }
            | Self::ClassMismatch { .. }
            | Self::SeedMatrixMismatch { .. }
            | Self::SessionMatrixMismatch
            | Self::ObservationPolicyMismatch
            | Self::ExecutionMatrixMismatch
            | Self::AcceptanceMatrixMismatch
            | Self::PromptArityMismatch { .. }
            | Self::PromptLengthOutOfBounds { .. }
            | Self::ChapterShaMismatch { .. }
            | Self::PromptGenerationTargetTooSmall { .. }
            | Self::PromptRepeatLimitTooLarge { .. }
            | Self::PromptDecodeModeMismatch { .. }
            | Self::PromptRngSpecMismatch { .. }
            | Self::WorkloadSelfHashMismatch { .. } => None,
        }
    }
}

fn deserialize_v0_success_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == V0_SUCCESS_WORKLOAD_SCHEMA {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format_args!(
            "expected schema id {V0_SUCCESS_WORKLOAD_SCHEMA:?}, got {value:?}"
        )))
    }
}

fn v0_success_held_out_chapter_sha() -> Hash256 {
    V0_SUCCESS_HELD_OUT_CHAPTER_SHA
        .parse()
        .expect("pinned v0_success chapter hash is valid")
}

fn format_v0_success_toml(manifest: &WorkloadManifest_v0) -> String {
    let mut out = String::new();
    writeln!(out, "schema = {:?}", manifest.schema).expect("write string");
    writeln!(out, "id = {:?}", manifest.id.as_str()).expect("write string");
    writeln!(out, "class = {:?}", workload_class_name(manifest.class)).expect("write string");
    write!(out, "seeds = [").expect("write string");
    write_u64_array(&mut out, &manifest.seeds);
    writeln!(out, "]").expect("write string");
    writeln!(
        out,
        "workload_self_hash = {:?}",
        manifest.workload_self_hash.to_string()
    )
    .expect("write string");
    writeln!(out).expect("write string");

    for prompt in &manifest.prompts {
        writeln!(out, "[[prompts]]").expect("write string");
        writeln!(out, "id = {:?}", prompt.id.as_str()).expect("write string");
        write!(out, "prompt_chars = [").expect("write string");
        write_u8_array(&mut out, prompt.prompt_chars.as_slice());
        writeln!(out, "]").expect("write string");
        writeln!(
            out,
            "held_out_chapter_sha = {:?}",
            prompt.held_out_chapter_sha.to_string()
        )
        .expect("write string");
        writeln!(out, "expected_min_gen = {}", prompt.expected_min_gen).expect("write string");
        writeln!(out, "expected_max_repeat = {}", prompt.expected_max_repeat)
            .expect("write string");
        writeln!(
            out,
            "decode_mode = {:?}",
            decode_mode_name(prompt.decode_mode)
        )
        .expect("write string");
        writeln!(out, "rng_spec = {:?}", rng_spec_name(prompt.rng_spec)).expect("write string");
        writeln!(out).expect("write string");
    }

    writeln!(out, "[session.decode]").expect("write string");
    writeln!(
        out,
        "mode = {:?}",
        decode_mode_name(manifest.session.decode.mode)
    )
    .expect("write string");
    writeln!(out).expect("write string");

    writeln!(out, "[observation]").expect("write string");
    write!(out, "checkpoints = [").expect("write string");
    for (index, checkpoint) in manifest.observation.checkpoints.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").expect("write string");
        }
        write!(out, "{:?}", observation_checkpoint_name(*checkpoint)).expect("write string");
    }
    writeln!(out, "]").expect("write string");
    writeln!(
        out,
        "trace_level = {:?}",
        s3_trace_level_name(manifest.observation.trace_level)
    )
    .expect("write string");
    writeln!(
        out,
        "compare_domain = {:?}",
        s3_compare_domain_name(manifest.observation.compare_domain)
    )
    .expect("write string");
    writeln!(
        out,
        "determinism_requirement = {:?}",
        s3_determinism_requirement_name(manifest.observation.determinism_requirement)
    )
    .expect("write string");
    writeln!(out).expect("write string");

    writeln!(out, "[observation.agreement_trace]").expect("write string");
    writeln!(
        out,
        "generated_steps = {}",
        manifest.observation.agreement_trace.generated_steps
    )
    .expect("write string");
    writeln!(
        out,
        "stop_on_eos = {}",
        manifest.observation.agreement_trace.stop_on_eos
    )
    .expect("write string");
    writeln!(out).expect("write string");

    writeln!(out, "[observation.checkpoint_roles]").expect("write string");
    writeln!(
        out,
        "post_embedding = {:?}",
        checkpoint_role_name(manifest.observation.checkpoint_roles.post_embedding)
    )
    .expect("write string");
    writeln!(
        out,
        "post_logits = {:?}",
        checkpoint_role_name(manifest.observation.checkpoint_roles.post_logits)
    )
    .expect("write string");
    writeln!(
        out,
        "post_decode = {:?}",
        checkpoint_role_name(manifest.observation.checkpoint_roles.post_decode)
    )
    .expect("write string");
    writeln!(out).expect("write string");

    writeln!(out, "[execution]").expect("write string");
    writeln!(out, "denotational = {}", manifest.execution.denotational).expect("write string");
    writeln!(out, "artifact = {}", manifest.execution.artifact).expect("write string");
    writeln!(out, "schedule = {}", manifest.execution.schedule).expect("write string");
    writeln!(out, "harness = {}", manifest.execution.harness).expect("write string");
    writeln!(out, "hardware = {}", manifest.execution.hardware).expect("write string");
    writeln!(out).expect("write string");

    writeln!(out, "[acceptance]").expect("write string");
    writeln!(
        out,
        "bundle_vs_artifact = {:?}",
        acceptance_disposition_name(manifest.acceptance.bundle_vs_artifact)
    )
    .expect("write string");
    writeln!(out).expect("write string");

    if let Some(gate) = manifest.acceptance.live_phase_a_vs_bundle {
        write_envelope_gate(&mut out, "live_phase_a_vs_bundle", gate);
    }
    if let Some(gate) = manifest.acceptance.live_phase_d_vs_artifact {
        write_envelope_gate(&mut out, "live_phase_d_vs_artifact", gate);
    }

    out
}

fn write_u64_array(out: &mut String, values: &[u64]) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").expect("write string");
        }
        write!(out, "{value}").expect("write string");
    }
}

fn write_u8_array(out: &mut String, values: &[u8]) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").expect("write string");
        }
        write!(out, "{value}").expect("write string");
    }
}

fn write_envelope_gate(out: &mut String, name: &str, gate: EnvelopeGate) {
    writeln!(out, "[acceptance.{name}]").expect("write string");
    writeln!(
        out,
        "max_per_token_logit_abs_diff = {}",
        format_gate_float(gate.max_per_token_logit_abs_diff)
    )
    .expect("write string");
    writeln!(
        out,
        "numeric_mode = {:?}",
        envelope_numeric_mode_name(gate.numeric_mode)
    )
    .expect("write string");
    writeln!(
        out,
        "argmax_token_must_match = {}",
        gate.argmax_token_must_match
    )
    .expect("write string");
    if name != "live_phase_d_vs_artifact" {
        writeln!(out).expect("write string");
    }
}

fn format_gate_float(value: f64) -> &'static str {
    if value == 4.0e-6 {
        "4e-6"
    } else if value == 0.0 {
        "0.0"
    } else {
        panic!("unexpected S3 envelope gate float {value}");
    }
}

fn workload_class_name(class: WorkloadClass) -> &'static str {
    match class {
        WorkloadClass::Conformance => "conformance",
        WorkloadClass::Performance => "performance",
        WorkloadClass::Endurance => "endurance",
        WorkloadClass::Persistence => "persistence",
        WorkloadClass::Ui => "ui",
        WorkloadClass::Quality => "quality",
        WorkloadClass::Interactive => "interactive",
    }
}

fn decode_mode_name(mode: DecodeMode) -> &'static str {
    match mode {
        DecodeMode::Argmax => "argmax",
    }
}

fn rng_spec_name(spec: RngSpec) -> &'static str {
    match spec {
        RngSpec::NoRng => "no_rng",
    }
}

fn observation_checkpoint_name(checkpoint: ObservationCheckpoint) -> &'static str {
    match checkpoint {
        ObservationCheckpoint::PostEmbedding => "post_embedding",
        ObservationCheckpoint::PostLogits => "post_logits",
        ObservationCheckpoint::PostDecode => "post_decode",
    }
}

fn s3_trace_level_name(level: S3TraceLevel) -> &'static str {
    match level {
        S3TraceLevel::Standard => "standard",
    }
}

fn s3_compare_domain_name(domain: S3CompareDomain) -> &'static str {
    match domain {
        S3CompareDomain::LogitsF32CanonicalReduction => "logits_f32_canonical_reduction",
    }
}

fn s3_determinism_requirement_name(requirement: S3DeterminismRequirement) -> &'static str {
    match requirement {
        S3DeterminismRequirement::BitExact => "bit_exact",
    }
}

fn checkpoint_role_name(role: CheckpointRole) -> &'static str {
    match role {
        CheckpointRole::ObservationOnly => "observation_only",
        CheckpointRole::AgreementGated => "agreement_gated",
    }
}

fn acceptance_disposition_name(disposition: AcceptanceDisposition) -> &'static str {
    match disposition {
        AcceptanceDisposition::ReportOnly => "report_only",
    }
}

fn envelope_numeric_mode_name(mode: EnvelopeNumericMode) -> &'static str {
    match mode {
        EnvelopeNumericMode::Fp32 => "fp32",
        EnvelopeNumericMode::Bitwise => "bitwise",
    }
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
    pub cases: Vec<DensePromptCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DensePromptCase {
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
