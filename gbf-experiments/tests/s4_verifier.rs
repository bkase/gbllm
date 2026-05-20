#![cfg(feature = "s4")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s4::schema::{
    HypothesisStatus, S4_CANONICAL_SEEDS, S4Completion, S4Decision, S4Hypothesis, S4Outcome,
    S4VerifierBundle,
};
use gbf_experiments::s4::verifier::{
    S4_VERIFIER_FINALIZED_EVENT, S4_VERIFIER_STARTED_EVENT, S4H1CorpusIntegrityEvidence,
    S4H2ContaminationEvidence, S4H3PromotionGateEvidence, S4H4GeneralizationEvidence,
    S4H5OracleAgreementEvidence, S4H6DeterminismEvidence, S4H7DistributionShiftEvidence,
    S4PromotionPredicateFamily, S4PromotionPredicateFamilyEvidence, S4SeedQualityEvidence,
    S4SeedReplayEvidence, S4SeedShiftEvidence, S4VerifierEvidence, decision_for_s4_outcome,
    dispatch_s4_outcome, required_h6_artifacts, verify_h1_corpus_integrity,
    verify_h2_contamination, verify_h3_promotion_gate, verify_h4_generalization,
    verify_h5_oracle_agreement, verify_h6_determinism, verify_h7_distribution_shift, verify_s4,
};
use gbf_foundation::Hash256;
use serde_json::json;

#[test]
fn h1_corpus_integrity_pins_rfc_caps_and_accounting() {
    let pass = verify_h1_corpus_integrity(&h1_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);

    let mut high_unmappable = h1_pass();
    high_unmappable.unmappable_rate_corpus_gutenberg = 0.005_001;
    assert_refuted(
        verify_h1_corpus_integrity(&high_unmappable),
        S4Outcome::FailCorpusIntegrity,
    );

    let mut marker_cap = h1_pass();
    marker_cap.drop_count_marker_missing = 76;
    assert_refuted(
        verify_h1_corpus_integrity(&marker_cap),
        S4Outcome::FailCorpusIntegrity,
    );

    let mut unmappable_density_cap = h1_pass();
    unmappable_density_cap.drop_count_unmappable_density = 31;
    assert_refuted(
        verify_h1_corpus_integrity(&unmappable_density_cap),
        S4Outcome::FailCorpusIntegrity,
    );

    let mut bad_accounting = h1_pass();
    bad_accounting.drop_count_total += 1;
    assert_refuted(
        verify_h1_corpus_integrity(&bad_accounting),
        S4Outcome::FailCorpusIntegrity,
    );

    let mut low_retained = h1_pass();
    low_retained.train_book_count = 1_214;
    low_retained.val_book_count = 135;
    low_retained.drop_count_total = 151;
    assert_refuted(
        verify_h1_corpus_integrity(&low_retained),
        S4Outcome::FailCorpusIntegrity,
    );

    let mut hash_mismatch = h1_pass();
    hash_mismatch.manifest_self_hash_round_trips = false;
    assert_refuted(
        verify_h1_corpus_integrity(&hash_mismatch),
        S4Outcome::FailCorpusIntegrity,
    );
}

#[test]
fn h2_contamination_warns_without_refuting_until_hard_threshold() {
    let clean = verify_h2_contamination(h2_clean());
    assert_eq!(clean.status, HypothesisStatus::Confirmed);
    assert!(!clean.contamination_warning);

    let warning = verify_h2_contamination(S4H2ContaminationEvidence {
        ts_train_to_gb_val_overlap: 0.0005,
        gb_train_to_ts_val_overlap: 0.0010,
        corpus_oracle_c_or_6_passed: true,
    });
    assert_eq!(warning.status, HypothesisStatus::Confirmed);
    assert!(warning.contamination_warning);

    let hard_fail = verify_h2_contamination(S4H2ContaminationEvidence {
        ts_train_to_gb_val_overlap: 0.001_001,
        gb_train_to_ts_val_overlap: 0.0,
        corpus_oracle_c_or_6_passed: true,
    });
    assert_refuted(hard_fail, S4Outcome::FailContamination);

    let cor6_fail = verify_h2_contamination(S4H2ContaminationEvidence {
        ts_train_to_gb_val_overlap: 0.0,
        gb_train_to_ts_val_overlap: 0.0,
        corpus_oracle_c_or_6_passed: false,
    });
    assert_refuted(cor6_fail, S4Outcome::FailContamination);
}

