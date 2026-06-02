use gbf_foundation::{BlobCodec, BlobRef, Hash256};
use gbf_policy::compile::{
    CanonicalProjectionTensor, CanonicalProjectionTensorSet, ProjectionTensorEncoding,
    ProjectionTensorName, ProjectionTensorSource, S5_ATTENTION_ORACLE_INTEGRATION_OWNER,
    S5AttentionOracleInputs, S5AttentionOracleQuantSpecBinding,
};

#[test]
fn attention_oracle_contract_consumes_serialized_inputs_not_live_handles() {
    let inputs = S5AttentionOracleInputs::new(
        blob(0x11),
        CanonicalProjectionTensorSet::from_checkpoint_artifact(
            ProjectionTensorSource::PhaseAFpProjectionTensors,
            vec![
                tensor(ProjectionTensorName::Query, 0x21),
                tensor(ProjectionTensorName::Key, 0x22),
                tensor(ProjectionTensorName::Value, 0x23),
                tensor(ProjectionTensorName::Output, 0x24),
            ],
        ),
        quant_spec(ProjectionTensorSource::PhaseAFpProjectionTensors, 0x12),
        6.5,
        vec![1, 2, 3],
    );

    let value = serde_json::to_value(&inputs).expect("oracle inputs serialize");
    assert_eq!(value["schema"], "s5_attention_oracle_spec.v1");
    assert_eq!(value["checkpoint_artifact"]["hash"], hash(0x11).to_string());
    assert_eq!(value["checkpoint_artifact"]["len"], 4096);
    assert_eq!(value["checkpoint_artifact"]["codec"], "raw");
    assert_eq!(
        value["projection_tensors"]["source"],
        serde_json::json!({"kind": "PhaseAFpProjectionTensors"})
    );
    assert_eq!(
        value["quant_spec"]["source"],
        serde_json::json!({"kind": "PhaseAFpProjectionTensors"})
    );
    assert_eq!(
        value["projection_tensors"]["tensors"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(value["activation_fake_quant_clip"], 6.5);
    assert_eq!(value["fixture_token_ids"], serde_json::json!([1, 2, 3]));

    let keys = value
        .as_object()
        .expect("top-level object")
        .keys()
        .collect::<Vec<_>>();
    for forbidden in [
        "burn_module",
        "burn_module_handle",
        "live_trainer_state",
        "host_runtime_state",
        "rng",
        "random_stream",
    ] {
        assert!(
            !keys.iter().any(|key| key.as_str() == forbidden),
            "oracle input contract must not expose live dependency field {forbidden}"
        );
    }

    let prohibited = S5AttentionOracleInputs::prohibited_dependencies();
    assert!(prohibited.contains(&"Burn module handle"));
    assert!(prohibited.contains(&"live trainer state"));
    assert!(prohibited.contains(&"host runtime state"));
    assert!(prohibited.contains(&"random number stream"));
    assert!(
        S5AttentionOracleInputs::required_inputs()
            .contains(&"canonical serialized projection tensors")
    );
    assert!(S5_ATTENTION_ORACLE_INTEGRATION_OWNER.contains("bd-1gmy"));
}

#[test]
fn attention_oracle_contract_allows_quant_spec_weight_quant_source() {
    let inputs = S5AttentionOracleInputs::new(
        blob(0x31),
        CanonicalProjectionTensorSet::from_checkpoint_artifact(
            ProjectionTensorSource::QuantSpecWeightQuant,
            vec![CanonicalProjectionTensor {
                name: ProjectionTensorName::Output,
                rows: 8,
                cols: 8,
                encoding: ProjectionTensorEncoding::QuantizedWeightBytes,
                canonical_bytes_sha256: hash(0x41),
            }],
        ),
        quant_spec(ProjectionTensorSource::QuantSpecWeightQuant, 0x32),
        4.0,
        vec![7],
    );

    let value = serde_json::to_value(inputs).expect("oracle inputs serialize");
    assert_eq!(
        value["projection_tensors"]["source"],
        serde_json::json!({"kind": "QuantSpecWeightQuant"})
    );
    assert_eq!(
        value["projection_tensors"]["tensors"][0]["encoding"],
        serde_json::json!({"kind": "QuantizedWeightBytes"})
    );
}

fn tensor(name: ProjectionTensorName, hash_byte: u8) -> CanonicalProjectionTensor {
    CanonicalProjectionTensor {
        name,
        rows: 8,
        cols: 8,
        encoding: ProjectionTensorEncoding::F64LeMatrix,
        canonical_bytes_sha256: hash(hash_byte),
    }
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn quant_spec(source: ProjectionTensorSource, hash_byte: u8) -> S5AttentionOracleQuantSpecBinding {
    S5AttentionOracleQuantSpecBinding {
        source,
        canonical_bytes_sha256: hash(hash_byte),
    }
}

fn blob(byte: u8) -> BlobRef {
    BlobRef {
        hash: hash(byte),
        len: 4096,
        codec: BlobCodec::Raw,
    }
}
