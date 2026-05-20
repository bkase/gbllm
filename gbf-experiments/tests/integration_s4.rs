#![cfg(feature = "s4")]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::{BOS_ID, EOS_ID, GutenbergManifest, TextCharSeq};
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s4::baseline::{
    S4BaselineGutenbergReport, S4BaselineInputs, s4_fit_kn5_gutenberg,
};
use gbf_experiments::s4::contamination::{
    ContaminationOutcome, CrossCorpusInputs, CrossCorpusSplit, S4_CONTAMINATION_NGRAM_N,
    s4_cross_corpus_contamination,
};
use gbf_experiments::s4::corpus_progression::S4CorpusProgressionReport;
use gbf_experiments::s4::manifest::{GutenbergBuildOptions, build_gutenberg_corpus};
use gbf_experiments::s4::promote::{
    PromotionGateArtifactRef, PromotionGateBoundArtifact, PromotionGateInputs,
    PromotionGateOutcome, S3CheckpointPromotionArtifact, S3OracleAgreementOutcome,
    S3OracleAgreementPromotionArtifact, S3RepetitionCollapseOutcome,
    S3RepetitionCollapsePromotionArtifact, S3V0SuccessPromotionArtifact,
    S4BaselineGutenbergPromotionArtifact, S4ContaminationOutcome, S4ContaminationPromotionArtifact,
    V0SuccessAcceptanceBits, V0SuccessGateOutcome, promotion_gate,
};
use gbf_experiments::s4::run::{
    S4ContinuationInitInputs, S4SeedRunLoop, initialize_gutenberg_continuation,
};
use gbf_experiments::s4::schema::{
    S4_CANONICAL_SEEDS, S4_OPTIMIZER_STEPS_GUTENBERG, S4TrainConfig, S4TrainPhase,
};
use gbf_experiments::s4::score::{
    S4BpcValue, S4InheritedV0SuccessBits, S4StrictV0SuccessOutcome, S4V0SuccessInputs,
    s4_v0_success_gutenberg, strict_v0_success_on_gutenberg,
};
use gbf_foundation::{Hash256, sha256};
use serde_json::json;

const SMOKE_MANIFEST: &str = "fixtures/corpora/gutenberg_smoke.toml";
const TINYSTORIES_MANIFEST: &str = "fixtures/corpora/tinystories.toml";
const S4_INTEGRATION_STARTED_EVENT: &str = "s4_integration_started";
const S4_INTEGRATION_STEP_EVENT: &str = "s4_integration_step";
const S4_INTEGRATION_FINALIZED_EVENT: &str = "s4_integration_finalized";

const S4_INTEGRATION_OWNER_ROUTES: [S4IntegrationOwnerRoute; 3] = [
    S4IntegrationOwnerRoute {
        step: "ablation_compile_check",
        owner_bead: "bd-u6tn",
        rationale: "closure packet records the amended ablation evidence route",
    },
    S4IntegrationOwnerRoute {
        step: "falsification_suite",
        owner_bead: "bd-fii7",
        rationale: "F1-broken-S4 through F6-broken-S4 are verified by the falsification suite",
    },
    S4IntegrationOwnerRoute {
        step: "determinism_replay",
        owner_bead: "bd-14ln",
        rationale: "determinism transcript and replay gate are owned outside the smoke integration test",
    },
];

const REQUIRED_CLOSURE_PACKET_TERMS: [&str; 33] = [
    "predictions_section_hash",
    "tinystories_manifest_self_hash",
    "gutenberg_manifest_self_hash",
    "baseline_gutenberg_self_hash",
    "promotion_gate_self_hash",
    "contamination_self_hash",
    "oracle_agreement_self_hash",
    "report_self_hash",
    "seed_0_score_self_hash",
    "seed_1_score_self_hash",
    "seed_2_score_self_hash",
    "seed_3_score_self_hash",
    "seed_4_score_self_hash",
    "H1=Confirmed",
    "H2=Confirmed",
    "H3=Confirmed",
    "H4=Confirmed",
    "H5=Confirmed",
    "H6=Confirmed",
    "H7=Confirmed",
    "F1-broken-S4=Refuted",
    "F2-broken-S4=Refuted",
    "F3-broken-S4=Refuted",
    "F4-broken-S4=Refuted",
    "F5-broken-S4=Refuted",
    "F6-broken-S4=Refuted",
    "determinism_transcript",
    "integration_s4=passed",
    "s4_verifier=passed",
    "s4_report=passed",
    "s4_falsification=passed",
    "s4_corpus_oracle=passed",
    "s4_determinism_check=passed",
];