#[test]
fn h3_promotion_gate_soundness_requires_each_p_family() {
    let pass = verify_h3_promotion_gate(&h3_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);

    let mut missing_p9 = h3_pass();
    missing_p9.broken_predicate_families.pop();
    assert_refuted(
        verify_h3_promotion_gate(&missing_p9),
        S4Outcome::FailPromotionGate,
    );

    let mut promoted_p3_broken = h3_pass();
    promoted_p3_broken.broken_predicate_families[2].rejected_when_broken = false;
    assert_refuted(
        verify_h3_promotion_gate(&promoted_p3_broken),
        S4Outcome::FailPromotionGate,
    );

    let mut unsound = h3_pass();
    unsound.reference_positive_bundle_promoted = false;
    unsound.invalid_bundle_rejected = false;
    unsound.referentially_transparent = false;
    unsound.promotion_gate_self_hash_round_trips = false;
    assert_refuted(
        verify_h3_promotion_gate(&unsound),
        S4Outcome::FailPromotionGate,
    );
}

#[test]
fn h4_generalization_is_strict_all_seed_and_detects_suspicious_low_bpc() {
    let pass = verify_h4_generalization(&h4_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);
    assert!(!pass.suspicious_low_bpc);

    let mut exact_margin = h4_pass();
    exact_margin.per_seed[0].bpc_ternary_gutenberg_val = 1.20;
    assert_refuted(
        verify_h4_generalization(&exact_margin),
        S4Outcome::FailQualityOnGutenberg,
    );

    let mut failed_v0 = h4_pass();
    failed_v0.per_seed[3].v0_success_passed = false;
    assert_refuted(
        verify_h4_generalization(&failed_v0),
        S4Outcome::FailQualityOnGutenberg,
    );

    let suspicious = verify_h4_generalization(&S4H4GeneralizationEvidence {
        bpc_kn5_gutenberg_val: 1.25,
        per_seed: S4_CANONICAL_SEEDS
            .into_iter()
            .map(|seed| S4SeedQualityEvidence {
                seed,
                bpc_ternary_gutenberg_val: 0.40,
                v0_success_passed: true,
            })
            .collect(),
    });
    assert_eq!(suspicious.status, HypothesisStatus::Refuted);
    assert!(suspicious.suspicious_low_bpc);
}

#[test]
fn h5_oracle_agreement_checks_seed_zero_outcome_and_pairwise_tolerances() {
    let pass = verify_h5_oracle_agreement(h5_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);

    let mut wrong_seed = h5_pass();
    wrong_seed.seed = 1;
    assert_refuted(
        verify_h5_oracle_agreement(wrong_seed),
        S4Outcome::FailOracleDisagreement,
    );

    let mut disagree = h5_pass();
    disagree.outcome_agree = false;
    assert_refuted(
        verify_h5_oracle_agreement(disagree),
        S4Outcome::FailOracleDisagreement,
    );

    let mut gap_exceeds = h5_pass();
    gap_exceeds.gap_live_vs_artifact = 0.11;
    gap_exceeds.tolerance_live_vs_artifact = 0.10;
    assert_refuted(
        verify_h5_oracle_agreement(gap_exceeds),
        S4Outcome::FailOracleDisagreement,
    );
}

#[test]
fn h6_determinism_requires_seed_payloads_and_full_artifact_self_hash_set() {
    let pass = verify_h6_determinism(&h6_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);

    let mut tensor_mismatch = h6_pass();
    tensor_mismatch.per_seed_tensor_payloads[2].replay_tensor_payload_sha = hash(240);
    assert_refuted(
        verify_h6_determinism(&tensor_mismatch),
        S4Outcome::FailSubstrate,
    );

    let mut artifact_mismatch = h6_pass();
    artifact_mismatch.artifact_self_hashes[0].replay_self_hash = hash(241);
    assert_refuted(
        verify_h6_determinism(&artifact_mismatch),
        S4Outcome::FailSubstrate,
    );

    let mut missing_artifact = h6_pass();
    missing_artifact.artifact_self_hashes.pop();
    assert_refuted(
        verify_h6_determinism(&missing_artifact),
        S4Outcome::FailSubstrate,
    );
}

