mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::report::{
    Hypothesis, HypothesisFinding, HypothesisStatus, ObservedSeed, ReportError, ReportFile,
    ReportInput, ReportValidationError, decision_for_outcome, emit_report,
    predictions_section_hash, report_self_hash, validate_report,
};
use gbf_experiments::s1::schema::{
    GitCommitId, PerSeedArtifacts, ReportFrontMatter, RfcRevisionRef, S1Completion, S1Decision,
    S1Outcome,
};
use gbf_foundation::{Hash256, sha256};
use serde_json::{Value, json};

const PREDICTIONS: &str = "- Toy0 validation bpc should beat the trigram baseline by 0.05.\n- H5 metric oracles must all pass.";

#[test]
fn emitter_self_hashes_report_and_logs_report_events() {
    let capture = TraceCapture::default();
    let input = fixture_input(S1Outcome::PassClean);

    let report = with_trace_capture(&capture, || emit_report(&input).expect("report emits"));
    assert_eq!(
        report.front_matter.report_self_hash,
        recomputed_report_hash(&report)
    );
    let markdown = report.to_markdown().expect("markdown renders");
    for section in [
        "## Pre-registered predictions",
        "## Observed",
        "## Hypothesis verdicts",
        "## Falsification analysis",
        "## Surprises",
        "## Decision",
        "## Reproducibility statement",
    ] {
        assert!(markdown.contains(section), "missing section {section}");
    }

    let outcome_events = captured_events(&capture)
        .into_iter()
        .filter(|captured| {
            captured.name.starts_with("s1.report.") || captured.name == "report.validators_run"
        })
        .collect::<Vec<_>>();
    assert_eq!(outcome_events[0].name, event::REPORT_EMIT_START);
    assert_eq!(
        outcome_events[0].fields.get(field::PASS_VERSION),
        Some(&json!("1.0.0"))
    );
    let validator_events = outcome_events
        .iter()
        .filter(|captured| captured.name == event::REPORT_VALIDATOR)
        .collect::<Vec<_>>();
    assert_eq!(validator_events.len(), 6);
    assert!(
        validator_events
            .iter()
            .all(|captured| { captured.fields.get(field::STATUS) == Some(&json!("PASS")) })
    );
    assert_eq!(
        outcome_events.last().map(|captured| captured.name.as_str()),
        Some(event::REPORT_EMIT_COMPLETE)
    );
}

#[test]
fn generated_at_is_excluded_from_report_self_hash() {
    let input = fixture_input(S1Outcome::PassClean);
    let report = emit_report(&input).expect("report emits");
    let mut front_matter = report.front_matter.clone();
    front_matter.report_self_hash = Hash256::ZERO;
    let original = report_self_hash(&front_matter, &report.body).expect("hash");

    front_matter.generated_at = "2030-01-01T00:00:00Z".to_owned();
    assert_eq!(
        original,
        report_self_hash(&front_matter, &report.body).expect("generated_at hash")
    );
}

#[test]
fn predictions_section_hash_contract_is_canonical_json_string_not_raw_bytes() {
    let markdown = "  H1: finite loss\nH2: capacity margin\n";
    let expected = sha256(
        gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&markdown.trim())
            .expect("canonical JSON string"),
    );

    assert_eq!(
        predictions_section_hash(markdown).expect("predictions hash"),
        expected
    );
    assert_ne!(
        predictions_section_hash(markdown).expect("predictions hash"),
        sha256(markdown.trim().as_bytes()),
        "predictions_section_hash intentionally hashes a canonical JSON string, not raw trimmed bytes"
    );
}

