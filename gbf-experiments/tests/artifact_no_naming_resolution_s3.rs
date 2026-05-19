#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::export_product;

#[test]
fn artifact_no_naming_resolution_s3() {
    let product = export_product(0);

    assert_eq!(
        product
            .metadata
            .weight_resolution_summary
            .tensors_resolved_via_naming,
        0
    );
    assert!(
        product
            .artifact_validation
            .weight_resolution_log
            .iter()
            .all(|entry| !entry.resolved_via_naming)
    );
}
