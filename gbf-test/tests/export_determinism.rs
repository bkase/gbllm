use std::collections::BTreeMap;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::quant::TernaryQuantEntry;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorKind};
use gbf_model::qat::{ExportedQatArtifact, TernaryLinearQat};
use gbf_test::fixtures::{
    assert_artifact_valid, assert_bytes_equal, export_tiny_model_with_router_and_expert_block,
    make_tiny_exported_artifact, make_tiny_model,
};
use gbf_train::adapter::burn::{BurnDevice, BurnNdArrayAutodiffBackend};
use gbf_train::qat::{ExpertBlockBurnQat, Top1RouterBurnQat};
use gbf_verify::ternary::{
    pack_reference_scale_values, pack_reference_ternary, quantize_reference_scales,
    unpack_reference_scale_values, unpack_reference_ternary_values,
};

#[test]
fn export_determinism_tiny_model_reexports_identical_core_and_bytes() {
    let first = make_tiny_exported_artifact();
    let second = make_tiny_exported_artifact();

    assert_artifact_valid(&first.core);
    assert_artifact_valid(&second.core);
    assert_eq!(first.artifact_core_hash(), second.artifact_core_hash());
    assert_exported_artifact_bytes_equal(&first, &second);
}

#[test]
fn export_determinism_supported_burn_router_and_expert_handoff_matches_scalar_tiny_export() {
    let scalar_export = make_tiny_exported_artifact();
    let burn_export = export_tiny_model_through_supported_burn_handoff();

    assert_artifact_valid(&burn_export.core);
    assert_eq!(
        scalar_export.artifact_core_hash(),
        burn_export.artifact_core_hash(),
        "Burn handoff must preserve the scalar export ArtifactCore hash"
    );
    assert_exported_artifact_bytes_equal(&scalar_export, &burn_export);
}

#[test]
fn export_determinism_exported_qat_artifact_json_round_trip_is_byte_identical() {
    let artifact = make_tiny_exported_artifact();
    let encoded = serde_json::to_vec(&artifact).expect("exported artifact serializes");
    let decoded: ExportedQatArtifact =
        serde_json::from_slice(&encoded).expect("exported artifact deserializes");
    let reencoded = serde_json::to_vec(&decoded).expect("decoded artifact reserializes");

    assert_eq!(artifact.artifact_core_hash(), decoded.artifact_core_hash());
    assert_bytes_equal(&reencoded, &encoded);
}

#[test]
fn export_determinism_canonical_tensors_round_trip_and_hashes_are_stable() {
    let artifact = make_tiny_exported_artifact();

    for tensor in artifact.core.tensors() {
        let encoded = serde_json::to_vec(tensor)
            .unwrap_or_else(|error| panic!("tensor {} failed to serialize: {error}", tensor.id));
        let decoded: CanonicalTensor = serde_json::from_slice(&encoded)
            .unwrap_or_else(|error| panic!("tensor {} failed to deserialize: {error}", tensor.id));
        let reencoded = serde_json::to_vec(&decoded)
            .unwrap_or_else(|error| panic!("tensor {} failed to reserialize: {error}", tensor.id));
        let reconstructed = CanonicalTensor::new(
            decoded.id.clone(),
            decoded.kind,
            decoded.layout.clone(),
            decoded.payload.clone(),
        )
        .unwrap_or_else(|error| panic!("tensor {} failed reconstruction: {error}", tensor.id));

        assert_bytes_equal(&reencoded, &encoded);
        assert_eq!(
            decoded.content_hash, reconstructed.content_hash,
            "tensor {} has unstable content hash after serde round-trip",
            tensor.id
        );
    }
}

#[test]
fn export_determinism_canonical_ternary_tensors_are_reference_packable() {
    let model = make_tiny_model();
    let artifact = make_tiny_exported_artifact();
    let expected_scale_raw_by_tensor = expected_tiny_expert_scale_tensors(&model);

    for entry in artifact.core.quant().ternary_weight_plans() {
        assert_ternary_entry_repackable(&artifact.core, entry);

        let scale = tensor_by_id(&artifact.core, entry.scale.as_str());
        let actual_scale_values = scale
            .payload
            .as_u16_slice()
            .expect("ternary scale tensor must use u16 Q8_8 payload");
        let expected = expected_scale_raw_by_tensor
            .get(entry.scale.as_str())
            .unwrap_or_else(|| panic!("missing source scale oracle for {}", entry.scale));
        assert_eq!(
            actual_scale_values, expected,
            "scale tensor {} must match round_to_nearest(scale * 256) Q8_8 source values",
            entry.scale
        );
        let weight = tensor_by_id(&artifact.core, entry.weight.as_str());
        let dims = weight.layout.shape.dims();
        assert_bytes_equal(
            &pack_reference_scale_values(entry.plan, dims[0], dims[1], actual_scale_values)
                .expect("Q8_8 scale tensor should pack deterministically"),
            &scale_values_to_le_bytes(actual_scale_values),
        );
    }
}

