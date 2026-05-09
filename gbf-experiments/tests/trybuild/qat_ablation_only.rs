fn main() {
    let _ = gbf_experiments::S1_LOG_TARGET;
    assert_eq!(
        gbf_experiments::s1::build_metadata::BUILD_KIND,
        "ablation"
    );
    assert!(!gbf_experiments::s1::build_metadata::FALSIFY_ENABLED);
    assert_eq!(
        gbf_experiments::s1::build_metadata::build_metadata().build_kind,
        "ablation"
    );
}
