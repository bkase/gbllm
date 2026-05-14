#![cfg(feature = "falsify")]

mod common;

#[path = "falsification/f1_nan_forward.rs"]
mod f1_nan_forward;
#[path = "falsification/f2_zero_grad.rs"]
mod f2_zero_grad;
#[path = "falsification/f3_no_reset_scorer.rs"]
mod f3_no_reset_scorer;
#[path = "falsification/f4_phase_a_leaks_ternary.rs"]
mod f4_phase_a_leaks_ternary;
#[path = "falsification/f5_toytiny_undersized.rs"]
mod f5_toytiny_undersized;
#[path = "falsification/f6_modulo_biased_shuffle.rs"]
mod f6_modulo_biased_shuffle;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType,
};
use gbf_experiments::s1::report::{
    Hypothesis, HypothesisStatus, OutcomeDispatchInput, Verdict, dispatch_outcome,
};
use gbf_experiments::s1::run::{RunInputs, TrainBudgetProfile, TrainConfig};
use gbf_experiments::s1::schema::{
    CheckpointMetadata, S1BuildKind, S1Completion, S1Decision, S1Outcome,
};
use gbf_foundation::{Hash256, SemVer};
use gbf_policy::model_profile::ModelSizeProfile;
use serde_json::json;

const START_EVENT: &str = "s1.falsification.scenario.start";
const COMPLETE_EVENT: &str = "s1.falsification.scenario.complete";

fn assert_falsification_outcome(
    substitute_id: &'static str,
    input: OutcomeDispatchInput,
    expected_outcome: S1Outcome,
    expected_decision: S1Decision,
) {
    let capture = TraceCapture::default();
    let dispatch = with_trace_capture(&capture, || {
        tracing::info!(
            target: gbf_experiments::S1_LOG_TARGET,
            event_name = START_EVENT,
            substitute_id,
        );
        let dispatch = dispatch_outcome(&input).expect("falsification dispatch");
        tracing::info!(
            target: gbf_experiments::S1_LOG_TARGET,
            event_name = COMPLETE_EVENT,
            substitute_id,
            expected_outcome = %expected_outcome,
            observed_outcome = %dispatch.outcome,
            r#match = dispatch.outcome == expected_outcome,
        );
        dispatch
    });

    assert_eq!(dispatch.outcome, expected_outcome);
    assert_eq!(dispatch.decision, expected_decision);

    let events = captured_events(&capture)
        .into_iter()
        .filter(|event| matches!(event.name.as_str(), START_EVENT | COMPLETE_EVENT))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2, "falsification events: {events:?}");
    assert_eq!(
        events[0].fields.get("substitute_id"),
        Some(&json!(substitute_id))
    );
    assert_eq!(
        events[1].fields.get("substitute_id"),
        Some(&json!(substitute_id))
    );
    assert_eq!(
        events[1].fields.get("expected_outcome"),
        Some(&json!(expected_outcome.to_string()))
    );
    assert_eq!(
        events[1].fields.get("observed_outcome"),
        Some(&json!(expected_outcome.to_string()))
    );
    assert_eq!(events[1].fields.get("match"), Some(&json!(true)));
}

fn confirmed_input() -> OutcomeDispatchInput {
    OutcomeDispatchInput {
        h1: Verdict::Confirmed.into(),
        h2: Verdict::Confirmed.into(),
        h3: Verdict::Confirmed.into(),
        h4: Verdict::Confirmed.into(),
        h5: Verdict::Confirmed.into(),
        any_seed_diverged: false,
        suspicious_low_bpc: false,
    }
}

fn refute(mut input: OutcomeDispatchInput, hypothesis: Hypothesis) -> OutcomeDispatchInput {
    match hypothesis {
        Hypothesis::H1 => input.h1 = HypothesisStatus::Refuted,
        Hypothesis::H2 => input.h2 = HypothesisStatus::Refuted,
        Hypothesis::H3 => input.h3 = HypothesisStatus::Refuted,
        Hypothesis::H4 => input.h4 = HypothesisStatus::Refuted,
        Hypothesis::H5 => input.h5 = HypothesisStatus::Refuted,
    }
    input
}

fn patterned_corpus(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn integration_inputs(seed: u64) -> RunInputs {
    RunInputs {
        corpus_train: patterned_corpus(512),
        corpus_val: patterned_corpus(384),
        model_config: ModelSizeProfile::toy0(),
        train_config: TrainConfig::integration_fixture(),
        seed,
        budget_profile: TrainBudgetProfile::IntegrationFixture,
    }
}

fn canonical_env() -> [(&'static str, &'static str); 4] {
    [
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "1"),
    ]
}

fn checkpoint_metadata(build_kind: S1BuildKind) -> CheckpointMetadata {
    CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed: 0,
        corpus_train_sha: Hash256::ZERO,
        corpus_val_sha: Hash256::ZERO,
        model_config_hash: Hash256::ZERO,
        train_config_hash: Hash256::ZERO,
        build_kind,
        build_config_hash: Hash256::ZERO,
        dependency_lockfile_sha: Hash256::ZERO,
        rust_toolchain_hash: Hash256::ZERO,
        device_profile_hash: Hash256::ZERO,
        rng_stream_def_hash: gbf_experiments::s1::rng::rng_stream_def_hash(),
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: "integration_fixture".to_owned(),
        final_step: 0,
        final_train_loss: 0.0,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: Hash256::ZERO,
        checkpoint_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("checkpoint metadata self hash")
}

fn tensor(name: &str, values: Vec<f32>) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).expect("artifact path"),
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[values.len()]).expect("shape"),
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(values),
    )
    .expect("tensor")
}

fn fail_substrate_decision() -> S1Decision {
    S1Decision::Investigate {
        reason: "burn-or-autodiff".to_owned(),
    }
}

fn fail_metric_decision() -> S1Decision {
    S1Decision::Halt {
        reason: "measurement-broken".to_owned(),
    }
}

fn fail_phase_decision() -> S1Decision {
    S1Decision::Investigate {
        reason: "F4-phase-contract".to_owned(),
    }
}

fn fail_capacity_decision() -> S1Decision {
    S1Decision::Investigate {
        reason: "propose-Toy1".to_owned(),
    }
}

fn tiny_val_bytes() -> Vec<u8> {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml");
    let manifest =
        gbf_experiments::s1::manifest::read_tinystories_manifest(manifest_path).expect("manifest");
    gbf_experiments::s1::manifest::load_val_bytes(&manifest).expect("val bytes")
}
