use gbf_experiments::s1::ablation::{AblationCheckpoint, compare};
use gbf_experiments::s1::report::Hypothesis;
use gbf_experiments::s1::schema::{S1BuildKind, S1Outcome};
use gbf_foundation::Hash256;

#[test]
fn f4_phase_a_leaks_ternary_refutes_h4_and_fails_phase() {
    let phase_a_metadata = crate::checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = crate::checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![crate::tensor("toy0.blocks.0.weight", vec![1.0, 0.0, -1.0])];
    let ablation_tensors = vec![crate::tensor("toy0.blocks.0.weight", vec![1.0, 0.5, -1.0])];

    let report = compare(
        AblationCheckpoint {
            metadata: &phase_a_metadata,
            checkpoint_sha: Hash256::ZERO,
            tensors: &phase_a_tensors,
        },
        AblationCheckpoint {
            metadata: &ablation_metadata,
            checkpoint_sha: Hash256::ZERO,
            tensors: &ablation_tensors,
        },
    )
    .expect("ablation comparison");

    assert!(
        !report.phase_a_eq_ablation,
        "soft ternary leak must change canonical tensor payload bytes"
    );
    assert!(report.first_mismatch.is_some());

    crate::assert_falsification_outcome(
        "F4",
        crate::refute(crate::confirmed_input(), Hypothesis::H4),
        S1Outcome::FailPhase,
        crate::fail_phase_decision(),
    );
}