#[test]
fn h7_distribution_shift_is_observational_and_requires_four_improved_seeds() {
    let pass = verify_h7_distribution_shift(&h7_pass());
    assert_eq!(pass.status, HypothesisStatus::Confirmed);

    let mut only_three_improved = h7_pass();
    only_three_improved.per_seed[3].c_gb_bpc_gutenberg_val = 1.21;
    let refuted = verify_h7_distribution_shift(&only_three_improved);
    assert_eq!(refuted.status, HypothesisStatus::Refuted);
    assert_eq!(refuted.outcome_if_refuted, None);

    let mut evidence = verifier_pass();
    evidence.h7 = only_three_improved;
    let report = verify_s4(&evidence);
    assert_eq!(
        report
            .hypothesis_outputs
            .get(&S4Hypothesis::H7)
            .expect("H7 output")
            .status,
        HypothesisStatus::Refuted
    );
    assert_eq!(report.outcome, S4Outcome::PassClean);
}

#[test]
fn full_verifier_report_dispatches_pass_warning_and_readiness_paths() {
    let clean = verify_s4(&verifier_pass());
    assert_eq!(clean.outcome, S4Outcome::PassClean);
    assert_eq!(clean.decision, S4Decision::ProceedToS5);

    let mut warning = verifier_pass();
    warning.h2.ts_train_to_gb_val_overlap = 0.0005;
    let warning_report = verify_s4(&warning);
    assert_eq!(
        warning_report.outcome,
        S4Outcome::PassWithContaminationWarning
    );
    assert_eq!(
        warning_report.decision,
        S4Decision::ProceedToS5WithContaminationWarning
    );

    let mut readiness = verifier_pass();
    readiness.promotion_gate_accepted_canonical = false;
    let readiness_report = verify_s4(&readiness);
    assert_eq!(
        readiness_report.outcome,
        S4Outcome::FailPromotionGateReadiness
    );
    assert_eq!(
        readiness_report.decision,
        S4Decision::Halt {
            reason: "promotion-gate-rejected-canonical".to_owned()
        }
    );
}

#[test]
fn verifier_emits_subscriber_captured_h1_h7_events() {
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || verify_s4(&verifier_pass()));

    assert_eq!(report.outcome, S4Outcome::PassClean);
    let events = captured_events(&capture);
    let started = events
        .iter()
        .filter(|event| event.name == S4_VERIFIER_STARTED_EVENT)
        .collect::<Vec<_>>();
    let finalized = events
        .iter()
        .filter(|event| event.name == S4_VERIFIER_FINALIZED_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 7);
    assert_eq!(finalized.len(), 7);

    for (index, name) in ["H1", "H2", "H3", "H4", "H5", "H6", "H7"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(started[index].fields.get("name"), Some(&json!(name)));
        assert_eq!(finalized[index].fields.get("name"), Some(&json!(name)));
        assert_eq!(
            finalized[index].fields.get("outcome"),
            Some(&json!("confirmed"))
        );
        assert!(
            finalized[index]
                .fields
                .get("reason")
                .and_then(|value| value.as_str())
                .is_some_and(|reason| reason.starts_with(name)),
            "{name} finalized event should include a verifier reason"
        );
    }
}

