use gbf_experiments::s2::falsify::{
    BrokenKind, FalsificationCaseResult, threshold_per_weight_verifier_evidence,
};

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F4ThresholdPerWeight, || {
        let evidence = threshold_per_weight_verifier_evidence().expect("F4 verifier evidence");
        let threshold_evidence = &evidence.threshold_evidence;
        evidence.report.validate().expect("F4 H5 report validates");
        let caught = evidence.h5_4_refuted && evidence.h5_refuted;
        FalsificationCaseResult::new(
            BrokenKind::F4ThresholdPerWeight,
            format!(
                "H5.4 structural mask fixture Refuted h5_4_refuted={} h5_refuted={} mask legal={:?} illegal={:?} shape_rejected={}",
                evidence.h5_4_refuted,
                evidence.h5_refuted,
                threshold_evidence.legal_per_row_mask,
                threshold_evidence.illegal_per_weight_mask,
                threshold_evidence.per_weight_shape_rejected
            ),
            caught,
        )
    })
}

#[test]
fn falsify_f4_threshold_per_weight() {
    crate::assert_case(run());
}

#[test]
fn f4_without_guard_keeps_deployable_per_weight_shape_rejected_without_refuting_h5_4() {
    let evidence = threshold_per_weight_verifier_evidence().expect("F4 verifier evidence");
    let threshold_evidence = &evidence.threshold_evidence;

    evidence.report.validate().expect("F4 H5 report validates");
    assert!(
        threshold_evidence.per_weight_shape_rejected,
        "deployable per-weight thresholds must stay rejected even outside the broken Guard"
    );
    assert_eq!(
        threshold_evidence.legal_per_row_mask, threshold_evidence.illegal_per_weight_mask,
        "inactive F4 Guard should document the mask_deviates=false branch"
    );
    assert!(
        !threshold_evidence.mask_deviates && !evidence.h5_4_refuted && !evidence.h5_refuted,
        "inactive F4 evidence must not hand-model a refutation: {evidence:#?}"
    );
}
