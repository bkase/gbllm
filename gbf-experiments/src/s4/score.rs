//! S4 Gutenberg score surface.

use std::error::Error;
use std::fmt;

use gbf_artifact::{TextCharSeq, canonical_payload_sha};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, canonical_json_bytes_omitting_fields,
    sha256,
};
use serde::{Deserialize, Serialize};

use crate::s3::score::{Evaluator, S3_SCORE_CHUNK_SIZE, ScoreError, try_s3_score_bpc_char};
use crate::s3::workload::{V0_SUCCESS_MAX_QUANT_GAP, V0_SUCCESS_MIN_BPC_GAIN_VS_KN5};
use crate::s4::schema::{S4_CANONICAL_SEEDS, S4SchemaError, validate_s4_seed};

pub use crate::s3::bundle::{
    BundleExportError as S4ReferenceBundleExportError,
    BundleExportInputs as S4ReferenceBundleExportInputs,
    BundleExportProduct as S4ReferenceBundleExportProduct,
    s3_export_fixture_reference_bundle as s4_reexport_fixture_reference_model_bundle,
    s3_export_reference_bundle as s4_reexport_reference_model_bundle,
    write_bundle_export_product as write_s4_reference_model_bundle_export_product,
};
pub use gbf_artifact::ReferenceModelBundle;

/// Schema id for the per-seed Gutenberg score and v0_success product.
pub const S4_GUTENBERG_SCORE_SCHEMA: &str = "s4_gutenberg_score.v1";

/// Schema id for synthetic S4 ReferenceModelBundle re-export binding evidence.
pub const S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA: &str =
    "s4_reference_model_bundle_reexport_evidence.v1";

/// Fixture semantics pinned by the synthetic re-export binding evidence.
pub const S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS: &str =
    "s3_pinned_conformance_fixture_on_gutenberg_val";

/// S4 scorer tracing target.
pub const S4_SCORE_LOG_TARGET: &str = "gbf_experiments::s4::score";

/// RFC-pinned S4 score chunk size in normalized Gutenberg token ids.
pub const S4_SCORE_CHUNK_SIZE: usize = S3_SCORE_CHUNK_SIZE;

/// Strict Gutenberg-vs-KN5 bpc margin inherited from S3 v0_success.
pub const S4_MIN_BPC_MARGIN_VS_KN5: f64 = V0_SUCCESS_MIN_BPC_GAIN_VS_KN5;

/// Maximum ternary-vs-full-precision bpc gap inherited from S3 v0_success.
pub const S4_MAX_TERNARY_QAT_GAP: f64 = V0_SUCCESS_MAX_QUANT_GAP;

/// Finite non-negative Gutenberg bits-per-character value.
pub type S4BpcValue = crate::s3::score::BpcCharValue;

const S4_GUTENBERG_SCORE_SCHEMA_VERSION: &str = "1";
const S4_SCORE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4V0SuccessProduct",
    S4_GUTENBERG_SCORE_SCHEMA,
    S4_GUTENBERG_SCORE_SCHEMA_VERSION,
);
const S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA_VERSION: &str = "1";
const S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4ReferenceModelBundleReexportEvidence",
    S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA,
    S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA_VERSION,
);

/// Inputs to the reset-context S4 Gutenberg bpc primitive.
pub struct S4ScoreBpcInputs<'a, E>
where
    E: Evaluator,
{
    /// Per-token evaluator for the checkpoint being scored.
    pub evaluator: E,
    /// Normalized Gutenberg validation token-id stream.
    pub gutenberg_val: &'a TextCharSeq,
    /// Expected SHA-256 of `gutenberg_val`.
    pub corpus_val_sha: Hash256,
}