#[test]
fn verifier_hash_round_trip_events_include_recorded_hash_values() {
    let mut evidence = verifier_pass();
    evidence.h1.manifest_self_hash_round_trips = false;
    evidence.h1.manifest_self_hash_recorded = Some(hash(200));
    evidence.h3.promotion_gate_self_hash_round_trips = false;
    evidence.h3.promotion_gate_self_hash_recorded = Some(hash(201));
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || verify_s4(&evidence));

    assert_eq!(report.outcome, S4Outcome::FailCorpusIntegrity);
    assert_eq!(
        report
            .hypothesis_outputs
            .get(&S4Hypothesis::H1)
            .expect("H1 output")
            .status,
        HypothesisStatus::Refuted
    );
    assert_eq!(
        report
            .hypothesis_outputs
            .get(&S4Hypothesis::H3)
            .expect("H3 output")
            .status,
        HypothesisStatus::Refuted
    );

    let events = captured_events(&capture);
    let h1_finalized = verifier_event(&events, S4_VERIFIER_FINALIZED_EVENT, "H1");
    assert_eq!(h1_finalized.fields.get("outcome"), Some(&json!("refuted")));
    assert!(
        h1_finalized
            .fields
            .get("reason")
            .and_then(|value| value.as_str())
            .is_some_and(|reason| reason.contains(&hash(200).to_string())),
        "H1 finalized reason should include the recorded manifest self-hash"
    );

    let h3_finalized = verifier_event(&events, S4_VERIFIER_FINALIZED_EVENT, "H3");
    assert_eq!(h3_finalized.fields.get("outcome"), Some(&json!("refuted")));
    assert!(
        h3_finalized
            .fields
            .get("reason")
            .and_then(|value| value.as_str())
            .is_some_and(|reason| reason.contains(&hash(201).to_string())),
        "H3 finalized reason should include the recorded promotion-gate self-hash"
    );
}

#[test]
fn outcome_dispatcher_reaches_every_rfc_outcome_and_first_rung_wins() {
    let cases: [(&str, fn(&mut S4VerifierBundle), S4Outcome); 11] = [
        ("pass_clean", |_| {}, S4Outcome::PassClean),
        (
            "pass_warning",
            |bundle| bundle.contamination_warning = true,
            S4Outcome::PassWithContaminationWarning,
        ),
        (
            "h1",
            |bundle| set_status(bundle, S4Hypothesis::H1, HypothesisStatus::Refuted),
            S4Outcome::FailCorpusIntegrity,
        ),
        (
            "h2",
            |bundle| set_status(bundle, S4Hypothesis::H2, HypothesisStatus::Refuted),
            S4Outcome::FailContamination,
        ),
        (
            "h3",
            |bundle| set_status(bundle, S4Hypothesis::H3, HypothesisStatus::Refuted),
            S4Outcome::FailPromotionGate,
        ),
        (
            "readiness",
            |bundle| bundle.promotion_gate_accepted_canonical = false,
            S4Outcome::FailPromotionGateReadiness,
        ),
        (
            "diverged",
            |bundle| bundle.completions[0] = S4Completion::DivergedAt { step: 17 },
            S4Outcome::FailSubstrate,
        ),
        (
            "suspicious",
            |bundle| bundle.suspicious_low_bpc = true,
            S4Outcome::FailSuspicious,
        ),
        (
            "h4",
            |bundle| set_status(bundle, S4Hypothesis::H4, HypothesisStatus::Refuted),
            S4Outcome::FailQualityOnGutenberg,
        ),
        (
            "h5",
            |bundle| set_status(bundle, S4Hypothesis::H5, HypothesisStatus::Refuted),
            S4Outcome::FailOracleDisagreement,
        ),
        (
            "h6",
            |bundle| set_status(bundle, S4Hypothesis::H6, HypothesisStatus::Refuted),
            S4Outcome::FailSubstrate,
        ),
    ];

    let mut reached = Vec::new();
    for (name, mutate, expected) in cases {
        let mut bundle = S4VerifierBundle::closure_candidate();
        mutate(&mut bundle);
        let actual = dispatch_s4_outcome(&bundle);
        assert_eq!(actual, expected, "{name}");
        reached.push(actual);
    }
    for outcome in S4Outcome::ALL {
        assert!(
            reached.contains(&outcome),
            "{outcome:?} lacks a direct S4 dispatcher fixture"
        );
    }

    let mut first_rung = S4VerifierBundle::closure_candidate();
    set_status(&mut first_rung, S4Hypothesis::H1, HypothesisStatus::Refuted);
    set_status(&mut first_rung, S4Hypothesis::H4, HypothesisStatus::Refuted);
    first_rung.promotion_gate_accepted_canonical = false;
    first_rung.completions[0] = S4Completion::DivergedAt { step: 1 };
    first_rung.suspicious_low_bpc = true;
    assert_eq!(
        dispatch_s4_outcome(&first_rung),
        S4Outcome::FailCorpusIntegrity
    );

    let mut not_evaluated_without_prior_branch = S4VerifierBundle::closure_candidate();
    set_status(
        &mut not_evaluated_without_prior_branch,
        S4Hypothesis::H4,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "missing scores".to_owned(),
        },
    );
    not_evaluated_without_prior_branch.gutenberg_quality_passed = false;
    assert_eq!(
        dispatch_s4_outcome(&not_evaluated_without_prior_branch),
        S4Outcome::FailSubstrate
    );

    let mut h1_not_evaluated = S4VerifierBundle::closure_candidate();
    set_status(
        &mut h1_not_evaluated,
        S4Hypothesis::H1,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "missing corpus manifest".to_owned(),
        },
    );
    h1_not_evaluated.corpus_integrity_passed = false;
    assert_eq!(
        dispatch_s4_outcome(&h1_not_evaluated),
        S4Outcome::FailSubstrate
    );
}

