#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::proptest_strategies_s3::{
    arb_oracle_fallback_tag, arb_s3_build_kind, arb_s3_completion, arb_s3_decision, arb_s3_outcome,
    arb_s3_verifier_bundle,
};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s3::schema::{
    OracleFallbackTag, S3BuildKind, S3Completion, S3Decision, S3Hypothesis, S3Outcome,
    S3VerifierBundle,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn s3_build_kind_round_trips(kind in arb_s3_build_kind()) {
        assert_canonical_round_trip::<S3BuildKind>(&kind);
        prop_assert!(serde_json::from_str::<S3BuildKind>(r#""s3-phase-d""#).is_err());
    }

    #[test]
    fn s3_outcome_round_trips(outcome in arb_s3_outcome()) {
        assert_canonical_round_trip::<S3Outcome>(&outcome);
    }

    #[test]
    fn s3_decision_round_trips(decision in arb_s3_decision()) {
        assert_canonical_round_trip::<S3Decision>(&decision);
    }

    #[test]
    fn s3_completion_round_trips(completion in arb_s3_completion()) {
        assert_canonical_round_trip::<S3Completion>(&completion);
    }

    #[test]
    fn oracle_fallback_tag_round_trips(tag in arb_oracle_fallback_tag()) {
        assert_canonical_round_trip::<OracleFallbackTag>(&tag);
    }

    #[test]
    fn s3_verifier_bundle_round_trips(bundle in arb_s3_verifier_bundle()) {
        assert_canonical_round_trip::<S3VerifierBundle>(&bundle);
    }
}

#[test]
fn s3_taxonomy_all_arrays_match_rfc_counts() {
    assert_eq!(S3Hypothesis::ALL.len(), 7);
    assert_eq!(S3Outcome::ALL.len(), 17);
    assert_eq!(S3BuildKind::ALL.len(), 3);
}

#[test]
fn s3_closure_candidate_has_all_gates_confirmed() {
    let bundle = S3VerifierBundle::closure_candidate();

    assert!(bundle.preregistration_passed);
    assert!(bundle.artifact_integrity_passed);
    assert!(bundle.oracle_re_run_passed);
    assert!(bundle.api_drift_check_passed);
    assert!(bundle.falsification_s3_passed);
    assert!(bundle.bundle_determinism_passed);
    assert!(bundle.artifact_determinism_passed);
    assert!(bundle.charset_idempotence_passed);
    assert!(bundle.kn_oracle_passed);
    assert!(bundle.oracle_agreement_passed);
    assert!(bundle.quantspec_resolution_passed);
    assert!(bundle.methodological_controls_present);
    assert!(!bundle.suspicious_low_bpc);
    assert!(bundle.oracle_fallback_used.is_empty());
    assert_eq!(bundle.completions.len(), 15);
    assert!(bundle.first_not_evaluated().is_none());
}

fn assert_canonical_round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = S1CanonicalJson::to_vec(value).expect("canonical S3 JSON");
    let decoded: T = serde_json::from_slice(&bytes).expect("round trip");
    let decoded_bytes = S1CanonicalJson::to_vec(&decoded).expect("canonical S3 JSON");

    assert_eq!(&decoded, value);
    assert_eq!(decoded_bytes, bytes);
}
