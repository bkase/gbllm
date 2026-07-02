use gbf_train::selection::{
    CapacityEviction, CapacityEvictionReason, CheckpointFrontierPoint, CheckpointId,
    ParetoFrontier, SelectionStatus, dominates, training_selection_json, training_selection_report,
};
use serde_json::json;

#[test]
fn pareto_tests_dominance_witness_chain_and_tradeoff_cases_are_stable() {
    let a = point("a", 10, 0.90, true, true, 300, 100, 2);
    let b = point("b", 20, 0.80, true, true, 200, 150, 3);
    let d = point("d", 30, 0.70, true, true, 100, 200, 4);
    let cheaper_tradeoff = point("c", 40, 0.85, true, true, 400, 80, 1);

    // These cross-crate tests intentionally duplicate key private-module
    // examples from gbf-train as public API consumer regressions.
    assert!(dominates(&a, &b));
    assert!(dominates(&b, &d));
    assert!(dominates(&a, &d));
    assert!(!dominates(&b, &a));

    assert!(!dominates(&a, &cheaper_tradeoff));
    assert!(!dominates(&cheaper_tradeoff, &a));
}

#[test]
fn pareto_tests_single_passing_point_frontier_is_selected() {
    let report = training_selection_report(
        [point("solo", 10, 0.90, true, true, 256, 100, 2)],
        4,
        1_700_000_000,
    )
    .expect("single-point selection report builds");

    assert_eq!(ids(&report.frontier), vec!["solo"]);
    assert_eq!(report.selection.status, SelectionStatus::Selected);
    assert_eq!(
        report.selection.selected_checkpoint_id,
        Some(CheckpointId::new("solo").unwrap())
    );
    assert_eq!(
        report
            .selected
            .as_ref()
            .map(|point| point.checkpoint_id.as_str()),
        Some("solo")
    );
    assert!(report.capacity_evictions.is_empty());
    assert!(report.hard_failure_filter.is_empty());
}

