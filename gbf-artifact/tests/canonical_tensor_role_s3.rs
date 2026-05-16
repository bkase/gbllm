use gbf_artifact::{Dtype, PayloadRole, byte_length, canonical_payload_sha};
use proptest::prelude::*;

proptest! {
    #[test]
    fn canonical_tensor_payload_role_round_trips(role in payload_role_strategy()) {
        let encoded = serde_json::to_string(&role).expect("payload role serializes");
        let decoded: PayloadRole = serde_json::from_str(&encoded).expect("payload role decodes");
        prop_assert_eq!(decoded, role);
    }
}

#[test]
fn canonical_tensor_byte_length_is_exact_for_known_shapes() {
    assert_eq!(byte_length(Dtype::Fp32, &[2, 3]).unwrap(), 24);
    assert_eq!(byte_length(Dtype::Q8_8, &[2, 3]).unwrap(), 12);
    assert_eq!(byte_length(Dtype::I32, &[2, 3]).unwrap(), 24);
    assert_eq!(byte_length(Dtype::Ternary2, &[4]).unwrap(), 1);
    assert_eq!(byte_length(Dtype::Ternary2, &[5]).unwrap(), 2);
}

#[test]
fn canonical_tensor_payload_sha_uses_sha256_wrapper() {
    assert_eq!(
        canonical_payload_sha(b"abc").to_string(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn payload_role_strategy() -> impl Strategy<Value = PayloadRole> {
    prop_oneof![
        Just(PayloadRole::DeployableWeight),
        Just(PayloadRole::DeployableQuantParam),
        Just(PayloadRole::ReferenceFp32),
    ]
}
