//! Structured tracing contracts for S1 First Pulse.
//!
//! This module owns the central S1 logging schema and subscriber setup. It
//! deliberately does not wire real training, oracle, ablation, report, or CLI
//! producers; those adoption paths are owned by the downstream S1 beads that
//! implement the corresponding workflows.

use std::error::Error;
use std::fmt;

use tracing::Span;
use tracing_subscriber::EnvFilter;

/// Current S1 structured-log schema version.
pub const GBF_LOG_SCHEMA_VERSION: &str = "1.0.0";

/// Field names pinned for S1 structured log ingestion.
pub mod field {
    /// Canonical event-name field.
    pub const EVENT_NAME: &str = "event_name";
    /// SemVer schema version recorded on S1 log events and spans.
    pub const GBF_LOG_SCHEMA_VERSION: &str = "gbf_log_schema_version";
    /// Experiment seed.
    pub const SEED: &str = "seed";
    /// Pass/version label for a run.
    pub const PASS_VERSION: &str = "pass_version";
    /// Optimizer step.
    pub const STEP: &str = "step";
    /// Evaluation step.
    pub const EVAL_STEP: &str = "eval_step";
    /// S1 phase name.
    pub const PHASE: &str = "phase";
    /// Build kind under test.
    pub const BUILD_KIND: &str = "build_kind";
    /// Loss in natural-log units per byte.
    pub const LOSS_NATS_PER_BYTE: &str = "loss_nats_per_byte";
    /// L2 gradient norm.
    pub const GRAD_NORM_L2: &str = "grad_norm_l2";
    /// Bits-per-character metric value.
    pub const BPC_VALUE: &str = "bpc_value";
    /// Chunk index within a corpus or artifact stream.
    pub const CHUNK_INDEX: &str = "chunk_index";
    /// Byte offset for the first mismatch or diagnostic location.
    pub const BYTE_OFFSET: &str = "byte_offset";
    /// Tensor name for tensor-scoped diagnostics.
    pub const TENSOR_NAME: &str = "tensor_name";
    /// Measurement oracle identifier.
    pub const ORACLE_ID: &str = "oracle_id";
    /// Scenario outcome label.
    pub const OUTCOME: &str = "outcome";
    /// Decision label.
    pub const DECISION: &str = "decision";
    /// Hypothesis label.
    pub const HYPOTHESIS: &str = "hypothesis";
    /// Verdict label.
    pub const VERDICT: &str = "verdict";
    /// Canonical manifest-tree self hash.
    pub const CMT_SELF_HASH: &str = "cmt_self_hash";
    /// Training corpus hash.
    pub const CORPUS_TRAIN_SHA: &str = "corpus_train_sha";
    /// Validation corpus hash.
    pub const CORPUS_VAL_SHA: &str = "corpus_val_sha";
    /// End-to-end scenario name.
    pub const SCENARIO: &str = "scenario";
    /// Training budget profile.
    pub const BUDGET_PROFILE: &str = "budget_profile";
    /// Number of seeds covered by a scenario.
    pub const N_SEEDS: &str = "n_seeds";
    /// Boolean pass/fail summary.
    pub const PASS: &str = "pass";
    /// CLI subcommand name.
    pub const COMMAND: &str = "command";
    /// Deterministic fixture duration, in seconds.
    pub const DURATION_SECONDS: &str = "duration_seconds";
    /// Deterministic device profile hash.
    pub const DEVICE_PROFILE_HASH: &str = "device_profile_hash";
    /// Environment variable or profile field rejected by a precondition check.
    pub const REJECTED_VAR: &str = "rejected_var";
    /// Expected value for a rejected precondition input.
    pub const EXPECTED: &str = "expected";
    /// Human-readable failure or rejection reason.
    pub const REASON: &str = "reason";
    /// Divergence observation class.
    pub const OBSERVED: &str = "observed";
    /// Last finite loss before divergence.
    pub const LAST_FINITE_LOSS: &str = "last_finite_loss";
    /// Token count for scoring or evaluation events.
    pub const TOKEN_COUNT: &str = "token_count";
    /// Filesystem path metadata.
    pub const PATH: &str = "path";
    /// Checkpoint self hash.
    pub const CHECKPOINT_SELF_HASH: &str = "checkpoint_self_hash";
    /// Score artifact self hash.
    pub const SCORE_SELF_HASH: &str = "score_self_hash";
    /// Baseline artifact self hash.
    pub const BASELINE_SELF_HASH: &str = "baseline_self_hash";
    /// Baseline counts blob hash.
    pub const COUNTS_BLOB_SHA256: &str = "counts_blob_sha256";
    /// Baseline counts cardinality summary.
    pub const COUNTS_SUMMARY: &str = "counts_summary";
    /// Bytes processed so far.
    pub const BYTES_DONE: &str = "bytes_done";
    /// Trigram baseline BPC.
    pub const BPC_3GRAM: &str = "bpc_3gram";
    /// Bigram baseline BPC.
    pub const BPC_2GRAM: &str = "bpc_2gram";
    /// Unigram baseline BPC.
    pub const BPC_UNIGRAM: &str = "bpc_unigram";
    /// Diagnostic text that excludes corpus bytes, weights, and gradients.
    pub const DIAGNOSTIC: &str = "diagnostic";
    /// Report validator result summary.
    pub const VALIDATORS: &str = "validators";
    /// Report validator name.
    pub const VALIDATOR: &str = "validator";
    /// Report validator status.
    pub const STATUS: &str = "status";
    /// Report RFC revision.
    pub const RFC_REVISION: &str = "rfc_revision";
    /// Report self hash.
    pub const REPORT_SELF_HASH: &str = "report_self_hash";
    /// Report output path.
    pub const OUTPUT_PATH: &str = "output_path";
    /// Concrete hypothesis observation.
    pub const OBSERVATION: &str = "observation";
    /// Negative-test shuffled validation hash.
    pub const SHUFFLED_VAL_SHA256: &str = "shuffled_val_sha256";
    /// Negative-test original validation BPC.
    pub const BPC_ORIGINAL: &str = "bpc_original";
    /// Negative-test shuffled validation BPC.
    pub const BPC_SHUFFLED: &str = "bpc_shuffled";
    /// Negative-test shuffled-minus-original BPC delta.
    pub const DELTA: &str = "delta";
    /// Negative-test sensitivity verdict.
    pub const SENSITIVE: &str = "sensitive";
    /// Negative-test artifact self hash.
    pub const NEGATIVE_SELF_HASH: &str = "negative_self_hash";
    /// Fisher-Yates shuffle seed.
    pub const SHUFFLE_SEED: &str = "shuffle_seed";
    /// Whether any S1 seed diverged before completion.
    pub const ANY_SEED_DIVERGED: &str = "any_seed_diverged";
    /// Whether validation BPC was suspiciously low.
    pub const SUSPICIOUS_LOW_BPC: &str = "suspicious_low_bpc";
    /// Whether Phase A and ablation payload hashes matched.
    pub const PHASE_A_EQ_ABLATION: &str = "phase_a_eq_ablation";
    /// Ablation report self hash.
    pub const ABLATION_SELF_HASH: &str = "ablation_self_hash";
    /// Oracle report self hash.
    pub const ORACLE_SELF_HASH: &str = "oracle_self_hash";
    /// Aggregate metric-oracle verdict.
    pub const METRIC_ORACLE_PASSED: &str = "metric_oracle_passed";
    /// JSON array string of failed oracle ids.
    pub const FAILED_ORACLE_IDS: &str = "failed_oracle_ids";

    /// Full pinned field-name list for schema consumers.
    pub const ALL: &[&str] = &[
        EVENT_NAME,
        GBF_LOG_SCHEMA_VERSION,
        SEED,
        PASS_VERSION,
        STEP,
        EVAL_STEP,
        PHASE,
        BUILD_KIND,
        LOSS_NATS_PER_BYTE,
        GRAD_NORM_L2,
        BPC_VALUE,
        CHUNK_INDEX,
        BYTE_OFFSET,
        TENSOR_NAME,
        ORACLE_ID,
        OUTCOME,
        DECISION,
        HYPOTHESIS,
        VERDICT,
        CMT_SELF_HASH,
        CORPUS_TRAIN_SHA,
        CORPUS_VAL_SHA,
        SCENARIO,
        BUDGET_PROFILE,
        N_SEEDS,
        PASS,
        COMMAND,
        DURATION_SECONDS,
        DEVICE_PROFILE_HASH,
        REJECTED_VAR,
        EXPECTED,
        REASON,
        OBSERVED,
        LAST_FINITE_LOSS,
        TOKEN_COUNT,
        PATH,
        CHECKPOINT_SELF_HASH,
        SCORE_SELF_HASH,
        BASELINE_SELF_HASH,
        COUNTS_BLOB_SHA256,
        COUNTS_SUMMARY,
        BYTES_DONE,
        BPC_3GRAM,
        BPC_2GRAM,
        BPC_UNIGRAM,
        DIAGNOSTIC,
        VALIDATORS,
        VALIDATOR,
        STATUS,
        RFC_REVISION,
        REPORT_SELF_HASH,
        OUTPUT_PATH,
        OBSERVATION,
        SHUFFLED_VAL_SHA256,
        BPC_ORIGINAL,
        BPC_SHUFFLED,
        DELTA,
        SENSITIVE,
        NEGATIVE_SELF_HASH,
        SHUFFLE_SEED,
        ANY_SEED_DIVERGED,
        SUSPICIOUS_LOW_BPC,
        PHASE_A_EQ_ABLATION,
        ABLATION_SELF_HASH,
        ORACLE_SELF_HASH,
        METRIC_ORACLE_PASSED,
        FAILED_ORACLE_IDS,
    ];
}