/// Score a checkpoint against `gutenberg_val` with S3's reset-context primitive.
pub fn s4_score_bpc<E>(inputs: S4ScoreBpcInputs<'_, E>) -> Result<S4BpcValue, S4ScoreError>
where
    E: Evaluator,
{
    validate_hash(
        "corpus_val_sha",
        inputs.corpus_val_sha,
        sha256(inputs.gutenberg_val.as_slice()),
    )?;
    tracing::info!(
        target: S4_SCORE_LOG_TARGET,
        event_name = "s4::score::bpc_started",
        corpus_val_sha = %inputs.corpus_val_sha,
        token_count = inputs.gutenberg_val.len() as u64,
        chunk_size = S4_SCORE_CHUNK_SIZE as u64,
        "s4 gutenberg bpc scoring started"
    );
    let product =
        try_s3_score_bpc_char(inputs.evaluator, inputs.gutenberg_val, S4_SCORE_CHUNK_SIZE)?;
    tracing::info!(
        target: S4_SCORE_LOG_TARGET,
        event_name = "s4::score::bpc_complete",
        corpus_val_sha = %inputs.corpus_val_sha,
        bpc_ternary = product.bpc_char.get(),
        score_self_hash = %product.score_self_hash,
        "s4 gutenberg bpc scoring complete"
    );
    Ok(product.bpc_char)
}

/// Inputs for synthetic evidence tying an S4 checkpoint to its re-exported bundle.
pub struct S4ReferenceModelBundleReexportEvidenceInputs<'a> {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Self-hash of the Gutenberg checkpoint that was re-exported.
    pub checkpoint_self_hash: Hash256,
    /// Expected SHA-256 of the Gutenberg validation token-id stream.
    pub corpus_val_sha: Hash256,
    /// Normalized Gutenberg validation token-id stream.
    pub gutenberg_val: &'a TextCharSeq,
    /// Canonical S3-pinned conformance fixture set self-hash.
    pub conformance_fixture_set_self_hash: Hash256,
    /// Re-export product produced by the inherited S3 ReferenceModelBundle path.
    pub bundle_export: &'a S4ReferenceBundleExportProduct,
}

/// Synthetic binding evidence for the inherited ReferenceModelBundle re-export path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4ReferenceModelBundleReexportEvidence {
    /// Schema id, always `s4_reference_model_bundle_reexport_evidence.v1`.
    pub schema: String,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Self-hash of the Gutenberg checkpoint that was re-exported.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation token-id stream.
    pub corpus_val_sha: Hash256,
    /// S3-pinned conformance fixture set self-hash.
    pub conformance_fixture_set_self_hash: Hash256,
    /// Human-readable semantics label for the fixture binding.
    pub conformance_fixture_semantics: String,
    /// Re-exported `ReferenceModelBundle` self-hash.
    pub bundle_self_hash: Hash256,
    /// SHA-256 of canonical re-exported bundle bytes.
    pub canonical_bundle_payload_sha: Hash256,
    /// Self-hash over canonical JSON with this field omitted.
    pub reference_model_bundle_reexport_self_hash: Hash256,
}