#[derive(Debug)]
struct S4IntegrationSmokeEvidence {
    tinystories_manifest_self_hash: Hash256,
    gutenberg_manifest_self_hash: Hash256,
    baseline_gutenberg_self_hash: Hash256,
    contamination_self_hash: Hash256,
    promotion_gate_self_hash: Hash256,
    corpus_progression_self_hash: Hash256,
    score_seed_count: usize,
    train_token_count: usize,
    val_token_count: usize,
    owner_routes: &'static [S4IntegrationOwnerRoute],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct S4IntegrationOwnerRoute {
    step: &'static str,
    owner_bead: &'static str,
    rationale: &'static str,
}

#[test]
fn integration_s4_smoke_wires_current_closure_substrate() {
    let capture = TraceCapture::default();

    let evidence = with_trace_capture(&capture, run_integration_smoke);

    assert_eq!(
        evidence.tinystories_manifest_self_hash,
        hash(9),
        "integration smoke pins the TinyStories fixture hash"
    );
    assert_ne!(evidence.gutenberg_manifest_self_hash, Hash256::ZERO);
    assert_ne!(evidence.baseline_gutenberg_self_hash, Hash256::ZERO);
    assert_ne!(evidence.contamination_self_hash, Hash256::ZERO);
    assert_ne!(evidence.promotion_gate_self_hash, Hash256::ZERO);
    assert_ne!(evidence.corpus_progression_self_hash, Hash256::ZERO);
    assert_eq!(evidence.score_seed_count, S4_CANONICAL_SEEDS.len());
    assert!(
        evidence.train_token_count > 0 && evidence.val_token_count > 0,
        "smoke corpus should produce non-empty train and val token streams"
    );
    assert_eq!(evidence.owner_routes, integration_owner_routes());

    assert_integration_events(&captured_events(&capture), &evidence);
}

#[test]
fn integration_s4_owner_routes_amended_scope_without_claiming_them() {
    let routes = integration_owner_routes();
    assert_eq!(routes.len(), 3);
    assert_eq!(
        routes
            .iter()
            .map(|route| (route.step, route.owner_bead))
            .collect::<Vec<_>>(),
        vec![
            ("ablation_compile_check", "bd-u6tn"),
            ("falsification_suite", "bd-fii7"),
            ("determinism_replay", "bd-14ln"),
        ]
    );
    assert!(routes.iter().all(|route| {
        !route.step.is_empty() && !route.owner_bead.is_empty() && !route.rationale.is_empty()
    }));
}

#[test]
fn integration_s4_closure_packet_linter_requires_named_entries() {
    let packet = REQUIRED_CLOSURE_PACKET_TERMS.join("\n");
    lint_s4_closure_packet(&packet).expect("complete packet passes");

    let incomplete = packet.replace("report_self_hash", "report_hash_missing");
    let missing = lint_s4_closure_packet(&incomplete).expect_err("missing packet entry fails");
    assert_eq!(missing, vec!["report_self_hash"]);
}

