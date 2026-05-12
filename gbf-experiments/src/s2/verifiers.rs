//! Scalar S2 hypothesis verifiers that do not own artifact schemas.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::S2_LOG_TARGET;
use crate::s2::gap::{S2GapError, try_gap_ternary_vs_fp};
use crate::s2::run::hardness::hardness_for_global_step;
use crate::s2::run::lambdas::phase_effective_lambdas;
use crate::s2::run::scheduler::{PhasePlan, phase_for_global_step, transition_for_step};
use crate::s2::schema::{
    GlobalStep, HypothesisStatus, PhaseEffectiveLambda, PhaseEntry, PhaseEvent, PhaseKindS2,
    PhaseLog, S2_OPTIMIZER_STEPS, S2_TEACHER_FREEZE_STEP, S2BuildKind, S2ScoreReport,
    TrainConfigS2Full, quant_hardness_override_for_build_kind,
};
use serde::Serialize;
use serde_json::{Value, json};

/// H1 gradient-norm surprise threshold. This warning does not change verdict.
pub const H1_GRAD_NORM_SPIKE_THRESHOLD: f32 = 1.0e3;

/// H1 early mean train-loss surprise lower bound. This warning does not change verdict.
pub const H1_MEAN_TRAIN_LOSS_MIN: f32 = 4.0;

/// H1 early mean train-loss surprise upper bound. This warning does not change verdict.
pub const H1_MEAN_TRAIN_LOSS_MAX: f32 = 6.5;

/// H2 maximum ternary-vs-fp BPC gap per seed.
pub const H2_MAX_TERNARY_FP_GAP_BPC: f64 = 0.5;

/// H2 maximum full-precision BPC sanity bound.
pub const H2_MAX_FP_BPC: f64 = 2.5;

/// H2 maximum ternary BPC sanity bound.
pub const H2_MAX_TERNARY_BPC: f64 = 3.0;

/// H2 suspicious-low median full-precision BPC sentinel.
pub const H2_SUSPICIOUS_LOW_MEDIAN_FP_BPC: f64 = 0.5;

/// H2 suspicious-low median ternary BPC sentinel.
pub const H2_SUSPICIOUS_LOW_MEDIAN_TERNARY_BPC: f64 = 0.5;

/// Weak-form H3 tolerance in bits per character.
pub const H3_WEAK_TOLERANCE_BPC: f64 = 0.10;

/// Shared structured diagnostic emitted by H1 and H2 verifier refutations.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiagnosticHit {
    /// Hypothesis or sub-hypothesis id, e.g. `H1.a`.
    pub hypothesis_id: &'static str,
    /// Stable check name.
    pub check_name: &'static str,
    /// Offending optimizer step for H1 checks.
    pub step: Option<u64>,
    /// Offending seed for H2 checks.
    pub seed: Option<u64>,
    /// Build kind associated with this diagnostic, when applicable.
    pub build_kind: Option<S2BuildKind>,
    /// Expected value or invariant.
    pub expected: Value,
    /// Observed value.
    pub observed: Value,
    /// Short remediation pointer.
    pub remediation: &'static str,
}

/// H1 scheduler-integrity verifier result.
#[derive(Debug, Clone, PartialEq)]
pub struct H1Verdict {
    /// Closure hypothesis status.
    pub status: HypothesisStatus,
    /// Structured refutation diagnostics.
    pub hits: Vec<DiagnosticHit>,
}

/// H2 matched-protocol gap verifier result.
#[derive(Debug, Clone, PartialEq)]
pub struct H2Verdict {
    /// Closure hypothesis status.
    pub status: HypothesisStatus,
    /// `s2_ternary_full.bpc - s2_fp_full.bpc` for each aligned seed.
    ///
    /// `None` means the score inputs failed validation before a five-seed gap
    /// vector could be computed.
    pub gap: Option<[f64; 5]>,
    /// Structured refutation diagnostics.
    pub hits: Vec<DiagnosticHit>,
}

/// Per-seed score input consumed by H3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct H3Score {
    /// Seed for the S2 run.
    pub seed: u64,
    /// Validation bits per character for the relevant build.
    pub bpc: f64,
}

