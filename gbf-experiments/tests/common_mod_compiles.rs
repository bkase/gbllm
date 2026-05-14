mod common;

use common::helpers::phase_log_capture::PhaseLogCapture;
use common::proptest_strategies::arb_train_config_s2;
use proptest::strategy::Strategy;

#[test]
fn common_module_public_surface_compiles() {
    let mut capture = PhaseLogCapture::new();
    capture.push_transition(None, "PhaseA", 0);
    assert_eq!(capture.entries().len(), 1);

    let _strategy = arb_train_config_s2().boxed();
}
