use gbf_foundation::{ByteCost, SemVer};
use gbf_policy::{
    BiasPolicy, DenseMatchedBytesPolicy, LinearShape, MATCHED_BYTES_FORMULA_VERSION,
    MatchedBytesConfig, MatchedBytesError, MatchedBytesPolicy, S7_CANONICAL_BIAS_POLICY,
    S7_ONE_BANK_BYTES, S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM, S7_TERNARY_METADATA_BYTES,
    bias_byte_cost, compute_dense_ffn_total, compute_linear_deployed_byte_cost,
    compute_weight_byte_cost, d6_tolerance_bytes, solve_d_ff_dense,
};
use serde_json::json;

#[test]
fn compute_weight_byte_cost_uses_exact_ternary_packing_ceiling() {
    let policy = MatchedBytesPolicy::s7_canonical();

    assert_eq!(
        compute_weight_byte_cost(LinearShape::new(1, 1), policy),
        ByteCost::new(1 + 2 + S7_TERNARY_METADATA_BYTES.as_u64())
    );
    assert_eq!(
        compute_weight_byte_cost(LinearShape::new(1, 3), policy),
        ByteCost::new(1 + 2 + S7_TERNARY_METADATA_BYTES.as_u64())
    );
    assert_eq!(
        compute_weight_byte_cost(LinearShape::new(1, 4), policy),
        ByteCost::new(1 + 2 + S7_TERNARY_METADATA_BYTES.as_u64())
    );
    assert_eq!(
        compute_weight_byte_cost(LinearShape::new(1, 5), policy),
        ByteCost::new(2 + 2 + S7_TERNARY_METADATA_BYTES.as_u64())
    );
}

#[test]
fn compute_weight_byte_cost_handles_large_shapes_and_saturation() {
    let policy = MatchedBytesPolicy::s7_canonical();
    let linear = LinearShape::new(u32::MAX, u32::MAX);
    let weights = u128::from(u32::MAX) * u128::from(u32::MAX);
    let expected = weights.div_ceil(4) + u128::from(u32::MAX) * 2 + 50;

    assert_eq!(
        compute_weight_byte_cost(linear, policy),
        ByteCost::new(u64::try_from(expected).expect("u32::MAX square packed bytes fit u64"))
    );

    let saturating_policy =
        MatchedBytesPolicy::new(ByteCost::new(u64::MAX), BiasPolicy::NotDeployed);
    assert_eq!(
        compute_weight_byte_cost(linear, saturating_policy),
        ByteCost::new(u64::MAX)
    );
}

#[test]
fn bias_byte_cost_respects_all_d6_bias_policies() {
    let linear = LinearShape::new(7, 3);

    assert_eq!(
        bias_byte_cost(linear, BiasPolicy::NotDeployed),
        ByteCost::ZERO
    );
    assert_eq!(bias_byte_cost(linear, BiasPolicy::Folded), ByteCost::ZERO);
    assert_eq!(
        bias_byte_cost(linear, BiasPolicy::Q8_8PerOutput),
        ByteCost::new(14)
    );
    assert_eq!(
        bias_byte_cost(linear, BiasPolicy::Fp16PerOutput),
        ByteCost::new(14)
    );
}

#[test]
fn bias_byte_cost_handles_zero_and_large_rows() {
    assert_eq!(
        bias_byte_cost(LinearShape::new(0, u32::MAX), BiasPolicy::Q8_8PerOutput),
        ByteCost::ZERO
    );
    assert_eq!(
        bias_byte_cost(
            LinearShape::new(u32::MAX, u32::MAX),
            BiasPolicy::Fp16PerOutput
        ),
        ByteCost::new(u64::from(u32::MAX) * 2)
    );
}

#[test]
fn compute_linear_deployed_byte_cost_adds_pinned_bias_policy() {
    let linear = LinearShape::new(1, 1);
    let policy = MatchedBytesPolicy::new(S7_TERNARY_METADATA_BYTES, BiasPolicy::Q8_8PerOutput);

    assert_eq!(
        compute_linear_deployed_byte_cost(linear, policy),
        ByteCost::new(1 + 2 + S7_TERNARY_METADATA_BYTES.as_u64() + 2)
    );
}