fn run_integration_smoke() -> S4IntegrationSmokeEvidence {
    emit_integration_started();

    let temp = tempfile::tempdir().expect("tempdir");
    let root = workspace_root();
    let tinystories_manifest_self_hash = hash(9);
    let (manifest, train, val, baseline) =
        build_fixture_corpus_and_baseline(&root, temp.path(), tinystories_manifest_self_hash);
    emit_integration_step(
        "gutenberg_manifest_and_kn5_baseline",
        "wired",
        "bd-29lv/bd-2nca",
        "smoke Gutenberg corpus and KN-5 baseline validated",
    );

    let contamination = s4_cross_corpus_contamination(clean_contamination_inputs(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
    ))
    .expect("clean contamination report builds");
    assert_eq!(contamination.outcome, ContaminationOutcome::Clean);
    contamination
        .validate_canonical_write()
        .expect("contamination report validates");
    emit_integration_step(
        "cross_corpus_contamination",
        "wired",
        "bd-2p3n",
        "clean contamination report validated",
    );

    let promotion = promotion_gate(positive_promotion_inputs(
        tinystories_manifest_self_hash,
        &manifest,
        &baseline,
        S4ContaminationOutcome::Clean,
    ))
    .expect("promotion gate evaluates");
    assert!(
        matches!(promotion.outcome, PromotionGateOutcome::Promoted { .. }),
        "current smoke substrate should reach promoted state"
    );
    promotion
        .validate_canonical_write()
        .expect("promotion gate validates");
    emit_integration_step(
        "promotion_gate",
        "wired",
        "bd-13sa",
        "promotion gate reached promoted state and validated canonical write",
    );

    let progression = S4CorpusProgressionReport::new(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
        None,
    )
    .expect("corpus progression report builds");
    let bound_promotion = promotion
        .with_corpus_progression_self_hash(progression.corpus_progression_self_hash)
        .expect("promotion binds progression hash");
    let bound_progression = progression
        .with_bound_promotion_gate(bound_promotion.promotion_gate_self_hash)
        .expect("progression binds promotion hash");
    bound_progression
        .validate_promotion_gate_binding(&bound_promotion)
        .expect("progression and promotion mutually bind");
    emit_integration_step(
        "corpus_progression_binding",
        "wired",
        "bd-2gpf",
        "promotion gate and corpus progression self-hashes mutually bind",
    );

    assert_five_seed_phase_d_run_loop(&bound_promotion.promotion_gate_self_hash);
    emit_integration_step(
        "phase_d_run_loop",
        "wired",
        "bd-slik",
        "all canonical seeds reach the Gutenberg optimizer-step budget",
    );

    assert_strict_five_seed_score_passes(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
        sha256(val.as_slice()),
    );
    emit_integration_step(
        "strict_five_seed_score",
        "wired",
        "bd-p1rt",
        "all canonical seed score artifacts satisfy the strict v0 success gate",
    );

    for route in integration_owner_routes() {
        emit_integration_step(
            route.step,
            "owner_routed",
            route.owner_bead,
            route.rationale,
        );
    }

    emit_integration_finalized();

    S4IntegrationSmokeEvidence {
        tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: manifest.manifest_self_hash,
        baseline_gutenberg_self_hash: baseline.baseline_gutenberg_self_hash,
        contamination_self_hash: contamination.contamination_self_hash,
        promotion_gate_self_hash: bound_promotion.promotion_gate_self_hash,
        corpus_progression_self_hash: bound_progression.corpus_progression_self_hash,
        score_seed_count: S4_CANONICAL_SEEDS.len(),
        train_token_count: train.as_slice().len(),
        val_token_count: val.as_slice().len(),
        owner_routes: integration_owner_routes(),
    }
}

fn integration_owner_routes() -> &'static [S4IntegrationOwnerRoute] {
    &S4_INTEGRATION_OWNER_ROUTES
}

fn lint_s4_closure_packet(packet: &str) -> Result<(), Vec<&'static str>> {
    let missing = REQUIRED_CLOSURE_PACKET_TERMS
        .into_iter()
        .filter(|term| !packet.contains(term))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

fn emit_integration_started() {
    tracing::info!(
        target: gbf_experiments::S4_LOG_TARGET,
        event_name = S4_INTEGRATION_STARTED_EVENT,
        fixture = SMOKE_MANIFEST,
        canonical_seed_count = S4_CANONICAL_SEEDS.len() as u64,
        "S4 integration smoke started"
    );
}

fn emit_integration_step(
    step: &'static str,
    status: &'static str,
    owner_bead: &'static str,
    detail: &'static str,
) {
    tracing::info!(
        target: gbf_experiments::S4_LOG_TARGET,
        event_name = S4_INTEGRATION_STEP_EVENT,
        step,
        status,
        owner_bead,
        detail,
        "S4 integration smoke step"
    );
}