/// Event names pinned for S1 structured log ingestion.
pub mod event {
    /// Run precondition failure.
    pub const RUN_PRECONDITION_FAILED: &str = "run.precondition_failed";
    /// Non-finite run divergence.
    pub const RUN_DIVERGENCE: &str = "run.divergence";
    /// Evaluation progress emitted at evaluation points.
    pub const RUN_EVAL_PROGRESS: &str = "run.eval_progress";
    /// Checkpoint emission.
    pub const RUN_CHECKPOINT_EMITTED: &str = "run.checkpoint_emitted";
    /// Score computation start.
    pub const SCORE_START: &str = "score.start";
    /// Score computation progress.
    pub const SCORE_PROGRESS: &str = "score.progress";
    /// Score completion.
    pub const SCORE_COMPLETE: &str = "score.complete";
    /// Baseline fitting start.
    pub const BASELINE_FIT_START: &str = "s1.baseline.fit.start";
    /// Baseline fitting progress.
    pub const BASELINE_FIT_PROGRESS: &str = "s1.baseline.fit.progress";
    /// Baseline fitting completion.
    pub const BASELINE_FIT_COMPLETE: &str = "s1.baseline.fit.complete";
    /// Baseline validation scoring start.
    pub const BASELINE_SCORE_START: &str = "s1.baseline.score.start";
    /// Baseline validation scoring completion.
    pub const BASELINE_SCORE_COMPLETE: &str = "s1.baseline.score.complete";
    /// Baseline completion.
    pub const BASELINE_COMPLETE: &str = "baseline.complete";
    /// Measurement oracle failure.
    pub const ORACLE_FAILED: &str = "s1.oracle.failed";
    /// Measurement oracle start.
    pub const ORACLE_START: &str = "s1.oracle.start";
    /// Measurement oracle completion.
    pub const ORACLE_COMPLETE: &str = "s1.oracle.complete";
    /// Measurement oracle aggregate completion.
    pub const ORACLE_AGGREGATE_COMPLETE: &str = "s1.oracle.aggregate.complete";
    /// Ablation comparison start.
    pub const ABLATION_COMPARE_START: &str = "s1.ablation.compare.start";
    /// Ablation metadata preflight failure.
    pub const ABLATION_METADATA_CHECK_FAIL: &str = "s1.ablation.metadata_check.fail";
    /// Ablation per-tensor comparison trace.
    pub const ABLATION_TENSOR_COMPARE: &str = "s1.ablation.tensor_compare";
    /// Ablation mismatch.
    pub const ABLATION_MISMATCH: &str = "s1.ablation.mismatch";
    /// Ablation comparison completion.
    pub const ABLATION_COMPLETE: &str = "s1.ablation.complete";
    /// Report validator execution summary.
    pub const REPORT_VALIDATORS_RUN: &str = "report.validators_run";
    /// Report emission start.
    pub const REPORT_EMIT_START: &str = "s1.report.emit.start";
    /// Single report validator result.
    pub const REPORT_VALIDATOR: &str = "s1.report.validator";
    /// Report emission completion.
    pub const REPORT_EMIT_COMPLETE: &str = "s1.report.emit.complete";
    /// Outcome dispatch start.
    pub const OUTCOME_DISPATCH_START: &str = "s1.outcome.dispatch.start";
    /// Outcome dispatch completion.
    pub const OUTCOME_DISPATCH_COMPLETE: &str = "s1.outcome.dispatch.complete";
    /// Observation-bearing refuted outcome input.
    pub const OUTCOME_REFUTED_INPUT: &str = "s1.outcome.refuted_input";
    /// S1 device-profile enforcement started before tensor allocation.
    pub const DEVICE_PROFILE_ENFORCE_START: &str = "s1.device_profile.enforce.start";
    /// S1 device-profile enforcement rejected an input before tensor allocation.
    pub const DEVICE_PROFILE_ENFORCE_FAIL: &str = "s1.device_profile.enforce.fail";
    /// S1 device-profile enforcement completed before tensor allocation.
    pub const DEVICE_PROFILE_ENFORCE_OK: &str = "s1.device_profile.enforce.ok";
    /// Negative-test shuffle start.
    pub const NEG_TEST_SHUFFLE_START: &str = "s1.neg_test.shuffle.start";
    /// Negative-test shuffle completion.
    pub const NEG_TEST_SHUFFLE_COMPLETE: &str = "s1.neg_test.shuffle.complete";
    /// Negative-test shuffle pin mismatch.
    pub const NEG_TEST_SHUFFLE_PIN_MISMATCH: &str = "s1.neg_test.shuffle.pin_mismatch";
    /// Negative-test scoring start.
    pub const NEG_TEST_SCORE_START: &str = "s1.neg_test.score.start";
    /// Negative-test scoring completion.
    pub const NEG_TEST_SCORE_COMPLETE: &str = "s1.neg_test.score.complete";
    /// Negative-test completion.
    pub const NEG_TEST_COMPLETE: &str = "s1.neg_test.complete";
    /// Manifest shuffle-pin computation.
    pub const MANIFEST_SHUFFLE_PIN_COMPUTE: &str = "s1.manifest.shuffle_pin.compute";
    /// Manifest shuffle-pin verification success.
    pub const MANIFEST_SHUFFLE_PIN_VERIFY_OK: &str = "s1.manifest.shuffle_pin.verify.ok";
    /// Manifest shuffle-pin verification failure.
    pub const MANIFEST_SHUFFLE_PIN_VERIFY_FAIL: &str = "s1.manifest.shuffle_pin.verify.fail";
    /// E2E scenario start.
    pub const E2E_SCENARIO_START: &str = "s1.e2e.scenario.start";
    /// E2E scenario phase transition.
    pub const E2E_PHASE: &str = "s1.e2e.phase";
    /// E2E scenario completion.
    pub const E2E_SCENARIO_COMPLETE: &str = "s1.e2e.scenario.complete";
    /// Tiny-fixture integration smoke scenario start.
    pub const INTEGRATION_SMOKE_SCENARIO_START: &str = "s1.integration_smoke.scenario.start";
    /// Tiny-fixture integration smoke scenario completion.
    pub const INTEGRATION_SMOKE_SCENARIO_COMPLETE: &str = "s1.integration_smoke.scenario.complete";
    /// Diagnostic CLI subcommand started.
    pub const CLI_DIAGNOSTIC_START: &str = "s1.cli.diagnostic.start";
    /// Diagnostic CLI subcommand completed.
    pub const CLI_DIAGNOSTIC_COMPLETE: &str = "s1.cli.diagnostic.complete";
    /// Diagnostic CLI subcommand failed.
    pub const CLI_DIAGNOSTIC_FAILED: &str = "s1.cli.diagnostic.failed";

    /// Full pinned event-name list for schema consumers.
    pub const ALL: &[&str] = &[
        RUN_PRECONDITION_FAILED,
        RUN_DIVERGENCE,
        RUN_EVAL_PROGRESS,
        RUN_CHECKPOINT_EMITTED,
        SCORE_START,
        SCORE_PROGRESS,
        SCORE_COMPLETE,
        BASELINE_FIT_START,
        BASELINE_FIT_PROGRESS,
        BASELINE_FIT_COMPLETE,
        BASELINE_SCORE_START,
        BASELINE_SCORE_COMPLETE,
        BASELINE_COMPLETE,
        ORACLE_START,
        ORACLE_COMPLETE,
        ORACLE_FAILED,
        ORACLE_AGGREGATE_COMPLETE,
        ABLATION_COMPARE_START,
        ABLATION_METADATA_CHECK_FAIL,
        ABLATION_TENSOR_COMPARE,
        ABLATION_MISMATCH,
        ABLATION_COMPLETE,
        REPORT_VALIDATORS_RUN,
        REPORT_EMIT_START,
        REPORT_VALIDATOR,
        REPORT_EMIT_COMPLETE,
        OUTCOME_DISPATCH_START,
        OUTCOME_DISPATCH_COMPLETE,
        OUTCOME_REFUTED_INPUT,
        DEVICE_PROFILE_ENFORCE_START,
        DEVICE_PROFILE_ENFORCE_FAIL,
        DEVICE_PROFILE_ENFORCE_OK,
        NEG_TEST_SHUFFLE_START,
        NEG_TEST_SHUFFLE_COMPLETE,
        NEG_TEST_SHUFFLE_PIN_MISMATCH,
        NEG_TEST_SCORE_START,
        NEG_TEST_SCORE_COMPLETE,
        NEG_TEST_COMPLETE,
        MANIFEST_SHUFFLE_PIN_COMPUTE,
        MANIFEST_SHUFFLE_PIN_VERIFY_OK,
        MANIFEST_SHUFFLE_PIN_VERIFY_FAIL,
        E2E_SCENARIO_START,
        E2E_PHASE,
        E2E_SCENARIO_COMPLETE,
        INTEGRATION_SMOKE_SCENARIO_START,
        INTEGRATION_SMOKE_SCENARIO_COMPLETE,
        CLI_DIAGNOSTIC_START,
        CLI_DIAGNOSTIC_COMPLETE,
        CLI_DIAGNOSTIC_FAILED,
    ];
}

/// Span names pinned for S1 structured log ingestion.
pub mod span {
    /// Per-seed S1 run span.
    pub const RUN: &str = "s1.run";
    /// Per-optimizer-step span.
    pub const RUN_STEP: &str = "s1.run.step";
    /// Per-evaluation span.
    pub const RUN_EVAL: &str = "s1.run.eval";
    /// Reset-context scoring span.
    pub const SCORE: &str = "s1.score";
    /// Baseline fitting span.
    pub const BASELINE_FIT: &str = "s1.baseline.fit";
    /// Baseline scoring span.
    pub const BASELINE_SCORE: &str = "s1.baseline.score";
    /// Negative-test span.
    pub const NEG_TEST: &str = "s1.neg_test";
    /// Ablation-comparison span.
    pub const ABLATION: &str = "s1.ablation";
    /// Measurement oracle 0 span.
    pub const ORACLE_0: &str = "s1.oracle.0";
    /// Measurement oracle 1 span.
    pub const ORACLE_1: &str = "s1.oracle.1";
    /// Measurement oracle 2 span.
    pub const ORACLE_2: &str = "s1.oracle.2";
    /// Measurement oracle 3 span.
    pub const ORACLE_3: &str = "s1.oracle.3";
    /// Measurement oracle 4 span.
    pub const ORACLE_4: &str = "s1.oracle.4";
    /// Report emission span.
    pub const REPORT_EMIT: &str = "s1.report.emit";
    /// CLI replay subcommand span.
    pub const CLI_REPLAY: &str = "s1.cli.replay";
    /// CLI fit-baseline subcommand span.
    pub const CLI_FIT_BASELINE: &str = "s1.cli.fit-baseline";
    /// CLI oracle subcommand span.
    pub const CLI_ORACLE: &str = "s1.cli.oracle";
    /// CLI verify-determinism subcommand span.
    pub const CLI_VERIFY_DETERMINISM: &str = "s1.cli.verify-determinism";
    /// CLI doctor diagnostic subcommand span.
    pub const CLI_DOCTOR: &str = "s1.cli.doctor";
    /// CLI inspect diagnostic subcommand span.
    pub const CLI_INSPECT: &str = "s1.cli.inspect";
    /// CLI diff-checkpoints diagnostic subcommand span.
    pub const CLI_DIFF_CHECKPOINTS: &str = "s1.cli.diff-checkpoints";
    /// CLI print-config diagnostic subcommand span.
    pub const CLI_PRINT_CONFIG: &str = "s1.cli.print-config";
    /// Tiny-fixture end-to-end scenario span.
    pub const E2E_SCENARIO: &str = "s1.e2e.scenario";

