#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::export_product;
use gbf_experiments::s3::bundle::PHASE_A_LOGIT_TOLERANCE;

#[test]
fn toy_fixture_program_validates_against_live_frozen_teacher_interface() {
    let product = export_product(0);

    // This toy fixture exercises the B13 exporter contract and frozen-teacher
    // interface. Burn-backed teacher parity is owned by the real-run export
    // path that supplies a concrete training snapshot.
    assert!(product.program_validation.structural_valid);
    assert!(
        product.program_validation.semantic_max_logit_abs_diff <= PHASE_A_LOGIT_TOLERANCE,
        "max logit diff {} exceeded tolerance {}",
        product.program_validation.semantic_max_logit_abs_diff,
        PHASE_A_LOGIT_TOLERANCE
    );
    assert!(product.program_validation.argmax_token_all_match);
    assert!(product.program_validation.prompt_subset_pass());
}
