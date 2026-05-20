use gbf_experiments::s4::contamination::S4_CONTAMINATION_NGRAM_N;
use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn contamination_window_too_small_refutes_h2() {
    let inputs = suite_inputs();
    assert_eq!(inputs.corpus_oracle_inputs.contamination_math.n, 13);
    assert_eq!(S4_CONTAMINATION_NGRAM_N, 13);

    let result =
        run_s4_falsification_case(&inputs, S4FalsificationCase::ContaminationWindowTooSmall);
    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H2);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.refuted_as_expected());
}
