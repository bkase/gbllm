use gbf_experiments::s7::falsify::{S7FalsificationCase, S7FalsificationEvidence, f5_z_uncentered};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_train::loss::router::{RawRouterLogits, router_z_loss};

#[test]
fn f5_z_uncentered_refutes_h7() {
    let zero_logits = vec![0.0; 8];
    let centered = router_z_loss(RawRouterLogits::from_raw_router_logits(&zero_logits), 4)
        .expect("centered z-loss");
    let uncentered_zero_logit_baseline = (4.0_f32.ln()).powi(2);
    assert!(
        centered.abs() <= 1.0e-12,
        "centered zero-logit router baseline should be zero, got {centered}"
    );
    assert!(uncentered_zero_logit_baseline > 0.0);

    let evidence = f5_z_uncentered::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::ZUncentered {
            centered_mu_declared: true,
            zero_logit_loss_nonzero: true,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::ZUncentered,
        S7Outcome::FailGradProvenance,
        S7Decision::Halt {
            reason: "loss-math-dishonest",
        },
        f5_z_uncentered::run,
    );
}
