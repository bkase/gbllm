mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::BuildKindDispatch;
use gbf_experiments::s2::schema::{
    HardnessTriple, QuantHardness, QuantHardnessOverride, S2BuildKind, TrainConfigS2Full,
};
use serde_json::json;

#[test]
fn build_kind_dispatch_maps_fp_to_all_off_runtime_override() {
    let config = TrainConfigS2Full::pinned();
    let scheduled = HardnessTriple::new(
        QuantHardness::Hard,
        QuantHardness::Soft,
        QuantHardness::Hard,
    );

    let ternary =
        BuildKindDispatch::resolve(S2BuildKind::s2_ternary_full, config.lambda_distill_default);
    let fp = BuildKindDispatch::resolve(S2BuildKind::s2_fp_full, config.lambda_distill_default);

    assert_eq!(ternary.quant_hardness_override, QuantHardnessOverride::None);
    assert_eq!(ternary.effective_hardness(scheduled), scheduled);
    assert_eq!(fp.quant_hardness_override, QuantHardnessOverride::AllOff);
    assert_eq!(fp.effective_hardness(scheduled), HardnessTriple::all_off());
    assert_eq!(fp.lambda_distill_default, config.lambda_distill_default);
}

#[test]
fn build_kind_dispatch_maps_nodistill_to_zero_lambda_without_hardness_override() {
    let scheduled =
        HardnessTriple::new(QuantHardness::Soft, QuantHardness::Soft, QuantHardness::Off);

    let nodistill = BuildKindDispatch::resolve(S2BuildKind::s2_ternary_nodistill, 1.0);

    assert_eq!(
        nodistill.quant_hardness_override,
        QuantHardnessOverride::None
    );
    assert_eq!(nodistill.effective_hardness(scheduled), scheduled);
    assert_eq!(nodistill.lambda_distill_default, 0.0);
}

#[test]
fn s2_ablation_is_not_a_full_runtime_dispatch_path() {
    let ablation = BuildKindDispatch::resolve(S2BuildKind::s2_ablation, 1.0);

    assert_eq!(
        ablation.quant_hardness_override,
        QuantHardnessOverride::None
    );
    assert_eq!(ablation.lambda_distill_default, 1.0);
}

#[test]
fn buildkind_dispatch_event_shape_is_captured() {
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        let _ = BuildKindDispatch::resolve(S2BuildKind::s2_fp_full, 1.0);
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "buildkind_dispatch")
        .expect("buildkind_dispatch event");
    assert_eq!(event.fields.get("build_kind"), Some(&json!("s2_fp_full")));
    assert_eq!(
        event.fields.get("quant_hardness_override"),
        Some(&json!("AllOff"))
    );
    assert_eq!(
        event.fields.get("lambda_distill_default"),
        Some(&json!(1.0))
    );
}

#[test]
fn gbf_train_dependency_keeps_default_features_disabled_for_s2_feature_paths() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("gbf-experiments Cargo.toml must be readable");
    let parsed: toml::Value = toml::from_str(&manifest).expect("Cargo.toml parses as TOML");
    let gbf_train = &parsed["dependencies"]["gbf-train"];

    assert_eq!(gbf_train["default-features"].as_bool(), Some(false));
}