    /// Full pinned span-name list for schema consumers.
    pub const ALL: &[&str] = &[
        RUN,
        RUN_STEP,
        RUN_EVAL,
        SCORE,
        BASELINE_FIT,
        BASELINE_SCORE,
        NEG_TEST,
        ABLATION,
        ORACLE_0,
        ORACLE_1,
        ORACLE_2,
        ORACLE_3,
        ORACLE_4,
        REPORT_EMIT,
        CLI_REPLAY,
        CLI_FIT_BASELINE,
        CLI_ORACLE,
        CLI_VERIFY_DETERMINISM,
        CLI_DOCTOR,
        CLI_INSPECT,
        CLI_DIFF_CHECKPOINTS,
        CLI_PRINT_CONFIG,
        E2E_SCENARIO,
    ];
}

/// Follow-up beads that own producer adoption of this logging contract.
pub const PRODUCER_ADOPTION_BOUNDARIES: &[(&str, &str)] = &[
    ("run producer and run-log sidecar", "bd-1xo5"),
    ("run-log gbf_log_schema_version artifact field", "bd-1xo5"),
    ("checkpoint writer trace centralization", "bd-1xo5"),
    ("DomainHash/report schema trace centralization", "bd-16mx"),
    (
        "oracle producer canonical full-protocol invocation",
        "bd-1ehz",
    ),
    ("report producer events", "bd-3v7y"),
    ("SURPRISE outcome producer events", "bd-16mx"),
    ("CLI span propagation", "bd-7ljt"),
    ("end-to-end golden structured logs", "bd-16mx"),
];

/// S1 logging profile selected by binaries and integration tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggingProfile {
    /// Pretty stderr logging, controlled by `RUST_LOG`.
    Human,
    /// Newline-delimited JSON stderr logging for CI and ingestion.
    Json,
    /// Test-writer logging for `tracing-test` based assertions.
    Test,
}

impl LoggingProfile {
    /// Default filter used when `RUST_LOG` is not present.
    pub const fn default_filter_spec(self) -> &'static str {
        match self {
            Self::Human | Self::Json => "gbf_experiments::s1=info",
            Self::Test => "gbf_experiments::s1=trace",
        }
    }
}

/// Initialize global S1 tracing according to a profile.
pub fn init(profile: LoggingProfile) -> Result<(), LoggingInitError> {
    let filter = env_filter(profile)?;
    match profile {
        LoggingProfile::Human => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
            .map_err(LoggingInitError::Subscriber),
        LoggingProfile::Json => tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
            .map_err(LoggingInitError::Subscriber),
        LoggingProfile::Test => tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(filter)
            .try_init()
            .map_err(LoggingInitError::Subscriber),
    }
}

fn env_filter(profile: LoggingProfile) -> Result<EnvFilter, LoggingInitError> {
    let spec = std::env::var("RUST_LOG").unwrap_or_else(|_| profile.default_filter_spec().into());
    EnvFilter::try_new(spec).map_err(|error| LoggingInitError::Filter {
        detail: error.to_string(),
    })
}

/// Errors returned while initializing S1 logging.
#[derive(Debug)]
pub enum LoggingInitError {
    /// `RUST_LOG` or the profile default could not be parsed.
    Filter {
        /// Parser detail.
        detail: String,
    },
    /// A global subscriber could not be installed.
    Subscriber(Box<dyn Error + Send + Sync + 'static>),
}

impl fmt::Display for LoggingInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Filter { detail } => write!(f, "invalid S1 tracing filter: {detail}"),
            Self::Subscriber(error) => {
                write!(f, "failed to initialize S1 tracing subscriber: {error}")
            }
        }
    }
}

impl Error for LoggingInitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Subscriber(error) => Some(error.as_ref()),
            Self::Filter { .. } => None,
        }
    }
}

/// Errors returned when constructing S1 log events or spans.
#[derive(Debug, Clone, PartialEq)]
pub enum LoggingEventError {
    /// A required string field was empty.
    EmptyField {
        /// Field name.
        name: &'static str,
    },
    /// A floating-point field was not finite.
    NonFiniteField {
        /// Field name.
        name: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// An oracle id outside the D7 0..4 range was requested.
    InvalidOracleId {
        /// Rejected oracle id.
        oracle_id: u8,
    },
}

impl fmt::Display for LoggingEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { name } => write!(f, "S1 log field {name} must not be empty"),
            Self::NonFiniteField { name, value } => {
                write!(f, "S1 log field {name} must be finite, got {value}")
            }
            Self::InvalidOracleId { oracle_id } => {
                write!(f, "S1 oracle_id must be in 0..=4, got {oracle_id}")
            }
        }
    }
}

impl Error for LoggingEventError {}

/// Divergence observation class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceObserved {
    /// Loss became non-finite.
    NonFiniteLoss,
    /// Gradient norm became non-finite.
    NonFiniteGrad,
    /// Gradient norm collapsed to zero under a falsification hook.
    ZeroGrad,
}

impl DivergenceObserved {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NonFiniteLoss => "non_finite_loss",
            Self::NonFiniteGrad => "non_finite_grad",
            Self::ZeroGrad => "zero_grad",
        }
    }
}

/// Fields latched on the per-seed `s1.run` span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpanFields {
    seed: u64,
    pass_version: String,
    phase: String,
    build_kind: String,
    corpus_train_sha: String,
    corpus_val_sha: String,
    device_profile_hash: String,
}

impl RunSpanFields {
    /// Construct fields for a per-seed run span.
    pub fn new(
        seed: u64,
        pass_version: impl Into<String>,
        phase: impl Into<String>,
        build_kind: impl Into<String>,
        corpus_train_sha: impl Into<String>,
        corpus_val_sha: impl Into<String>,
        device_profile_hash: impl Into<String>,
    ) -> Result<Self, LoggingEventError> {
        let fields = Self {
            seed,
            pass_version: pass_version.into(),
            phase: phase.into(),
            build_kind: build_kind.into(),
            corpus_train_sha: corpus_train_sha.into(),
            corpus_val_sha: corpus_val_sha.into(),
            device_profile_hash: device_profile_hash.into(),
        };
        validate_nonempty(field::PASS_VERSION, &fields.pass_version)?;
        validate_nonempty(field::PHASE, &fields.phase)?;
        validate_nonempty(field::BUILD_KIND, &fields.build_kind)?;
        validate_nonempty(field::CORPUS_TRAIN_SHA, &fields.corpus_train_sha)?;
        validate_nonempty(field::CORPUS_VAL_SHA, &fields.corpus_val_sha)?;
        validate_nonempty(field::DEVICE_PROFILE_HASH, &fields.device_profile_hash)?;
        Ok(fields)
    }
}

/// Fields latched on an optimizer-step span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunStepSpanFields {
    /// Experiment seed.
    pub seed: u64,
    /// Optimizer step.
    pub step: u64,
    /// Current phase.
    pub phase: &'static str,
    /// Loss diagnostic.
    pub loss_nats_per_byte: f64,
    /// Gradient norm diagnostic.
    pub grad_norm_l2: f64,
}

/// Fields latched on an evaluation span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunEvalSpanFields {
    /// Experiment seed.
    pub seed: u64,
    /// Evaluation step.
    pub eval_step: u64,
    /// Current phase.
    pub phase: &'static str,
}

/// A run precondition failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPreconditionFailedEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Human-readable reason, excluding sensitive payload bytes.
    pub reason: String,
}

/// A device-profile enforcement start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfileEnforceStartEvent {
    /// Deterministic device profile hash.
    pub device_profile_hash: String,
}

/// A device-profile enforcement failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfileEnforceFailEvent {
    /// Deterministic device profile hash.
    pub device_profile_hash: String,
    /// Rejected environment variable or profile field.
    pub rejected_var: String,
    /// Expected value.
    pub expected: String,
    /// Observed value.
    pub observed: String,
}

/// A device-profile enforcement success event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfileEnforceOkEvent {
    /// Deterministic device profile hash.
    pub device_profile_hash: String,
}

/// A run divergence event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunDivergenceEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Optimizer step.
    pub step: u64,
    /// Divergence observation class.
    pub observed: DivergenceObserved,
    /// Last finite loss before divergence.
    pub last_finite_loss: f64,
}

/// An evaluation progress event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunEvalProgressEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Evaluation step.
    pub eval_step: u64,
    /// Evaluation BPC.
    pub bpc_value: f64,
    /// Token count used for evaluation.
    pub token_count: u64,
}

/// A checkpoint emission event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCheckpointEmittedEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Optimizer step.
    pub step: u64,
    /// Checkpoint path.
    pub path: String,
    /// Checkpoint self hash.
    pub checkpoint_self_hash: String,
}

/// A score start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Total validation tokens scheduled for scoring.
    pub token_count: u64,
}

/// A score progress event emitted after a reset-context chunk is scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreProgressEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Zero-based reset-context chunk index.
    pub chunk_index: u64,
    /// Validation tokens scored so far.
    pub token_count: u64,
}

/// A score completion event.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Score BPC.
    pub bpc_value: f64,
    /// Token count used for scoring.
    pub token_count: u64,
    /// Score artifact self hash.
    pub score_self_hash: String,
}

/// A baseline fit start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineFitStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Training corpus hash.
    pub corpus_train_sha: String,
    /// Training byte count scheduled for fitting.
    pub train_bytes: u64,
}

/// A baseline fit progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineFitProgressEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Bytes processed so far.
    pub bytes_done: u64,
    /// Total training bytes scheduled for fitting.
    pub train_bytes: u64,
}

/// A baseline fit complete event.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineFitCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Trigram BPC.
    pub bpc_3gram: f64,
    /// Bigram BPC.
    pub bpc_2gram: f64,
    /// Unigram BPC.
    pub bpc_unigram: f64,
    /// Baseline counts blob hash.
    pub counts_blob_sha256: String,
    /// Redacted counts cardinality summary.
    pub counts_summary: String,
    /// Baseline report self hash.
    pub baseline_self_hash: String,
}

/// A baseline score start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineScoreStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Validation token count.
    pub token_count: u64,
}

/// A baseline score complete event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineScoreCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Validation token count.
    pub token_count: u64,
    /// Baseline report self hash.
    pub baseline_self_hash: String,
}

/// A baseline completion event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaselineCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Trigram BPC.
    pub bpc_3gram: f64,
    /// Bigram BPC.
    pub bpc_2gram: f64,
    /// Unigram BPC.
    pub bpc_unigram: f64,
}

/// A measurement oracle start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Measurement oracle id.
    pub oracle_id: u8,
}

/// A measurement oracle completion event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Measurement oracle id.
    pub oracle_id: u8,
}

/// A measurement oracle failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleFailedEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Measurement oracle id.
    pub oracle_id: u8,
    /// Diagnostic detail, excluding corpus bytes, weights, and gradients.
    pub diagnostic: String,
}

/// A measurement oracle aggregate completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleAggregateCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Aggregate metric-oracle verdict.
    pub metric_oracle_passed: bool,
    /// JSON array string of failed oracle ids.
    pub failed_oracle_ids: String,
    /// Oracle report self hash.
    pub oracle_self_hash: String,
}