fn export_tiny_model_through_supported_burn_handoff() -> ExportedQatArtifact {
    type B = BurnNdArrayAutodiffBackend;

    let model = make_tiny_model();
    let device = BurnDevice::<B>::default();
    let router = Top1RouterBurnQat::<B>::from_core(model.router().clone(), &device)
        .expect("tiny router converts to Burn")
        .to_core_from_trained_state()
        .expect("tiny Burn router exports back to scalar core");
    let expert_block = ExpertBlockBurnQat::<B>::from_core(model.expert_block().clone(), &device)
        .expect("tiny expert block converts to Burn")
        .to_core_from_trained_state()
        .expect("tiny Burn expert block exports back to scalar core");

    export_tiny_model_with_router_and_expert_block(&model, &router, &expert_block)
}

fn assert_exported_artifact_bytes_equal(
    expected: &ExportedQatArtifact,
    actual: &ExportedQatArtifact,
) {
    let expected_bytes =
        serde_json::to_vec(expected).expect("expected exported artifact serializes");
    let actual_bytes = serde_json::to_vec(actual).expect("actual exported artifact serializes");

    assert_bytes_equal(&actual_bytes, &expected_bytes);
}

fn assert_ternary_entry_repackable(artifact: &ArtifactCore, entry: &TernaryQuantEntry) {
    let weight = tensor_by_id(artifact, entry.weight.as_str());
    let scale = tensor_by_id(artifact, entry.scale.as_str());
    assert_eq!(weight.kind, CanonicalTensorKind::TernaryWeight);
    assert_eq!(scale.kind, CanonicalTensorKind::TernaryScale);

    let dims = weight.layout.shape.dims();
    assert_eq!(
        dims.len(),
        2,
        "ternary weight {} must be a matrix, got {dims:?}",
        weight.id
    );
    let rows = dims[0];
    let cols = dims[1];
    let ternary_values = weight
        .payload
        .as_i8_slice()
        .expect("ternary weight tensor must use i8 payload");
    let scale_values = scale
        .payload
        .as_u16_slice()
        .expect("ternary scale tensor must use u16 payload");
    let packet = pack_reference_ternary(entry.plan, rows, cols, ternary_values, scale_values)
        .unwrap_or_else(|error| {
            panic!(
                "reference pack failed for projection={} rows={rows} cols={cols}: {error}",
                entry.projection
            )
        });

    assert_eq!(
        packet.total_byte_len(),
        entry.plan.compute_byte_cost(rows, cols).as_u64() as usize,
        "packed byte count mismatch for projection={}",
        entry.projection
    );
    assert_eq!(
        unpack_reference_ternary_values(entry.plan, rows, cols, packet.weight_bytes()).unwrap(),
        ternary_values,
        "reference unpack must recover exported ternary values for {}",
        entry.weight
    );
    assert_eq!(
        unpack_reference_scale_values(entry.plan, rows, cols, packet.scale_bytes()).unwrap(),
        scale_values,
        "reference unpack must recover exported scale values for {}",
        entry.scale
    );
}

fn expected_tiny_expert_scale_tensors(
    model: &gbf_test::fixtures::TinyModel,
) -> BTreeMap<String, Vec<u16>> {
    let mut expected = BTreeMap::new();
    for (expert_index, expert) in model.expert_block().experts().iter().enumerate() {
        expected.insert(
            format!("block.1.expert_block.expert.{expert_index}.up.scale"),
            scale_raw_values(expert.up_projection()),
        );
        expected.insert(
            format!("block.1.expert_block.expert.{expert_index}.down.scale"),
            scale_raw_values(expert.down_projection()),
        );
    }
    expected
}

fn scale_raw_values(layer: &TernaryLinearQat) -> Vec<u16> {
    let shape = layer.shape();
    let cols = shape.input_cols();
    let float_scales = layer
        .full_precision_weights()
        .chunks_exact(cols)
        .zip(layer.thresholds().iter())
        .map(|(row, threshold)| {
            let mut active_count = 0usize;
            let active_abs_sum = row
                .iter()
                .copied()
                .filter(|weight| {
                    let threshold = threshold.value();
                    *weight > threshold || *weight < -threshold
                })
                .inspect(|_| active_count += 1)
                .map(f32::abs)
                .sum::<f32>();

            if active_count == 0 {
                0.0
            } else {
                active_abs_sum / active_count as f32
            }
        })
        .collect::<Vec<_>>();

    quantize_reference_scales(
        layer.plan(),
        shape.output_rows() as u32,
        shape.input_cols() as u32,
        &float_scales,
    )
    .expect("independent Q8_8 scale formula must support tiny expert plan")
}

fn scale_values_to_le_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn tensor_by_id<'a>(artifact: &'a ArtifactCore, id: &str) -> &'a CanonicalTensor {
    artifact
        .tensors()
        .iter()
        .find(|tensor| tensor.id.as_str() == id)
        .unwrap_or_else(|| panic!("missing tensor {id}"))
}
