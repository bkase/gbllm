use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn oracle_drift_under_corpus_switch_refutes_h5_even_when_bpc_numbers_match() {
    let inputs = suite_inputs();
    assert_eq!(
        inputs.oracle_drift.expected_artifact_oracle_corpus,
        "gutenberg_val"
    );
    assert_eq!(
        inputs.oracle_drift.observed_artifact_oracle_corpus,
        "tinystories_val"
    );
    assert_eq!(
        inputs.oracle_drift.live_training_bpc,
        inputs.oracle_drift.artifact_oracle_bpc
    );

    let result =
        run_s4_falsification_case(&inputs, S4FalsificationCase::OracleDriftUnderCorpusSwitch);

    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H5);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.detail.contains("non-Gutenberg"));
    assert!(result.refuted_as_expected());
}
