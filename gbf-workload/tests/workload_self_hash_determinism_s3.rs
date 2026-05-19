#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

#[test]
fn workload_self_hash_determinism_s3() {
    let text = v0_success_s3_support::fixture_text();
    let mut observed = Vec::new();

    for _ in 0..10 {
        let manifest = gbf_workload::WorkloadManifest_v0::from_toml_str(&text)
            .expect("v0_success manifest validates");
        observed.push(
            manifest
                .compute_self_hash()
                .expect("workload self hash computes"),
        );
        assert_eq!(manifest.workload_self_hash, *observed.last().unwrap());
    }

    assert!(observed.windows(2).all(|window| window[0] == window[1]));
}
