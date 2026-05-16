#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::{export_product, frozen_missing_quant};
use gbf_artifact::PayloadRole;
use gbf_experiments::s3::artifact::{
    ArtifactExportError, ArtifactExportInputs, s3_export_model_artifact,
};
use gbf_train::export_visitor::ExportVisitor;

#[test]
fn artifact_quantspec_resolution_s3() {
    let product = export_product(0);
    let summary = &product.artifact_validation.weight_resolution_summary;

    assert_eq!(summary.tensors_resolved_via_naming, 0);
    assert_eq!(
        summary.total_tensors,
        summary.tensors_resolved_via_quant_spec
    );
    for tensor in product
        .artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
    {
        assert!(
            product
                .artifact
                .core
                .quant
                .weight_quant(&tensor.id)
                .is_some(),
            "deployable weight {} must resolve through QuantSpec",
            tensor.id
        );
    }

    let frozen = frozen_missing_quant(0);
    let error =
        s3_export_model_artifact(ArtifactExportInputs::new(&frozen, ExportVisitor::pinned()))
            .expect_err("missing quant spec entry rejects");
    assert!(matches!(
        error,
        ArtifactExportError::QuantSpecCoverageMissing { tensor_id }
            if tensor_id.as_str() == "tensor.linear.weight"
    ));
}
