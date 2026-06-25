use gbf_foundation::WorkloadId;
use gbf_policy::{CostEstimate, EstimatedCostDelta, EvidenceClass, UncertaintyEnvelope};
use gbf_train::shadow::{
    QualitySummary, SequenceStateComparisonReport, SequenceStateVariantId, SequenceVariantSummary,
};
use serde_json::json;

#[test]
fn sequence_state_comparison_report_json_shape_is_stable() {
    let report = SequenceStateComparisonReport::new(
        WorkloadId::from("tiny-smoke"),
        SequenceVariantSummary::new(
            SequenceStateVariantId::LinearState,
            QualitySummary::new(1.0, 2.5).unwrap(),
            None,
            0.0,
            8,
            true,
        )
        .unwrap(),
        SequenceVariantSummary::new(
            SequenceStateVariantId::BoundedKv,
            QualitySummary::new(0.875, 2.25).unwrap(),
            Some(cost_delta(3)),
            3.0,
            24,
            true,
        )
        .unwrap(),
        "2026-06-07T00:00:00Z",
    )
    .unwrap();

    let value = serde_json::to_value(&report).unwrap();

    assert_eq!(
        value,
        json!({
            "workload": "tiny-smoke",
            "linear_state": {
                "variant": "linear_state",
                "quality": {
                    "lm_loss": 1.0,
                    "perplexity": 2.5
                },
                "schedule_cost": null,
                "projected_bank_switches_per_token": 0.0,
                "projected_state_bytes": 8,
                "fits_envelope": true
            },
            "bounded_kv": {
                "variant": "bounded_kv",
                "quality": {
                    "lm_loss": 0.875,
                    "perplexity": 2.25
                },
                "schedule_cost": cost_delta_json(3),
                "projected_bank_switches_per_token": 3.0,
                "projected_state_bytes": 24,
                "fits_envelope": true
            },
            "generated_at": "2026-06-07T00:00:00Z"
        })
    );

    let decoded: SequenceStateComparisonReport = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, report);
}

#[test]
fn sequence_state_comparison_report_rejects_invalid_shape() {
    let err = SequenceVariantSummary::new(
        SequenceStateVariantId::LinearState,
        QualitySummary::new(1.0, 2.5).unwrap(),
        None,
        0.0,
        0,
        true,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        gbf_train::shadow::ShadowContractError::ZeroProjectedStateBytes
    ));

    let err: serde_json::Error = serde_json::from_value::<SequenceStateComparisonReport>(json!({
        "workload": "tiny-smoke",
        "linear_state": {
            "variant": "bounded_kv",
            "quality": { "lm_loss": 1.0, "perplexity": 2.5 },
            "schedule_cost": null,
            "projected_bank_switches_per_token": 0.0,
            "projected_state_bytes": 8,
            "fits_envelope": true
        },
        "bounded_kv": {
            "variant": "bounded_kv",
            "quality": { "lm_loss": 0.875, "perplexity": 2.25 },
            "schedule_cost": null,
            "projected_bank_switches_per_token": 0.0,
            "projected_state_bytes": 24,
            "fits_envelope": true
        },
        "generated_at": "2026-06-07T00:00:00Z"
    }))
    .unwrap_err();
    assert!(err.to_string().contains("sequence variant mismatch"));
}

fn cost_delta_json(bank_switches_per_token: i64) -> serde_json::Value {
    serde_json::to_value(cost_delta(bank_switches_per_token)).unwrap()
}

fn cost_delta(bank_switches_per_token: i64) -> EstimatedCostDelta {
    EstimatedCostDelta {
        cycles_per_token: estimate(10),
        bank_switches_per_token: estimate(bank_switches_per_token),
        sram_page_switches_per_token: None,
        yields_per_token: estimate(1),
        scheduler_headroom_utilization: estimate(0),
        video_commit_cost_margin: None,
        max_no_progress_estimate: estimate(0),
        time_to_first_token: estimate(100),
        sustained_throughput_tokens_per_megacycle: estimate(5),
        frame_jitter: None,
    }
}

fn estimate(units: i64) -> CostEstimate {
    CostEstimate {
        evidence_class: EvidenceClass::Heuristic,
        envelope: UncertaintyEnvelope::exact_units(units),
        refs: Vec::new(),
        fallback_reason: None,
    }
}
