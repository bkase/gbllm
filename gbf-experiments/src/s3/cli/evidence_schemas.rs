//! Canonical JSON evidence envelopes emitted by `gbf s3 ...` commands.

use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256};
use gbf_oracle::phase_surface_agreement::AgreementProduct;
use serde::{Deserialize, Serialize};

use crate::s3::baseline::KnBaselineProduct;
use crate::s3::oracle_re_run::OracleReRunReport;
use crate::s3::schema::{
    CharsetProductRecord, OracleFallbackTag, S3ArtifactMetadata, S3ArtifactTiedEmbeddingAlias,
    S3BuildKind, S3BundleMetadata,
};

/// Evidence schema emitted by `gbf s3 replay-full` and `gbf s3 replay-fallback`.
pub const S3_REPLAY_FULL_CLI_SCHEMA: &str = "s3_replay_full_cli.v1";
/// Evidence schema emitted by `gbf s3 verify-determinism`.
pub const S3_VERIFY_DETERMINISM_CLI_SCHEMA: &str = "s3_verify_determinism_cli.v1";
/// Evidence schema emitted by `gbf s3 export-bundle`.
pub const S3_EXPORT_BUNDLE_CLI_SCHEMA: &str = "s3_export_bundle_cli.v1";
/// Evidence schema emitted by `gbf s3 export-artifact`.
pub const S3_EXPORT_ARTIFACT_CLI_SCHEMA: &str = "s3_export_artifact_cli.v1";
/// Evidence schema emitted by `gbf s3 oracle-agreement`.
pub const S3_ORACLE_AGREEMENT_CLI_SCHEMA: &str = "s3_oracle_agreement_cli.v1";
/// Evidence schema emitted by `gbf s3 normalize-corpus`.
pub const S3_CHARSET_NORMALIZE_CLI_SCHEMA: &str = "s3_charset_normalize_cli.v1";
/// Evidence schema emitted by `gbf s3 fit-baseline`.
pub const S3_FIT_BASELINE_CLI_SCHEMA: &str = "s3_fit_baseline_cli.v1";
/// Evidence schema emitted by `gbf s3 oracle-re-run`.
pub const S3_ORACLE_RE_RUN_CLI_SCHEMA: &str = "s3_oracle_re_run_cli.v1";
/// Evidence schema emitted by `gbf s3 report`.
pub const S3_REPORT_CLI_SCHEMA: &str = "s3_report_cli.v1";

/// Per-seed evidence carried by `s3_replay_full_cli.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ReplaySeedEvidence {
    /// S3 seed.
    pub seed: u64,
    /// Teacher checkpoint/freeze evidence hash.
    pub teacher_checkpoint_self_hash: Hash256,
    /// Student checkpoint/freeze evidence hash.
    pub student_checkpoint_self_hash: Hash256,
    /// Reference bundle self-hash.
    pub bundle_self_hash: Hash256,
    /// Model artifact self-hash.
    pub artifact_self_hash: Hash256,
    /// Oracle agreement product self-hash.
    pub agreement_self_hash: Hash256,
    /// Generation log evidence self-hash.
    pub generation_log_self_hash: Hash256,
}

/// Replay evidence for a seed/build matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ReplayFullCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Corpus manifest used by the replay.
    pub manifest: String,
    /// Workload manifest used by the replay.
    pub workload: String,
    /// S3 pass version requested by the caller.
    #[serde(rename = "pass_version_S3")]
    pub pass_version_s3: String,
    /// S3 build kind requested by the caller.
    pub build_kind: S3BuildKind,
    /// Deterministic device profile requested by the caller.
    pub device_profile: String,
    /// Export visitor id requested by the caller.
    pub export_visitor_id: String,
    /// Per-seed checkpoint/export/agreement evidence.
    pub per_seed: Vec<S3ReplaySeedEvidence>,
    /// Workload manifest self-hash.
    pub workload_self_hash: Hash256,
    /// Baseline product self-hash consumed by the replay.
    pub baseline_self_hash: Hash256,
    /// v0_success product self-hash.
    pub v0_success_self_hash: Hash256,
    /// B19 conformance envelope self-hash consumed by B21 reporting.
    pub conformance_self_hash: Hash256,
    /// Named oracle fallback/provenance paths used during replay.
    pub oracle_fallback_used: Vec<OracleFallbackTag>,
}