/// An ablation comparison start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AblationCompareStartEvent {
    /// Experiment seed.
    pub seed: u64,
}

/// An ablation metadata preflight failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AblationMetadataCheckFailEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Human-readable reason, excluding tensor payload bytes.
    pub reason: String,
}

/// An ablation per-tensor comparison event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AblationTensorCompareEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Tensor name.
    pub tensor_name: String,
}

/// An ablation mismatch event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AblationMismatchEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Tensor name.
    pub tensor_name: String,
    /// First mismatching byte offset.
    pub byte_offset: u64,
}

/// An ablation comparison completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AblationCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Whether Phase A and ablation payload hashes matched.
    pub phase_a_eq_ablation: bool,
    /// Ablation report self hash.
    pub ablation_self_hash: String,
}

/// A report validators-run event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportValidatorsRunEvent {
    /// Report decision.
    pub decision: String,
    /// Overall verdict.
    pub verdict: String,
    /// Validator result summary.
    pub validators: String,
}

/// A report emission start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEmitStartEvent {
    /// RFC revision used for the report.
    pub rfc_revision: String,
    /// Pass implementation version.
    pub pass_version: String,
}

/// A report validator event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportValidatorEvent {
    /// Validator name.
    pub validator: String,
    /// PASS or FAIL.
    pub status: String,
    /// Human-readable detail.
    pub diagnostic: String,
}

/// A report emission completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEmitCompleteEvent {
    /// Report self hash.
    pub report_self_hash: String,
    /// Output path.
    pub output_path: String,
    /// Selected outcome.
    pub outcome: String,
    /// Selected decision.
    pub decision: String,
}

/// An outcome-dispatch start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDispatchStartEvent {
    /// H1 status.
    pub h1: String,
    /// H2 status.
    pub h2: String,
    /// H3 status.
    pub h3: String,
    /// H4 status.
    pub h4: String,
    /// H5 status.
    pub h5: String,
    /// Whether any seed diverged.
    pub any_seed_diverged: bool,
    /// Whether validation BPC was suspiciously low.
    pub suspicious_low_bpc: bool,
}

/// An outcome-dispatch completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDispatchCompleteEvent {
    /// Selected outcome.
    pub outcome: String,
    /// Selected decision.
    pub decision: String,
}

/// An observation-bearing refuted outcome input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeRefutedInputEvent {
    /// Refuted hypothesis id.
    pub hypothesis: String,
    /// Concrete observation that drove the refutation.
    pub observation: String,
}

/// A negative-test shuffle start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegTestShuffleStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Shuffle RNG seed.
    pub shuffle_seed: u64,
    /// Validation byte count.
    pub token_count: u64,
}

/// A negative-test shuffle completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTestShuffleCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Shuffle RNG seed.
    pub shuffle_seed: u64,
    /// Validation byte count.
    pub token_count: u64,
    /// Shuffled validation hash.
    pub shuffled_val_sha256: String,
}

/// A negative-test shuffle pin-mismatch event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTestShufflePinMismatchEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Expected shuffled validation hash.
    pub expected: String,
    /// Observed shuffled validation hash.
    pub observed: String,
}

/// A negative-test scoring start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegTestScoreStartEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Validation byte count.
    pub token_count: u64,
}

/// A negative-test scoring completion event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NegTestScoreCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Original validation BPC.
    pub bpc_original: f64,
    /// Shuffled validation BPC.
    pub bpc_shuffled: f64,
}

/// A negative-test completion event.
#[derive(Debug, Clone, PartialEq)]
pub struct NegTestCompleteEvent {
    /// Experiment seed.
    pub seed: u64,
    /// Original validation BPC.
    pub bpc_original: f64,
    /// Shuffled validation BPC.
    pub bpc_shuffled: f64,
    /// Shuffled-minus-original BPC delta.
    pub delta: f64,
    /// Whether the delta crossed the sensitivity threshold.
    pub sensitive: bool,
    /// Negative-test report self hash.
    pub negative_self_hash: String,
}

/// A manifest shuffle-pin computation event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestShufflePinComputeEvent {
    /// Fisher-Yates shuffle seed.
    pub shuffle_seed: u64,
    /// Validation byte count.
    pub token_count: u64,
    /// Computed shuffled validation hash.
    pub shuffled_val_sha256: String,
}

/// A manifest shuffle-pin verification success event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestShufflePinVerifyOkEvent {
    /// Manifest-pinned hash.
    pub expected: String,
    /// Computed shuffled validation hash.
    pub observed: String,
}

/// A manifest shuffle-pin verification failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestShufflePinVerifyFailEvent {
    /// Manifest-pinned hash.
    pub expected: String,
    /// Computed shuffled validation hash.
    pub observed: String,
}

/// E2E scenario start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E2eScenarioStartEvent {
    /// Scenario name.
    pub scenario: String,
    /// Training budget profile.
    pub budget_profile: String,
    /// Number of seeds covered by the scenario.
    pub n_seeds: u64,
}

/// E2E phase transition event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E2ePhaseEvent {
    /// Scenario name.
    pub scenario: String,
    /// Phase name.
    pub phase: String,
}

/// E2E scenario completion event.
#[derive(Debug, Clone, PartialEq)]
pub struct E2eScenarioCompleteEvent {
    /// Scenario name.
    pub scenario: String,
    /// Selected S1 outcome.
    pub outcome: String,
    /// Selected S1 decision.
    pub decision: String,
    /// Whether scenario assertions passed.
    pub pass: bool,
    /// Deterministic fixture duration, in seconds.
    pub duration_seconds: f64,
}

/// Tiny-fixture integration smoke scenario start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationSmokeScenarioStartEvent {
    /// Scenario name.
    pub scenario: String,
    /// Training budget profile.
    pub budget_profile: String,
    /// Number of seeds covered by the scenario.
    pub n_seeds: u64,
}

/// Tiny-fixture integration smoke scenario completion event.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationSmokeScenarioCompleteEvent {
    /// Scenario name.
    pub scenario: String,
    /// Whether scenario assertions passed.
    pub pass: bool,
    /// Measured local fixture duration in seconds.
    pub duration_seconds: f64,
}

/// A diagnostic CLI subcommand start event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDiagnosticStartEvent {
    /// CLI subcommand name.
    pub command: &'static str,
}

/// A diagnostic CLI subcommand completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDiagnosticCompleteEvent {
    /// CLI subcommand name.
    pub command: &'static str,
}

/// A diagnostic CLI subcommand failure event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDiagnosticFailedEvent {
    /// CLI subcommand name.
    pub command: &'static str,
    /// Redacted diagnostic that excludes corpus/tensor payload bytes.
    pub diagnostic: String,
}

/// Emitter for S1 structured-log helper events and spans.
///
/// Payload boundary: these helpers accept hashes, scalar summaries, tensor
/// names, offsets, paths, and caller-redacted diagnostics only. They must not
/// accept or emit raw corpus bytes, tensor payload bytes, trainable weights, or
/// gradients. Producer beads own redaction before calling this helper layer.
#[derive(Debug, Clone, Copy, Default)]
pub struct S1LogEmitter;

impl S1LogEmitter {
    /// Construct an S1 logging helper.
    pub const fn new() -> Self {
        Self
    }

