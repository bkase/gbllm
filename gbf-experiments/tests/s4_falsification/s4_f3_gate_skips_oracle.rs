use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn promotion_gate_skip_oracle_agreement_refutes_h3() {
    let inputs = suite_inputs();
    assert!(inputs.promotion_gate.oracle_agreement_required);
    assert!(!inputs.promotion_gate.oracle_agreement_artifact_present);
    assert!(inputs.promotion_gate.broken_gate_promotes_without_oracle);

    let result = run_s4_falsification_case(
        &inputs,
        S4FalsificationCase::PromotionGateSkipsOracleAgreement,
    );
    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H3);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.detail.contains("P-2"));
    assert!(result.refuted_as_expected());
}
