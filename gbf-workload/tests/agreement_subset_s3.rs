#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

use gbf_workload::V0_SUCCESS_AGREEMENT_SUBSET_SIZE;

#[test]
fn agreement_subset_s3() {
    let manifest = v0_success_s3_support::load_v0_success();
    let subset = manifest.agreement_subset();

    assert_eq!(subset.len(), V0_SUCCESS_AGREEMENT_SUBSET_SIZE);
    assert_eq!(subset[0].id.as_str(), "v0-success-001");
    assert_eq!(subset[1].id.as_str(), "v0-success-002");
    assert_eq!(subset[2].id.as_str(), "v0-success-003");

    let reparsed = gbf_workload::WorkloadManifest_v0::from_toml_str(
        &manifest
            .to_toml_string()
            .expect("v0_success manifest serializes"),
    )
    .expect("serialized v0_success reparses");
    let reparsed_ids = reparsed
        .agreement_subset()
        .iter()
        .map(|prompt| prompt.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        reparsed_ids,
        ["v0-success-001", "v0-success-002", "v0-success-003"]
    );
}
