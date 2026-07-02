#![cfg(feature = "s7")]

use gbf_artifact::{
    MatchedBytesPin, ParetoVerdict, S7DenseVsMoeParetoFields, S7FrontierParetoFields,
};
use gbf_experiments::s7::outcome::{
    AggregateParityVerdict, S7Outcome, S7OutcomeDispatchInput, dispatch_s7_outcome,
};
use gbf_experiments::s7::pareto::{
    S7ParetoError, S7ParetoPoint, h4_confirmed_from_dense_vs_moe, h4_confirmed_from_frontier,
    h4_confirmed_from_pareto, s7_pareto_closure_signals, s7_pareto_verdict,
    s7_pareto_verdict_from_matched_bytes_pin,
};
use gbf_foundation::Hash256;
use gbf_policy::{BiasPolicy, MATCHED_BYTES_FORMULA_VERSION};

#[test]
fn pareto_decision_tree_covers_all_six_variants() {
    let cases = [
        (
            point(1.0, 100),
            point(1.1, 100),
            10,
            ParetoVerdict::MoeDominates,
        ),
        (
            point(1.1, 100),
            point(1.0, 100),
            10,
            ParetoVerdict::DenseDominates,
        ),
        (point(1.0, 100), point(1.0, 100), 10, ParetoVerdict::Tied),
        (
            point(1.0, 110),
            point(1.1, 100),
            10,
            ParetoVerdict::MoeWinsUnderByteEquivalence,
        ),
        (
            point(1.1, 100),
            point(1.0, 110),
            10,
            ParetoVerdict::DenseWinsUnderByteEquivalence,
        ),
        (
            point(1.0, 111),
            point(1.1, 100),
            10,
            ParetoVerdict::Incomparable,
        ),
    ];

    for (moe, dense, tolerance, expected) in cases {
        assert_eq!(s7_pareto_verdict(moe, dense, tolerance).unwrap(), expected);
    }
}

#[test]
fn strict_dominance_uses_exact_bytes_not_d6_tolerance() {
    let moe_better_bpc_but_more_bytes = point(1.0, 110);
    let dense = point(1.1, 100);

    assert_eq!(
        s7_pareto_verdict(moe_better_bpc_but_more_bytes, dense, 10).unwrap(),
        ParetoVerdict::MoeWinsUnderByteEquivalence
    );
    assert_eq!(
        s7_pareto_verdict(moe_better_bpc_but_more_bytes, dense, 9).unwrap(),
        ParetoVerdict::Incomparable
    );
}

#[test]
fn pareto_from_pin_consumes_pinned_d6_tolerance() {
    let pin = matched_bytes_pin(110, 100, 10);

    assert_eq!(
        s7_pareto_verdict_from_matched_bytes_pin(1.0, 1.1, &pin).unwrap(),
        ParetoVerdict::MoeWinsUnderByteEquivalence
    );

    let tighter_pin = matched_bytes_pin(110, 100, 9);
    assert_eq!(
        s7_pareto_verdict_from_matched_bytes_pin(1.0, 1.1, &tighter_pin).unwrap(),
        ParetoVerdict::Incomparable
    );
}

#[test]
fn h4_confirmed_uses_two_variant_confirmed_set() {
    let cases = [
        (ParetoVerdict::MoeDominates, true),
        (ParetoVerdict::DenseDominates, false),
        (ParetoVerdict::MoeWinsUnderByteEquivalence, true),
        (ParetoVerdict::DenseWinsUnderByteEquivalence, false),
        (ParetoVerdict::Incomparable, false),
        (ParetoVerdict::Tied, false),
    ];

    for (verdict, expected) in cases {
        assert_eq!(h4_confirmed_from_pareto(verdict), expected);

        let dense_vs_moe = S7DenseVsMoeParetoFields::new(verdict).unwrap();
        assert_eq!(h4_confirmed_from_dense_vs_moe(&dense_vs_moe), expected);

        let frontier = S7FrontierParetoFields::new(verdict).unwrap();
        assert_eq!(h4_confirmed_from_frontier(&frontier), expected);
    }
}