    /// Create a tiny-fixture end-to-end scenario span.
    pub fn e2e_scenario_span(&self, scenario: &str) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.e2e.scenario",
            event_name = span::E2E_SCENARIO,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %scenario,
        )
    }

    /// Create a per-seed run span.
    pub fn run_span(&self, fields: &RunSpanFields) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.run",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            pass_version = %fields.pass_version,
            phase = %fields.phase,
            build_kind = %fields.build_kind,
            corpus_train_sha = %fields.corpus_train_sha,
            corpus_val_sha = %fields.corpus_val_sha,
            device_profile_hash = %fields.device_profile_hash,
        )
    }

    /// Create an optimizer-step span.
    pub fn run_step_span(&self, fields: RunStepSpanFields) -> Result<Span, LoggingEventError> {
        validate_nonempty(field::PHASE, fields.phase)?;
        validate_finite(field::LOSS_NATS_PER_BYTE, fields.loss_nats_per_byte)?;
        validate_finite(field::GRAD_NORM_L2, fields.grad_norm_l2)?;
        Ok(tracing::trace_span!(
            target: crate::S1_LOG_TARGET,
            "s1.run.step",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            step = fields.step,
            phase = fields.phase,
            loss_nats_per_byte = fields.loss_nats_per_byte,
            grad_norm_l2 = fields.grad_norm_l2,
        ))
    }

    /// Create an evaluation span.
    pub fn run_eval_span(&self, fields: RunEvalSpanFields) -> Result<Span, LoggingEventError> {
        validate_nonempty(field::PHASE, fields.phase)?;
        Ok(tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.run.eval",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            eval_step = fields.eval_step,
            phase = fields.phase,
        ))
    }

    /// Create a score span.
    pub fn score_span(&self, seed: u64) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.score",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = seed,
        )
    }

    /// Create a baseline-fit span.
    pub fn baseline_fit_span(&self, seed: u64) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.baseline.fit",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = seed,
        )
    }

    /// Create a baseline-score span.
    pub fn baseline_score_span(&self, seed: u64) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.baseline.score",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = seed,
        )
    }

    /// Create a negative-test span.
    pub fn neg_test_span(&self, seed: u64) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.neg_test",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = seed,
        )
    }

    /// Create an ablation span.
    pub fn ablation_span(&self, seed: u64) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.ablation",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = seed,
        )
    }

    /// Create a measurement-oracle span.
    pub fn oracle_span(&self, seed: u64, oracle_id: u8) -> Result<Span, LoggingEventError> {
        validate_oracle_id(oracle_id)?;
        Ok(match oracle_id {
            0 => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.oracle.0",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                seed = seed,
                oracle_id = oracle_id,
            ),
            1 => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.oracle.1",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                seed = seed,
                oracle_id = oracle_id,
            ),
            2 => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.oracle.2",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                seed = seed,
                oracle_id = oracle_id,
            ),
            3 => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.oracle.3",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                seed = seed,
                oracle_id = oracle_id,
            ),
            4 => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.oracle.4",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                seed = seed,
                oracle_id = oracle_id,
            ),
            _ => unreachable!("oracle id validated before span construction"),
        })
    }

    /// Create a report-emission span.
    pub fn report_emit_span(&self) -> Span {
        tracing::info_span!(
            target: crate::S1_LOG_TARGET,
            "s1.report.emit",
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
        )
    }

    /// Create a diagnostic CLI subcommand span.
    pub fn cli_diagnostic_span(&self, command: &'static str) -> Result<Span, LoggingEventError> {
        validate_cli_diagnostic_command(command)?;
        Ok(match command {
            "doctor" => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.cli.doctor",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                command = %command,
            ),
            "inspect" => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.cli.inspect",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                command = %command,
            ),
            "diff-checkpoints" => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.cli.diff-checkpoints",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                command = %command,
            ),
            "print-config" => tracing::info_span!(
                target: crate::S1_LOG_TARGET,
                "s1.cli.print-config",
                gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
                command = %command,
            ),
            _ => unreachable!("diagnostic command validated before span construction"),
        })
    }

    /// Emit a run precondition failure.
    pub fn run_precondition_failed(
        &self,
        fields: &RunPreconditionFailedEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::REASON, &fields.reason)?;
        tracing::warn!(
            target: crate::S1_LOG_TARGET,
            event_name = event::RUN_PRECONDITION_FAILED,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            reason = %fields.reason,
        );
        Ok(())
    }

    /// Emit diagnostic CLI start.
    pub fn cli_diagnostic_start(
        &self,
        fields: &CliDiagnosticStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_cli_diagnostic_command(fields.command)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::CLI_DIAGNOSTIC_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            command = %fields.command,
        );
        Ok(())
    }

    /// Emit diagnostic CLI completion.
    pub fn cli_diagnostic_complete(
        &self,
        fields: &CliDiagnosticCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_cli_diagnostic_command(fields.command)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::CLI_DIAGNOSTIC_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            command = %fields.command,
        );
        Ok(())
    }

    /// Emit diagnostic CLI failure.
    pub fn cli_diagnostic_failed(
        &self,
        fields: &CliDiagnosticFailedEvent,
    ) -> Result<(), LoggingEventError> {
        validate_cli_diagnostic_command(fields.command)?;
        validate_nonempty(field::DIAGNOSTIC, &fields.diagnostic)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::CLI_DIAGNOSTIC_FAILED,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            command = %fields.command,
            diagnostic = %fields.diagnostic,
        );
        Ok(())
    }

    /// Emit device-profile enforcement start.
    pub fn device_profile_enforce_start(
        &self,
        fields: &DeviceProfileEnforceStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::DEVICE_PROFILE_HASH, &fields.device_profile_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::DEVICE_PROFILE_ENFORCE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            device_profile_hash = %fields.device_profile_hash,
        );
        Ok(())
    }

    /// Emit device-profile enforcement failure.
    pub fn device_profile_enforce_fail(
        &self,
        fields: &DeviceProfileEnforceFailEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::DEVICE_PROFILE_HASH, &fields.device_profile_hash)?;
        validate_nonempty(field::REJECTED_VAR, &fields.rejected_var)?;
        validate_nonempty(field::EXPECTED, &fields.expected)?;
        validate_nonempty(field::OBSERVED, &fields.observed)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::DEVICE_PROFILE_ENFORCE_FAIL,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            device_profile_hash = %fields.device_profile_hash,
            rejected_var = %fields.rejected_var,
            expected = %fields.expected,
            observed = %fields.observed,
        );
        Ok(())
    }

    /// Emit device-profile enforcement success.
    pub fn device_profile_enforce_ok(
        &self,
        fields: &DeviceProfileEnforceOkEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::DEVICE_PROFILE_HASH, &fields.device_profile_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::DEVICE_PROFILE_ENFORCE_OK,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            device_profile_hash = %fields.device_profile_hash,
        );
        Ok(())
    }

    /// Emit a run divergence.
    pub fn run_divergence(&self, fields: RunDivergenceEvent) -> Result<(), LoggingEventError> {
        validate_finite(field::LAST_FINITE_LOSS, fields.last_finite_loss)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::RUN_DIVERGENCE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            step = fields.step,
            observed = fields.observed.as_str(),
            last_finite_loss = fields.last_finite_loss,
        );
        Ok(())
    }

    /// Emit evaluation progress.
    pub fn run_eval_progress(&self, fields: RunEvalProgressEvent) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_VALUE, fields.bpc_value)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::RUN_EVAL_PROGRESS,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            eval_step = fields.eval_step,
            bpc_value = fields.bpc_value,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit checkpoint emission.
    pub fn run_checkpoint_emitted(
        &self,
        fields: &RunCheckpointEmittedEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::PATH, &fields.path)?;
        validate_nonempty(field::CHECKPOINT_SELF_HASH, &fields.checkpoint_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::RUN_CHECKPOINT_EMITTED,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            step = fields.step,
            path = %fields.path,
            checkpoint_self_hash = %fields.checkpoint_self_hash,
        );
        Ok(())
    }

    /// Emit score computation start.
    pub fn score_start(&self, fields: ScoreStartEvent) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::SCORE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit score computation progress after a reset-context chunk.
    pub fn score_progress(&self, fields: ScoreProgressEvent) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::SCORE_PROGRESS,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            chunk_index = fields.chunk_index,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit score completion.
    pub fn score_complete(&self, fields: &ScoreCompleteEvent) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_VALUE, fields.bpc_value)?;
        validate_nonempty(field::SCORE_SELF_HASH, &fields.score_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::SCORE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bpc_value = fields.bpc_value,
            token_count = fields.token_count,
            score_self_hash = %fields.score_self_hash,
        );
        Ok(())
    }

    /// Emit baseline fitting start.
    pub fn baseline_fit_start(
        &self,
        fields: &BaselineFitStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::CORPUS_TRAIN_SHA, &fields.corpus_train_sha)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_FIT_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            corpus_train_sha = %fields.corpus_train_sha,
            token_count = fields.train_bytes,
        );
        Ok(())
    }

    /// Emit baseline fitting progress.
    pub fn baseline_fit_progress(
        &self,
        fields: BaselineFitProgressEvent,
    ) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_FIT_PROGRESS,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bytes_done = fields.bytes_done,
            token_count = fields.train_bytes,
        );
        Ok(())
    }

    /// Emit baseline fitting completion.
    pub fn baseline_fit_complete(
        &self,
        fields: &BaselineFitCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_3GRAM, fields.bpc_3gram)?;
        validate_finite(field::BPC_2GRAM, fields.bpc_2gram)?;
        validate_finite(field::BPC_UNIGRAM, fields.bpc_unigram)?;
        validate_nonempty(field::COUNTS_BLOB_SHA256, &fields.counts_blob_sha256)?;
        validate_nonempty(field::COUNTS_SUMMARY, &fields.counts_summary)?;
        validate_nonempty(field::BASELINE_SELF_HASH, &fields.baseline_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_FIT_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bpc_3gram = fields.bpc_3gram,
            bpc_2gram = fields.bpc_2gram,
            bpc_unigram = fields.bpc_unigram,
            counts_blob_sha256 = %fields.counts_blob_sha256,
            counts_summary = %fields.counts_summary,
            baseline_self_hash = %fields.baseline_self_hash,
        );
        Ok(())
    }

    /// Emit baseline validation scoring start.
    pub fn baseline_score_start(
        &self,
        fields: BaselineScoreStartEvent,
    ) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_SCORE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit baseline validation scoring completion.
    pub fn baseline_score_complete(
        &self,
        fields: &BaselineScoreCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::BASELINE_SELF_HASH, &fields.baseline_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_SCORE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            token_count = fields.token_count,
            baseline_self_hash = %fields.baseline_self_hash,
        );
        Ok(())
    }

    /// Emit baseline completion.
    pub fn baseline_complete(
        &self,
        fields: BaselineCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_3GRAM, fields.bpc_3gram)?;
        validate_finite(field::BPC_2GRAM, fields.bpc_2gram)?;
        validate_finite(field::BPC_UNIGRAM, fields.bpc_unigram)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::BASELINE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bpc_3gram = fields.bpc_3gram,
            bpc_2gram = fields.bpc_2gram,
            bpc_unigram = fields.bpc_unigram,
        );
        Ok(())
    }

    /// Emit measurement-oracle start.
    pub fn oracle_start(&self, fields: OracleStartEvent) -> Result<(), LoggingEventError> {
        validate_oracle_id(fields.oracle_id)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ORACLE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            oracle_id = fields.oracle_id,
        );
        Ok(())
    }

    /// Emit measurement-oracle completion.
    pub fn oracle_complete(&self, fields: OracleCompleteEvent) -> Result<(), LoggingEventError> {
        validate_oracle_id(fields.oracle_id)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ORACLE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            oracle_id = fields.oracle_id,
        );
        Ok(())
    }

    /// Emit measurement-oracle failure.
    pub fn oracle_failed(&self, fields: &OracleFailedEvent) -> Result<(), LoggingEventError> {
        validate_oracle_id(fields.oracle_id)?;
        validate_nonempty(field::DIAGNOSTIC, &fields.diagnostic)?;
        tracing::warn!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ORACLE_FAILED,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            oracle_id = fields.oracle_id,
            diagnostic = %fields.diagnostic,
        );
        Ok(())
    }

    /// Emit aggregate measurement-oracle completion.
    pub fn oracle_aggregate_complete(
        &self,
        fields: &OracleAggregateCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::ORACLE_SELF_HASH, &fields.oracle_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ORACLE_AGGREGATE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            metric_oracle_passed = fields.metric_oracle_passed,
            failed_oracle_ids = %fields.failed_oracle_ids,
            oracle_self_hash = %fields.oracle_self_hash,
        );
        Ok(())
    }

    /// Emit ablation comparison start.
    pub fn ablation_compare_start(
        &self,
        fields: AblationCompareStartEvent,
    ) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ABLATION_COMPARE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
        );
        Ok(())
    }

    /// Emit ablation metadata preflight failure.
    pub fn ablation_metadata_check_fail(
        &self,
        fields: &AblationMetadataCheckFailEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::REASON, &fields.reason)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ABLATION_METADATA_CHECK_FAIL,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            reason = %fields.reason,
        );
        Ok(())
    }

    /// Emit ablation tensor comparison progress.
    pub fn ablation_tensor_compare(
        &self,
        fields: &AblationTensorCompareEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::TENSOR_NAME, &fields.tensor_name)?;
        tracing::trace!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ABLATION_TENSOR_COMPARE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            tensor_name = %fields.tensor_name,
        );
        Ok(())
    }

    /// Emit ablation mismatch.
    pub fn ablation_mismatch(
        &self,
        fields: &AblationMismatchEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::TENSOR_NAME, &fields.tensor_name)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ABLATION_MISMATCH,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            tensor_name = %fields.tensor_name,
            byte_offset = fields.byte_offset,
        );
        Ok(())
    }

    /// Emit ablation comparison completion.
    pub fn ablation_complete(
        &self,
        fields: &AblationCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::ABLATION_SELF_HASH, &fields.ablation_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::ABLATION_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            phase_a_eq_ablation = fields.phase_a_eq_ablation,
            ablation_self_hash = %fields.ablation_self_hash,
        );
        Ok(())
    }

    /// Emit report-validator execution.
    pub fn report_validators_run(
        &self,
        fields: &ReportValidatorsRunEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::DECISION, &fields.decision)?;
        validate_nonempty(field::VERDICT, &fields.verdict)?;
        validate_nonempty(field::VALIDATORS, &fields.validators)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::REPORT_VALIDATORS_RUN,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            decision = %fields.decision,
            verdict = %fields.verdict,
            validators = %fields.validators,
        );
        Ok(())
    }

    /// Emit report emission start.
    pub fn report_emit_start(
        &self,
        fields: &ReportEmitStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::RFC_REVISION, &fields.rfc_revision)?;
        validate_nonempty(field::PASS_VERSION, &fields.pass_version)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::REPORT_EMIT_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            rfc_revision = %fields.rfc_revision,
            pass_version = %fields.pass_version,
        );
        Ok(())
    }

    /// Emit one report validator result.
    pub fn report_validator(&self, fields: &ReportValidatorEvent) -> Result<(), LoggingEventError> {
        validate_nonempty(field::VALIDATOR, &fields.validator)?;
        validate_nonempty(field::STATUS, &fields.status)?;
        validate_nonempty(field::DIAGNOSTIC, &fields.diagnostic)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::REPORT_VALIDATOR,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            validator = %fields.validator,
            status = %fields.status,
            diagnostic = %fields.diagnostic,
        );
        Ok(())
    }

    /// Emit report emission completion.
    pub fn report_emit_complete(
        &self,
        fields: &ReportEmitCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::REPORT_SELF_HASH, &fields.report_self_hash)?;
        validate_nonempty(field::OUTPUT_PATH, &fields.output_path)?;
        validate_nonempty(field::OUTCOME, &fields.outcome)?;
        validate_nonempty(field::DECISION, &fields.decision)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::REPORT_EMIT_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            report_self_hash = %fields.report_self_hash,
            output_path = %fields.output_path,
            outcome = %fields.outcome,
            decision = %fields.decision,
        );
        Ok(())
    }

    /// Emit outcome dispatch start.
    pub fn outcome_dispatch_start(
        &self,
        fields: &OutcomeDispatchStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty("h1", &fields.h1)?;
        validate_nonempty("h2", &fields.h2)?;
        validate_nonempty("h3", &fields.h3)?;
        validate_nonempty("h4", &fields.h4)?;
        validate_nonempty("h5", &fields.h5)?;
        tracing::debug!(
            target: crate::S1_LOG_TARGET,
            event_name = event::OUTCOME_DISPATCH_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            h1 = %fields.h1,
            h2 = %fields.h2,
            h3 = %fields.h3,
            h4 = %fields.h4,
            h5 = %fields.h5,
            any_seed_diverged = fields.any_seed_diverged,
            suspicious_low_bpc = fields.suspicious_low_bpc,
        );
        Ok(())
    }

    /// Emit outcome dispatch completion.
    pub fn outcome_dispatch_complete(
        &self,
        fields: &OutcomeDispatchCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::OUTCOME, &fields.outcome)?;
        validate_nonempty(field::DECISION, &fields.decision)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::OUTCOME_DISPATCH_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            outcome = %fields.outcome,
            decision = %fields.decision,
        );
        Ok(())
    }

    /// Emit an observation-bearing refuted outcome input.
    pub fn outcome_refuted_input(
        &self,
        fields: &OutcomeRefutedInputEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::HYPOTHESIS, &fields.hypothesis)?;
        validate_nonempty(field::OBSERVATION, &fields.observation)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::OUTCOME_REFUTED_INPUT,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            hypothesis = %fields.hypothesis,
            observation = %fields.observation,
        );
        Ok(())
    }

    /// Emit negative-test shuffle start.
    pub fn neg_test_shuffle_start(
        &self,
        fields: NegTestShuffleStartEvent,
    ) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_SHUFFLE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            shuffle_seed = fields.shuffle_seed,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit negative-test shuffle completion.
    pub fn neg_test_shuffle_complete(
        &self,
        fields: &NegTestShuffleCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SHUFFLED_VAL_SHA256, &fields.shuffled_val_sha256)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_SHUFFLE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            shuffle_seed = fields.shuffle_seed,
            token_count = fields.token_count,
            shuffled_val_sha256 = %fields.shuffled_val_sha256,
        );
        Ok(())
    }

    /// Emit negative-test shuffle pin mismatch before returning the typed error.
    pub fn neg_test_shuffle_pin_mismatch(
        &self,
        fields: &NegTestShufflePinMismatchEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::EXPECTED, &fields.expected)?;
        validate_nonempty(field::OBSERVED, &fields.observed)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_SHUFFLE_PIN_MISMATCH,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            expected = %fields.expected,
            observed = %fields.observed,
        );
        Ok(())
    }

    /// Emit negative-test score start.
    pub fn neg_test_score_start(
        &self,
        fields: NegTestScoreStartEvent,
    ) -> Result<(), LoggingEventError> {
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_SCORE_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            token_count = fields.token_count,
        );
        Ok(())
    }

    /// Emit negative-test score completion.
    pub fn neg_test_score_complete(
        &self,
        fields: NegTestScoreCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_ORIGINAL, fields.bpc_original)?;
        validate_finite(field::BPC_SHUFFLED, fields.bpc_shuffled)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_SCORE_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bpc_original = fields.bpc_original,
            bpc_shuffled = fields.bpc_shuffled,
        );
        Ok(())
    }

    /// Emit negative-test completion.
    pub fn neg_test_complete(
        &self,
        fields: &NegTestCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_finite(field::BPC_ORIGINAL, fields.bpc_original)?;
        validate_finite(field::BPC_SHUFFLED, fields.bpc_shuffled)?;
        validate_finite(field::DELTA, fields.delta)?;
        validate_nonempty(field::NEGATIVE_SELF_HASH, &fields.negative_self_hash)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::NEG_TEST_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            seed = fields.seed,
            bpc_original = fields.bpc_original,
            bpc_shuffled = fields.bpc_shuffled,
            delta = fields.delta,
            sensitive = fields.sensitive,
            negative_self_hash = %fields.negative_self_hash,
        );
        Ok(())
    }

    /// Emit manifest shuffle-pin computation.
    pub fn manifest_shuffle_pin_compute(
        &self,
        fields: &ManifestShufflePinComputeEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SHUFFLED_VAL_SHA256, &fields.shuffled_val_sha256)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::MANIFEST_SHUFFLE_PIN_COMPUTE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            shuffle_seed = fields.shuffle_seed,
            token_count = fields.token_count,
            shuffled_val_sha256 = %fields.shuffled_val_sha256,
        );
        Ok(())
    }

    /// Emit manifest shuffle-pin verification success.
    pub fn manifest_shuffle_pin_verify_ok(
        &self,
        fields: &ManifestShufflePinVerifyOkEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::EXPECTED, &fields.expected)?;
        validate_nonempty(field::OBSERVED, &fields.observed)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::MANIFEST_SHUFFLE_PIN_VERIFY_OK,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            expected = %fields.expected,
            observed = %fields.observed,
        );
        Ok(())
    }

    /// Emit manifest shuffle-pin verification failure.
    pub fn manifest_shuffle_pin_verify_fail(
        &self,
        fields: &ManifestShufflePinVerifyFailEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::EXPECTED, &fields.expected)?;
        validate_nonempty(field::OBSERVED, &fields.observed)?;
        tracing::error!(
            target: crate::S1_LOG_TARGET,
            event_name = event::MANIFEST_SHUFFLE_PIN_VERIFY_FAIL,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            expected = %fields.expected,
            observed = %fields.observed,
        );
        Ok(())
    }

    /// Emit E2E scenario start.
    pub fn e2e_scenario_start(
        &self,
        fields: &E2eScenarioStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SCENARIO, &fields.scenario)?;
        validate_nonempty(field::BUDGET_PROFILE, &fields.budget_profile)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::E2E_SCENARIO_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %fields.scenario,
            budget_profile = %fields.budget_profile,
            n_seeds = fields.n_seeds,
        );
        Ok(())
    }

    /// Emit E2E phase transition.
    pub fn e2e_phase(&self, fields: &E2ePhaseEvent) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SCENARIO, &fields.scenario)?;
        validate_nonempty(field::PHASE, &fields.phase)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::E2E_PHASE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %fields.scenario,
            phase = %fields.phase,
        );
        Ok(())
    }

    /// Emit E2E scenario completion.
    pub fn e2e_scenario_complete(
        &self,
        fields: &E2eScenarioCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SCENARIO, &fields.scenario)?;
        validate_nonempty(field::OUTCOME, &fields.outcome)?;
        validate_nonempty(field::DECISION, &fields.decision)?;
        validate_finite(field::DURATION_SECONDS, fields.duration_seconds)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::E2E_SCENARIO_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %fields.scenario,
            outcome = %fields.outcome,
            decision = %fields.decision,
            pass = fields.pass,
            duration_seconds = fields.duration_seconds,
        );
        Ok(())
    }

    /// Emit tiny-fixture integration smoke scenario start.
    pub fn integration_smoke_scenario_start(
        &self,
        fields: &IntegrationSmokeScenarioStartEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SCENARIO, &fields.scenario)?;
        validate_nonempty(field::BUDGET_PROFILE, &fields.budget_profile)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::INTEGRATION_SMOKE_SCENARIO_START,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %fields.scenario,
            budget_profile = %fields.budget_profile,
            n_seeds = fields.n_seeds,
        );
        Ok(())
    }

    /// Emit tiny-fixture integration smoke scenario completion.
    pub fn integration_smoke_scenario_complete(
        &self,
        fields: &IntegrationSmokeScenarioCompleteEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty(field::SCENARIO, &fields.scenario)?;
        validate_finite(field::DURATION_SECONDS, fields.duration_seconds)?;
        tracing::info!(
            target: crate::S1_LOG_TARGET,
            event_name = event::INTEGRATION_SMOKE_SCENARIO_COMPLETE,
            gbf_log_schema_version = GBF_LOG_SCHEMA_VERSION,
            scenario = %fields.scenario,
            pass = fields.pass,
            duration_seconds = fields.duration_seconds,
        );
        Ok(())
    }
}

