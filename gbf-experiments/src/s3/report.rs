//! S3 deterministic outcome dispatch and report helpers.

pub mod dispatcher;
pub mod emitter;

pub use dispatcher::{decision_for_outcome, dispatch, dispatch_outcome};
pub use emitter::{
    EVENT_NAME_EMISSION_COMPLETE, EVENT_NAME_EMISSION_STARTED, EVENT_NAME_R_VALIDATOR_PASSED,
    MarkdownBytes, OracleOwnerBeads, PhaseCompletion, ReportError, ReportValidationError,
    S2EnvironmentHashRecord, S3EnvironmentHashRecord, S3PerSeedArtifacts, S3Report,
    S3ReportFrontMatter, S3ReportValidator, emit_report, emit_report_to_path,
    generated_at_commit_time, predictions_section_hash, report_self_hash,
    validate_r_all_hypotheses, validate_r_all_seeds, validate_r_closure_artifacts,
    validate_r_decision, validate_r_owner_beads, validate_r_predictions, validate_r_self_hash,
    validate_report, validate_report_validator,
};