#[test]
fn s1_report_records_preregistration_or_capacity_failure_state() {
    let markdown = include_str!("../../docs/experiments/S1-report.md");
    let (front_matter, body) = split_report_markdown(markdown);
    let predictions = pre_registered_predictions_section(body);

    assert_eq!(front_matter["schema"], json!("s1_report.v1"));
    assert_eq!(
        front_matter["predictions_section_hash"],
        json!(
            predictions_section_hash(predictions)
                .expect("predictions hash")
                .to_string()
        )
    );

    for required in [
        "### H1 Plumbing",
        "### H2 Capacity",
        "### H3 Sequence-state utility",
        "### H4 Phase A cleanliness",
        "### H5 Measurement",
        "D6 per-seed strict pass criterion:",
        "Prediction-status rule:",
    ] {
        assert!(
            predictions.contains(required),
            "missing pre-registration content {required}"
        );
    }

    match &front_matter["s1_outcome"] {
        Value::Null => {
            assert_eq!(front_matter["decision"], json!("NotYetRun"));
            assert_eq!(front_matter["baseline_self_hash"], Value::Null);
            assert_eq!(front_matter["predictions_commit"], Value::Null);
            assert_eq!(front_matter["first_result_commit"], Value::Null);
            for row in front_matter["per_seed_artifacts"]
                .as_array()
                .expect("per_seed_artifacts array")
            {
                assert_eq!(row["completion"], json!({"kind": "not_reached"}));
                for field in [
                    "checkpoint_self_hash",
                    "run_log_self_hash",
                    "score_self_hash",
                    "negative_self_hash",
                    "ablation_self_hash",
                ] {
                    assert_eq!(
                        row[field],
                        Value::Null,
                        "{field} must be a prereg placeholder"
                    );
                }
            }
            assert!(
                body.contains("Populated by F-S1.29 after the run completes."),
                "pre-result report must keep result sections stubbed"
            );
        }
        Value::String(outcome) if outcome == "Fail-capacity" => {
            assert_eq!(
                front_matter["decision"],
                json!({"kind": "Investigate", "reason": "propose-Toy1"})
            );
            assert_eq!(
                front_matter["baseline_self_hash"],
                json!("sha256:ab10244caffbdedf7727b08a17edc970f392f258a05c8b2de486b1a20d8e731c")
            );
            assert!(
                front_matter["predictions_commit"].is_null()
                    || front_matter["predictions_commit"].as_str().is_some()
            );
            assert!(
                front_matter["first_result_commit"].is_null()
                    || front_matter["first_result_commit"].as_str().is_some()
            );

            let rows = front_matter["per_seed_artifacts"]
                .as_array()
                .expect("per_seed_artifacts array");
            assert_eq!(rows.len(), 5);
            for (seed, row) in rows.iter().enumerate() {
                assert_eq!(row["seed"], json!(seed));
                assert_eq!(row["completion"], json!({"kind": "completed"}));
                for field in [
                    "checkpoint_self_hash",
                    "run_log_self_hash",
                    "score_self_hash",
                ] {
                    assert_ne!(row[field], Value::Null, "{field} must be populated");
                }
            }
            assert_ne!(rows[0]["negative_self_hash"], Value::Null);
            assert_ne!(rows[0]["ablation_self_hash"], Value::Null);

            for required in ["H2 | Refuted", "`Investigate(propose-Toy1)`"] {
                assert!(
                    body.contains(required),
                    "missing final report content {required}"
                );
            }
        }
        other => panic!("unexpected report outcome state: {other:?}"),
    }
}

#[test]
fn closure_checklist_forbids_fixture_or_dummy_final_evidence() {
    let checklist = include_str!("../../docs/experiments/S1-closure-checklist.md");

    for required in [
        "Do not close `bd-1261` or `bd-12pl` from IntegrationFixture output",
        "`bd-1ehz` is closed with five completed TinyStories Production seed runs.",
        "`budget_profile = \"production\"`",
        "`predictions_commit` is a strict ancestor of `first_result_commit`.",
        "No final report field uses fixture-only constants, zero hashes, null",
        "The F-S1.19 dispatcher is the only source of `S1Outcome` and `Decision`.",
        "`Decision` is exactly one of `ProceedToS2` or",
        "s1_report.v1` R-Decision, R-AllSeeds, R-ClosureArtifacts,",
        "Any checkpoint metadata has `budget_profile = \"integration_fixture\"`.",
        "The final report relies on fixture goldens, dummy commit ids, placeholder",
    ] {
        assert!(
            checklist.contains(required),
            "closure checklist missing required guardrail: {required}"
        );
    }

    let checked_boxes = checklist.matches("- [x]").count() + checklist.matches("- [X]").count();
    assert_eq!(
        checked_boxes, 0,
        "scaffold checklist must not pre-claim any closure item"
    );
}

#[test]
fn r_decision_rejects_outcome_decision_mismatch() {
    let mut input = fixture_input(S1Outcome::PassClean);
    input.front_matter.decision = S1Decision::Halt {
        reason: "measurement-broken".to_owned(),
    };

    assert!(matches!(
        emit_report(&input),
        Err(ReportError::Validation(
            ReportValidationError::DecisionMismatch { .. }
        ))
    ));
}