impl S3ReplayFullCliEvidence {
    /// Construct a replay evidence envelope.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        evidence_source: impl Into<String>,
        manifest: impl Into<String>,
        workload: impl Into<String>,
        pass_version_s3: impl Into<String>,
        build_kind: S3BuildKind,
        device_profile: impl Into<String>,
        export_visitor_id: impl Into<String>,
        per_seed: Vec<S3ReplaySeedEvidence>,
        workload_self_hash: Hash256,
        baseline_self_hash: Hash256,
        v0_success_self_hash: Hash256,
        conformance_self_hash: Hash256,
        oracle_fallback_used: Vec<OracleFallbackTag>,
    ) -> Self {
        Self {
            schema: S3_REPLAY_FULL_CLI_SCHEMA.to_owned(),
            evidence_source: evidence_source.into(),
            manifest: manifest.into(),
            workload: workload.into(),
            pass_version_s3: pass_version_s3.into(),
            build_kind,
            device_profile: device_profile.into(),
            export_visitor_id: export_visitor_id.into(),
            per_seed,
            workload_self_hash,
            baseline_self_hash,
            v0_success_self_hash,
            conformance_self_hash,
            oracle_fallback_used,
        }
    }
}

/// Determinism replay comparison evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3VerifyDeterminismCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// S3 seed list compared by the verifier.
    pub seed_list: Vec<u64>,
    /// S3 build kind compared by the verifier.
    pub build_kind: S3BuildKind,
    /// SHA-256 of the first replay evidence bytes.
    pub first_replay_sha: Hash256,
    /// SHA-256 of the second replay evidence bytes.
    pub second_replay_sha: Hash256,
    /// True when replay bytes were exactly identical.
    pub passed: bool,
}

impl S3VerifyDeterminismCliEvidence {
    /// Construct verifier evidence.
    #[must_use]
    pub fn new(
        seed_list: Vec<u64>,
        build_kind: S3BuildKind,
        first_replay_sha: Hash256,
        second_replay_sha: Hash256,
    ) -> Self {
        Self {
            schema: S3_VERIFY_DETERMINISM_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 verify-determinism".to_owned(),
            seed_list,
            build_kind,
            first_replay_sha,
            second_replay_sha,
            passed: first_replay_sha == second_replay_sha,
        }
    }
}

/// Bundle-export evidence emitted alongside `s3_bundle.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ExportBundleCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// S3 seed.
    pub seed: u64,
    /// Reference bundle self-hash.
    pub bundle_self_hash: Hash256,
    /// SHA-256 of the canonical bundle payload bytes.
    pub canonical_bundle_payload_sha: Hash256,
    /// Full `s3_bundle.v1` metadata record.
    pub metadata: S3BundleMetadata,
}

impl S3ExportBundleCliEvidence {
    /// Construct bundle CLI evidence.
    #[must_use]
    pub fn new(metadata: S3BundleMetadata) -> Self {
        Self {
            schema: S3_EXPORT_BUNDLE_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 export-bundle".to_owned(),
            seed: metadata.seed,
            bundle_self_hash: metadata.bundle_self_hash,
            canonical_bundle_payload_sha: metadata.canonical_bundle_payload_sha,
            metadata,
        }
    }
}

/// Artifact-export evidence emitted alongside `s3_artifact.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ExportArtifactCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// S3 seed.
    pub seed: u64,
    /// Model artifact self-hash.
    pub artifact_self_hash: Hash256,
    /// SHA-256 of canonical artifact payload bytes.
    pub canonical_artifact_payload_sha: Hash256,
    /// Tied embedding/classifier alias metadata.
    pub tied_embedding_alias: Option<S3ArtifactTiedEmbeddingAlias>,
    /// Full `s3_artifact.v1` metadata record.
    pub metadata: S3ArtifactMetadata,
}

impl S3ExportArtifactCliEvidence {
    /// Construct artifact CLI evidence.
    #[must_use]
    pub fn new(metadata: S3ArtifactMetadata) -> Self {
        Self {
            schema: S3_EXPORT_ARTIFACT_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 export-artifact".to_owned(),
            seed: metadata.seed,
            artifact_self_hash: metadata.artifact_self_hash,
            canonical_artifact_payload_sha: metadata.canonical_artifact_payload_sha,
            tied_embedding_alias: metadata.tied_embedding_alias.clone(),
            metadata,
        }
    }
}

/// Oracle-agreement CLI evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3OracleAgreementCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Agreement product emitted by B17.
    pub agreement_product: AgreementProduct,
}

impl S3OracleAgreementCliEvidence {
    /// Construct oracle-agreement CLI evidence.
    #[must_use]
    pub fn new(agreement_product: AgreementProduct) -> Self {
        Self {
            schema: S3_ORACLE_AGREEMENT_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 oracle-agreement".to_owned(),
            agreement_product,
        }
    }
}