fn emit_integration_finalized() {
    tracing::info!(
        target: gbf_experiments::S4_LOG_TARGET,
        event_name = S4_INTEGRATION_FINALIZED_EVENT,
        outcome = "pass",
        reason = "smoke closure substrate wired; ablation, falsification, and determinism are owner-routed",
        wired_step_count = 6_u64,
        owner_routed_step_count = S4_INTEGRATION_OWNER_ROUTES.len() as u64,
        "S4 integration smoke finalized"
    );
}

fn assert_integration_events(
    events: &[common::tracing_capture::TracingEvent],
    evidence: &S4IntegrationSmokeEvidence,
) {
    let started = events
        .iter()
        .filter(|event| event.name == S4_INTEGRATION_STARTED_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 1);
    assert_eq!(
        started[0].fields.get("fixture"),
        Some(&json!(SMOKE_MANIFEST))
    );
    assert_eq!(
        started[0].fields.get("canonical_seed_count"),
        Some(&json!(S4_CANONICAL_SEEDS.len() as u64))
    );

    let steps = events
        .iter()
        .filter(|event| event.name == S4_INTEGRATION_STEP_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), 6 + evidence.owner_routes.len());
    let wired_steps = steps
        .iter()
        .filter(|event| event.fields.get("status") == Some(&json!("wired")))
        .filter_map(|event| event.fields.get("step").and_then(|value| value.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        wired_steps,
        BTreeSet::from([
            "gutenberg_manifest_and_kn5_baseline",
            "cross_corpus_contamination",
            "promotion_gate",
            "corpus_progression_binding",
            "phase_d_run_loop",
            "strict_five_seed_score",
        ])
    );

    for route in evidence.owner_routes {
        let event = steps
            .iter()
            .find(|event| {
                event.fields.get("step") == Some(&json!(route.step))
                    && event.fields.get("status") == Some(&json!("owner_routed"))
            })
            .unwrap_or_else(|| panic!("missing owner-routed integration step {}", route.step));
        assert_eq!(
            event.fields.get("owner_bead"),
            Some(&json!(route.owner_bead))
        );
        assert_eq!(event.fields.get("detail"), Some(&json!(route.rationale)));
    }

    let finalized = events
        .iter()
        .filter(|event| event.name == S4_INTEGRATION_FINALIZED_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].fields.get("outcome"), Some(&json!("pass")));
    assert_eq!(
        finalized[0].fields.get("wired_step_count"),
        Some(&json!(6_u64))
    );
    assert_eq!(
        finalized[0].fields.get("owner_routed_step_count"),
        Some(&json!(evidence.owner_routes.len() as u64))
    );
    assert!(
        finalized[0]
            .fields
            .get("reason")
            .and_then(|value| value.as_str())
            .is_some_and(|reason| reason.contains("owner-routed"))
    );
}

fn build_fixture_corpus_and_baseline(
    root: &Path,
    temp: &Path,
    tinystories_manifest_self_hash: Hash256,
) -> (
    GutenbergManifest,
    TextCharSeq,
    TextCharSeq,
    S4BaselineGutenbergReport,
) {
    let manifest_path = temp.join("gutenberg-manifest.json");
    let train_path = temp.join("gutenberg-train.bin");
    let val_path = temp.join("gutenberg-val.bin");
    let quality_path = temp.join("corpus-quality.json");
    let summary = build_gutenberg_corpus(&GutenbergBuildOptions {
        fixture_path: root.join(SMOKE_MANIFEST),
        manifest_path: manifest_path.clone(),
        train_path: train_path.clone(),
        val_path: val_path.clone(),
        corpus_quality_path: Some(quality_path),
        tinystories_manifest_path: Some(root.join(TINYSTORIES_MANIFEST)),
    })
    .expect("smoke fixture corpus builds");

    let manifest: GutenbergManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("manifest reads"))
            .expect("manifest parses");
    assert_eq!(manifest.manifest_self_hash, summary.manifest_self_hash);
    assert_eq!(manifest.train_book_count, 7);
    assert_eq!(manifest.val_book_count, 1);

    let train = text_seq_without_boundaries(std::fs::read(&train_path).expect("train reads"));
    let val = text_seq_without_boundaries(std::fs::read(&val_path).expect("val reads"));
    let (baseline_train, baseline_val) = baseline_oracle_sequences(root);
    let baseline = s4_fit_kn5_gutenberg(S4BaselineInputs {
        tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: manifest.manifest_self_hash,
        corpus_train_sha: sha256(baseline_train.as_slice()),
        corpus_val_sha: sha256(baseline_val.as_slice()),
        corpus_train: baseline_train,
        corpus_val: baseline_val,
    })
    .expect("Gutenberg KN baseline fits");
    baseline
        .validate_canonical_write()
        .expect("baseline validates");

    (manifest, train, val, baseline)
}