impl S4ReferenceModelBundleReexportEvidence {
    /// Compute the evidence self-hash with the self-hash field omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4ScoreError> {
        let bytes = canonical_json_bytes_omitting_fields(
            self,
            &["reference_model_bundle_reexport_self_hash"],
        )?;
        S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_DOMAIN
            .hash_canonical_bytes(&bytes)
            .map_err(S4ScoreError::CanonicalJson)
    }

    /// Canonical JSON bytes including the evidence self-hash.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4ScoreError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4ScoreError::CanonicalJson)
    }

    /// Validate structure and evidence self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4ScoreError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.reference_model_bundle_reexport_self_hash {
            return Err(S4ScoreError::ReferenceModelBundleReexportSelfHashMismatch {
                expected: recomputed,
                observed: self.reference_model_bundle_reexport_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4ScoreError> {
        if self.schema != S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA {
            return Err(S4ScoreError::InvalidReexportEvidenceSchema {
                observed: self.schema.clone(),
            });
        }
        if self.conformance_fixture_semantics
            != S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS
        {
            return Err(S4ScoreError::InvalidReexportFixtureSemantics {
                observed: self.conformance_fixture_semantics.clone(),
            });
        }
        validate_s4_seed(self.seed)?;
        validate_nonzero_hash("checkpoint_self_hash", self.checkpoint_self_hash)?;
        validate_nonzero_hash("corpus_val_sha", self.corpus_val_sha)?;
        validate_nonzero_hash(
            "conformance_fixture_set_self_hash",
            self.conformance_fixture_set_self_hash,
        )?;
        validate_nonzero_hash("bundle_self_hash", self.bundle_self_hash)?;
        validate_nonzero_hash(
            "canonical_bundle_payload_sha",
            self.canonical_bundle_payload_sha,
        )?;
        Ok(())
    }
}

/// Build synthetic S4 evidence for the inherited ReferenceModelBundle re-export path.
pub fn s4_reference_model_bundle_reexport_evidence(
    inputs: S4ReferenceModelBundleReexportEvidenceInputs<'_>,
) -> Result<S4ReferenceModelBundleReexportEvidence, S4ScoreError> {
    validate_s4_seed(inputs.seed)?;
    validate_nonzero_hash("checkpoint_self_hash", inputs.checkpoint_self_hash)?;
    validate_nonzero_hash(
        "conformance_fixture_set_self_hash",
        inputs.conformance_fixture_set_self_hash,
    )?;
    validate_hash(
        "corpus_val_sha",
        inputs.corpus_val_sha,
        sha256(inputs.gutenberg_val.as_slice()),
    )?;
    validate_hash(
        "reference_model_bundle.bundle_self_hash",
        inputs.bundle_export.bundle_self_hash,
        inputs.bundle_export.bundle.bundle_self_hash,
    )?;
    validate_hash(
        "reference_model_bundle.computed_self_hash",
        inputs.bundle_export.bundle_self_hash,
        inputs.bundle_export.bundle.compute_self_hash(),
    )?;
    validate_hash(
        "reference_model_bundle.metadata.bundle_self_hash",
        inputs.bundle_export.bundle_self_hash,
        inputs.bundle_export.metadata.bundle_self_hash,
    )?;
    validate_hash(
        "reference_model_bundle.metadata.canonical_bundle_payload_sha",
        inputs.bundle_export.canonical_bundle_payload_sha,
        inputs.bundle_export.metadata.canonical_bundle_payload_sha,
    )?;
    validate_hash(
        "canonical_bundle_payload_sha",
        inputs.bundle_export.canonical_bundle_payload_sha,
        canonical_payload_sha(&inputs.bundle_export.canonical_bundle_bytes),
    )?;
    if !inputs.bundle_export.program_validation.prompt_subset_pass() {
        return Err(S4ScoreError::ReferenceModelBundleReexportValidationFailed);
    }

    let mut evidence = S4ReferenceModelBundleReexportEvidence {
        schema: S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA.to_owned(),
        seed: inputs.seed,
        checkpoint_self_hash: inputs.checkpoint_self_hash,
        corpus_val_sha: inputs.corpus_val_sha,
        conformance_fixture_set_self_hash: inputs.conformance_fixture_set_self_hash,
        conformance_fixture_semantics: S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS
            .to_owned(),
        bundle_self_hash: inputs.bundle_export.bundle_self_hash,
        canonical_bundle_payload_sha: inputs.bundle_export.canonical_bundle_payload_sha,
        reference_model_bundle_reexport_self_hash: Hash256::ZERO,
    };
    evidence.validate_structure()?;
    evidence.reference_model_bundle_reexport_self_hash = evidence.compute_self_hash()?;
    tracing::info!(
        target: S4_SCORE_LOG_TARGET,
        event_name = "s4::score::reference_model_bundle_reexport_evidence_complete",
        seed = evidence.seed,
        checkpoint_self_hash = %evidence.checkpoint_self_hash,
        corpus_val_sha = %evidence.corpus_val_sha,
        bundle_self_hash = %evidence.bundle_self_hash,
        reference_model_bundle_reexport_self_hash =
            %evidence.reference_model_bundle_reexport_self_hash,
        "s4 reference model bundle re-export binding evidence emitted"
    );
    Ok(evidence)
}

/// Inherited non-BPC v0_success predicate bits supplied by the S4 workload run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S4InheritedV0SuccessBits {
    /// Prompt length predicate inherited from the S3 workload manifest.
    pub prompt_length_ok: bool,
    /// Generation length predicate inherited from the S3 workload manifest.
    pub generation_length_ok: bool,
    /// Repetition-collapse predicate inherited from the S3 workload manifest.
    pub no_repetition_collapse: bool,
    /// Charset-v1 validity predicate inherited from the S3 workload manifest.
    pub only_charset_v1_ids: bool,
    /// Conservative runtime chrome-budget predicate inherited from S3 stubs.
    pub runtime_chrome_budget_ok: bool,
    /// Emulator-smoke predicate inherited from S3 stubs.
    pub emulator_smoke_ok: bool,
}

