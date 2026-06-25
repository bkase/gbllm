#![cfg(feature = "s7")]

use gbf_experiments::s7::baseline_match::canonical_s7_matched_bytes_pin;
use gbf_experiments::s7::outcome::{
    AggregateParityVerdict, MATCHED_BYTES_INVALID_REASON, S7Decision, S7Outcome,
    S7OutcomeDispatchInput, S7OutcomeError, aggregate_parity_verdict, decision_for_s7_outcome,
    dense_only_closure_permitted, dispatch_s7_outcome,
};
use gbf_experiments::s7::parity::s7_parity_aggregate;

#[test]
fn aggregate_parity_separates_bytes_invalid_from_scientific_parity_fail() {
    assert_eq!(
        s7_parity_aggregate(&[true, true, true, true, true], 11, 10).unwrap(),
        AggregateParityVerdict::FailBytes
    );
    assert_eq!(
        aggregate_parity_verdict(&[true, false, true, true, true], 10, 10).unwrap(),
        AggregateParityVerdict::FailParity
    );
    assert_eq!(
        aggregate_parity_verdict(&[true, true, true, true, true], 10, 10).unwrap(),
        AggregateParityVerdict::PassClean
    );
    assert_eq!(
        aggregate_parity_verdict(&[], 0, 10).unwrap_err(),
        S7OutcomeError::MissingPerSeedParityVerdict
    );
}

#[test]
fn bytes_mismatch_fixture_halts_instead_of_dense_only() {
    let pin = canonical_s7_matched_bytes_pin().expect("canonical matched-bytes pin");
    let scaled_dense_total = pin.b_deployed_total_moe + pin.tolerance_bytes + 1;
    let bytes_diff = pin.b_deployed_total_moe.abs_diff(scaled_dense_total);
    let aggregate = s7_parity_aggregate(
        &[true, true, true, true, true],
        bytes_diff,
        pin.tolerance_bytes,
    )
    .expect("bytes mismatch routes before per-seed parity");
    let outcome = dispatch_s7_outcome(S7OutcomeDispatchInput {
        aggregate_parity_verdict: aggregate,
        h3_refuted: true,
        ..S7OutcomeDispatchInput::default()
    });
    let decision = decision_for_s7_outcome(outcome);

    assert_eq!(aggregate, AggregateParityVerdict::FailBytes);
    assert_eq!(outcome, S7Outcome::FailBytes);
    assert_eq!(
        decision,
        S7Decision::Halt {
            reason: MATCHED_BYTES_INVALID_REASON,
        }
    );
    assert_ne!(decision, S7Decision::ProceedToS8DenseOnly);
    assert!(!dense_only_closure_permitted(outcome, false, true));
}

#[test]
fn outcome_dispatch_routes_fail_bytes_before_h3_fail_parity() {
    let outcome = dispatch_s7_outcome(S7OutcomeDispatchInput {
        aggregate_parity_verdict: AggregateParityVerdict::FailBytes,
        h3_refuted: true,
        ..S7OutcomeDispatchInput::default()
    });

    assert_eq!(outcome, S7Outcome::FailBytes);
}

#[test]
fn decision_dispatch_keeps_fail_bytes_out_of_dense_only_closure() {
    assert_eq!(
        decision_for_s7_outcome(S7Outcome::FailParity),
        S7Decision::ProceedToS8DenseOnly
    );
    assert_eq!(
        decision_for_s7_outcome(S7Outcome::FailBytes),
        S7Decision::Halt {
            reason: MATCHED_BYTES_INVALID_REASON,
        }
    );
    assert!(dense_only_closure_permitted(
        S7Outcome::FailParity,
        true,
        true
    ));
    assert!(!dense_only_closure_permitted(
        S7Outcome::FailBytes,
        false,
        true
    ));
    assert!(!dense_only_closure_permitted(
        S7Outcome::FailParity,
        false,
        true
    ));
}

#[test]
fn decision_dispatch_matches_section_12_reason_tags() {
    let cases = [
        (S7Outcome::PassClean, S7Decision::ProceedToS8),
        (S7Outcome::FailParity, S7Decision::ProceedToS8DenseOnly),
        (
            S7Outcome::FailBytes,
            S7Decision::Halt {
                reason: MATCHED_BYTES_INVALID_REASON,
            },
        ),
        (
            S7Outcome::FailPareto,
            S7Decision::Investigate {
                reason: "pareto-incomparable",
            },
        ),
        (
            S7Outcome::FailMoeTrain,
            S7Decision::Investigate {
                reason: "burn-or-loss-substrate",
            },
        ),
        (
            S7Outcome::FailRouterCollapse,
            S7Decision::Investigate {
                reason: "reduce-lambda-switch-or-tune-dropout",
            },
        ),
        (
            S7Outcome::FailRouterCollapseGuardrail,
            S7Decision::Investigate {
                reason: "sweep-grid-or-thresholds",
            },
        ),
        (
            S7Outcome::FailDenseBaseline,
            S7Decision::Investigate {
                reason: "dense-topology-constructor",
            },
        ),
        (
            S7Outcome::FailSwitchStats,
            S7Decision::Halt {
                reason: "export-schema-broken",
            },
        ),
        (
            S7Outcome::FailGradProvenance,
            S7Decision::Halt {
                reason: "loss-math-dishonest",
            },
        ),
        (
            S7Outcome::FailBurnGrad,
            S7Decision::Halt {
                reason: "burn-adapter-broken",
            },
        ),
        (
            S7Outcome::FailSuspicious,
            S7Decision::Halt {
                reason: "audit-split-and-bpc",
            },
        ),
        (
            S7Outcome::FailOracleRouted,
            S7Decision::Halt {
                reason: "oracle-cannot-resolve-routed-FFN",
            },
        ),
        (
            S7Outcome::FailEmulatorRouted,
            S7Decision::Halt {
                reason: "routed-encoded-rom-broken",
            },
        ),
    ];

    for (outcome, expected) in cases {
        assert_eq!(decision_for_s7_outcome(outcome), expected);
    }
}