fn clean_contamination_inputs(
    tinystories_manifest_self_hash: Hash256,
    gutenberg_manifest_self_hash: Hash256,
) -> CrossCorpusInputs {
    CrossCorpusInputs {
        tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash,
        ts_train: CrossCorpusSplit::from_fixture_documents(vec![window_doc(1)]),
        ts_val: CrossCorpusSplit::from_fixture_documents(vec![window_doc(20)]),
        gb_train: CrossCorpusSplit::from_fixture_documents(vec![window_doc(40)]),
        gb_val: CrossCorpusSplit::from_fixture_documents(vec![window_doc(60)]),
    }
}

fn positive_promotion_inputs(
    tinystories_manifest_self_hash: Hash256,
    manifest: &GutenbergManifest,
    baseline: &S4BaselineGutenbergReport,
    contamination_outcome: S4ContaminationOutcome,
) -> PromotionGateInputs {
    let checkpoint = S3CheckpointPromotionArtifact::new("phase_d_resumable", true, true)
        .expect("checkpoint summary builds");
    let v0_success = S3V0SuccessPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        V0SuccessAcceptanceBits::all_pass(),
        V0SuccessGateOutcome::Pass,
        0.25,
    )
    .expect("v0_success summary builds");
    let oracle = S3OracleAgreementPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3OracleAgreementOutcome::Agree,
        true,
    )
    .expect("oracle summary builds");
    let contamination = S4ContaminationPromotionArtifact::new(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
        contamination_outcome,
    )
    .expect("contamination summary builds");
    let baseline_promotion = S4BaselineGutenbergPromotionArtifact::new(
        manifest.manifest_self_hash,
        manifest.train_sha256,
        manifest.val_sha256,
        baseline.bpc_kn5,
    )
    .expect("baseline promotion summary builds");
    let repetition = S3RepetitionCollapsePromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3RepetitionCollapseOutcome::Pass,
    )
    .expect("repetition summary builds");

    PromotionGateInputs {
        tinystories_manifest_self_hash,
        c_ts: bound(
            "experiments/S3/checkpoints/seed-0/checkpoint.json",
            checkpoint.checkpoint_self_hash,
            checkpoint,
        ),
        c_ts_v0success: Some(bound(
            "experiments/S3/v0_success/seed-0.json",
            v0_success.v0_success_self_hash,
            v0_success,
        )),
        c_ts_oracle_agreement: Some(bound(
            "experiments/S3/oracle_agreement/seed-0.json",
            oracle.oracle_agreement_self_hash,
            oracle,
        )),
        gb_manifest: bound(
            "experiments/S4/gutenberg_manifest.json",
            manifest.manifest_self_hash,
            manifest.clone(),
        ),
        contamination_report: bound(
            "experiments/S4/contamination/cross_corpus.json",
            contamination.contamination_self_hash,
            contamination,
        ),
        baseline_gutenberg: Some(bound(
            "experiments/S4/baseline/baseline_gutenberg.json",
            baseline_promotion.baseline_self_hash,
            baseline_promotion,
        )),
        repetition_collapse_check: bound(
            "experiments/S3/repetition/seed-0.json",
            repetition.repetition_self_hash,
            repetition,
        ),
    }
}

