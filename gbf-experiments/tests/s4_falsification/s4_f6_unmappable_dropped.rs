use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn unmappable_rate_silently_dropped_refutes_h1() {
    let result = run_s4_falsification_case(
        &suite_inputs(),
        S4FalsificationCase::UnmappableRateSilentlyDropped,
    );

    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H1);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.detail.contains("unmappable_rate_silently_dropped"));
    assert!(result.detail.contains("fixture_local_slow_reference"));
    assert!(result.refuted_as_expected());
}