#[test]
fn r_all_seeds_rejects_missing_seed_in_front_matter_or_observed_rows() {
    let mut missing_artifact_seed = fixture_input(S1Outcome::PassClean);
    missing_artifact_seed
        .front_matter
        .per_seed_artifacts
        .retain(|row| row.seed != 3);
    assert!(matches!(
        emit_report(&missing_artifact_seed),
        Err(ReportError::Validation(
            ReportValidationError::MissingSeed {
                surface: "per_seed_artifacts",
                seed: 3,
            }
        ))
    ));

    let mut missing_observed_seed = fixture_input(S1Outcome::PassClean);
    missing_observed_seed
        .observed_per_seed
        .retain(|row| row.seed != 4);
    assert!(matches!(
        emit_report(&missing_observed_seed),
        Err(ReportError::Validation(
            ReportValidationError::MissingSeed {
                surface: "observed_per_seed",
                seed: 4,
            }
        ))
    ));
}

#[test]
fn r_closure_artifacts_rejects_missing_required_hashes_for_proceed_decisions() {
    let mut missing_score = fixture_input(S1Outcome::PassClean);
    missing_score.front_matter.per_seed_artifacts[2].score_self_hash = None;
    assert!(matches!(
        emit_report(&missing_score),
        Err(ReportError::Validation(
            ReportValidationError::MissingClosureArtifact {
                seed: 2,
                field: "score_self_hash",
            }
        ))
    ));

    let mut missing_seed0_negative = fixture_input(S1Outcome::PassWithWarning);
    missing_seed0_negative.front_matter.per_seed_artifacts[0].negative_self_hash = None;
    assert!(matches!(
        emit_report(&missing_seed0_negative),
        Err(ReportError::Validation(
            ReportValidationError::MissingClosureArtifact {
                seed: 0,
                field: "negative_self_hash",
            }
        ))
    ));
}

#[test]
fn r_self_hash_rejects_body_tampering() {
    let input = fixture_input(S1Outcome::PassClean);
    let mut report = emit_report(&input).expect("report emits");
    report.body.push_str("\nTampered after hashing.\n");

    assert!(matches!(
        validate_report(&report, &input),
        Err(ReportError::Validation(
            ReportValidationError::SelfHashMismatch { .. }
        ))
    ));
}

#[test]
fn r_predictions_rejects_section_hash_mismatch_and_equal_commits() {
    let mut bad_hash = fixture_input(S1Outcome::PassClean);
    bad_hash.front_matter.predictions_section_hash = hash(99);
    assert!(matches!(
        emit_report(&bad_hash),
        Err(ReportError::Validation(
            ReportValidationError::PredictionsSectionHashMismatch { .. }
        ))
    ));

    let mut equal_commits = fixture_input(S1Outcome::PassClean);
    equal_commits.front_matter.first_result_commit =
        equal_commits.front_matter.predictions_commit.clone();
    assert!(matches!(
        emit_report(&equal_commits),
        Err(ReportError::Validation(
            ReportValidationError::PredictionsCommitEqualsFirstResult { .. }
        ))
    ));
}

#[test]
fn r_all_hypotheses_rejects_missing_status_and_not_evaluated_closure_status() {
    let mut missing_h3 = fixture_input(S1Outcome::PassClean);
    missing_h3
        .hypotheses
        .retain(|finding| finding.hypothesis != Hypothesis::H3);
    assert!(matches!(
        emit_report(&missing_h3),
        Err(ReportError::Validation(
            ReportValidationError::MissingHypothesis {
                hypothesis: Hypothesis::H3,
            }
        ))
    ));

    let mut skipped_h4 = fixture_input(S1Outcome::PassClean);
    let h4 = skipped_h4
        .hypotheses
        .iter_mut()
        .find(|finding| finding.hypothesis == Hypothesis::H4)
        .expect("H4 finding");
    h4.status = HypothesisStatus::NotEvaluatedDueToPriorGate("prior gate".to_owned());
    assert!(matches!(
        emit_report(&skipped_h4),
        Err(ReportError::Validation(
            ReportValidationError::NotEvaluatedClosureHypothesis {
                hypothesis: Hypothesis::H4,
                ..
            }
        ))
    ));
}

#[test]
fn refuted_input_event_carries_real_hypothesis_observation() {
    let capture = TraceCapture::default();
    let input = fixture_input(S1Outcome::FailCapacity);

    let report = with_trace_capture(&capture, || emit_report(&input).expect("report emits"));
    assert_eq!(report.front_matter.s1_outcome, S1Outcome::FailCapacity);

    let events = captured_events(&capture)
        .into_iter()
        .filter(|captured| captured.name == event::OUTCOME_REFUTED_INPUT)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].fields.get(field::HYPOTHESIS), Some(&json!("H2")));
    assert_eq!(
        events[0].fields.get(field::OBSERVATION),
        Some(&json!(
            "val_bpc 2.400000 did not beat trigram baseline 2.300000 by 0.05"
        ))
    );
}

