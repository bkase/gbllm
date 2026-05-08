//! Build products, run manifests, failure capsules, reports, and certificate schemas.

pub mod build;
pub mod canonical_json;
pub mod certificate;
pub mod failure;
pub mod realism;
pub mod report_envelope;
pub mod report_schemas;
pub mod run;

pub use report_envelope::{
    ReportBody, ReportEnvelope, ReportEnvelopeError, ReportOutcome, compute_self_hash,
    round_trip_self_hash,
};