#[test]
fn pareto_closure_signals_route_dense_byte_equivalence_as_parity_failure() {
    let cases = [
        (
            ParetoVerdict::MoeDominates,
            false,
            false,
            S7Outcome::PassClean,
        ),
        (
            ParetoVerdict::DenseDominates,
            true,
            true,
            S7Outcome::FailParity,
        ),
        (
            ParetoVerdict::MoeWinsUnderByteEquivalence,
            false,
            false,
            S7Outcome::PassClean,
        ),
        (
            ParetoVerdict::DenseWinsUnderByteEquivalence,
            true,
            true,
            S7Outcome::FailParity,
        ),
        (
            ParetoVerdict::Incomparable,
            false,
            true,
            S7Outcome::FailPareto,
        ),
        (ParetoVerdict::Tied, false, true, S7Outcome::FailPareto),
    ];

    for (verdict, h3_refuted, h4_refuted, expected_outcome) in cases {
        let signals = s7_pareto_closure_signals(verdict);
        assert_eq!(signals.h3_refuted, h3_refuted, "{verdict:?} H3");
        assert_eq!(signals.h4_refuted, h4_refuted, "{verdict:?} H4");
        assert_eq!(
            dispatch_s7_outcome(S7OutcomeDispatchInput {
                aggregate_parity_verdict: AggregateParityVerdict::PassClean,
                h3_refuted: signals.h3_refuted,
                h4_refuted: signals.h4_refuted,
                ..S7OutcomeDispatchInput::default()
            }),
            expected_outcome,
            "{verdict:?} outcome"
        );
    }
}

#[test]
fn f3_pareto_unequal_bytes_falsification_routes_to_fail_pareto() {
    let verdict =
        s7_pareto_verdict(point(1.0, 111), point(1.1, 100), 10).expect("unequal bytes verdict");
    let signals = s7_pareto_closure_signals(verdict);
    let outcome = dispatch_s7_outcome(S7OutcomeDispatchInput {
        aggregate_parity_verdict: AggregateParityVerdict::PassClean,
        h3_refuted: signals.h3_refuted,
        h4_refuted: signals.h4_refuted,
        ..S7OutcomeDispatchInput::default()
    });

    assert_eq!(verdict, ParetoVerdict::Incomparable);
    assert!(!signals.h3_refuted);
    assert!(signals.h4_refuted);
    assert_eq!(outcome, S7Outcome::FailPareto);
}

#[test]
fn pareto_rejects_non_finite_bpc_inputs() {
    let err = S7ParetoPoint::new(f64::NAN, 100).expect_err("NaN bpc must fail");
    assert!(matches!(err, S7ParetoError::NonFiniteBpc { .. }));

    let err = s7_pareto_verdict(
        point(1.0, 100),
        S7ParetoPoint {
            median_val_bpc: f64::INFINITY,
            deployed_bytes_total: 100,
        },
        10,
    )
    .expect_err("infinite bpc must fail");
    assert!(matches!(err, S7ParetoError::NonFiniteBpc { .. }));
}

fn point(median_val_bpc: f64, deployed_bytes_total: u64) -> S7ParetoPoint {
    S7ParetoPoint::new(median_val_bpc, deployed_bytes_total).expect("valid point")
}

fn matched_bytes_pin(
    b_deployed_total_moe: u64,
    b_deployed_total_dense: u64,
    tolerance_bytes: u64,
) -> MatchedBytesPin {
    MatchedBytesPin {
        formula_version: MATCHED_BYTES_FORMULA_VERSION,
        d_ff_dense_resolved: 1,
        bias_policy: BiasPolicy::Q8_8PerOutput,
        b_experts_total: 0,
        b_router_overhead_total: 0,
        b_dense_ffn_total: b_deployed_total_dense,
        b_deployed_total_moe,
        b_deployed_total_dense,
        tolerance_bytes,
        matched_bytes_self_hash: Hash256::ZERO,
    }
}