#[test]
fn outcome_dispatcher_is_total_over_binary_h1_h6_observable_controls() {
    for refuted_mask in 0_u8..64 {
        for promotion_accepted in [false, true] {
            for diverged in [false, true] {
                for suspicious in [false, true] {
                    for contamination_warning in [false, true] {
                        let mut bundle = S4VerifierBundle::closure_candidate();
                        for (bit, hypothesis) in [
                            S4Hypothesis::H1,
                            S4Hypothesis::H2,
                            S4Hypothesis::H3,
                            S4Hypothesis::H4,
                            S4Hypothesis::H5,
                            S4Hypothesis::H6,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if refuted_mask & (1 << bit) != 0 {
                                set_status(&mut bundle, hypothesis, HypothesisStatus::Refuted);
                            }
                        }
                        bundle.promotion_gate_accepted_canonical = promotion_accepted;
                        if diverged {
                            bundle.completions[4] = S4Completion::DivergedAt { step: 2 };
                        }
                        bundle.suspicious_low_bpc = suspicious;
                        bundle.contamination_warning = contamination_warning;

                        let outcome = dispatch_s4_outcome(&bundle);
                        assert!(S4Outcome::ALL.contains(&outcome));
                    }
                }
            }
        }
    }
}

#[test]
fn decision_table_matches_rfc_section_11() {
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::PassClean),
        S4Decision::ProceedToS5
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::PassWithContaminationWarning),
        S4Decision::ProceedToS5WithContaminationWarning
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailCorpusIntegrity),
        S4Decision::Halt {
            reason: "corpus-integrity-broken".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailContamination),
        S4Decision::Halt {
            reason: "contamination-dirty".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailPromotionGate),
        S4Decision::Halt {
            reason: "promotion-gate-unsound".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailPromotionGateReadiness),
        S4Decision::Halt {
            reason: "promotion-gate-rejected-canonical".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailQualityOnGutenberg),
        S4Decision::Investigate {
            reason: "propose-step-budget-or-Toy1".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailOracleDisagreement),
        S4Decision::Halt {
            reason: "oracle-disagrees-on-gutenberg".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailSubstrate),
        S4Decision::Investigate {
            reason: "burn-or-corpus-loader".to_owned()
        }
    );
    assert_eq!(
        decision_for_s4_outcome(S4Outcome::FailSuspicious),
        S4Decision::Halt {
            reason: "audit-split-and-bpc".to_owned()
        }
    );
}

fn assert_refuted(
    output: gbf_experiments::s4::verifier::S4HypothesisVerifierOutput,
    outcome: S4Outcome,
) {
    assert_eq!(output.status, HypothesisStatus::Refuted);
    assert_eq!(output.outcome_if_refuted, Some(outcome));
}

fn verifier_pass() -> S4VerifierEvidence {
    S4VerifierEvidence {
        h1: h1_pass(),
        h2: h2_clean(),
        h3: h3_pass(),
        h4: h4_pass(),
        h5: h5_pass(),
        h6: h6_pass(),
        h7: h7_pass(),
        promotion_gate_accepted_canonical: true,
        completions: vec![S4Completion::Completed; S4_CANONICAL_SEEDS.len()],
    }
}