#[test]
fn pareto_tests_frontier_contains_known_non_dominated_points() {
    let report = training_selection_report(
        [
            point("a", 10, 0.90, true, true, 300, 100, 2),
            point("b", 20, 0.80, true, true, 200, 150, 3),
            point("c", 30, 0.85, true, true, 400, 80, 1),
            point("d", 40, 0.95, true, true, 250, 200, 4),
            point("e-hard-fail", 50, 0.99, true, false, -128, 50, 1),
        ],
        8,
        1_700_000_000,
    )
    .expect("selection report builds");

    assert_eq!(ids(&report.frontier), vec!["a", "c", "d", "e-hard-fail"]);
    assert_eq!(
        report.selection.selected_checkpoint_id,
        Some(CheckpointId::new("d").unwrap())
    );
    assert_eq!(
        report
            .hard_failure_filter
            .iter()
            .map(|reason| reason.checkpoint_id.as_str())
            .collect::<Vec<_>>(),
        vec!["e-hard-fail"]
    );
    assert_eq!(
        report
            .domination_reasoning
            .iter()
            .map(|reason| {
                (
                    reason.dominant_checkpoint_id.as_str(),
                    reason.dominated_checkpoint_id.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        vec![("a", "b"), ("c", "b")]
    );
}

#[test]
fn pareto_tests_frontier_capacity_evicts_oldest_non_dominated_point() {
    let mut frontier = ParetoFrontier::new(2).expect("valid frontier capacity");
    frontier
        .add_point(point(
            "old-quality-low-cost",
            10,
            0.80,
            true,
            true,
            300,
            80,
            1,
        ))
        .expect("insert old point");
    frontier
        .add_point(point("middle", 20, 0.85, true, true, 300, 90, 1))
        .expect("insert middle point");
    let update = frontier
        .add_point(point(
            "new-quality-high-cost",
            30,
            0.90,
            true,
            true,
            300,
            100,
            1,
        ))
        .expect("insert new point");

    assert!(update.accepted);
    assert_eq!(
        update.evicted_for_capacity,
        vec![CapacityEviction {
            checkpoint_id: CheckpointId::new("old-quality-low-cost").unwrap(),
            reason: CapacityEvictionReason::OldestAfterDominancePruning,
        }]
    );
    assert_eq!(
        ids(frontier.points()),
        vec!["middle", "new-quality-high-cost"]
    );
}

#[test]
fn pareto_tests_selection_filters_hard_failures_and_handles_empty_or_all_failed() {
    let report = training_selection_report(
        [
            point("fit-good", 10, 0.90, true, true, 256, 120, 2),
            point("fit-best", 20, 0.95, true, true, 128, 200, 4),
            point("no-fit", 30, 0.99, true, false, -1, 80, 1),
            point("no-conformance", 40, 0.98, false, true, 256, 90, 1),
        ],
        8,
        1_700_000_000,
    )
    .expect("selection report builds");

    assert_eq!(report.selection.status, SelectionStatus::Selected);
    assert_eq!(
        report.selection.selected_checkpoint_id,
        Some(CheckpointId::new("fit-best").unwrap())
    );
    assert_eq!(
        report
            .hard_failure_filter
            .iter()
            .map(|reason| reason.checkpoint_id.as_str())
            .collect::<Vec<_>>(),
        vec!["no-conformance", "no-fit"]
    );

    let empty = training_selection_report([], 3, 1_700_000_000).expect("empty is reportable");
    assert_eq!(empty.selection.status, SelectionStatus::EmptyFrontier);
    assert!(empty.selected.is_none());

    let all_failed = training_selection_report(
        [
            point("no-fit", 10, 0.90, true, false, -1, 100, 2),
            point("no-conformance", 20, 0.91, false, true, 256, 90, 1),
        ],
        3,
        1_700_000_000,
    )
    .expect("all hard failures are reportable");
    assert_eq!(
        all_failed.selection.status,
        SelectionStatus::AllCandidatesHardFailed
    );
    assert!(all_failed.selected.is_none());
}

#[test]
fn pareto_tests_selection_tie_break_and_training_selection_json_are_deterministic() {
    let tied = training_selection_report(
        [
            point("z", 20, 0.90, true, true, 256, 100, 2),
            point("a", 30, 0.90, true, true, 256, 100, 2),
        ],
        4,
        1_700_000_000,
    )
    .expect("tie selection report builds");
    assert_eq!(
        tied.selection.selected_checkpoint_id,
        Some(CheckpointId::new("a").unwrap())
    );

    let json_string = training_selection_json(
        [
            point("winner", 10, 0.90, true, true, 256, 100, 2),
            point("dominated", 20, 0.80, true, true, 128, 150, 3),
        ],
        3,
        1_700_000_000,
    )
    .expect("json serializes");
    let value: serde_json::Value = serde_json::from_str(&json_string).expect("json parses");

    assert_eq!(
        value,
        json!({
            "schema": "training_selection.v1",
            "scope_note": "pure-selection-only; not canonical s7_frontier.v1; shadow compile point production is owned by bd-1f7/bd-2am",
            "generated_at_unix_seconds": 1_700_000_000_u64,
            "keep_frontier": 3,
            "input_points_count": 2,
            "frontier": [
                {
                    "checkpoint_id": "winner",
                    "observed_at_step": 10,
                    "quality": { "score": 0.9 },
                    "conformance": { "passes": true, "max_divergence": 0.001 },
                    "projected_fit": { "fits": true, "margin_bytes": 256 },
                    "schedule_cost": {
                        "cycles_per_token": 100,
                        "bank_switches_per_token": 2
                    }
                }
            ],
            "selected": {
                "checkpoint_id": "winner",
                "observed_at_step": 10,
                "quality": { "score": 0.9 },
                "conformance": { "passes": true, "max_divergence": 0.001 },
                "projected_fit": { "fits": true, "margin_bytes": 256 },
                "schedule_cost": {
                    "cycles_per_token": 100,
                    "bank_switches_per_token": 2
                }
            },
            "selection": {
                "status": "selected",
                "selected_checkpoint_id": "winner",
                "reason": "selected highest-quality hard-pass frontier point with deterministic tie-break"
            },
            "domination_reasoning": [
                {
                    "dominant_checkpoint_id": "winner",
                    "dominated_checkpoint_id": "dominated",
                    "better_or_equal_axes": [
                        "quality",
                        "conformance",
                        "projected_fit",
                        "schedule_cost"
                    ],
                    "strictly_better_axes": [
                        "quality",
                        "projected_fit",
                        "schedule_cost"
                    ]
                }
            ],
            "capacity_evictions": [],
            "hard_failure_filter": []
        })
    );
}

fn point(
    checkpoint_id: &str,
    observed_at_step: u64,
    quality_score: f64,
    conformance_passes: bool,
    projected_fit: bool,
    margin_bytes: i64,
    cycles_per_token: u64,
    bank_switches_per_token: u64,
) -> CheckpointFrontierPoint {
    CheckpointFrontierPoint::new(
        checkpoint_id,
        observed_at_step,
        quality_score,
        conformance_passes,
        if conformance_passes { 0.001 } else { 1.0 },
        projected_fit,
        margin_bytes,
        cycles_per_token,
        bank_switches_per_token,
    )
    .expect("fixture point is valid")
}

fn ids(points: &[CheckpointFrontierPoint]) -> Vec<&str> {
    points
        .iter()
        .map(|point| point.checkpoint_id.as_str())
        .collect()
}
