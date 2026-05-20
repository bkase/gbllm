use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn train_random_init_refutes_h6_via_actual_payload_lineage() {
    let inputs = suite_inputs();
    assert_ne!(
        inputs.lineage.actual_initial_checkpoint_payload_sha,
        inputs.lineage.c_ts_checkpoint_payload_sha
    );
    assert_eq!(
        inputs.lineage.recorded_initial_checkpoint_payload_sha,
        inputs.lineage.c_ts_checkpoint_payload_sha
    );

    let result = run_s4_falsification_case(&inputs, S4FalsificationCase::TrainRandomInit);

    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H6);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.detail.contains("actual in-memory weights"));
    assert!(result.refuted_as_expected());
}
