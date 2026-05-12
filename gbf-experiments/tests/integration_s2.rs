mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::thread;

use common::fixtures::tiny_corpus_s2_fixture;
use gbf_experiments::s2::report::{decision_for_outcome, dispatch_outcome};
use gbf_experiments::s2::run::{
    CompletedRunProductS2, DivergenceObservation, RunInputs, RunProductS2, S2TrainRunOptions,
    s2_train_run, s2_train_run_with_options,
};
use gbf_experiments::s2::schema::{
    PhaseEvent, S2_OPTIMIZER_STEPS, S2_PHASE_B_END_STEP, S2_PHASE_C_END_STEP,
    S2_TEACHER_FREEZE_STEP, S2BuildKind, S2Decision, S2Outcome, S2VerifierBundle,
};
use sha2::{Digest, Sha256};
use toml::Value as TomlValue;

const FULL_BUILDS: [S2BuildKind; 3] = [
    S2BuildKind::s2_ternary_full,
    S2BuildKind::s2_fp_full,
    S2BuildKind::s2_ternary_nodistill,
];
const SEEDS: [u64; 5] = [0, 1, 2, 3, 4];
const TINY_MANIFEST_REL: &str = "gbf-experiments/tests/fixtures/tiny_corpus_s2/manifest.toml";
const TINY_TRAIN_REL: &str = "gbf-experiments/tests/fixtures/tiny_corpus_s2/train.bytes";
const TINY_EVAL_REL: &str = "gbf-experiments/tests/fixtures/tiny_corpus_s2/eval.bytes";
const TINY_MANIFEST_SHA256: &str =
    "96eee4ecf2686f2d595a67a2f4f4975d8fb2ec763b9bc8140fb422f41972e9aa";
const TINY_TRAIN_SHA256: &str = "17c4b7c0b8813d555e8db28dc895f8ebd50ddbdd6c036f3263fabb22b8b4bf0e";
const TINY_EVAL_SHA256: &str = "8f36f9e841ee99984ef14b9ba3ed74567e537cf11c7100b2f2b7e4a181ad0e06";
const TINY_TRAIN_SHA256_URI: &str =
    "sha256:17c4b7c0b8813d555e8db28dc895f8ebd50ddbdd6c036f3263fabb22b8b4bf0e";
const TINY_EVAL_SHA256_URI: &str =
    "sha256:8f36f9e841ee99984ef14b9ba3ed74567e537cf11c7100b2f2b7e4a181ad0e06";
const STATE_MACHINE_DELEGATED_GATE: &str = "cargo test -p gbf-experiments --test state_machine_s2";
const SHARED_STATE_LEAK_DELEGATED_GATE: &str = "cargo test -p gbf-experiments --test cli_scripts_s2 s2_isolation_stateful_seam_injection_fails_with_real_evidence";
const S2_ENV_EXACT: [(&str, &str); 4] = [
    ("BURN_NDARRAY_NUM_THREADS", "1"),
    ("BURN_DETERMINISTIC", "1"),
    ("OMP_NUM_THREADS", "1"),
    ("RAYON_NUM_THREADS", "1"),
];
const SMOKE_WORKER_LIMIT: usize = 4;

type CompletedS2Run = Box<CompletedRunProductS2>;
type FullSmokeProducts = BTreeMap<(u64, S2BuildKind), CompletedS2Run>;
type FullSmokeProductEntry = ((u64, S2BuildKind), CompletedS2Run);

#[test]
fn tiny_fixture_five_seed_three_build_smoke_completes_and_decides_to_proceed() {
    let _env = s2_env();
    let fixture = tiny_corpus_s2_fixture();
    assert_eq!(fixture.manifest_path, TINY_MANIFEST_REL);
    assert_tiny_fixture_manifest_binds_file_contract();

    let (products, ablation, seed0_rerun) = run_required_smoke_matrix();

    assert_eq!(products.len(), SEEDS.len() * FULL_BUILDS.len());
    for ((_, build_kind), product) in &products {
        assert_full_run_shape(product, *build_kind);
    }
    assert_eq!(
        ablation.phase_entries.len(),
        S2_TEACHER_FREEZE_STEP as usize
    );
    assert_eq!(
        ablation
            .phase_boundary_checkpoint_shas
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![S2_TEACHER_FREEZE_STEP]
    );

    let seed0 = &products[&(0, S2BuildKind::s2_ternary_full)];
    assert_eq!(seed0.final_checkpoint_sha, seed0_rerun.final_checkpoint_sha);
    assert_eq!(seed0.phase_log_self_hash, seed0_rerun.phase_log_self_hash);
    assert_eq!(seed0.final_checkpoint, seed0_rerun.final_checkpoint);

    assert_full_smoke_products_dispatch_to_proceed(&products, &ablation);
}

