#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::fixtures::{build_kind_matrix, fixed_kn_fixture, toy0_model_factory};
use common_s3::helpers::ndjson_capture::NdjsonCaptureSink;
use common_s3::proptest_strategies_s3::{
    arb_hardness_triple, arb_phase_kind_s2, arb_s3_build_kind, arb_s3_verifier_bundle,
};
use proptest::strategy::Strategy;

#[test]
fn common_s3_module_public_surface_compiles() {
    let model = toy0_model_factory(0);
    assert_eq!(model.logits().len(), 80);
    assert_eq!(fixed_kn_fixture().order, 5);
    assert_eq!(build_kind_matrix().count(), 3);

    let mut sink = NdjsonCaptureSink::new();
    sink.push(&serde_json::json!({"event": "compile"}));
    assert_eq!(sink.entries().len(), 1);

    let _kind_strategy = arb_s3_build_kind().boxed();
    let _bundle_strategy = arb_s3_verifier_bundle().boxed();
    let _hardness_strategy = arb_hardness_triple().boxed();
    let _phase_strategy = arb_phase_kind_s2().boxed();
}
