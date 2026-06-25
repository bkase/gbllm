use gbf_artifact::ParetoVerdict;
use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f3_pareto_unequal_bytes,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_experiments::s7::pareto::{S7ParetoPoint, s7_pareto_closure_signals, s7_pareto_verdict};

#[test]
fn f3_pareto_unequal_bytes_refutes_h4() {
    let verdict = s7_pareto_verdict(point(1.0, 111), point(1.1, 100), 10).expect("Pareto verdict");
    let signals = s7_pareto_closure_signals(verdict);
    assert_eq!(verdict, ParetoVerdict::Incomparable);
    assert!(!signals.h3_refuted);
    assert!(signals.h4_refuted);

    let evidence = f3_pareto_unequal_bytes::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::ParetoUnequalBytes {
            bytes_diff: 11,
            d6_tolerance_bytes: 10,
            broken_compared_as_equivalent: true,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::ParetoUnequalBytes,
        S7Outcome::FailPareto,
        S7Decision::Investigate {
            reason: "pareto-incomparable",
        },
        f3_pareto_unequal_bytes::run,
    );
}

fn point(median_val_bpc: f64, deployed_bytes_total: u64) -> S7ParetoPoint {
    S7ParetoPoint::new(median_val_bpc, deployed_bytes_total).expect("valid Pareto point")
}
