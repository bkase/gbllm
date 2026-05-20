use gbf_artifact::{
    FrontierRecommendation, S5FrontierPointMetrics, S5FrontierRecommendationReport,
    S5FrontierVariantId, S5FrontierVariantRecord,
};

fn point(val_bpc_ternary: f64, encoded_rom_byte_cost: Option<u64>) -> S5FrontierPointMetrics {
    S5FrontierPointMetrics {
        val_bpc_fp: 1.5,
        val_bpc_ternary,
        ternary_gap: 0.2,
        v0_success_pass: true,
        v0_success_score: 0.8,
        param_count: 10_000,
        projected_deployed_bytes: 20_000,
        shadow_compile_ok_at_end: true,
        shadow_byte_cost_at_end: 18_000,
        shadow_kernel_count_at_end: 12,
        latency_proxy_cycles: 50_000,
        encoded_rom_byte_cost,
        fits_envelope: Some(true),
        reachability_cert_valid: Some(true),
        resource_state_cert_valid: Some(true),
    }
}

#[test]
fn s5_frontier_recommendation_report_round_trips_leader_variant() {
    let report = S5FrontierRecommendationReport::from_variant_records(vec![
        S5FrontierVariantRecord {
            variant: S5FrontierVariantId::BoundedKv,
            aggregate: point(1.30, Some(21_000)),
        },
        S5FrontierVariantRecord {
            variant: S5FrontierVariantId::LFix1,
            aggregate: point(1.20, Some(22_000)),
        },
        S5FrontierVariantRecord {
            variant: S5FrontierVariantId::LMt4,
            aggregate: point(1.24, Some(20_000)),
        },
    ])
    .unwrap();

    assert_eq!(report.frontier_recommendation, FrontierRecommendation::B);
    assert_eq!(
        report.frontier_leader_variant,
        Some(S5FrontierVariantId::LFix1)
    );

    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["frontier_leader_variant"],
        serde_json::json!("linearstate_fixed_0_5")
    );
    let decoded: S5FrontierRecommendationReport = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, report);
}
