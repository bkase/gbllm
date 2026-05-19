use std::collections::BTreeMap;

use gbf_experiments::s2::schema::PhaseKindS2;
use gbf_experiments::s3::schema::{
    HypothesisStatus, OracleFallbackTag, S3BuildKind, S3Completion, S3Decision, S3Hypothesis,
    S3Outcome, S3VerifierBundle,
};
use proptest::prelude::*;

use crate::common_s3::fixtures::ToyHardTernaryStudent;

#[allow(unused_imports)]
pub use crate::common::proptest_strategies::{HardnessTriple, arb_hardness_triple};

pub fn arb_s3_build_kind() -> impl Strategy<Value = S3BuildKind> {
    prop_oneof![
        Just(S3BuildKind::s3_v0_success_real_oracle),
        Just(S3BuildKind::s3_v0_success_fallback_oracle),
        Just(S3BuildKind::s3_oracle_adversarial),
    ]
}

pub fn arb_s3_outcome() -> impl Strategy<Value = S3Outcome> {
    prop_oneof![
        Just(S3Outcome::PassClean),
        Just(S3Outcome::PassWithFallbackOracle),
        Just(S3Outcome::FailCharset),
        Just(S3Outcome::FailBaseline),
        Just(S3Outcome::FailQuality),
        Just(S3Outcome::FailSuspicious),
        Just(S3Outcome::FailOracleAgreement),
        Just(S3Outcome::FailBundle),
        Just(S3Outcome::FailQuantspec),
        Just(S3Outcome::FailSubstrate),
        Just(S3Outcome::FailPhase),
        Just(S3Outcome::FailFalsification),
        Just(S3Outcome::FailApiDrift),
        Just(S3Outcome::FailMetric),
        Just(S3Outcome::FailPreregistration),
        Just(S3Outcome::FailArtifact),
        Just(S3Outcome::FailIncomplete),
    ]
}

pub fn arb_s3_decision() -> impl Strategy<Value = S3Decision> {
    prop_oneof![
        Just(S3Decision::ProceedToS4),
        Just(S3Decision::ProceedToS4WithDeferredClause),
        reason_tag().prop_map(|reason| S3Decision::Investigate { reason }),
        reason_tag().prop_map(|reason| S3Decision::Halt { reason }),
    ]
}

pub fn arb_s3_completion() -> impl Strategy<Value = S3Completion> {
    prop_oneof![
        Just(S3Completion::Completed),
        (0_u64..=10_000).prop_map(|step| S3Completion::DivergedAt { step }),
        Just(S3Completion::NotReached),
    ]
}

pub fn arb_oracle_fallback_tag() -> impl Strategy<Value = OracleFallbackTag> {
    prop_oneof![
        Just(OracleFallbackTag::S3DenotationalFallback),
        Just(OracleFallbackTag::S3ArtifactFallback),
        Just(OracleFallbackTag::S3LiveObservationFixture),
    ]
}

pub fn arb_s3_hypothesis() -> impl Strategy<Value = S3Hypothesis> {
    prop_oneof![
        Just(S3Hypothesis::H1),
        Just(S3Hypothesis::H2),
        Just(S3Hypothesis::H3),
        Just(S3Hypothesis::H4),
        Just(S3Hypothesis::H5),
        Just(S3Hypothesis::H6),
        Just(S3Hypothesis::H7),
    ]
}

pub fn arb_hypothesis_status() -> impl Strategy<Value = HypothesisStatus> {
    prop_oneof![
        Just(HypothesisStatus::Confirmed),
        Just(HypothesisStatus::Refuted),
        reason_tag().prop_map(|reason| HypothesisStatus::NotEvaluatedDueToPriorGate { reason }),
    ]
}

pub fn arb_s3_verifier_bundle() -> impl Strategy<Value = S3VerifierBundle> {
    (
        prop::collection::vec(any::<bool>(), 11),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(arb_s3_completion(), 0..=15),
        prop::collection::vec(arb_hypothesis_status(), S3Hypothesis::ALL.len()),
        prop::collection::vec(arb_oracle_fallback_tag(), 0..=2),
    )
        .prop_map(
            |(
                gate_bools,
                methodological_controls_present,
                suspicious_low_bpc,
                completions,
                statuses,
                oracle_fallback_used,
            )| {
                let hypothesis_statuses = S3Hypothesis::ALL
                    .into_iter()
                    .zip(statuses)
                    .collect::<BTreeMap<_, _>>();
                S3VerifierBundle {
                    preregistration_passed: gate_bools[0],
                    artifact_integrity_passed: gate_bools[1],
                    oracle_re_run_passed: gate_bools[2],
                    api_drift_check_passed: gate_bools[3],
                    falsification_s3_passed: gate_bools[4],
                    bundle_determinism_passed: gate_bools[5],
                    artifact_determinism_passed: gate_bools[6],
                    charset_idempotence_passed: gate_bools[7],
                    kn_oracle_passed: gate_bools[8],
                    oracle_agreement_passed: gate_bools[9],
                    quantspec_resolution_passed: gate_bools[10],
                    methodological_controls_present,
                    suspicious_low_bpc,
                    completions,
                    hypothesis_statuses,
                    oracle_fallback_used,
                }
            },
        )
}

pub fn arbitrary_hard_ternary_student_model() -> impl Strategy<Value = ToyHardTernaryStudent> {
    prop::collection::vec(-2_i8..=2, 1..=16).prop_map(|weights| {
        ToyHardTernaryStudent::new(weights.into_iter().map(f32::from).collect::<Vec<_>>(), true)
    })
}

pub fn arb_phase_kind_s2() -> impl Strategy<Value = PhaseKindS2> {
    prop_oneof![
        Just(PhaseKindS2::PhaseA),
        Just(PhaseKindS2::PhaseB),
        Just(PhaseKindS2::PhaseC),
        Just(PhaseKindS2::PhaseD),
    ]
}

fn reason_tag() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,31}"
}