fn validate_nonempty(name: &'static str, value: &str) -> Result<(), LoggingEventError> {
    if value.trim().is_empty() {
        Err(LoggingEventError::EmptyField { name })
    } else {
        Ok(())
    }
}

fn validate_finite(name: &'static str, value: f64) -> Result<(), LoggingEventError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LoggingEventError::NonFiniteField { name, value })
    }
}

fn validate_oracle_id(oracle_id: u8) -> Result<(), LoggingEventError> {
    if oracle_id <= 4 {
        Ok(())
    } else {
        Err(LoggingEventError::InvalidOracleId { oracle_id })
    }
}

fn validate_cli_diagnostic_command(command: &'static str) -> Result<(), LoggingEventError> {
    validate_nonempty(field::COMMAND, command)?;
    match command {
        "doctor" | "inspect" | "diff-checkpoints" | "print-config" => Ok(()),
        _ => Err(LoggingEventError::EmptyField {
            name: field::COMMAND,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    #[test]
    fn profile_defaults_keep_test_profile_trace_enabled() {
        assert_eq!(
            LoggingProfile::Human.default_filter_spec(),
            "gbf_experiments::s1=info"
        );
        assert_eq!(
            LoggingProfile::Json.default_filter_spec(),
            "gbf_experiments::s1=info"
        );
        assert_eq!(
            LoggingProfile::Test.default_filter_spec(),
            "gbf_experiments::s1=trace"
        );
    }

    #[test]
    fn schema_constants_pin_required_names() {
        for name in [
            field::SEED,
            field::STEP,
            field::PHASE,
            field::LOSS_NATS_PER_BYTE,
            field::GRAD_NORM_L2,
            field::BPC_VALUE,
            field::TENSOR_NAME,
            field::ORACLE_ID,
            field::CORPUS_TRAIN_SHA,
            field::CORPUS_VAL_SHA,
            field::DEVICE_PROFILE_HASH,
            field::SHUFFLED_VAL_SHA256,
            field::BPC_ORIGINAL,
            field::BPC_SHUFFLED,
            field::DELTA,
            field::SENSITIVE,
            field::NEGATIVE_SELF_HASH,
            field::SHUFFLE_SEED,
            field::ANY_SEED_DIVERGED,
            field::SUSPICIOUS_LOW_BPC,
            field::VALIDATOR,
            field::STATUS,
            field::RFC_REVISION,
            field::REPORT_SELF_HASH,
            field::OUTPUT_PATH,
            field::OBSERVATION,
            field::PHASE_A_EQ_ABLATION,
            field::ABLATION_SELF_HASH,
            field::COMMAND,
        ] {
            assert!(field::ALL.contains(&name), "missing field constant {name}");
        }

        for name in [
            event::RUN_PRECONDITION_FAILED,
            event::RUN_DIVERGENCE,
            event::RUN_EVAL_PROGRESS,
            event::RUN_CHECKPOINT_EMITTED,
            event::SCORE_START,
            event::SCORE_PROGRESS,
            event::SCORE_COMPLETE,
            event::BASELINE_COMPLETE,
            event::ORACLE_START,
            event::ORACLE_COMPLETE,
            event::ORACLE_FAILED,
            event::ORACLE_AGGREGATE_COMPLETE,
            event::ABLATION_COMPARE_START,
            event::ABLATION_METADATA_CHECK_FAIL,
            event::ABLATION_TENSOR_COMPARE,
            event::ABLATION_MISMATCH,
            event::ABLATION_COMPLETE,
            event::REPORT_VALIDATORS_RUN,
            event::REPORT_EMIT_START,
            event::REPORT_VALIDATOR,
            event::REPORT_EMIT_COMPLETE,
            event::OUTCOME_DISPATCH_START,
            event::OUTCOME_DISPATCH_COMPLETE,
            event::OUTCOME_REFUTED_INPUT,
            event::NEG_TEST_SHUFFLE_START,
            event::NEG_TEST_SHUFFLE_COMPLETE,
            event::NEG_TEST_SHUFFLE_PIN_MISMATCH,
            event::NEG_TEST_SCORE_START,
            event::NEG_TEST_SCORE_COMPLETE,
            event::NEG_TEST_COMPLETE,
            event::MANIFEST_SHUFFLE_PIN_COMPUTE,
            event::MANIFEST_SHUFFLE_PIN_VERIFY_OK,
            event::MANIFEST_SHUFFLE_PIN_VERIFY_FAIL,
            event::CLI_DIAGNOSTIC_START,
            event::CLI_DIAGNOSTIC_COMPLETE,
            event::CLI_DIAGNOSTIC_FAILED,
        ] {
            assert!(event::ALL.contains(&name), "missing event constant {name}");
        }

        for name in [
            span::RUN,
            span::RUN_STEP,
            span::RUN_EVAL,
            span::SCORE,
            span::BASELINE_FIT,
            span::BASELINE_SCORE,
            span::NEG_TEST,
            span::ABLATION,
            span::ORACLE_0,
            span::ORACLE_1,
            span::ORACLE_2,
            span::ORACLE_3,
            span::ORACLE_4,
            span::REPORT_EMIT,
            span::CLI_REPLAY,
            span::CLI_FIT_BASELINE,
            span::CLI_ORACLE,
            span::CLI_VERIFY_DETERMINISM,
            span::CLI_DOCTOR,
            span::CLI_INSPECT,
            span::CLI_DIFF_CHECKPOINTS,
            span::CLI_PRINT_CONFIG,
        ] {
            assert!(span::ALL.contains(&name), "missing span constant {name}");
        }
    }

    #[test]
    fn helper_events_are_captured_by_tracing_subscriber() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            let emitter = S1LogEmitter::new();
            let run_span = emitter.run_span(
                &RunSpanFields::new(
                    7,
                    "s1_run_log.v1",
                    "phase_a",
                    "qat",
                    "train-sha",
                    "val-sha",
                    "device-profile-hash",
                )
                .unwrap(),
            );
            let _run_guard = run_span.enter();
            let step_span = emitter
                .run_step_span(RunStepSpanFields {
                    seed: 7,
                    step: 42,
                    phase: "phase_a",
                    loss_nats_per_byte: 0.125,
                    grad_norm_l2: 1.5,
                })
                .unwrap();
            let _step_guard = step_span.enter();

            emitter
                .run_divergence(RunDivergenceEvent {
                    seed: 7,
                    step: 42,
                    observed: DivergenceObserved::NonFiniteLoss,
                    last_finite_loss: 0.125,
                })
                .unwrap();
        });

        let records = capture.records();
        assert_span_field(&records, span::RUN, field::GBF_LOG_SCHEMA_VERSION, "1.0.0");
        assert_span_field(&records, span::RUN, field::SEED, "7");
        assert_span_field(&records, span::RUN, field::PASS_VERSION, "s1_run_log.v1");
        assert_span_field(&records, span::RUN_STEP, field::STEP, "42");
        assert_span_field(&records, span::RUN_STEP, field::LOSS_NATS_PER_BYTE, "0.125");

        assert_event_field(
            &records,
            event::RUN_DIVERGENCE,
            field::EVENT_NAME,
            event::RUN_DIVERGENCE,
        );
        assert_event_field(
            &records,
            event::RUN_DIVERGENCE,
            field::GBF_LOG_SCHEMA_VERSION,
            GBF_LOG_SCHEMA_VERSION,
        );
        assert_event_field(&records, event::RUN_DIVERGENCE, field::SEED, "7");
        assert_event_field(&records, event::RUN_DIVERGENCE, field::STEP, "42");
        assert_event_field(
            &records,
            event::RUN_DIVERGENCE,
            field::OBSERVED,
            "non_finite_loss",
        );
        assert_event_scope(
            &records,
            event::RUN_DIVERGENCE,
            &[span::RUN, span::RUN_STEP],
        );
        assert_no_event_field(&records, event::RUN_DIVERGENCE, "message");
    }

    #[test]
    fn diagnostic_cli_events_are_captured_by_tracing_subscriber() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            let emitter = S1LogEmitter::new();
            {
                let span = emitter.cli_diagnostic_span("doctor").unwrap();
                let _guard = span.enter();
                emitter
                    .cli_diagnostic_start(&CliDiagnosticStartEvent { command: "doctor" })
                    .unwrap();
                emitter
                    .cli_diagnostic_failed(&CliDiagnosticFailedEvent {
                        command: "doctor",
                        diagnostic: "manifest missing".to_owned(),
                    })
                    .unwrap();
            }

            {
                let span = emitter.cli_diagnostic_span("print-config").unwrap();
                let _guard = span.enter();
                emitter
                    .cli_diagnostic_complete(&CliDiagnosticCompleteEvent {
                        command: "print-config",
                    })
                    .unwrap();
            }
        });

        let records = capture.records();
        assert_span_field(&records, span::CLI_DOCTOR, field::COMMAND, "doctor");
        assert_span_field(
            &records,
            span::CLI_PRINT_CONFIG,
            field::COMMAND,
            "print-config",
        );
        assert_event_field(
            &records,
            event::CLI_DIAGNOSTIC_START,
            field::COMMAND,
            "doctor",
        );
        assert_event_field(
            &records,
            event::CLI_DIAGNOSTIC_FAILED,
            field::DIAGNOSTIC,
            "manifest missing",
        );
        assert_event_scope(&records, event::CLI_DIAGNOSTIC_FAILED, &[span::CLI_DOCTOR]);
        assert_event_field(
            &records,
            event::CLI_DIAGNOSTIC_COMPLETE,
            field::COMMAND,
            "print-config",
        );
        assert_event_scope(
            &records,
            event::CLI_DIAGNOSTIC_COMPLETE,
            &[span::CLI_PRINT_CONFIG],
        );
    }

    #[test]
    fn representative_failure_events_have_structured_fields() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            let emitter = S1LogEmitter::new();
            let oracle_span = emitter.oracle_span(11, 2).unwrap();
            let _oracle_guard = oracle_span.enter();
            emitter
                .oracle_failed(&OracleFailedEvent {
                    seed: 11,
                    oracle_id: 2,
                    diagnostic: "metric threshold missed".to_owned(),
                })
                .unwrap();
            drop(_oracle_guard);

            let ablation_span = emitter.ablation_span(11);
            let _ablation_guard = ablation_span.enter();
            emitter
                .ablation_mismatch(&AblationMismatchEvent {
                    seed: 11,
                    tensor_name: "encoder.block0.weight".to_owned(),
                    byte_offset: 4096,
                })
                .unwrap();
        });

        let records = capture.records();
        assert_event_field(&records, event::ORACLE_FAILED, field::ORACLE_ID, "2");
        assert_event_field(
            &records,
            event::ORACLE_FAILED,
            field::DIAGNOSTIC,
            "metric threshold missed",
        );
        assert_event_scope(&records, event::ORACLE_FAILED, &[span::ORACLE_2]);
        assert_no_event_field(&records, event::ORACLE_FAILED, "message");

        assert_event_field(
            &records,
            event::ABLATION_MISMATCH,
            field::TENSOR_NAME,
            "encoder.block0.weight",
        );
        assert_event_field(
            &records,
            event::ABLATION_MISMATCH,
            field::BYTE_OFFSET,
            "4096",
        );
        assert_event_scope(&records, event::ABLATION_MISMATCH, &[span::ABLATION]);
        assert_no_event_field(&records, event::ABLATION_MISMATCH, "message");
    }

    #[test]
    fn json_profile_output_is_newline_delimited_json_with_schema_and_span_context() {
        let output = SharedOutput::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_env_filter(EnvFilter::new(LoggingProfile::Json.default_filter_spec()))
            .with_writer(output.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let emitter = S1LogEmitter::new();
            let run_span = emitter.run_span(
                &RunSpanFields::new(
                    3,
                    "s1_run_log.v1",
                    "phase_a",
                    "qat",
                    "sha256:train",
                    "sha256:val",
                    "sha256:device",
                )
                .unwrap(),
            );
            let _run_guard = run_span.enter();
            emitter
                .run_eval_progress(RunEvalProgressEvent {
                    seed: 3,
                    eval_step: 100,
                    bpc_value: 1.25,
                    token_count: 4096,
                })
                .unwrap();
        });

        let bytes = output.as_string();
        assert!(
            bytes.ends_with('\n'),
            "JSON logging profile must emit newline-delimited records: {bytes:?}"
        );
        let lines = bytes.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1, "expected one NDJSON event: {bytes}");
        let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();

        insta::assert_snapshot!(stable_json_projection(&event), @r###"
{"fields":{"bpc_value":1.25,"eval_step":100,"event_name":"run.eval_progress","gbf_log_schema_version":"1.0.0","seed":3,"token_count":4096},"level":"INFO","span":{"name":"s1.run"},"spans":["s1.run"],"target":"gbf_experiments::s1"}
"###);
    }

    #[test]
    fn human_profile_output_is_not_json_and_includes_structured_fields() {
        let output = SharedOutput::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_env_filter(EnvFilter::new(LoggingProfile::Human.default_filter_spec()))
            .with_writer(output.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            S1LogEmitter::new()
                .run_precondition_failed(&RunPreconditionFailedEvent {
                    seed: 5,
                    reason: "device profile mismatch".to_owned(),
                })
                .unwrap();
        });

        let line = output.as_string();
        assert!(
            !line.trim_start().starts_with('{'),
            "human profile must not emit NDJSON: {line}"
        );
        assert!(line.contains("run.precondition_failed"), "{line}");
        assert!(line.contains("gbf_log_schema_version=\"1.0.0\""), "{line}");
        assert!(line.contains("reason=device profile mismatch"), "{line}");
    }

    #[test]
    fn helper_event_shape_excludes_raw_payload_leak_fields() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            let emitter = S1LogEmitter::new();
            emit_representative_helper_events(&emitter);
        });

        let records = capture.records();
        let denied_fields = [
            "corpus_bytes",
            "validation_bytes",
            "train_bytes_raw",
            "tensor_payload",
            "payload_bytes",
            "raw_payload",
            "weights",
            "weight_bytes",
            "gradients",
            "gradient_bytes",
            "canonical_json",
        ];
        for record in records {
            for field in denied_fields {
                assert!(
                    !record.fields.contains_key(field),
                    "S1 log helper emitted raw payload field {field:?} in {record:?}"
                );
            }
        }
    }

    fn event_record<'a>(records: &'a [TraceRecord], event_name: &str) -> &'a TraceRecord {
        records
            .iter()
            .find(|record| {
                record.kind == TraceRecordKind::Event
                    && record.field(field::EVENT_NAME) == Some(event_name)
            })
            .unwrap_or_else(|| panic!("missing structured event {event_name}"))
    }

    fn span_record<'a>(records: &'a [TraceRecord], span_name: &str) -> &'a TraceRecord {
        records
            .iter()
            .find(|record| record.kind == TraceRecordKind::Span && record.name == span_name)
            .unwrap_or_else(|| panic!("missing span {span_name}"))
    }

    fn assert_event_field(records: &[TraceRecord], event_name: &str, field: &str, expected: &str) {
        let record = event_record(records, event_name);
        assert_eq!(record.field(field), Some(expected));
    }

    fn assert_no_event_field(records: &[TraceRecord], event_name: &str, field: &str) {
        let record = event_record(records, event_name);
        assert!(
            record.field(field).is_none(),
            "{event_name} must not encode load-bearing data in {field:?}"
        );
    }

    fn assert_event_scope(records: &[TraceRecord], event_name: &str, expected: &[&str]) {
        let record = event_record(records, event_name);
        let actual = record
            .span_scope
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    fn assert_span_field(records: &[TraceRecord], span_name: &str, field: &str, expected: &str) {
        let record = span_record(records, span_name);
        assert_eq!(record.field(field), Some(expected));
    }

    fn emit_representative_helper_events(emitter: &S1LogEmitter) {
        emitter
            .run_precondition_failed(&RunPreconditionFailedEvent {
                seed: 1,
                reason: "redacted precondition detail".to_owned(),
            })
            .unwrap();
        emitter
            .device_profile_enforce_start(&DeviceProfileEnforceStartEvent {
                device_profile_hash: "sha256:device".to_owned(),
            })
            .unwrap();
        emitter
            .device_profile_enforce_fail(&DeviceProfileEnforceFailEvent {
                device_profile_hash: "sha256:device".to_owned(),
                rejected_var: "OMP_NUM_THREADS".to_owned(),
                expected: "1".to_owned(),
                observed: "8".to_owned(),
            })
            .unwrap();
        emitter
            .device_profile_enforce_ok(&DeviceProfileEnforceOkEvent {
                device_profile_hash: "sha256:device".to_owned(),
            })
            .unwrap();
        emitter
            .run_divergence(RunDivergenceEvent {
                seed: 1,
                step: 2,
                observed: DivergenceObserved::NonFiniteGrad,
                last_finite_loss: 0.5,
            })
            .unwrap();
        emitter
            .run_eval_progress(RunEvalProgressEvent {
                seed: 1,
                eval_step: 3,
                bpc_value: 1.0,
                token_count: 128,
            })
            .unwrap();
        emitter
            .run_checkpoint_emitted(&RunCheckpointEmittedEvent {
                seed: 1,
                step: 4,
                path: "checkpoints/seed-1.safetensors".to_owned(),
                checkpoint_self_hash: "sha256:checkpoint".to_owned(),
            })
            .unwrap();
        emitter
            .score_start(ScoreStartEvent {
                seed: 1,
                token_count: 128,
            })
            .unwrap();
        emitter
            .score_progress(ScoreProgressEvent {
                seed: 1,
                chunk_index: 0,
                token_count: 128,
            })
            .unwrap();
        emitter
            .score_complete(&ScoreCompleteEvent {
                seed: 1,
                bpc_value: 1.125,
                token_count: 128,
                score_self_hash: "sha256:score".to_owned(),
            })
            .unwrap();
        emitter
            .baseline_complete(BaselineCompleteEvent {
                seed: 1,
                bpc_3gram: 2.0,
                bpc_2gram: 2.5,
                bpc_unigram: 3.0,
            })
            .unwrap();
        emitter
            .oracle_failed(&OracleFailedEvent {
                seed: 1,
                oracle_id: 0,
                diagnostic: "redacted oracle detail".to_owned(),
            })
            .unwrap();
        emitter
            .ablation_mismatch(&AblationMismatchEvent {
                seed: 1,
                tensor_name: "layer.0.weight".to_owned(),
                byte_offset: 9,
            })
            .unwrap();
        emitter
            .report_validators_run(&ReportValidatorsRunEvent {
                decision: "blocked".to_owned(),
                verdict: "fail".to_owned(),
                validators: "schema,hash".to_owned(),
            })
            .unwrap();
        emitter
            .neg_test_shuffle_start(NegTestShuffleStartEvent {
                seed: 1,
                shuffle_seed: 0,
                token_count: 128,
            })
            .unwrap();
        emitter
            .neg_test_shuffle_complete(&NegTestShuffleCompleteEvent {
                seed: 1,
                shuffle_seed: 0,
                token_count: 128,
                shuffled_val_sha256: "sha256:shuffled".to_owned(),
            })
            .unwrap();
        emitter
            .neg_test_shuffle_pin_mismatch(&NegTestShufflePinMismatchEvent {
                seed: 1,
                expected: "sha256:expected".to_owned(),
                observed: "sha256:observed".to_owned(),
            })
            .unwrap();
        emitter
            .neg_test_score_start(NegTestScoreStartEvent {
                seed: 1,
                token_count: 128,
            })
            .unwrap();
        emitter
            .neg_test_score_complete(NegTestScoreCompleteEvent {
                seed: 1,
                bpc_original: 8.0,
                bpc_shuffled: 10.5,
            })
            .unwrap();
        emitter
            .neg_test_complete(&NegTestCompleteEvent {
                seed: 1,
                bpc_original: 8.0,
                bpc_shuffled: 10.5,
                delta: 2.5,
                sensitive: true,
                negative_self_hash: "sha256:negative".to_owned(),
            })
            .unwrap();
    }

    fn stable_json_projection(event: &serde_json::Value) -> String {
        let fields = event.get("fields").cloned().unwrap_or_else(|| json!({}));
        let span = event
            .get("span")
            .and_then(|span| span.get("name"))
            .map(|name| json!({ "name": name }))
            .unwrap_or_else(|| json!(null));
        let spans = event
            .get("spans")
            .and_then(serde_json::Value::as_array)
            .map(|spans| {
                spans
                    .iter()
                    .filter_map(|span| span.get("name").cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::to_string(&json!({
            "fields": fields,
            "level": event.get("level").cloned().unwrap_or_else(|| json!(null)),
            "span": span,
            "spans": spans,
            "target": event.get("target").cloned().unwrap_or_else(|| json!(null)),
        }))
        .unwrap()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TraceRecordKind {
        Event,
        Span,
    }

    #[derive(Debug, Clone)]
    struct TraceRecord {
        kind: TraceRecordKind,
        name: String,
        fields: BTreeMap<String, String>,
        span_scope: Vec<String>,
    }

    impl TraceRecord {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    #[derive(Debug, Clone, Default)]
    struct TraceCapture {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    impl TraceCapture {
        fn records(&self) -> Vec<TraceRecord> {
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            attrs.record(&mut visitor);
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    kind: TraceRecordKind::Span,
                    name: attrs.metadata().name().to_owned(),
                    fields: visitor.fields,
                    span_scope: Vec::new(),
                });
        }

        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            let span_scope = ctx
                .event_scope(event)
                .map(|scope| {
                    scope
                        .from_root()
                        .map(|span| span.metadata().name().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    kind: TraceRecordKind::Event,
                    name: event.metadata().name().to_owned(),
                    fields: visitor.fields,
                    span_scope,
                });
        }
    }

    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: String) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.insert(field, format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, value.to_owned());
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, value.to_string());
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.insert(field, value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, value.to_string());
        }

        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            self.insert(field, value.to_string());
        }
    }

    #[derive(Clone, Debug, Default)]
    struct SharedOutput {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedOutput {
        fn as_string(&self) -> String {
            String::from_utf8(
                self.bytes
                    .lock()
                    .expect("trace output mutex is not poisoned")
                    .clone(),
            )
            .expect("trace output is utf-8")
        }
    }

    impl<'writer> MakeWriter<'writer> for SharedOutput {
        type Writer = SharedOutputWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            SharedOutputWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    struct SharedOutputWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedOutputWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("trace output mutex is not poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