impl S4InheritedV0SuccessBits {
    /// All inherited predicate bits pass.
    #[must_use]
    pub const fn all_pass() -> Self {
        Self {
            prompt_length_ok: true,
            generation_length_ok: true,
            no_repetition_collapse: true,
            only_charset_v1_ids: true,
            runtime_chrome_budget_ok: true,
            emulator_smoke_ok: true,
        }
    }
}

/// Acceptance bits stored in `s4_gutenberg_score.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4V0SuccessAcceptanceBits {
    /// Prompt length predicate inherited from the S3 workload manifest.
    pub prompt_length_ok: bool,
    /// Generation length predicate inherited from the S3 workload manifest.
    pub generation_length_ok: bool,
    /// Repetition-collapse predicate inherited from the S3 workload manifest.
    pub no_repetition_collapse: bool,
    /// Charset-v1 validity predicate inherited from the S3 workload manifest.
    pub only_charset_v1_ids: bool,
    /// Strict bpc margin predicate, `bpc_kn5 - bpc_ternary > 0.05`.
    pub beats_kn5_baseline: bool,
    /// Ternary-vs-full-precision bpc gap predicate, `gap <= 0.5`.
    pub ternary_qat_gap_ok: bool,
    /// Conservative runtime chrome-budget predicate inherited from S3 stubs.
    pub runtime_chrome_budget_ok: bool,
    /// Emulator-smoke predicate inherited from S3 stubs.
    pub emulator_smoke_ok: bool,
}

impl S4V0SuccessAcceptanceBits {
    /// True iff every per-seed acceptance bit passes.
    #[must_use]
    pub const fn all_set(self) -> bool {
        self.prompt_length_ok
            && self.generation_length_ok
            && self.no_repetition_collapse
            && self.only_charset_v1_ids
            && self.beats_kn5_baseline
            && self.ternary_qat_gap_ok
            && self.runtime_chrome_budget_ok
            && self.emulator_smoke_ok
    }
}

/// Strict all-seed H4 v0_success gate outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum S4StrictV0SuccessOutcome {
    /// All canonical S4 seeds passed every per-seed bit.
    Pass,
    /// The first failing canonical seed.
    Fail {
        /// Seed whose per-seed acceptance failed.
        failing_seed: u64,
    },
}

impl S4StrictV0SuccessOutcome {
    /// True when the strict all-seed gate passed.
    #[must_use]
    pub const fn passed(self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Inputs to `s4_v0_success_gutenberg`.
#[derive(Debug, Clone, PartialEq)]
pub struct S4V0SuccessInputs {
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Self-hash of the Gutenberg-trained checkpoint.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the checkpoint payload bytes.
    pub checkpoint_payload_sha: Hash256,
    /// SHA-256 of the Gutenberg validation token-id stream.
    pub corpus_val_sha: Hash256,
    /// S3 v0_success workload manifest template self-hash.
    pub workload_manifest_template_self_hash: Hash256,
    /// S4 corpus-bound workload manifest instance self-hash.
    pub workload_manifest_instance_self_hash: Hash256,
    /// FP reference artifact self-hash for the ternary QAT gap check.
    pub fp_reference_self_hash: Hash256,
    /// Measured ternary checkpoint bpc on Gutenberg validation.
    pub bpc_ternary: S4BpcValue,
    /// Measured KN-5 baseline bpc on Gutenberg validation.
    pub bpc_kn5: S4BpcValue,
    /// Measured FP-reference bpc on Gutenberg validation.
    pub bpc_fp_reference: S4BpcValue,
    /// Inherited non-BPC S3 v0_success predicate bits.
    pub inherited_acceptance: S4InheritedV0SuccessBits,
}

/// Per-seed `s4_gutenberg_score.v1` artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4V0SuccessProduct {
    /// Schema id, always `s4_gutenberg_score.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Self-hash of the Gutenberg-trained checkpoint.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the checkpoint payload bytes.
    pub checkpoint_payload_sha: Hash256,
    /// SHA-256 of the Gutenberg validation token-id stream.
    pub corpus_val_sha: Hash256,
    /// S3 v0_success workload manifest template self-hash.
    pub workload_manifest_template_self_hash: Hash256,
    /// S4 corpus-bound workload manifest instance self-hash.
    pub workload_manifest_instance_self_hash: Hash256,
    /// FP reference artifact self-hash for the ternary QAT gap check.
    pub fp_reference_self_hash: Hash256,
    /// Measured ternary checkpoint bpc on Gutenberg validation.
    pub bpc_ternary: S4BpcValue,
    /// Measured KN-5 baseline bpc on Gutenberg validation.
    pub bpc_kn5: S4BpcValue,
    /// `bpc_kn5 - bpc_ternary`.
    pub bpc_margin: f64,
    /// Strict per-seed S4 v0_success acceptance bits.
    pub v0_success_acceptance: S4V0SuccessAcceptanceBits,
    /// True iff every acceptance bit passes.
    pub pass: bool,
    /// Self-hash over canonical JSON with this field omitted.
    pub score_self_hash: Hash256,
}

impl S4V0SuccessProduct {
    /// Compute the score self-hash with `score_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4ScoreError> {
        let bytes = canonical_json_bytes_omitting_fields(self, &["score_self_hash"])?;
        S4_SCORE_DOMAIN
            .hash_canonical_bytes(&bytes)
            .map_err(S4ScoreError::CanonicalJson)
    }

