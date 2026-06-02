use gbf_artifact::{
    ShadowEmissionId, ShadowStep, shadow_compile_sample_real_emission_order,
    shadow_compile_sample_real_path,
};
use gbf_foundation::CanonicalJson;
use serde_json::json;

#[test]
fn shadow_emission_id_canonical_json_is_field_discriminated() {
    let cadence = ShadowEmissionId::cadence(ShadowStep::new(20000));
    let phase_e_final = ShadowEmissionId::phase_e_final();

    assert_eq!(
        CanonicalJson::to_vec(&cadence).expect("cadence canonicalizes"),
        br#"{"step":20000}"#
    );
    assert_eq!(
        CanonicalJson::to_vec(&phase_e_final).expect("phase-e final canonicalizes"),
        br#"{"phase_e_final":true}"#
    );
}

#[test]
fn artifact_reexport_resolves_distinct_s5_shadow_paths() {
    let paths: Vec<_> = shadow_compile_sample_real_emission_order()
        .into_iter()
        .map(|emission_id| shadow_compile_sample_real_path(0, emission_id))
        .collect();

    assert_eq!(paths.len(), 6);
    assert!(paths.iter().any(|path| path.ends_with("step-20000.json")));
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("phase-e-final.json"))
    );
}

#[test]
fn shadow_emission_id_rejects_ambiguous_json() {
    assert!(
        serde_json::from_value::<ShadowEmissionId>(json!({"step": 20000, "phase_e_final": true}))
            .is_err()
    );
}