/// Charset normalization CLI evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3CharsetNormalizeCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Corpus manifest used for normalization.
    pub manifest: String,
    /// Canonical charset product.
    pub charset_product: CharsetProductRecord,
}

impl S3CharsetNormalizeCliEvidence {
    /// Construct charset CLI evidence.
    #[must_use]
    pub fn new(manifest: impl Into<String>, charset_product: CharsetProductRecord) -> Self {
        Self {
            schema: S3_CHARSET_NORMALIZE_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 normalize-corpus".to_owned(),
            manifest: manifest.into(),
            charset_product,
        }
    }
}

/// Baseline fitting CLI evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3FitBaselineCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Corpus manifest used for fitting.
    pub manifest: String,
    /// Canonical baseline product.
    pub baseline_product: KnBaselineProduct,
}

impl S3FitBaselineCliEvidence {
    /// Construct baseline CLI evidence.
    #[must_use]
    pub fn new(manifest: impl Into<String>, baseline_product: KnBaselineProduct) -> Self {
        Self {
            schema: S3_FIT_BASELINE_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 fit-baseline".to_owned(),
            manifest: manifest.into(),
            baseline_product,
        }
    }
}

/// Oracle re-run CLI evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3OracleReRunCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Whether the inherited S1 D7 suite passed under S3.
    pub s1_oracle_re_run_passed: bool,
    /// Whether the inherited F-S2 O3 suite passed under S3.
    pub s2_oracle_re_run_passed: bool,
    /// Number of inherited metric rows summarized by the report.
    pub metric_count: u64,
    /// Oracle re-run report self-hash.
    pub oracle_re_run_self_hash: Hash256,
}

impl S3OracleReRunCliEvidence {
    /// Construct oracle re-run CLI evidence.
    #[must_use]
    pub fn from_report(report: &OracleReRunReport) -> Self {
        Self {
            schema: S3_ORACLE_RE_RUN_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 oracle-re-run".to_owned(),
            s1_oracle_re_run_passed: report.s1_oracle_re_run_passed,
            s2_oracle_re_run_passed: report.s2_oracle_re_run_passed,
            metric_count: report.per_metric.len() as u64,
            oracle_re_run_self_hash: report.oracle_re_run_self_hash,
        }
    }
}

/// Evidence file consumed by `gbf s3 report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ReportConsumedEvidence {
    /// Logical report input kind.
    pub evidence_kind: String,
    /// Filesystem path supplied to the report command.
    pub path: String,
    /// Pinned schema literal observed in the consumed evidence file.
    pub schema: String,
    /// SHA-256 of the canonical consumed evidence envelope.
    pub evidence_sha: Hash256,
    /// Product/report self-hash carried by the consumed evidence.
    pub product_self_hash: Hash256,
    /// Optional seed associated with seed-scoped evidence.
    pub seed: Option<u64>,
}

impl S3ReportConsumedEvidence {
    /// Construct a consumed-evidence row for `s3_report_cli.v1`.
    #[must_use]
    pub fn new(
        evidence_kind: impl Into<String>,
        path: impl Into<String>,
        schema: impl Into<String>,
        evidence_sha: Hash256,
        product_self_hash: Hash256,
        seed: Option<u64>,
    ) -> Self {
        Self {
            evidence_kind: evidence_kind.into(),
            path: path.into(),
            schema: schema.into(),
            evidence_sha,
            product_self_hash,
            seed,
        }
    }
}

/// Report emission CLI evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ReportCliEvidence {
    /// Pinned evidence schema id.
    pub schema: String,
    /// CLI command that emitted this evidence.
    pub evidence_source: String,
    /// Markdown report path.
    pub output: String,
    /// `s3_report.v1` self-hash.
    pub report_self_hash: Hash256,
    /// Evidence files parsed and consumed while emitting the report.
    pub consumed_evidence: Vec<S3ReportConsumedEvidence>,
}

impl S3ReportCliEvidence {
    /// Construct report CLI evidence.
    #[must_use]
    pub fn new(
        output: impl Into<String>,
        report_self_hash: Hash256,
        consumed_evidence: Vec<S3ReportConsumedEvidence>,
    ) -> Self {
        Self {
            schema: S3_REPORT_CLI_SCHEMA.to_owned(),
            evidence_source: "gbf s3 report".to_owned(),
            output: output.into(),
            report_self_hash,
            consumed_evidence,
        }
    }
}

/// Encode any S3 CLI evidence envelope as canonical JSON bytes.
pub fn canonical_evidence_bytes<T>(evidence: &T) -> Result<Vec<u8>, CanonicalJsonError>
where
    T: Serialize,
{
    CanonicalJson::to_vec(evidence)
}