    /// Canonical JSON bytes including `score_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4ScoreError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4ScoreError::CanonicalJson)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4ScoreError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.score_self_hash {
            return Err(S4ScoreError::SelfHashMismatch {
                expected: recomputed,
                observed: self.score_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4ScoreError> {
        if self.schema != S4_GUTENBERG_SCORE_SCHEMA {
            return Err(S4ScoreError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        validate_s4_seed(self.seed)?;
        validate_nonzero_hash(
            "tinystories_manifest_self_hash",
            self.tinystories_manifest_self_hash,
        )?;
        validate_nonzero_hash(
            "gutenberg_manifest_self_hash",
            self.gutenberg_manifest_self_hash,
        )?;
        validate_nonzero_hash("checkpoint_self_hash", self.checkpoint_self_hash)?;
        validate_nonzero_hash("checkpoint_payload_sha", self.checkpoint_payload_sha)?;
        validate_nonzero_hash("corpus_val_sha", self.corpus_val_sha)?;
        validate_nonzero_hash(
            "workload_manifest_template_self_hash",
            self.workload_manifest_template_self_hash,
        )?;
        validate_nonzero_hash(
            "workload_manifest_instance_self_hash",
            self.workload_manifest_instance_self_hash,
        )?;
        validate_nonzero_hash("fp_reference_self_hash", self.fp_reference_self_hash)?;
        validate_finite("bpc_margin", self.bpc_margin)?;
        let recomputed_margin = self.bpc_kn5.get() - self.bpc_ternary.get();
        if self.bpc_margin.to_bits() != recomputed_margin.to_bits() {
            return Err(S4ScoreError::BpcMarginMismatch {
                expected: recomputed_margin,
                observed: self.bpc_margin,
            });
        }
        let expected_beats = self.bpc_margin > S4_MIN_BPC_MARGIN_VS_KN5;
        if self.v0_success_acceptance.beats_kn5_baseline != expected_beats {
            return Err(S4ScoreError::AcceptanceBitMismatch {
                field: "v0_success_acceptance.beats_kn5_baseline",
                expected: expected_beats,
                observed: self.v0_success_acceptance.beats_kn5_baseline,
            });
        }
        let expected_pass = self.v0_success_acceptance.all_set();
        if self.pass != expected_pass {
            return Err(S4ScoreError::PassBitMismatch {
                expected: expected_pass,
                observed: self.pass,
            });
        }
        Ok(())
    }
}

/// Construct the per-seed Gutenberg v0_success score artifact.
pub fn s4_v0_success_gutenberg(
    inputs: S4V0SuccessInputs,
) -> Result<S4V0SuccessProduct, S4ScoreError> {
    validate_s4_seed(inputs.seed)?;
    let bpc_margin = inputs.bpc_kn5.get() - inputs.bpc_ternary.get();
    validate_finite("bpc_margin", bpc_margin)?;
    let ternary_qat_gap = inputs.bpc_ternary.get() - inputs.bpc_fp_reference.get();
    validate_finite("ternary_qat_gap", ternary_qat_gap)?;

    let v0_success_acceptance = S4V0SuccessAcceptanceBits {
        prompt_length_ok: inputs.inherited_acceptance.prompt_length_ok,
        generation_length_ok: inputs.inherited_acceptance.generation_length_ok,
        no_repetition_collapse: inputs.inherited_acceptance.no_repetition_collapse,
        only_charset_v1_ids: inputs.inherited_acceptance.only_charset_v1_ids,
        beats_kn5_baseline: bpc_margin > S4_MIN_BPC_MARGIN_VS_KN5,
        ternary_qat_gap_ok: ternary_qat_gap <= S4_MAX_TERNARY_QAT_GAP,
        runtime_chrome_budget_ok: inputs.inherited_acceptance.runtime_chrome_budget_ok,
        emulator_smoke_ok: inputs.inherited_acceptance.emulator_smoke_ok,
    };
    let pass = v0_success_acceptance.all_set();
    let mut product = S4V0SuccessProduct {
        schema: S4_GUTENBERG_SCORE_SCHEMA.to_owned(),
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        seed: inputs.seed,
        checkpoint_self_hash: inputs.checkpoint_self_hash,
        checkpoint_payload_sha: inputs.checkpoint_payload_sha,
        corpus_val_sha: inputs.corpus_val_sha,
        workload_manifest_template_self_hash: inputs.workload_manifest_template_self_hash,
        workload_manifest_instance_self_hash: inputs.workload_manifest_instance_self_hash,
        fp_reference_self_hash: inputs.fp_reference_self_hash,
        bpc_ternary: inputs.bpc_ternary,
        bpc_kn5: inputs.bpc_kn5,
        bpc_margin,
        v0_success_acceptance,
        pass,
        score_self_hash: Hash256::ZERO,
    };
    product.validate_structure()?;
    product.score_self_hash = product.compute_self_hash()?;
    tracing::info!(
        target: S4_SCORE_LOG_TARGET,
        event_name = "s4::score::v0_success_complete",
        seed = product.seed,
        bpc_ternary = product.bpc_ternary.get(),
        bpc_kn5 = product.bpc_kn5.get(),
        bpc_margin = product.bpc_margin,
        pass = product.pass,
        score_self_hash = %product.score_self_hash,
        "s4 gutenberg v0_success score emitted"
    );
    Ok(product)
}

/// Evaluate the H4 strict all-seed Gutenberg v0_success gate.
pub fn strict_v0_success_on_gutenberg(
    products: &[S4V0SuccessProduct],
) -> Result<S4StrictV0SuccessOutcome, S4ScoreError> {
    let mut observed = Vec::with_capacity(products.len());
    for product in products {
        product.validate_canonical_write()?;
        observed.push(product.seed);
    }
    observed.sort_unstable();
    if observed != S4_CANONICAL_SEEDS.to_vec() {
        return Err(S4ScoreError::NonCanonicalScoreSeedSet { observed });
    }
    let mut by_seed = products.iter().collect::<Vec<_>>();
    by_seed.sort_unstable_by_key(|product| product.seed);
    for product in by_seed {
        if !(product.pass && product.v0_success_acceptance.all_set()) {
            return Ok(S4StrictV0SuccessOutcome::Fail {
                failing_seed: product.seed,
            });
        }
    }
    Ok(S4StrictV0SuccessOutcome::Pass)
}

fn validate_hash(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S4ScoreError> {
    if expected == observed {
        Ok(())
    } else {
        Err(S4ScoreError::HashMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn validate_nonzero_hash(field: &'static str, hash: Hash256) -> Result<(), S4ScoreError> {
    if hash == Hash256::ZERO {
        Err(S4ScoreError::MissingHash { field })
    } else {
        Ok(())
    }
}

fn validate_finite(field: &'static str, value: f64) -> Result<(), S4ScoreError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(S4ScoreError::NonFinite { field, value })
    }
}

/// Errors from the S4 Gutenberg score surface.
#[derive(Debug)]
pub enum S4ScoreError {
    /// The validation corpus hash did not match the supplied bytes.
    HashMismatch {
        /// Rejected field.
        field: &'static str,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// A required lineage hash was zero.
    MissingHash {
        /// Rejected field.
        field: &'static str,
    },
    /// Schema id did not match `s4_gutenberg_score.v1`.
    InvalidSchema {
        /// Observed schema id.
        observed: String,
    },
    /// Schema id did not match `s4_reference_model_bundle_reexport_evidence.v1`.
    InvalidReexportEvidenceSchema {
        /// Observed schema id.
        observed: String,
    },
    /// Re-export evidence did not name the S3-pinned Gutenberg fixture semantics.
    InvalidReexportFixtureSemantics {
        /// Observed semantics label.
        observed: String,
    },
    /// A floating-point field must be finite.
    NonFinite {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// Serialized bpc margin drifted from `bpc_kn5 - bpc_ternary`.
    BpcMarginMismatch {
        /// Recomputed margin.
        expected: f64,
        /// Stored margin.
        observed: f64,
    },
    /// A derived acceptance bit drifted from its predicate.
    AcceptanceBitMismatch {
        /// Rejected field.
        field: &'static str,
        /// Expected bit.
        expected: bool,
        /// Observed bit.
        observed: bool,
    },
    /// The stored pass bit drifted from the AND over acceptance bits.
    PassBitMismatch {
        /// Expected pass bit.
        expected: bool,
        /// Observed pass bit.
        observed: bool,
    },
    /// Score self-hash mismatch.
    SelfHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// ReferenceModelBundle re-export evidence self-hash mismatch.
    ReferenceModelBundleReexportSelfHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// Re-export program validation failed before evidence binding.
    ReferenceModelBundleReexportValidationFailed,
    /// Strict H4 scoring requires the exact five canonical S4 seeds.
    NonCanonicalScoreSeedSet {
        /// Sorted observed seeds.
        observed: Vec<u64>,
    },
    /// Underlying reset-context scorer failed.
    Score(ScoreError),
    /// S4 schema validation failed.
    Schema(S4SchemaError),
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for S4ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "S4 score hash mismatch for {field}: expected {expected}, observed {observed}"
            ),
            Self::MissingHash { field } => write!(f, "S4 score field {field} must be non-zero"),
            Self::InvalidSchema { observed } => write!(
                f,
                "S4 score schema must be {S4_GUTENBERG_SCORE_SCHEMA}, got {observed}"
            ),
            Self::InvalidReexportEvidenceSchema { observed } => write!(
                f,
                "S4 ReferenceModelBundle re-export evidence schema must be {}, got {observed}",
                S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA
            ),
            Self::InvalidReexportFixtureSemantics { observed } => write!(
                f,
                "S4 ReferenceModelBundle re-export evidence fixture semantics must be {}, got {observed}",
                S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS
            ),
            Self::NonFinite { field, value } => {
                write!(f, "S4 score field {field} must be finite, got {value}")
            }
            Self::BpcMarginMismatch { expected, observed } => write!(
                f,
                "S4 bpc_margin mismatch: expected {expected}, observed {observed}"
            ),
            Self::AcceptanceBitMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "S4 acceptance bit {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::PassBitMismatch { expected, observed } => write!(
                f,
                "S4 pass bit mismatch: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "S4 score self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ReferenceModelBundleReexportSelfHashMismatch { expected, observed } => write!(
                f,
                "S4 ReferenceModelBundle re-export evidence self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ReferenceModelBundleReexportValidationFailed => write!(
                f,
                "S4 ReferenceModelBundle re-export evidence requires passing S3 program validation"
            ),
            Self::NonCanonicalScoreSeedSet { observed } => write!(
                f,
                "S4 strict Gutenberg v0_success requires canonical seeds {:?}, got {observed:?}",
                S4_CANONICAL_SEEDS
            ),
            Self::Score(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4ScoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Score(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::HashMismatch { .. }
            | Self::MissingHash { .. }
            | Self::InvalidSchema { .. }
            | Self::InvalidReexportEvidenceSchema { .. }
            | Self::InvalidReexportFixtureSemantics { .. }
            | Self::NonFinite { .. }
            | Self::BpcMarginMismatch { .. }
            | Self::AcceptanceBitMismatch { .. }
            | Self::PassBitMismatch { .. }
            | Self::SelfHashMismatch { .. }
            | Self::ReferenceModelBundleReexportSelfHashMismatch { .. }
            | Self::ReferenceModelBundleReexportValidationFailed
            | Self::NonCanonicalScoreSeedSet { .. } => None,
        }
    }
}

impl From<ScoreError> for S4ScoreError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<S4SchemaError> for S4ScoreError {
    fn from(error: S4SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<CanonicalJsonError> for S4ScoreError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