fn h1_pass() -> S4H1CorpusIntegrityEvidence {
    S4H1CorpusIntegrityEvidence {
        corpus_oracle_c_or_1_through_5_passed: true,
        book_id_count: 1_500,
        train_book_count: 1_260,
        val_book_count: 140,
        drop_count_total: 100,
        drop_count_marker_missing: 75,
        drop_count_unmappable_density: 30,
        unmappable_rate_corpus_gutenberg: 0.005,
        manifest_self_hash_round_trips: true,
        manifest_self_hash_recorded: Some(hash(90)),
    }
}

fn h2_clean() -> S4H2ContaminationEvidence {
    S4H2ContaminationEvidence {
        ts_train_to_gb_val_overlap: 0.000_499,
        gb_train_to_ts_val_overlap: 0.0,
        corpus_oracle_c_or_6_passed: true,
    }
}

fn h3_pass() -> S4H3PromotionGateEvidence {
    S4H3PromotionGateEvidence {
        reference_positive_bundle_promoted: true,
        broken_predicate_families: S4PromotionPredicateFamily::ALL
            .into_iter()
            .map(|family| S4PromotionPredicateFamilyEvidence {
                family,
                rejected_when_broken: true,
            })
            .collect(),
        invalid_bundle_rejected: true,
        referentially_transparent: true,
        promotion_gate_self_hash_round_trips: true,
        promotion_gate_self_hash_recorded: Some(hash(91)),
    }
}

fn h4_pass() -> S4H4GeneralizationEvidence {
    S4H4GeneralizationEvidence {
        bpc_kn5_gutenberg_val: 1.25,
        per_seed: S4_CANONICAL_SEEDS
            .into_iter()
            .map(|seed| S4SeedQualityEvidence {
                seed,
                bpc_ternary_gutenberg_val: 1.0 + (seed as f64 * 0.01),
                v0_success_passed: true,
            })
            .collect(),
    }
}

fn h5_pass() -> S4H5OracleAgreementEvidence {
    S4H5OracleAgreementEvidence {
        seed: 0,
        outcome_agree: true,
        gap_live_vs_denotational: 0.01,
        tolerance_live_vs_denotational: 0.10,
        gap_live_vs_artifact: 0.01,
        tolerance_live_vs_artifact: 0.10,
        gap_denotational_vs_artifact: 0.01,
        tolerance_denotational_vs_artifact: 0.10,
    }
}

fn h6_pass() -> S4H6DeterminismEvidence {
    S4H6DeterminismEvidence {
        per_seed_tensor_payloads: S4_CANONICAL_SEEDS
            .into_iter()
            .map(|seed| S4SeedReplayEvidence {
                seed,
                original_tensor_payload_sha: hash(seed as u8 + 1),
                replay_tensor_payload_sha: hash(seed as u8 + 1),
            })
            .collect(),
        artifact_self_hashes: required_h6_artifacts()
            .into_iter()
            .enumerate()
            .map(|(index, artifact)| {
                let self_hash = hash(index as u8 + 40);
                gbf_experiments::s4::verifier::S4ArtifactReplayEvidence {
                    artifact,
                    original_self_hash: self_hash,
                    replay_self_hash: self_hash,
                }
            })
            .collect(),
    }
}

fn h7_pass() -> S4H7DistributionShiftEvidence {
    S4H7DistributionShiftEvidence {
        c_ts_bpc_gutenberg_val: 1.30,
        per_seed: S4_CANONICAL_SEEDS
            .into_iter()
            .map(|seed| S4SeedShiftEvidence {
                seed,
                c_gb_bpc_gutenberg_val: if seed < 4 { 1.10 } else { 1.21 },
            })
            .collect(),
    }
}

fn set_status(bundle: &mut S4VerifierBundle, hypothesis: S4Hypothesis, status: HypothesisStatus) {
    bundle.hypothesis_statuses.insert(hypothesis, status);
}

fn verifier_event<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    event_name: &str,
    verifier_name: &str,
) -> &'a common::tracing_capture::TracingEvent {
    events
        .iter()
        .find(|event| {
            event.name == event_name
                && event.fields.get("name").and_then(|value| value.as_str()) == Some(verifier_name)
        })
        .unwrap_or_else(|| panic!("missing {event_name} for {verifier_name}"))
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