fn assert_five_seed_phase_d_run_loop(promotion_gate_self_hash: &Hash256) {
    let config = S4TrainConfig::pinned();
    let mut batch_rng_states = BTreeSet::new();
    for seed in S4_CANONICAL_SEEDS {
        let init = initialize_gutenberg_continuation(&S4ContinuationInitInputs {
            seed,
            train_config: config.clone(),
            c_ts_checkpoint_self_hash: hash(10),
            deployed_tensor_payload_sha: hash(11),
            fp_shadow_tensor_payload_sha: hash(12),
            promotion_gate_self_hash: *promotion_gate_self_hash,
        })
        .expect("D9 continuation initializes");
        assert_eq!(init.phase_state_initial, S4TrainPhase::PhaseD);
        assert_eq!(init.optimizer_step_initial, 0);
        assert_eq!(init.promotion_gate_self_hash, *promotion_gate_self_hash);
        batch_rng_states.insert(init.rng_streams.batch.initial_state_hex.clone());

        let mut run_loop = S4SeedRunLoop::new(seed, &config).expect("seed run loop builds");
        let evidence = run_loop.run_to_budget().expect("run reaches D10 budget");
        assert_eq!(evidence.seed, seed);
        assert_eq!(
            evidence.completed_optimizer_steps,
            S4_OPTIMIZER_STEPS_GUTENBERG
        );
        assert_eq!(
            evidence.final_optimizer_step,
            Some(S4_OPTIMIZER_STEPS_GUTENBERG)
        );
        assert_eq!(
            evidence.event_history.len(),
            S4_OPTIMIZER_STEPS_GUTENBERG as usize
        );
        assert!(
            evidence
                .event_history
                .iter()
                .all(|event| event.seed == seed && event.phase == S4TrainPhase::PhaseD)
        );
    }
    assert_eq!(
        batch_rng_states.len(),
        S4_CANONICAL_SEEDS.len(),
        "canonical seed continuations must not share BatchRng initial state"
    );
}

fn assert_strict_five_seed_score_passes(
    tinystories_manifest_self_hash: Hash256,
    gutenberg_manifest_self_hash: Hash256,
    corpus_val_sha: Hash256,
) {
    let products = S4_CANONICAL_SEEDS
        .into_iter()
        .map(|seed| {
            s4_v0_success_gutenberg(S4V0SuccessInputs {
                tinystories_manifest_self_hash,
                gutenberg_manifest_self_hash,
                seed,
                checkpoint_self_hash: hash(30 + seed as u8),
                checkpoint_payload_sha: hash(40 + seed as u8),
                corpus_val_sha,
                workload_manifest_template_self_hash: hash(50),
                workload_manifest_instance_self_hash: hash(51),
                fp_reference_self_hash: hash(60 + seed as u8),
                bpc_ternary: bpc(1.00),
                bpc_kn5: bpc(1.25),
                bpc_fp_reference: bpc(0.95),
                inherited_acceptance: S4InheritedV0SuccessBits::all_pass(),
            })
            .expect("per-seed v0_success builds")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        strict_v0_success_on_gutenberg(&products).expect("strict score gate evaluates"),
        S4StrictV0SuccessOutcome::Pass
    );
}

fn text_seq_without_boundaries(bytes: Vec<u8>) -> TextCharSeq {
    TextCharSeq::new(
        bytes
            .into_iter()
            .filter(|id| *id != BOS_ID && *id != EOS_ID)
            .collect(),
    )
    .expect("build-corpus text ids are valid after boundary stripping")
}

fn baseline_oracle_sequences(root: &Path) -> (TextCharSeq, TextCharSeq) {
    let fixture_root = root.join("fixtures/baselines/kn_oracle");
    (
        normalize_fixture_file(&fixture_root.join("train.bytes")),
        normalize_fixture_file(&fixture_root.join("eval.bytes")),
    )
}

fn normalize_fixture_file(path: &Path) -> TextCharSeq {
    let bytes = std::fs::read(path).expect("baseline oracle fixture reads");
    let normalized = normalize_raw(&bytes).expect("baseline oracle fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn window_doc(start: u8) -> Vec<u8> {
    (0..S4_CONTAMINATION_NGRAM_N)
        .map(|offset| start + offset as u8)
        .collect()
}

fn bound<T>(path: &str, self_hash: Hash256, artifact: T) -> PromotionGateBoundArtifact<T> {
    PromotionGateBoundArtifact::new(PromotionGateArtifactRef::new(path, self_hash), artifact)
}

fn bpc(value: f64) -> S4BpcValue {
    S4BpcValue::try_new(value).expect("fixture bpc is valid")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