#[test]
fn dense_matched_policy_rejects_unknown_bias_policy_strings() {
    let error = DenseMatchedBytesPolicy::new("int4_per_output").expect_err("unknown policy");

    assert!(error.to_string().contains("int4_per_output"));
    assert!(DenseMatchedBytesPolicy::new("q8_8_per_output").is_ok());
}

#[test]
fn dense_matched_policy_round_trips_canonical_bias_pin_json() {
    let policy = DenseMatchedBytesPolicy::s7_canonical();
    let matched_policy = policy.matched_bytes_policy();

    assert_eq!(
        matched_policy.formula_version,
        MATCHED_BYTES_FORMULA_VERSION
    );
    assert_eq!(matched_policy.formula_version, SemVer::new(0, 2, 0));
    assert_eq!(matched_policy.bias_policy, S7_CANONICAL_BIAS_POLICY);
    assert_eq!(matched_policy.bias_policy, BiasPolicy::Q8_8PerOutput);
    assert_eq!(matched_policy.one_bank_bytes, S7_ONE_BANK_BYTES);
    assert_eq!(
        matched_policy.router_parameter_bytes,
        S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM
    );

    let value = serde_json::to_value(policy).expect("policy serializes");
    assert_eq!(
        value,
        json!({
            "formula_version": {"major": 0, "minor": 2, "patch": 0},
            "ternary_metadata_bytes": S7_TERNARY_METADATA_BYTES.as_u64(),
            "bias_policy": "q8_8_per_output",
            "one_bank_bytes": S7_ONE_BANK_BYTES.as_u64(),
            "router_parameter_bytes": S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM,
        })
    );
    let decoded: DenseMatchedBytesPolicy =
        serde_json::from_value(value).expect("policy deserializes");

    assert_eq!(decoded, policy);
}

#[test]
fn solve_d_ff_dense_returns_canonical_s7_moe_tiny_pin_values() {
    let solution = solve_d_ff_dense(
        MatchedBytesConfig::s7_moe_tiny(),
        MatchedBytesPolicy::s7_canonical(),
    )
    .expect("canonical S7 MoeTiny is solvable");

    assert_eq!(solution.d_ff_dense, 572);
    assert_eq!(solution.b_experts_total, ByteCost::new(79_424));
    assert_eq!(solution.b_router_overhead_total, ByteCost::new(4_352));
    assert_eq!(solution.b_dense_ffn_total, ByteCost::new(83_792));
    assert_eq!(solution.b_deployed_total_moe, ByteCost::new(83_776));
    assert_eq!(solution.b_deployed_total_dense, ByteCost::new(83_792));
    assert_eq!(solution.tolerance_bytes, ByteCost::new(65_536));
    assert_eq!(solution.deployed_bytes_diff(), -16);
}

#[test]
fn solve_d_ff_dense_tie_break_prefers_dense_not_smaller_baseline() {
    let policy = MatchedBytesPolicy::from_parts(
        MATCHED_BYTES_FORMULA_VERSION,
        ByteCost::ZERO,
        BiasPolicy::Q8_8PerOutput,
        ByteCost::new(1),
        1,
    );
    let config = MatchedBytesConfig {
        d_model: 1,
        d_ff_moe: 1,
        n_blocks: 1,
        n_experts: 1,
        router_rank: 1,
        d_ff_dense_min: 1,
        d_ff_dense_max: 2,
        common_deployed_bytes: ByteCost::ZERO,
    };

    let dense_below = compute_dense_ffn_total(config, 1, policy);
    let dense_above = compute_dense_ffn_total(config, 2, policy);
    let solution = solve_d_ff_dense(config, policy).expect("crafted tie is solvable");

    assert_eq!(solution.b_deployed_total_moe, ByteCost::new(12));
    assert_eq!(dense_below, ByteCost::new(10));
    assert_eq!(dense_above, ByteCost::new(14));
    assert_eq!(
        solution.b_deployed_total_moe.as_u64() - dense_below.as_u64(),
        dense_above.as_u64() - solution.b_deployed_total_moe.as_u64()
    );
    assert_eq!(solution.d_ff_dense, 2);
    assert_eq!(solution.b_deployed_total_dense, dense_above);
}

