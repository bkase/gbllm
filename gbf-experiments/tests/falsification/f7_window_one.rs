use gbf_experiments::s7::falsify::{S7FalsificationCase, S7FalsificationEvidence, f7_window_one};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_model::loss::temporal_smoothness::{SmoothnessWindow, TemporalSmoothnessError};

#[test]
fn f7_window_one_refutes_h5() {
    let err = SmoothnessWindow::new(1).expect_err("window one must be rejected");
    assert!(matches!(
        err,
        TemporalSmoothnessError::SmoothnessWindowTooSmall { value: 1 }
    ));

    let evidence = f7_window_one::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::WindowOne {
            smoothness_window: 1,
            constructed: true,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::WindowOne,
        S7Outcome::FailSwitchStats,
        S7Decision::Halt {
            reason: "export-schema-broken",
        },
        f7_window_one::run,
    );
}