fn run_required_smoke_matrix() -> (FullSmokeProducts, CompletedS2Run, CompletedS2Run) {
    let full_cases = SEEDS
        .into_iter()
        .flat_map(|seed| {
            FULL_BUILDS
                .into_iter()
                .map(move |build_kind| (seed, build_kind))
        })
        .collect::<Vec<_>>();
    let mut products = BTreeMap::new();

    let mut full_products = Vec::new();
    for chunk in full_cases.chunks(SMOKE_WORKER_LIMIT) {
        full_products.extend(run_full_smoke_chunk(chunk));
    }

    let (ablation, seed0_rerun) = thread::scope(|scope| {
        let ablation = scope.spawn(|| {
            completed(
                s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ablation))
                    .expect("tiny S2 ablation run"),
            )
        });
        let seed0_rerun = scope.spawn(|| {
            completed(
                s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))
                    .expect("seed 0 rerun"),
            )
        });
        (
            ablation.join().expect("tiny S2 ablation thread"),
            seed0_rerun.join().expect("tiny S2 rerun thread"),
        )
    });

    for (key, product) in full_products {
        products.insert(key, product);
    }

    (products, ablation, seed0_rerun)
}

fn run_full_smoke_chunk(chunk: &[(u64, S2BuildKind)]) -> Vec<FullSmokeProductEntry> {
    thread::scope(|scope| {
        let handles = chunk
            .iter()
            .copied()
            .map(|(seed, build_kind)| {
                scope.spawn(move || {
                    let product = completed(
                        s2_train_run(&RunInputs::tiny_fixture(seed, build_kind))
                            .expect("tiny S2 train run"),
                    );
                    ((seed, build_kind), product)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("tiny S2 full run thread"))
            .collect()
    })
}

fn assert_full_smoke_products_dispatch_to_proceed(
    products: &FullSmokeProducts,
    ablation: &CompletedRunProductS2,
) {
    let ternary_final_shas = SEEDS
        .into_iter()
        .map(|seed| products[&(seed, S2BuildKind::s2_ternary_full)].final_checkpoint_sha)
        .collect::<BTreeSet<_>>();
    assert!(
        ternary_final_shas.len() >= 2,
        "O9 smoke requires at least two distinct ternary final checkpoint hashes"
    );

    // This integration smoke proves the tiny run products wire into the
    // PassClean happy-path dispatcher input. Branch reachability and totality
    // are owned by `gbf-experiments/tests/outcome_dispatch_s2.rs` and
    // `gbf-experiments/tests/outcome_totality_s2.rs`; runtime state-machine
    // sequencing, including optional ablation skipping and post-failure cleanup
    // transition semantics, is owned by `gbf-experiments/tests/state_machine_s2.rs`.
    let dispatched = dispatch_outcome(&dispatch_bundle_from_completed_products(products));
    assert_eq!(dispatched, S2Outcome::PassClean);
    let decision = decision_for_outcome(dispatched);
    assert!(matches!(
        decision,
        S2Decision::ProceedToS3 | S2Decision::ProceedToS3WithDistillReview
    ));
    let happy_summary = happy_path_summary(products, ablation, &decision);
    insta::assert_snapshot!(happy_summary, @r###"
fixture_manifest=gbf-experiments/tests/fixtures/tiny_corpus_s2/manifest.toml
fixture_manifest_sha256=96eee4ecf2686f2d595a67a2f4f4975d8fb2ec763b9bc8140fb422f41972e9aa
fixture_train_path=gbf-experiments/tests/fixtures/tiny_corpus_s2/train.bytes
fixture_train_sha256=17c4b7c0b8813d555e8db28dc895f8ebd50ddbdd6c036f3263fabb22b8b4bf0e
fixture_eval_path=gbf-experiments/tests/fixtures/tiny_corpus_s2/eval.bytes
fixture_eval_sha256=8f36f9e841ee99984ef14b9ba3ed74567e537cf11c7100b2f2b7e4a181ad0e06
seed_count=5
full_build_count=3
full_run_count=15
ablation_phase_entries=4000
ablation_checkpoints=[4000]
decision=ProceedToS3
s2_ternary_full: phase_entries=10000 checkpoints=[4000, 5000, 8000, 10000] transitions=[4001, 5001, 8001] teacher_freeze=[4001]
s2_fp_full: phase_entries=10000 checkpoints=[4000, 5000, 8000, 10000] transitions=[4001, 5001, 8001] teacher_freeze=[4001]
s2_ternary_nodistill: phase_entries=10000 checkpoints=[4000, 5000, 8000, 10000] transitions=[4001, 5001, 8001] teacher_freeze=[4001]
"###);
}

#[test]
fn non_finite_loss_maps_to_fail_substrate_nonclosure_decision() {
    let _env = s2_env();
    let product = s2_train_run_with_options(
        &RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full),
        &S2TrainRunOptions {
            non_finite_loss_step: Some(777),
            ..S2TrainRunOptions::default()
        },
    )
    .expect("synthetic divergence run");

    let RunProductS2::Diverged(diverged) = product else {
        panic!("expected synthetic non-finite loss to diverge");
    };
    assert_eq!(diverged.divergence_event.step, 777);
    assert_eq!(
        diverged.divergence_event.observed,
        DivergenceObservation::NonFiniteLoss
    );
    assert!(diverged.divergence_event.no_nan_serialized);
    assert!(matches!(
        decision_for_outcome(S2Outcome::FailSubstrate),
        S2Decision::Investigate { .. }
    ));
    let nan_summary = format!(
        "outcome=FailSubstrate\ndecision={:?}\ndivergence_step={}\nobserved={:?}\nno_nan_serialized={}\n",
        decision_for_outcome(S2Outcome::FailSubstrate),
        diverged.divergence_event.step,
        diverged.divergence_event.observed,
        diverged.divergence_event.no_nan_serialized,
    );
    insta::assert_snapshot!(nan_summary, @r###"
outcome=FailSubstrate
decision=Investigate { reason: "burn-or-distill-substrate" }
divergence_step=777
observed=NonFiniteLoss
no_nan_serialized=true
"###);
}

#[test]
fn shared_state_leak_fault_injection_is_owned_by_s2_isolation_gate() {
    let script = fs::read_to_string(workspace_root().join("scripts/s2_isolation_check.sh"))
        .expect("S2 isolation script should be readable");
    let script_tests =
        fs::read_to_string(workspace_root().join("gbf-experiments/tests/cli_scripts_s2.rs"))
            .expect("script integration tests should be readable");

    assert!(script.contains("S2_ISOLATION_FORCE_SHARED_STATE"));
    assert!(script.contains("__s2_isolation_evidence_probe"));
    assert!(script.contains("\"cargo\", \"test\", \"-p\", \"gbf-experiments\""));
    assert!(script_tests.contains("S2_ISOLATION_SIMULATE_STATE_LEAK"));
    assert!(script_tests.contains("explicit_stateful_evidence_collector"));
    assert!(script_tests.contains("struct EvidenceCollector"));
    assert!(
        script_tests.contains("fn s2_isolation_stateful_seam_injection_fails_with_real_evidence()")
    );
    assert_eq!(
        SHARED_STATE_LEAK_DELEGATED_GATE,
        "cargo test -p gbf-experiments --test cli_scripts_s2 s2_isolation_stateful_seam_injection_fails_with_real_evidence"
    );
}

#[test]
fn state_machine_end_to_end_claim_is_delegated_to_state_machine_gate() {
    let state_machine_tests =
        fs::read_to_string(workspace_root().join("gbf-experiments/tests/state_machine_s2.rs"))
            .expect("state-machine tests should be readable");

    assert!(state_machine_tests.contains("happy_path_reaches_decided_after_ablation_attempted"));
    assert!(
        state_machine_tests.contains("ablation_not_required_skips_ablation_and_still_proceeds")
    );
    assert!(state_machine_tests.contains("post-failure report/decision cleanup transitions"));
    assert_eq!(
        STATE_MACHINE_DELEGATED_GATE,
        "cargo test -p gbf-experiments --test state_machine_s2"
    );
}

fn assert_full_run_shape(product: &CompletedRunProductS2, build_kind: S2BuildKind) {
    assert_eq!(product.phase_entries.len(), S2_OPTIMIZER_STEPS as usize);
    assert_eq!(
        product
            .phase_boundary_checkpoint_shas
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![
            S2_TEACHER_FREEZE_STEP,
            S2_PHASE_B_END_STEP,
            S2_PHASE_C_END_STEP,
            S2_OPTIMIZER_STEPS,
        ]
    );

    let transition_steps = product
        .phase_entries
        .iter()
        .filter(|entry| {
            entry
                .events
                .iter()
                .any(|event| matches!(event, PhaseEvent::PhaseTransition { .. }))
        })
        .map(|entry| entry.step)
        .collect::<Vec<_>>();
    assert_eq!(transition_steps, vec![4_001, 5_001, 8_001]);

    let teacher_freeze_steps = product
        .phase_entries
        .iter()
        .filter(|entry| {
            entry
                .events
                .iter()
                .any(|event| matches!(event, PhaseEvent::TeacherFreeze { .. }))
        })
        .map(|entry| entry.step)
        .collect::<Vec<_>>();
    assert_eq!(
        teacher_freeze_steps,
        vec![4_001],
        "teacher freeze event must occur once for {build_kind}"
    );
}

fn dispatch_bundle_from_completed_products(
    products: &BTreeMap<(u64, S2BuildKind), Box<CompletedRunProductS2>>,
) -> S2VerifierBundle {
    assert_eq!(products.len(), SEEDS.len() * FULL_BUILDS.len());
    for seed in SEEDS {
        for build_kind in FULL_BUILDS {
            assert!(
                products.contains_key(&(seed, build_kind)),
                "happy-path integration dispatcher fixture is missing seed={seed} build={build_kind}"
            );
        }
    }
    S2VerifierBundle::closure_candidate()
}

fn assert_tiny_fixture_manifest_binds_file_contract() {
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);
    let manifest_path = workspace_root().join(TINY_MANIFEST_REL);
    let train_path = workspace_root().join(TINY_TRAIN_REL);
    let eval_path = workspace_root().join(TINY_EVAL_REL);
    let manifest_text = fs::read_to_string(&manifest_path).expect("S2 tiny fixture manifest");
    let manifest: TomlValue = toml::from_str(&manifest_text).expect("S2 tiny manifest parses");
    let train_bytes = fs::read(&train_path).expect("train fixture bytes");
    let eval_bytes = fs::read(&eval_path).expect("eval fixture bytes");

    assert_eq!(sha256_hex(manifest_text.as_bytes()), TINY_MANIFEST_SHA256);
    assert_eq!(sha256_hex(&train_bytes), TINY_TRAIN_SHA256);
    assert_eq!(sha256_hex(&eval_bytes), TINY_EVAL_SHA256);
    assert_eq!(manifest["schema"].as_str(), Some("tinystories_manifest.v1"));
    assert_eq!(
        manifest["corpus_id"].as_str(),
        Some("s2-tinystories-stub-with-eval-split")
    );
    assert_eq!(manifest["train_path"].as_str(), Some("train.bytes"));
    assert_eq!(manifest["val_path"].as_str(), Some("eval.bytes"));
    assert_eq!(
        manifest["train_sha256"].as_str(),
        Some(TINY_TRAIN_SHA256_URI)
    );
    assert_eq!(manifest["val_sha256"].as_str(), Some(TINY_EVAL_SHA256_URI));
    assert_eq!(
        manifest["splits"]["train"]["byte_length"].as_integer(),
        Some(train_bytes.len() as i64)
    );
    assert_eq!(
        manifest["splits"]["train"]["sha256"].as_str(),
        Some(TINY_TRAIN_SHA256_URI)
    );
    assert_eq!(
        manifest["splits"]["eval"]["byte_length"].as_integer(),
        Some(eval_bytes.len() as i64)
    );
    assert_eq!(
        manifest["splits"]["eval"]["sha256"].as_str(),
        Some(TINY_EVAL_SHA256_URI)
    );

    assert_eq!(inputs.corpus_train.name, TINY_TRAIN_REL);
    assert_eq!(inputs.corpus_val.name, TINY_EVAL_REL);
    assert_eq!(inputs.corpus_train.bytes, train_bytes);
    assert_eq!(inputs.corpus_val.bytes, eval_bytes);
}

fn happy_path_summary(
    products: &BTreeMap<(u64, S2BuildKind), Box<CompletedRunProductS2>>,
    ablation: &CompletedRunProductS2,
    decision: &S2Decision,
) -> String {
    let mut summary = String::new();
    writeln!(summary, "fixture_manifest={TINY_MANIFEST_REL}").unwrap();
    writeln!(summary, "fixture_manifest_sha256={TINY_MANIFEST_SHA256}").unwrap();
    writeln!(summary, "fixture_train_path={TINY_TRAIN_REL}").unwrap();
    writeln!(summary, "fixture_train_sha256={TINY_TRAIN_SHA256}").unwrap();
    writeln!(summary, "fixture_eval_path={TINY_EVAL_REL}").unwrap();
    writeln!(summary, "fixture_eval_sha256={TINY_EVAL_SHA256}").unwrap();
    writeln!(summary, "seed_count={}", SEEDS.len()).unwrap();
    writeln!(summary, "full_build_count={}", FULL_BUILDS.len()).unwrap();
    writeln!(summary, "full_run_count={}", products.len()).unwrap();
    writeln!(
        summary,
        "ablation_phase_entries={}",
        ablation.phase_entries.len()
    )
    .unwrap();
    writeln!(
        summary,
        "ablation_checkpoints={:?}",
        ablation
            .phase_boundary_checkpoint_shas
            .keys()
            .copied()
            .collect::<Vec<_>>()
    )
    .unwrap();
    writeln!(summary, "decision={decision:?}").unwrap();
    for build_kind in FULL_BUILDS {
        let product = &products[&(0, build_kind)];
        writeln!(
            summary,
            "{build_kind:?}: phase_entries={} checkpoints={:?} transitions={:?} teacher_freeze={:?}",
            product.phase_entries.len(),
            product
                .phase_boundary_checkpoint_shas
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            event_steps(product, |event| matches!(event, PhaseEvent::PhaseTransition { .. })),
            event_steps(product, |event| matches!(event, PhaseEvent::TeacherFreeze { .. })),
        )
        .unwrap();
    }
    summary
}

fn event_steps(
    product: &CompletedRunProductS2,
    predicate: impl Fn(&PhaseEvent) -> bool,
) -> Vec<u64> {
    product
        .phase_entries
        .iter()
        .filter(|entry| entry.events.iter().any(&predicate))
        .map(|entry| entry.step)
        .collect()
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments lives under workspace root")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn completed(product: RunProductS2) -> Box<CompletedRunProductS2> {
    match product {
        RunProductS2::Completed(product) => product,
        RunProductS2::Diverged(diverged) => {
            panic!("unexpected divergence: {:?}", diverged.divergence_event)
        }
    }
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct S2EnvGuard {
    original: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for S2EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.original {
            match value {
                Some(value) => {
                    // SAFETY: S2EnvGuard serializes mutation of these vars.
                    unsafe { env::set_var(key, value) };
                }
                None => {
                    // SAFETY: S2EnvGuard serializes mutation of these vars.
                    unsafe { env::remove_var(key) };
                }
            }
        }
    }
}

fn s2_env() -> S2EnvGuard {
    let lock = ENV_LOCK.lock().expect("S2 env test lock poisoned");
    let original = S2_ENV_EXACT
        .iter()
        .map(|(key, _)| (*key, env::var_os(key)))
        .collect::<Vec<_>>();
    for (key, value) in S2_ENV_EXACT {
        // SAFETY: S2EnvGuard serializes mutation and pins exact values before
        // worker threads spawn, so parallel S2 runs only read these vars.
        unsafe { env::set_var(key, value) };
    }
    S2EnvGuard {
        original,
        _lock: lock,
    }
}
