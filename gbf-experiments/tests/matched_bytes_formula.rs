#![cfg(feature = "s7")]

use std::fs;
use std::path::PathBuf;

use gbf_artifact::MatchedBytesPin;
use gbf_experiments::s7::baseline_match::{
    canonical_s7_matched_bytes_json_bytes, canonical_s7_matched_bytes_pin,
};
use gbf_foundation::ByteCost;
use gbf_policy::{
    DenseMatchedBytesPolicy, MatchedBytesConfig, compute_dense_ffn_total,
    compute_moe_experts_total, compute_router_overhead_total, d6_tolerance_bytes,
};

#[test]
fn o11_matched_bytes_formula_matches_preregistered_pin() {
    let pin_path = workspace_root().join("experiments/S7/profile/matched_bytes.json");
    let pinned_bytes = fs::read(&pin_path).expect("matched_bytes.json must be committed");
    assert!(
        !pinned_bytes
            .windows(b"<PIN_ME>".len())
            .any(|window| window == b"<PIN_ME>"),
        "matched_bytes.json must pin a concrete bias_policy, not <PIN_ME>"
    );

    let pinned: MatchedBytesPin =
        serde_json::from_slice(&pinned_bytes).expect("matched_bytes.json parses");
    pinned
        .verify_self_hash()
        .expect("matched_bytes.json self-hash verifies");

    let expected_pin = canonical_s7_matched_bytes_pin().expect("canonical pin computes");
    let expected_bytes = canonical_s7_matched_bytes_json_bytes().expect("canonical pin serializes");
    let config = MatchedBytesConfig::s7_moe_tiny();
    let policy = DenseMatchedBytesPolicy::s7_canonical().matched_bytes_policy();
    let common_deployed_bytes = config.common_deployed_bytes.as_u64();
    let b_experts_total = compute_moe_experts_total(config, policy).as_u64();
    let b_router_overhead_total = compute_router_overhead_total(config, policy).as_u64();
    let b_dense_ffn_total =
        compute_dense_ffn_total(config, pinned.d_ff_dense_resolved, policy).as_u64();

    assert_eq!(pinned, expected_pin);
    assert_eq!(pinned_bytes, expected_bytes);
    assert_eq!(pinned.d_ff_dense_resolved, 572);
    assert_eq!(pinned.formula_version, policy.formula_version);
    assert_eq!(pinned.b_experts_total, b_experts_total);
    assert_eq!(pinned.b_router_overhead_total, b_router_overhead_total);
    assert_eq!(pinned.b_dense_ffn_total, b_dense_ffn_total);
    assert_eq!(
        pinned.b_deployed_total_moe,
        b_experts_total + b_router_overhead_total + common_deployed_bytes
    );
    assert_eq!(
        pinned.b_deployed_total_dense,
        b_dense_ffn_total + common_deployed_bytes
    );
    assert_eq!(
        pinned.tolerance_bytes,
        d6_tolerance_bytes(ByteCost::new(pinned.b_deployed_total_moe), policy).as_u64()
    );
    assert!(
        pinned
            .b_deployed_total_moe
            .abs_diff(pinned.b_deployed_total_dense)
            <= pinned.tolerance_bytes
    );
}

#[test]
fn matched_bytes_pin_emitter_is_byte_deterministic() {
    let first = canonical_s7_matched_bytes_json_bytes().expect("canonical pin serializes");
    for _ in 0..100 {
        assert_eq!(
            canonical_s7_matched_bytes_json_bytes().expect("canonical pin serializes"),
            first
        );
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