#[test]
fn active_outcome_set_has_no_pass_with_warning_slot() {
    assert_eq!(S7Outcome::ALL.len(), 14);
    assert!(S7Outcome::ALL.contains(&S7Outcome::FailBytes));
    assert!(S7Outcome::ALL.contains(&S7Outcome::FailParity));

    for outcome in S7Outcome::ALL {
        let _decision = decision_for_s7_outcome(outcome);
    }
}

#[test]
fn o7_outcome_dispatch_totality_enumerates_observable_combinations() {
    let mut reachable = std::collections::HashSet::new();
    let aggregate_verdicts = [
        AggregateParityVerdict::PassClean,
        AggregateParityVerdict::FailParity,
        AggregateParityVerdict::FailBytes,
    ];

    for bits in 0_u16..(1 << 14) {
        for aggregate_parity_verdict in aggregate_verdicts {
            let input = S7OutcomeDispatchInput {
                moe_diverged: bit(bits, 0),
                moe_collapsed: bit(bits, 1),
                h1_refuted_non_collapse: bit(bits, 2),
                dense_diverged: bit(bits, 3),
                h2_refuted: bit(bits, 4),
                h7_refuted: bit(bits, 5),
                h8_refuted: bit(bits, 6),
                h5_refuted: bit(bits, 7),
                h6_refuted: bit(bits, 8),
                suspicious_moe_bpc: bit(bits, 9),
                aggregate_parity_verdict,
                h3_refuted: bit(bits, 10),
                h4_refuted: bit(bits, 11),
                h9_refuted: bit(bits, 12),
                h10_refuted: bit(bits, 13),
            };
            let outcome = dispatch_s7_outcome(input);
            let decision = decision_for_s7_outcome(outcome);

            reachable.insert(outcome);
            assert!(S7Outcome::ALL.contains(&outcome));
            assert_eq!(
                matches!(decision, S7Decision::ProceedToS8DenseOnly),
                matches!(outcome, S7Outcome::FailParity)
            );
            assert_ne!(
                (outcome, decision),
                (S7Outcome::FailBytes, S7Decision::ProceedToS8DenseOnly)
            );
        }
    }

    assert_eq!(reachable.len(), S7Outcome::ALL.len());
    for outcome in S7Outcome::ALL {
        assert!(
            reachable.contains(&outcome),
            "{outcome:?} must be reachable"
        );
    }
}

#[test]
fn rfc_pins_fail_bytes_and_dense_only_contract() {
    let rfc = include_str!("../../history/rfcs/F-S7-moe-beats-dense.md");
    let outcome_section = section_between(rfc, "# 12. Outcome algebra", "# 13. Artifact schemas");
    let objective_section =
        section_between(rfc, "O7  Outcome algebra totality", "O8  No hidden inputs");
    let theorem_section = section_between(rfc, "# 17. Minimal end-to-end theorem", "Not proven:");
    let closure_contract = section_between(
        rfc,
        "12. Decision is one of {ProceedToS8, ProceedToS8-DenseOnly}",
        "13. Every JSON artifact",
    );

    assert!(outcome_section.contains("| Fail-bytes"));
    assert!(outcome_section.contains("elif aggregate_parity_verdict = Fail-bytes"));
    assert!(
        outcome_section.contains("Fail-bytes                          \u{2192} Decision::Halt")
    );
    assert!(!outcome_section.contains("Pass-with-warning"));
    assert!(objective_section.contains("maps to exactly\n    one S7Outcome variant under §12"));
    assert!(theorem_section.contains("Fail-bytes"));
    assert!(!theorem_section.contains("Pass-with-warning"));
    assert!(closure_contract.contains("permitted iff S7Outcome = Fail-parity"));
    assert!(closure_contract.contains("Bytes mismatch"));
    assert!(closure_contract.contains("MUST\n    NOT close bd-2v9r as DenseOnly"));
}

const fn bit(bits: u16, index: u8) -> bool {
    ((bits >> index) & 1) == 1
}

fn section_between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let start_index = text.find(start).expect("section start");
    let rest = &text[start_index..];
    let end_index = rest.find(end).expect("section end");
    &rest[..end_index]
}
