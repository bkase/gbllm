#![allow(dead_code)]

#[path = "oracle_agreement_s3_support.rs"]
mod agreement_fixture;

use gbf_artifact::{
    ConformanceEnvelope, ConformanceError as ArtifactConformanceError, canonical_conformance_bytes,
};
use gbf_experiments::s3::conformance::{
    ConformanceError, build_conformance_envelope, emit_conformance_json,
};
use gbf_oracle::phase_surface_agreement::AgreementProduct;
use gbf_workload::WorkloadManifest_v0;

pub fn fixture_workload() -> WorkloadManifest_v0 {
    agreement_fixture::workload_with_first_three_prompt_ids(["prompt-00", "prompt-01", "prompt-02"])
}

pub fn fixture_agreement_product() -> AgreementProduct {
    let workload = fixture_workload();
    agreement_fixture::run_default_agreement_with_workload(&workload)
}

pub fn fixture_envelope() -> ConformanceEnvelope {
    let workload = fixture_workload();
    let agreement = agreement_fixture::run_default_agreement_with_workload(&workload);
    build_conformance_envelope(&workload, vec![agreement]).expect("conformance envelope builds")
}

pub fn fixture_envelope_with_product(
    agreement: AgreementProduct,
) -> Result<ConformanceEnvelope, ConformanceError> {
    let workload = fixture_workload();
    build_conformance_envelope(&workload, vec![agreement])
}

pub fn canonical_bytes(
    envelope: &ConformanceEnvelope,
) -> Result<Vec<u8>, ArtifactConformanceError> {
    canonical_conformance_bytes(envelope)
}

pub fn write_envelope_if_requested(envelope: &ConformanceEnvelope) {
    let Ok(path) = std::env::var("S3_CONFORMANCE_OUT") else {
        return;
    };
    emit_conformance_json(path, envelope).expect("emits conformance json");
}
