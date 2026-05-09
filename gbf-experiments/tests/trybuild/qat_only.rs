fn main() {
    let _ = gbf_experiments::S1_LOG_TARGET;
    assert_eq!(
        gbf_experiments::s1::build_metadata::BUILD_KIND,
        "phase_a"
    );
    assert!(!gbf_experiments::s1::build_metadata::FALSIFY_ENABLED);
    assert_eq!(
        gbf_experiments::s1::build_metadata::build_metadata().build_kind,
        "phase_a"
    );
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