/// Per-seed H3 gap comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct H3PerSeed {
    /// Seed being compared.
    pub seed: u64,
    /// `s2_ternary_full.bpc - s2_fp_full.bpc`.
    pub gap_distill: f64,
    /// `s2_ternary_nodistill.bpc - s2_fp_full.bpc`.
    pub gap_nodistill: f64,
    /// Whether `gap_distill <= gap_nodistill + 0.10`.
    pub passes: bool,
}

/// Full H3 verifier result, including informational strong-form diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct H3Verification {
    /// Closure hypothesis status.
    pub status: HypothesisStatus,
    /// Whether every comparable seed passed the weak form.
    pub weak_form_passed: bool,
    /// Median `(gap_nodistill - gap_distill)`; positive means distillation helped.
    pub strong_form_observed: Option<f64>,
    /// Per-seed weak-form records.
    pub per_seed: Vec<H3PerSeed>,
}

/// Verify H1 from already-recorded phase-log header and JSONL-equivalent entries.
#[must_use]
pub fn verify_h1(
    phase_log: &PhaseLog,
    entries: &[PhaseEntry],
    build_kind: S2BuildKind,
) -> H1Verdict {
    let mut hits = Vec::new();
    let cfg = TrainConfigS2Full::pinned();
    let plan = phase_plan_for(build_kind);

    record_h1_check(
        "H1.a",
        "phase_log_header",
        None,
        header_matches(phase_log, entries, build_kind),
        json!({
            "schema": "s2_phase_log.v1",
            "build_kind": build_kind,
            "optimizer_steps": expected_phase_log_steps(build_kind),
            "teacher_freeze_step": S2_TEACHER_FREEZE_STEP,
            "full_s2_phase_boundaries": [4000, 5000, 8000, 10000],
        }),
        json!({
            "schema": phase_log.schema,
            "build_kind": phase_log.build_kind,
            "optimizer_steps": phase_log.optimizer_steps,
            "entry_count": entries.len(),
            "teacher_freeze_step": phase_log.teacher_freeze_step,
            "full_s2_phase_boundaries": phase_log.full_s2_phase_boundaries,
        }),
    );
    if !header_matches(phase_log, entries, build_kind) {
        hits.push(hit(
            "H1.a",
            "phase_log_header",
            HitLocation::for_build(build_kind),
            json!({
                "schema": "s2_phase_log.v1",
                "build_kind": build_kind,
                "optimizer_steps": expected_phase_log_steps(build_kind),
                "teacher_freeze_step": S2_TEACHER_FREEZE_STEP,
                "full_s2_phase_boundaries": [4000, 5000, 8000, 10000],
            }),
            json!({
                "schema": phase_log.schema,
                "build_kind": phase_log.build_kind,
                "optimizer_steps": phase_log.optimizer_steps,
                "entry_count": entries.len(),
                "teacher_freeze_step": phase_log.teacher_freeze_step,
                "full_s2_phase_boundaries": phase_log.full_s2_phase_boundaries,
            }),
            "Rebuild s2_phase_log.v1 header from RFC §9.1 PL-0..PL-3.",
        ));
    }

    push_first_h1_step_hit(
        &mut hits,
        "H1.b",
        "contiguous_steps",
        build_kind,
        entries
            .iter()
            .enumerate()
            .find(|(index, entry)| entry.step != *index as u64 + 1)
            .map(|(index, entry)| {
                (
                    entry.step,
                    json!(index as u64 + 1),
                    json!(entry.step),
                    "Regenerate ordered JSONL entries with 1-indexed contiguous steps.",
                )
            }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.c",
        "finite_train_loss",
        build_kind,
        entries
            .iter()
            .find(|entry| !entry.train_loss.is_finite())
            .map(|entry| {
                (
                    entry.step,
                    json!("finite f32"),
                    json_f32(entry.train_loss),
                    "Fix the training loop diagnostic writer; train_loss must be finite.",
                )
            }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.d",
        "finite_grad_norm",
        build_kind,
        entries
            .iter()
            .find(|entry| !entry.grad_norm.is_finite() || entry.grad_norm < 0.0)
            .map(|entry| {
                (
                    entry.step,
                    json!("finite non-negative f32"),
                    json_f32(entry.grad_norm),
                    "Fix gradient norm capture before using the run for S2 closure.",
                )
            }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.e",
        "phase_schedule",
        build_kind,
        entries.iter().find_map(|entry| {
            phase_for_global_step(entry.step, &plan)
                .ok()
                .filter(|expected| *expected != entry.phase)
                .map(|expected| {
                    (
                        entry.step,
                        json!(expected),
                        json!(entry.phase),
                        "Use the D1 Phase A/B/C/D schedule from RFC §3.",
                    )
                })
        }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.f",
        "phase_transition_events",
        build_kind,
        entries.iter().find_map(|entry| {
            let expected = expected_transition_event(entry.step, &plan);
            let observed = observed_transition_events(entry);
            (expected != observed).then(|| {
                (
                    entry.step,
                    json!(expected),
                    json!(observed),
                    "Emit exactly one PhaseTransition at each D1 boundary and nowhere else.",
                )
            })
        }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.g",
        "teacher_freeze",
        build_kind,
        entries.iter().find_map(|entry| {
            let expected_frozen =
                build_kind != S2BuildKind::s2_ablation && entry.step > S2_TEACHER_FREEZE_STEP;
            let expected_event_count = u64::from(
                build_kind != S2BuildKind::s2_ablation && entry.step == S2_TEACHER_FREEZE_STEP + 1,
            );
            let observed_event_count = entry
                .events
                .iter()
                .filter(|event| matches!(event, PhaseEvent::TeacherFreeze { .. }))
                .count() as u64;
            (entry.teacher_frozen != expected_frozen
                || observed_event_count != expected_event_count)
                .then(|| {
                    (
                        entry.step,
                        json!({
                            "teacher_frozen": expected_frozen,
                            "teacher_freeze_event_count": expected_event_count,
                        }),
                        json!({
                            "teacher_frozen": entry.teacher_frozen,
                            "teacher_freeze_event_count": observed_event_count,
                        }),
                        "Freeze the teacher at the Phase A/B boundary, step 4001.",
                    )
                })
        }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.h",
        "d2_hardness_sequence",
        build_kind,
        entries.iter().find_map(|entry| {
            hardness_for_global_step(
                entry.step,
                &plan,
                quant_hardness_override_for_build_kind(build_kind),
            )
            .ok()
            .filter(|expected| *expected != entry.hardness)
            .map(|expected| {
                (
                    entry.step,
                    json!(expected),
                    json!(entry.hardness),
                    "Apply the full D2 HardnessTriple sequence after build-kind override.",
                )
            })
        }),
    );

    push_first_h1_step_hit(
        &mut hits,
        "H1.i",
        "phase_effective_lambdas",
        build_kind,
        entries.iter().find_map(|entry| {
            phase_effective_lambdas(entry.step, build_kind, &cfg)
                .ok()
                .filter(|expected| !lambdas_match(expected, &entry.lambda_effective))
                .map(|expected| {
                    (
                        entry.step,
                        json!(expected),
                        json!(entry.lambda_effective),
                        "Recompute phase-effective lambdas from build kind and D1/D2 phase state.",
                    )
                })
        }),
    );

    emit_h1_surprises(entries);
    emit_hits(&hits);

    let status = if hits.is_empty() {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h1_verdict",
        status = status_label(&status),
        hit_count = hits.len() as u32,
        "s2 H1 verifier verdict"
    );
    H1Verdict { status, hits }
}

/// Verify H2 over five aligned ternary-full and full-precision score reports.
#[must_use]
pub fn verify_h2(scores_t: &[S2ScoreReport; 5], scores_f: &[S2ScoreReport; 5]) -> H2Verdict {
    let mut hits = Vec::new();
    let gap = match try_gap_ternary_vs_fp(scores_t, scores_f) {
        Ok(observed_gap) => Some(observed_gap),
        Err(error) => {
            hits.push(h2_gap_input_hit(error));
            None
        }
    };

    if hits.is_empty() {
        let gap = gap.expect("gap input validation passed");
        for index in 0..5 {
            let seed = scores_t[index].seed;
            if gap[index] > H2_MAX_TERNARY_FP_GAP_BPC {
                hits.push(hit(
                    "H2.a",
                    "per_seed_gap",
                    HitLocation::for_seed(S2BuildKind::s2_ternary_full, seed),
                    json!({ "max_gap_bpc": H2_MAX_TERNARY_FP_GAP_BPC }),
                    json!({ "gap_bpc": gap[index] }),
                    "Investigate the matched-protocol ternary-vs-fp regression for this seed.",
                ));
            }
            if scores_f[index].bpc > H2_MAX_FP_BPC {
                hits.push(hit(
                    "H2.b",
                    "fp_quality",
                    HitLocation::for_seed(S2BuildKind::s2_fp_full, scores_f[index].seed),
                    json!({ "max_bpc": H2_MAX_FP_BPC }),
                    json!({ "bpc": scores_f[index].bpc }),
                    "Reject the run; full-precision quality failed the H2 absolute sanity gate.",
                ));
            }
            if scores_t[index].bpc > H2_MAX_TERNARY_BPC {
                hits.push(hit(
                    "H2.c",
                    "ternary_quality",
                    HitLocation::for_seed(S2BuildKind::s2_ternary_full, seed),
                    json!({ "max_bpc": H2_MAX_TERNARY_BPC }),
                    json!({ "bpc": scores_t[index].bpc }),
                    "Reject the run; ternary quality failed the H2 absolute sanity gate.",
                ));
            }
        }

        let median_fp = median_f64(scores_f.iter().map(|score| score.bpc).collect());
        if median_fp < H2_SUSPICIOUS_LOW_MEDIAN_FP_BPC {
            hits.push(hit(
                "H2.d",
                "suspicious_low_median_fp",
                HitLocation::for_build(S2BuildKind::s2_fp_full),
                json!({ "min_median_bpc": H2_SUSPICIOUS_LOW_MEDIAN_FP_BPC }),
                json!({ "median_bpc": median_fp }),
                "Route to the suspicious outcome layer for split/scoring audit.",
            ));
        }

        let median_ternary = median_f64(scores_t.iter().map(|score| score.bpc).collect());
        if median_ternary < H2_SUSPICIOUS_LOW_MEDIAN_TERNARY_BPC {
            hits.push(hit(
                "H2.e",
                "suspicious_low_median_ternary",
                HitLocation::for_build(S2BuildKind::s2_ternary_full),
                json!({ "min_median_bpc": H2_SUSPICIOUS_LOW_MEDIAN_TERNARY_BPC }),
                json!({ "median_bpc": median_ternary }),
                "Route to the suspicious outcome layer for split/scoring audit.",
            ));
        }
    }

    emit_hits(&hits);
    let status = if hits.is_empty() {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h2_verdict",
        status = status_label(&status),
        hit_count = hits.len() as u32,
        gaps = ?gap,
        "s2 H2 verifier verdict"
    );
    H2Verdict { status, gap, hits }
}

/// Verify H3 using ternary-full, fp-full, and ternary-nodistill score sets.
pub fn verify_h3(
    scores_t: &[H3Score],
    scores_f: &[H3Score],
    scores_nd: &[H3Score],
) -> Result<H3Verification, H3VerifierError> {
    let ternary = score_map("s2_ternary_full", scores_t)?;
    let fp = score_map("s2_fp_full", scores_f)?;
    let nodistill = score_map("s2_ternary_nodistill", scores_nd)?;

    if nodistill.is_empty() {
        let status = HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "nodistill control absent".to_owned(),
        };
        emit_h3_verdict(&status, false, None);
        return Ok(H3Verification {
            status,
            weak_form_passed: false,
            strong_form_observed: None,
            per_seed: Vec::new(),
        });
    }

    let mut per_seed = Vec::new();
    for (&seed, &ternary_bpc) in &ternary {
        let fp_bpc = *fp.get(&seed).ok_or(H3VerifierError::MissingScore {
            build: "s2_fp_full",
            seed,
        })?;
        let Some(&nodistill_bpc) = nodistill.get(&seed) else {
            let status = HypothesisStatus::NotEvaluatedDueToPriorGate {
                reason: format!("nodistill control absent for seed {seed}"),
            };
            emit_h3_verdict(&status, false, None);
            return Ok(H3Verification {
                status,
                weak_form_passed: false,
                strong_form_observed: None,
                per_seed,
            });
        };

        let gap_distill = ternary_bpc - fp_bpc;
        let gap_nodistill = nodistill_bpc - fp_bpc;
        let passes = gap_distill <= gap_nodistill + H3_WEAK_TOLERANCE_BPC;
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "h3_per_seed",
            seed,
            gap_distill,
            gap_nodistill,
            tolerance = H3_WEAK_TOLERANCE_BPC,
            passes,
        );
        per_seed.push(H3PerSeed {
            seed,
            gap_distill,
            gap_nodistill,
            passes,
        });
    }

    let weak_form_passed = !per_seed.is_empty() && per_seed.iter().all(|entry| entry.passes);
    let strong_form_observed = median_delta(&per_seed);
    let status = if weak_form_passed {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    emit_h3_verdict(&status, weak_form_passed, strong_form_observed);

    Ok(H3Verification {
        status,
        weak_form_passed,
        strong_form_observed,
        per_seed,
    })
}

fn header_matches(phase_log: &PhaseLog, entries: &[PhaseEntry], build_kind: S2BuildKind) -> bool {
    phase_log.schema == "s2_phase_log.v1"
        && phase_log.build_kind == build_kind
        && phase_log.full_s2_phase_boundaries == [4_000, 5_000, 8_000, 10_000]
        && phase_log.teacher_freeze_step == S2_TEACHER_FREEZE_STEP
        && phase_log.optimizer_steps == expected_phase_log_steps(build_kind)
        && entries.len() as u64 == expected_phase_log_steps(build_kind)
}

fn expected_phase_log_steps(build_kind: S2BuildKind) -> u64 {
    if build_kind == S2BuildKind::s2_ablation {
        S2_TEACHER_FREEZE_STEP
    } else {
        S2_OPTIMIZER_STEPS
    }
}

fn phase_plan_for(build_kind: S2BuildKind) -> PhasePlan {
    if build_kind == S2BuildKind::s2_ablation {
        PhasePlan::phase_a_only()
    } else {
        PhasePlan::full_s2()
    }
}

fn expected_transition_event(
    step: GlobalStep,
    plan: &PhasePlan,
) -> Vec<(PhaseKindS2, PhaseKindS2)> {
    transition_for_step(step, plan).into_iter().collect()
}

fn observed_transition_events(entry: &PhaseEntry) -> Vec<(PhaseKindS2, PhaseKindS2)> {
    entry
        .events
        .iter()
        .filter_map(|event| match event {
            PhaseEvent::PhaseTransition { from, to } => Some((*from, *to)),
            PhaseEvent::TeacherFreeze { .. } => None,
        })
        .collect()
}

fn push_first_h1_step_hit(
    hits: &mut Vec<DiagnosticHit>,
    hypothesis_id: &'static str,
    check_name: &'static str,
    build_kind: S2BuildKind,
    failed: Option<(u64, Value, Value, &'static str)>,
) {
    match failed {
        Some((step, expected, observed, remediation)) => {
            record_h1_check(
                hypothesis_id,
                check_name,
                Some(step),
                false,
                expected.clone(),
                observed.clone(),
            );
            hits.push(hit(
                hypothesis_id,
                check_name,
                HitLocation::for_step(build_kind, step),
                expected,
                observed,
                remediation,
            ));
        }
        None => record_h1_check(
            hypothesis_id,
            check_name,
            None,
            true,
            json!("check passed"),
            json!("check passed"),
        ),
    }
}

fn hit(
    hypothesis_id: &'static str,
    check_name: &'static str,
    location: HitLocation,
    expected: Value,
    observed: Value,
    remediation: &'static str,
) -> DiagnosticHit {
    DiagnosticHit {
        hypothesis_id,
        check_name,
        step: location.step,
        seed: location.seed,
        build_kind: location.build_kind,
        expected,
        observed,
        remediation,
    }
}

#[derive(Debug, Clone, Copy)]
struct HitLocation {
    step: Option<u64>,
    seed: Option<u64>,
    build_kind: Option<S2BuildKind>,
}

impl HitLocation {
    const fn for_build(build_kind: S2BuildKind) -> Self {
        Self {
            step: None,
            seed: None,
            build_kind: Some(build_kind),
        }
    }

    const fn for_step(build_kind: S2BuildKind, step: u64) -> Self {
        Self {
            step: Some(step),
            seed: None,
            build_kind: Some(build_kind),
        }
    }

    const fn for_seed(build_kind: S2BuildKind, seed: u64) -> Self {
        Self {
            step: None,
            seed: Some(seed),
            build_kind: Some(build_kind),
        }
    }

    const fn for_untyped_seed(seed: Option<u64>) -> Self {
        Self {
            step: None,
            seed,
            build_kind: None,
        }
    }
}

fn emit_hits(hits: &[DiagnosticHit]) {
    for hit in hits {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "diagnostic_hit",
            hypothesis_id = hit.hypothesis_id,
            check_name = hit.check_name,
            step = ?hit.step,
            seed = ?hit.seed,
            build_kind = ?hit.build_kind,
            expected = %hit.expected,
            observed = %hit.observed,
            remediation = hit.remediation,
            "s2 diagnostic hit"
        );
    }
}

fn record_h1_check(
    hypothesis_id: &'static str,
    check_name: &'static str,
    step: Option<u64>,
    passed: bool,
    expected: Value,
    observed: Value,
) {
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "h1_check",
        hypothesis_id,
        check = check_name,
        passed,
        step = ?step,
        expected = %expected,
        observed = %observed,
        "s2 H1 verifier check"
    );
}

fn emit_h1_surprises(entries: &[PhaseEntry]) {
    // Surprise WARNs are intentionally emitted before diagnostic_hit ERRORs so
    // dashboards can show non-refuting anomalies before hard refutations.
    for entry in entries
        .iter()
        .filter(|entry| entry.grad_norm > H1_GRAD_NORM_SPIKE_THRESHOLD)
    {
        tracing::warn!(
            target: S2_LOG_TARGET,
            event_name = "h1_surprise",
            check = "grad_norm_spike",
            step = entry.step,
            grad_norm = entry.grad_norm,
            threshold = H1_GRAD_NORM_SPIKE_THRESHOLD,
            "s2 H1 non-refuting surprise"
        );
    }

    let early_losses = entries
        .iter()
        .take(10)
        .map(|entry| entry.train_loss)
        .collect::<Vec<_>>();
    if early_losses.len() == 10 && early_losses.iter().all(|value| value.is_finite()) {
        let mean = early_losses.iter().sum::<f32>() / early_losses.len() as f32;
        if !(H1_MEAN_TRAIN_LOSS_MIN..=H1_MEAN_TRAIN_LOSS_MAX).contains(&mean) {
            tracing::warn!(
                target: S2_LOG_TARGET,
                event_name = "h1_surprise",
                check = "mean_train_loss",
                step = 10_u64,
                mean_train_loss = mean,
                expected_min = H1_MEAN_TRAIN_LOSS_MIN,
                expected_max = H1_MEAN_TRAIN_LOSS_MAX,
                "s2 H1 non-refuting surprise"
            );
        }
    }
}

fn lambdas_match(expected: &PhaseEffectiveLambda, observed: &PhaseEffectiveLambda) -> bool {
    lambda_close(expected.lambda_distill, observed.lambda_distill)
        && lambda_close(expected.lambda_balance, observed.lambda_balance)
        && lambda_close(expected.lambda_zrouter, observed.lambda_zrouter)
        && lambda_close(expected.lambda_switch, observed.lambda_switch)
        && lambda_close(expected.lambda_range, observed.lambda_range)
        && lambda_close(expected.lambda_zero, observed.lambda_zero)
        && lambda_close(expected.lambda_shape, observed.lambda_shape)
        && lambda_close(expected.lambda_overflow, observed.lambda_overflow)
}

fn lambda_close(left: f32, right: f32) -> bool {
    (left - right).abs() <= 1.0e-7
}

fn json_f32(value: f32) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

fn h2_gap_input_hit(error: S2GapError) -> DiagnosticHit {
    let (seed, observed) = match &error {
        S2GapError::SeedAlignment {
            index,
            expected_seed,
            got_seed,
        } => (
            Some(*expected_seed),
            json!({
                "error": error.to_string(),
                "index": index,
                "expected_seed": expected_seed,
                "got_seed": got_seed,
            }),
        ),
        S2GapError::BuildKindMismatch {
            seed,
            expected,
            got,
        } => (
            Some(*seed),
            json!({
                "error": error.to_string(),
                "expected": expected,
                "got": got,
            }),
        ),
        S2GapError::NonFiniteBpc {
            seed,
            ternary_bpc,
            fp_bpc,
        }
        | S2GapError::NonFiniteGap {
            seed,
            ternary_bpc,
            fp_bpc,
        } => (
            Some(*seed),
            json!({
                "error": error.to_string(),
                "ternary_bpc": json_f64(*ternary_bpc),
                "fp_bpc": json_f64(*fp_bpc),
            }),
        ),
    };
    hit(
        "H2.input",
        "gap_inputs",
        HitLocation::for_untyped_seed(seed),
        json!("five seed-aligned finite s2_ternary_full and s2_fp_full score reports"),
        observed,
        "Fix score artifact alignment before evaluating H2.",
    )
}

fn json_f64(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

fn median_f64(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn score_map(
    build: &'static str,
    scores: &[H3Score],
) -> Result<BTreeMap<u64, f64>, H3VerifierError> {
    let mut map = BTreeMap::new();
    for score in scores {
        if !score.bpc.is_finite() {
            return Err(H3VerifierError::NonFiniteBpc {
                build,
                seed: score.seed,
                value: score.bpc,
            });
        }
        if map.insert(score.seed, score.bpc).is_some() {
            return Err(H3VerifierError::DuplicateScore {
                build,
                seed: score.seed,
            });
        }
    }
    Ok(map)
}

fn median_delta(per_seed: &[H3PerSeed]) -> Option<f64> {
    if per_seed.is_empty() {
        return None;
    }

    let mut deltas = per_seed
        .iter()
        .map(|entry| entry.gap_nodistill - entry.gap_distill)
        .collect::<Vec<_>>();
    deltas.sort_by(f64::total_cmp);
    let middle = deltas.len() / 2;
    if deltas.len() % 2 == 0 {
        Some((deltas[middle - 1] + deltas[middle]) / 2.0)
    } else {
        Some(deltas[middle])
    }
}

fn emit_h3_verdict(
    status: &HypothesisStatus,
    weak_form_passed: bool,
    strong_form_observed: Option<f64>,
) {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h3_verdict",
        status = status_label(status),
        weak_form_passed,
        strong_form_observed_present = strong_form_observed.is_some(),
        strong_form_observed = strong_form_observed.unwrap_or(0.0),
    );
}

fn status_label(status: &HypothesisStatus) -> &'static str {
    match status {
        HypothesisStatus::Confirmed => "Confirmed",
        HypothesisStatus::Refuted => "Refuted",
        HypothesisStatus::NotEvaluatedDueToPriorGate { .. } => "NotEvaluatedDueToPriorGate",
    }
}

/// Errors returned by the scalar H3 verifier before a hypothesis can be judged.
#[derive(Debug, Clone, PartialEq)]
pub enum H3VerifierError {
    /// A score was not finite.
    NonFiniteBpc {
        /// Build containing the invalid score.
        build: &'static str,
        /// Seed containing the invalid score.
        seed: u64,
        /// Observed value.
        value: f64,
    },
    /// A score set contained duplicate entries for one seed.
    DuplicateScore {
        /// Build containing the duplicate score.
        build: &'static str,
        /// Duplicate seed.
        seed: u64,
    },
    /// A required comparator score was missing.
    MissingScore {
        /// Missing build.
        build: &'static str,
        /// Seed whose score was missing.
        seed: u64,
    },
}

impl fmt::Display for H3VerifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteBpc { build, seed, value } => {
                write!(f, "{build} seed {seed} bpc must be finite, got {value}")
            }
            Self::DuplicateScore { build, seed } => {
                write!(f, "{build} contains duplicate score for seed {seed}")
            }
            Self::MissingScore { build, seed } => {
                write!(f, "{build} score is missing for seed {seed}")
            }
        }
    }
}

impl Error for H3VerifierError {}