#[test]
fn golden_reports_pin_decision_variants() {
    let rows = [
        S1Outcome::PassClean,
        S1Outcome::PassWithWarning,
        S1Outcome::FailSubstrate,
        S1Outcome::FailCapacity,
        S1Outcome::FailSuspicious,
        S1Outcome::FailPhase,
        S1Outcome::FailMetric,
    ]
    .map(|outcome| {
        let input = fixture_input(outcome);
        let report = emit_report(&input).expect("report emits");
        format!(
            "{} => {} / {}",
            report.front_matter.s1_outcome,
            report.front_matter.decision,
            report.front_matter.report_self_hash
        )
    });

    insta::assert_debug_snapshot!(rows, @r###"
    [
        "Pass-clean => ProceedToS2 / sha256:c63bddfe8c265664f576348a652b6575996b5ede676b66247069ed1f65e56fdb",
        "Pass-with-warning => ProceedToS2-with-T12.5-prereq / sha256:252c7dd38e88de7d13bfedfa431d902ae353054d63b238a3d360bd1ab26a729d",
        "Fail-substrate => Investigate(burn-or-autodiff) / sha256:5f0ce5ac9a025e0feb37bf53683624931850321a3a7d026871502e00b0f5a77a",
        "Fail-capacity => Investigate(propose-Toy1) / sha256:d78600cbee2e70175988eac4ea655791acbb73b330b9e9d008501bf366286301",
        "Fail-suspicious => Halt(audit-split-and-bpc) / sha256:5f9fe7d765a4f6b5e8b6f9737c764a1a5f2fbc6453dcb89d6d8f4ea2b0b5e8b4",
        "Fail-phase => Investigate(F4-phase-contract) / sha256:c75571bfc8d8826af36fb18ef27f71d5fa131361b0541b9d28e5d8e82b6efa70",
        "Fail-metric => Halt(measurement-broken) / sha256:5749a4a1fb1aee7d1d9f0003f1f090ec33771a7d5b17f6377f9f53c146dd93a5",
    ]
    "###);
}

fn recomputed_report_hash(report: &ReportFile) -> Hash256 {
    let mut front_matter = report.front_matter.clone();
    front_matter.report_self_hash = Hash256::ZERO;
    report_self_hash(&front_matter, &report.body).expect("report hash")
}

fn split_report_markdown(markdown: &str) -> (Value, &str) {
    let markdown = markdown
        .strip_prefix("---\n")
        .expect("report starts with front matter marker");
    let (front_matter, body) = markdown
        .split_once("\n---\n")
        .expect("report has closing front matter marker");
    (
        serde_json::from_str(front_matter).expect("front matter is JSON"),
        body,
    )
}

fn pre_registered_predictions_section(body: &str) -> &str {
    let marker = "## Pre-registered predictions\n\n";
    let start = body
        .find(marker)
        .map(|index| index + marker.len())
        .expect("predictions section exists");
    let end = body[start..]
        .find("\n## Observed\n")
        .map(|index| start + index)
        .expect("observed section follows predictions");
    body[start..end].trim()
}

fn fixture_input(outcome: S1Outcome) -> ReportInput {
    let decision = decision_for_outcome(outcome);
    let mut per_seed_artifacts = (0..5)
        .map(|seed| PerSeedArtifacts {
            seed,
            completion: completion_for_outcome(outcome, seed),
            checkpoint_self_hash: Some(hash(10 + seed as u8)),
            run_log_self_hash: Some(hash(20 + seed as u8)),
            score_self_hash: Some(hash(30 + seed as u8)),
            negative_self_hash: (seed == 0).then(|| hash(40)),
            ablation_self_hash: (seed == 0).then(|| hash(41)),
        })
        .collect::<Vec<_>>();
    if !matches!(
        decision,
        S1Decision::ProceedToS2 | S1Decision::ProceedToS2WithT125Prereq
    ) {
        for row in &mut per_seed_artifacts {
            if row.seed != 0 {
                row.checkpoint_self_hash = None;
                row.run_log_self_hash = None;
                row.score_self_hash = None;
            }
        }
    }

    ReportInput {
        front_matter: ReportFrontMatter {
            schema: "s1_report.v1".to_owned(),
            s1_outcome: outcome,
            decision,
            baseline_self_hash: hash(1),
            per_seed_artifacts,
            generated_at: "2026-05-09T12:00:00Z".to_owned(),
            rfc_revision: RfcRevisionRef::GitCommitId(commit('a')),
            predictions_section_hash: predictions_section_hash(PREDICTIONS)
                .expect("predictions hash"),
            predictions_commit: commit('b'),
            first_result_commit: commit('c'),
            report_self_hash: Hash256::ZERO,
        },
        predictions_markdown: PREDICTIONS.to_owned(),
        observed_per_seed: (0..5)
            .map(|seed| ObservedSeed {
                seed,
                completion: completion_for_outcome(outcome, seed),
                val_bpc: Some(if outcome == S1Outcome::FailCapacity {
                    2.4
                } else {
                    1.7
                }),
                neg_test_delta: (seed == 0).then_some(0.35),
                ablation_eq: (seed == 0).then_some(outcome != S1Outcome::FailPhase),
            })
            .collect(),
        hypotheses: hypotheses_for_outcome(outcome),
        falsification_analysis: "Fixture analysis cites the rule that drove each refutation."
            .to_owned(),
        surprises: "None.".to_owned(),
        decision_justification: "Fixture decision follows RFC section 8.".to_owned(),
        replay_command: "gbf s1 replay --fixture tiny".to_owned(),
        manifest_hashes: "cmt_self_hash=sha256:fixture".to_owned(),
        pass_version: "1.0.0".to_owned(),
    }
}

fn completion_for_outcome(outcome: S1Outcome, seed: u64) -> S1Completion {
    if outcome == S1Outcome::FailSubstrate && seed == 2 {
        S1Completion::DivergedAt { step: 17 }
    } else if outcome == S1Outcome::FailSubstrate && seed > 2 {
        S1Completion::NotReached
    } else {
        S1Completion::Completed
    }
}

fn hypotheses_for_outcome(outcome: S1Outcome) -> Vec<HypothesisFinding> {
    let mut statuses = [
        (
            Hypothesis::H1,
            HypothesisStatus::Confirmed,
            "substrate run completed for required seeds",
        ),
        (
            Hypothesis::H2,
            HypothesisStatus::Confirmed,
            "val_bpc 1.700000 beat trigram baseline 2.300000 by 0.05",
        ),
        (
            Hypothesis::H3,
            HypothesisStatus::Confirmed,
            "shuffle delta 0.350000 exceeded sensitivity threshold",
        ),
        (
            Hypothesis::H4,
            HypothesisStatus::Confirmed,
            "Phase A and ablation tensor payload hashes matched",
        ),
        (
            Hypothesis::H5,
            HypothesisStatus::Confirmed,
            "all five metric oracles passed",
        ),
    ]
    .map(|(hypothesis, status, observation)| HypothesisFinding {
        hypothesis,
        status,
        observation: observation.to_owned(),
    })
    .to_vec();

    match outcome {
        S1Outcome::PassClean => {}
        S1Outcome::PassWithWarning => set_status(
            &mut statuses,
            Hypothesis::H3,
            HypothesisStatus::Refuted,
            "shuffle delta 0.010000 did not exceed sensitivity threshold",
        ),
        S1Outcome::FailSubstrate => set_status(
            &mut statuses,
            Hypothesis::H1,
            HypothesisStatus::Refuted,
            "seed 2 diverged at optimizer step 17",
        ),
        S1Outcome::FailCapacity => set_status(
            &mut statuses,
            Hypothesis::H2,
            HypothesisStatus::Refuted,
            "val_bpc 2.400000 did not beat trigram baseline 2.300000 by 0.05",
        ),
        S1Outcome::FailSuspicious => {
            // Suspicious-low-bpc is an outcome gate rather than a hypothesis refutation.
        }
        S1Outcome::FailPhase => set_status(
            &mut statuses,
            Hypothesis::H4,
            HypothesisStatus::Refuted,
            "Phase A and ablation tensor payload hashes differed at byte 9",
        ),
        S1Outcome::FailMetric => set_status(
            &mut statuses,
            Hypothesis::H5,
            HypothesisStatus::Refuted,
            "O-metric-3 reset-boundary spy failed",
        ),
    }
    statuses
}

fn set_status(
    findings: &mut [HypothesisFinding],
    hypothesis: Hypothesis,
    status: HypothesisStatus,
    observation: &str,
) {
    let finding = findings
        .iter_mut()
        .find(|finding| finding.hypothesis == hypothesis)
        .expect("fixture hypothesis exists");
    finding.status = status;
    finding.observation = observation.to_owned();
}

fn commit(fill: char) -> GitCommitId {
    GitCommitId::new(fill.to_string().repeat(40)).expect("valid commit")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