#[test]
fn solve_d_ff_dense_tolerance_uses_deployed_moe_total() {
    let policy = MatchedBytesPolicy::from_parts(
        MATCHED_BYTES_FORMULA_VERSION,
        ByteCost::ZERO,
        BiasPolicy::Q8_8PerOutput,
        ByteCost::new(1),
        20,
    );
    let config = MatchedBytesConfig {
        d_model: 1,
        d_ff_moe: 1,
        n_blocks: 1,
        n_experts: 2,
        router_rank: 10,
        d_ff_dense_min: 1,
        d_ff_dense_max: 128,
        common_deployed_bytes: ByteCost::ZERO,
    };

    let solution = solve_d_ff_dense(config, policy)
        .expect("router-heavy deployed total makes the dense candidate admissible");

    assert_eq!(solution.b_experts_total, ByteCost::new(20));
    assert_eq!(solution.b_router_overhead_total, ByteCost::new(600));
    assert_eq!(solution.b_deployed_total_moe, ByteCost::new(620));
    assert_eq!(
        d6_tolerance_bytes(solution.b_experts_total, policy),
        ByteCost::new(4)
    );
    assert_eq!(
        d6_tolerance_bytes(solution.b_deployed_total_moe, policy),
        ByteCost::new(62)
    );
    assert_eq!(solution.tolerance_bytes, ByteCost::new(62));
    assert_eq!(solution.d_ff_dense, 128);
    assert_eq!(
        solution
            .b_deployed_total_moe
            .as_u64()
            .abs_diff(solution.b_deployed_total_dense.as_u64()),
        40
    );
}

#[test]
fn solve_d_ff_dense_final_tie_break_prefers_smaller_width_on_same_side() {
    let policy = MatchedBytesPolicy::from_parts(
        MATCHED_BYTES_FORMULA_VERSION,
        ByteCost::ZERO,
        BiasPolicy::Q8_8PerOutput,
        ByteCost::new(1),
        1,
    );
    let config = MatchedBytesConfig {
        d_model: 1,
        d_ff_moe: 1,
        n_blocks: 1,
        n_experts: 1,
        router_rank: 1,
        d_ff_dense_min: 2,
        d_ff_dense_max: 3,
        common_deployed_bytes: ByteCost::new(u64::MAX - 13),
    };

    let solution = solve_d_ff_dense(config, policy).expect("saturating tie is solvable");
    let dense_2 = config.common_deployed_bytes + compute_dense_ffn_total(config, 2, policy);
    let dense_3 = config.common_deployed_bytes + compute_dense_ffn_total(config, 3, policy);

    assert_eq!(solution.b_deployed_total_moe, ByteCost::new(u64::MAX - 1));
    assert_eq!(dense_2, ByteCost::new(u64::MAX));
    assert_eq!(dense_3, ByteCost::new(u64::MAX));
    assert_eq!(
        dense_2.as_u64() - solution.b_deployed_total_moe.as_u64(),
        dense_3.as_u64() - solution.b_deployed_total_moe.as_u64()
    );
    assert_eq!(solution.d_ff_dense, 2);
    assert_eq!(solution.b_deployed_total_dense, dense_2);
}

#[test]
fn solve_d_ff_dense_rejects_unsolved_dense_ranges() {
    let config = MatchedBytesConfig {
        d_ff_dense_min: 64,
        d_ff_dense_max: 64,
        ..MatchedBytesConfig::s7_moe_tiny()
    };

    let error = solve_d_ff_dense(config, MatchedBytesPolicy::s7_canonical())
        .expect_err("single tiny dense candidate is outside D6 tolerance");

    assert!(matches!(
        error,
        MatchedBytesError::MatchedBytesInfeasible {
            min_d_ff_dense: 64,
            max_d_ff_dense: 64,
            ..
        }
    ));
}
