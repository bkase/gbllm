use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::schema::{
    HardnessTriple, PhaseEffectiveLambda, PhaseEffectiveLambdaValues, PhaseKindS2, QuantHardness,
    S2BuildKind,
};
use proptest::prelude::*;
use serde_json::Value;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn arb_phase_kind_s2_canonical_json_round_trips(phase in arb_phase_kind_s2()) {
        let bytes = S1CanonicalJson::to_vec(&phase).expect("canonical S2 JSON");
        let decoded: PhaseKindS2 = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(decoded, phase);
    }

    #[test]
    fn arb_hardness_triple_has_stable_field_order(triple in arb_hardness_triple()) {
        let bytes = S1CanonicalJson::to_vec(&triple).expect("canonical S2 JSON");
        let value: Value = serde_json::from_slice(&bytes).expect("JSON");
        let reparsed = S1CanonicalJson::value_to_vec(&value).expect("canonical S2 JSON");

        prop_assert_eq!(&reparsed, &bytes);
        prop_assert!(starts_with_activation_qat_key(&bytes));
        prop_assert!(bytes.windows(br#","expert_qat":"#.len()).any(|window| window == br#","expert_qat":"#));
        prop_assert!(bytes.windows(br#","norm_qat":"#.len()).any(|window| window == br#","norm_qat":"#));
    }

    #[test]
    fn arb_phase_effective_lambda_rejects_nan_and_round_trips(
        lambda_distill in finite_lambda(),
        lambda_balance in finite_lambda(),
        lambda_zrouter in finite_lambda(),
        lambda_switch in finite_lambda(),
        lambda_range in finite_lambda(),
        lambda_zero in finite_lambda(),
        lambda_shape in finite_lambda(),
        lambda_overflow in finite_lambda(),
    ) {
        let lambdas = PhaseEffectiveLambda::new(PhaseEffectiveLambdaValues {
            lambda_distill,
            lambda_balance,
            lambda_zrouter,
            lambda_switch,
            lambda_range,
            lambda_zero,
            lambda_shape,
            lambda_overflow,
        }).expect("finite non-negative lambdas");
        let bytes = S1CanonicalJson::to_vec(&lambdas).expect("canonical S2 JSON");
        let decoded: PhaseEffectiveLambda = serde_json::from_slice(&bytes).expect("round trip");

        prop_assert_eq!(decoded, lambdas);
    }

    #[test]
    fn arb_s2_build_kind_has_no_unknown_variants(kind in arb_s2_build_kind()) {
        let bytes = S1CanonicalJson::to_vec(&kind).expect("canonical S2 JSON");
        let decoded: S2BuildKind = serde_json::from_slice(&bytes).expect("round trip");

        prop_assert_eq!(decoded, kind);
        prop_assert!(serde_json::from_str::<S2BuildKind>(r#""s2-phase-a""#).is_err());
    }
}

#[test]
fn phase_effective_lambda_constructor_rejects_non_finite_values() {
    let mut values = PhaseEffectiveLambdaValues {
        lambda_distill: 1.0,
        lambda_balance: 0.0,
        lambda_zrouter: 0.0,
        lambda_switch: 0.0,
        lambda_range: 0.01,
        lambda_zero: 0.0001,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    };
    values.lambda_distill = f32::NAN;
    assert!(PhaseEffectiveLambda::new(values).is_err());
    values.lambda_distill = 1.0;
    values.lambda_zero = -0.1;
    assert!(PhaseEffectiveLambda::new(values).is_err());
}

fn arb_phase_kind_s2() -> impl Strategy<Value = PhaseKindS2> {
    prop_oneof![
        Just(PhaseKindS2::PhaseA),
        Just(PhaseKindS2::PhaseB),
        Just(PhaseKindS2::PhaseC),
        Just(PhaseKindS2::PhaseD),
    ]
}

fn arb_quant_hardness() -> impl Strategy<Value = QuantHardness> {
    prop_oneof![
        Just(QuantHardness::Off),
        Just(QuantHardness::Soft),
        Just(QuantHardness::Hard),
    ]
}

fn arb_hardness_triple() -> impl Strategy<Value = HardnessTriple> {
    (
        arb_quant_hardness(),
        arb_quant_hardness(),
        arb_quant_hardness(),
    )
        .prop_map(|(expert_qat, activation_qat, norm_qat)| {
            HardnessTriple::new(expert_qat, activation_qat, norm_qat)
        })
}

fn arb_s2_build_kind() -> impl Strategy<Value = S2BuildKind> {
    prop_oneof![
        Just(S2BuildKind::s2_ternary_full),
        Just(S2BuildKind::s2_fp_full),
        Just(S2BuildKind::s2_ternary_nodistill),
        Just(S2BuildKind::s2_ablation),
    ]
}

fn finite_lambda() -> impl Strategy<Value = f32> {
    (0_u16..=10_000).prop_map(|value| f32::from(value) / 10_000.0)
}

fn starts_with_activation_qat_key(bytes: &[u8]) -> bool {
    bytes
        .strip_prefix(b"{")
        .is_some_and(|tail| tail.starts_with(br#""activation_qat":"#))
}
