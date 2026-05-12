mod common;

use std::fs;

use common::assertions::assert_canonical_json_byte_eq;
use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType,
};
use gbf_experiments::s2::ablation::{AblationInputs, verify_h4, write_ablation_report};
use gbf_foundation::Hash256;
use proptest::prelude::*;
use serde_json::{Value, json};

#[test]
fn h4_byte_equal_checkpoints_match_and_exclude_qat_buffers() {
    let trainable = tensor_f32("toy0.ffn.weight", &[1.0, 2.0, 3.0, 4.0]);
    let ternary = vec![
        threshold_tensor("toy0.ffn.threshold.0", &[128, 129]),
        trainable.clone(),
    ];
    let ablation = vec![trainable];
    let capture = TraceCapture::default();

    let first = with_trace_capture(&capture, || verify_h4(inputs(&ternary, &ablation)).unwrap());
    let second = verify_h4(inputs(&ternary, &ablation)).unwrap();

    assert!(first.phase_a_eq_ablation);
    assert!(first.first_mismatch.is_none());
    assert_canonical_json_byte_eq(
        &first.canonical_json_bytes().unwrap(),
        &second.canonical_json_bytes().unwrap(),
    );
    let output_dir = tempfile::tempdir().unwrap();
    let output_path = output_dir.path().join("s2_ablation.v1.json");
    write_ablation_report(&output_path, &first).unwrap();
    assert_canonical_json_byte_eq(
        &fs::read(output_path).unwrap(),
        &gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&first).unwrap(),
    );
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h4_payload_extract"
            && event.fields.get("build").and_then(Value::as_str) == Some("s2_ternary_full")
            && event
                .fields
                .get("excluded_qat_buffers")
                .and_then(Value::as_u64)
                == Some(1)
    }));

    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("h4_payload_match__tiny_fixture", pretty_json(&json!({
            "phase_a_eq_ablation": first.phase_a_eq_ablation,
            "first_mismatch": first.first_mismatch,
            "ternary_payload_sha": first.s2_ternary_tensor_payload_sha,
            "ablation_payload_sha": first.s2_ablation_tensor_payload_sha,
        })));
    });
}

#[test]
fn h4_known_tensor_difference_records_first_mismatch() {
    let ternary = vec![tensor_f32("toy0.ffn.weight", &[1.0, 2.0])];
    let ablation = vec![tensor_f32("toy0.ffn.weight", &[1.0, -2.0])];
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || verify_h4(inputs(&ternary, &ablation)).unwrap());

    assert!(!report.phase_a_eq_ablation);
    let mismatch = report.first_mismatch.expect("mismatch is recorded");
    assert_eq!(mismatch.tensor, "toy0.ffn.weight");
    assert_eq!(mismatch.byte_offset, 7);
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h4_first_mismatch"
            && event.fields.get("tensor").and_then(Value::as_str) == Some("toy0.ffn.weight")
    }));
}

#[test]
fn h4_rejects_non_seed_zero_inputs() {
    let ternary = vec![tensor_f32("toy0.ffn.weight", &[1.0])];
    let ablation = ternary.clone();
    let error = verify_h4(AblationInputs {
        seed: 1,
        s2_ternary_phase_a_checkpoint_sha: Hash256::from_bytes([0x11; 32]),
        s2_ablation_phase_a_checkpoint_sha: Hash256::from_bytes([0x22; 32]),
        s2_ternary_tensors: &ternary,
        s2_ablation_tensors: &ablation,
    })
    .unwrap_err();

    assert!(error.to_string().contains("requires seed 0"));
}

proptest! {
    #[test]
    fn h4_payload_equality_is_reflexive_and_symmetric(values in prop::collection::vec(-16_i16..16, 1..8)) {
        let values = values.into_iter().map(f32::from).collect::<Vec<_>>();
        let left = vec![tensor_f32("toy0.prop.weight", &values)];
        let right = vec![tensor_f32("toy0.prop.weight", &values)];

        let forward = verify_h4(inputs(&left, &right)).unwrap();
        let reverse = verify_h4(inputs(&right, &left)).unwrap();

        prop_assert!(forward.phase_a_eq_ablation);
        prop_assert!(reverse.phase_a_eq_ablation);
        prop_assert_eq!(forward.s2_ternary_tensor_payload_sha, reverse.s2_ablation_tensor_payload_sha);
        prop_assert_eq!(forward.s2_ablation_tensor_payload_sha, reverse.s2_ternary_tensor_payload_sha);
    }
}

fn inputs<'a>(
    ternary: &'a [CanonicalTensor],
    ablation: &'a [CanonicalTensor],
) -> AblationInputs<'a> {
    AblationInputs {
        seed: 0,
        s2_ternary_phase_a_checkpoint_sha: Hash256::from_bytes([0x11; 32]),
        s2_ablation_phase_a_checkpoint_sha: Hash256::from_bytes([0x22; 32]),
        s2_ternary_tensors: ternary,
        s2_ablation_tensors: ablation,
    }
}

fn tensor_f32(name: &str, values: &[f32]) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).unwrap(),
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[values.len()]).unwrap(),
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(values.to_vec()),
    )
    .unwrap()
}

fn threshold_tensor(name: &str, values: &[u16]) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).unwrap(),
        CanonicalTensorKind::TernaryScale,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[values.len()]).unwrap(),
            TensorElementType::Q8_8,
        ),
        CanonicalTensorPayload::U16(values.to_vec()),
    )
    .unwrap()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot value serializes")
}
