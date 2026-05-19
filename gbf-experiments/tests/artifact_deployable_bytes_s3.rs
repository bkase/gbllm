#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::{
    artifact_from_tensors, id, quant_for, sparse_aux_with_sidecar, tensor, ternary_quant,
};
use gbf_artifact::{
    ArtifactAux, ClassifierView, Dtype, PayloadRole, QuantSpec_S3, TiedEmbeddingAlias,
};
use gbf_experiments::s3::artifact::{
    artifact_deployable_bytes, deployable_resolution_metadata_bytes,
};

#[test]
fn artifact_deployable_bytes_s3() {
    let deployable_weight = tensor(
        "tensor.embedding",
        Dtype::Ternary2,
        vec![409_600],
        PayloadRole::DeployableWeight,
        1,
    );
    let quant_param = tensor(
        "tensor.embedding.scale",
        Dtype::Q8_8,
        vec![5_120],
        PayloadRole::DeployableQuantParam,
        2,
    );
    let reference_fp32 = tensor(
        "tensor.embedding.reference_fp32",
        Dtype::Fp32,
        vec![51_200],
        PayloadRole::ReferenceFp32,
        3,
    );
    let quant = quant_for(&[deployable_weight.id.clone()]);
    let alias = Some(TiedEmbeddingAlias::new(
        id("tensor.embedding"),
        id("tensor.embedding"),
        true,
        ClassifierView::SameTensor,
    ));
    let artifact = artifact_from_tensors(
        vec![
            deployable_weight.clone(),
            quant_param.clone(),
            reference_fp32.clone(),
        ],
        quant.clone(),
        sparse_aux_with_sidecar(50),
        alias.clone(),
    );
    let metadata_overhead =
        u64::try_from(deployable_resolution_metadata_bytes(&artifact).expect("metadata bytes"))
            .unwrap();

    assert_eq!(
        artifact_deployable_bytes(&artifact).expect("deployable bytes"),
        100 * 1024 + 10 * 1024 + metadata_overhead
    );

    let sidecar_changed = artifact_from_tensors(
        vec![
            deployable_weight.clone(),
            quant_param.clone(),
            reference_fp32.clone(),
        ],
        quant.clone(),
        sparse_aux_with_sidecar(51),
        alias.clone(),
    );
    assert_eq!(
        artifact_deployable_bytes(&sidecar_changed).expect("sidecar ignored"),
        artifact_deployable_bytes(&artifact).expect("deployable bytes")
    );

    let extra_reference = tensor(
        "tensor.embedding.reference_fp32.extra",
        Dtype::Fp32,
        vec![25_600],
        PayloadRole::ReferenceFp32,
        4,
    );
    let reference_changed = artifact_from_tensors(
        vec![
            deployable_weight,
            quant_param,
            reference_fp32,
            extra_reference,
        ],
        QuantSpec_S3::new(
            [(id("tensor.embedding"), ternary_quant(128))]
                .into_iter()
                .collect(),
        ),
        ArtifactAux::sparse(),
        alias,
    );
    assert_eq!(
        artifact_deployable_bytes(&reference_changed).expect("reference fp32 ignored"),
        artifact_deployable_bytes(&artifact).expect("deployable bytes")
    );
}
