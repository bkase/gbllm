#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::proptest_strategies_s3::arbitrary_hard_ternary_student_model;
use gbf_train::student::{HardTernaryStudentModel, freeze_student_as_artifact};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn freeze_student_succeeds_and_breaks_storage_aliasing(
        student in arbitrary_hard_ternary_student_model()
    ) {
        let source_identity = student.student_storage_identity();
        let source_weight = student.student_weight_fingerprint();
        let source_storage = student.student_storage_fingerprint();

        let frozen = freeze_student_as_artifact(&student).expect("student freeze succeeds");

        prop_assert!(!frozen.requires_grad());
        prop_assert!(!frozen.snapshot().student_requires_grad());
        prop_assert_eq!(frozen.weight_fingerprint(), &source_weight);
        prop_assert_eq!(frozen.storage_fingerprint(), &source_storage);
        prop_assert_ne!(frozen.snapshot().student_storage_identity(), source_identity);
    }
}
