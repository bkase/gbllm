//! Build products, run manifests, failure capsules, reports, and certificate schemas.

pub mod build;
pub mod canonical_json;
pub mod certificate;
pub mod failure;
pub mod realism;
pub mod report_schemas;
pub mod run;

pub use canonical_json::{
    CanonicalJsonError, DiagnosticSeverity, ReportBody, ReportEnvelope, ReportEnvelopeError,
    ReportOutcome, ReportSchemaId, ReportSelfHashError, ValidationDiagnostic, canonicalize,
    compute_self_hash, round_trip_self_hash,
};
