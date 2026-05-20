#![cfg(feature = "s4")]

use std::collections::BTreeMap;

mod common;
mod score_s3_support;

use common::tracing_capture::{TraceCapture, TracingEvent, captured_events, with_trace_capture};
use gbf_artifact::{
    ArtifactAux, ArtifactCore, DecodeCapabilitySet, DecodeSpec, LexicalSpec_v1, ModelArtifact,
    ModelSpec_S3, QuantSpec_S3, ReferenceEdge, ReferenceEvalGraph, ReferenceManifest,
    ReferenceModelBundle, ReferenceModelSpec, ReferenceNode, ReferenceNumericProfile, ReferenceOp,
    ReferenceProgram, ReferenceTensor, ReferenceTensorRole, SequenceSemanticsSpec, TextCharSeq,
    VOCAB_SIZE,
};
use gbf_experiments::s3::score::{Evaluator, EvaluatorOutput, ScorerKind};
use gbf_experiments::s4::oracle::{
    S4_ORACLE_AGREEMENT_FINALIZED_EVENT_NAME, S4_ORACLE_AGREEMENT_SCHEMA,
    S4_ORACLE_AGREEMENT_STARTED_EVENT_NAME, S4_ORACLE_MANDATORY_SEED, S4_ORACLE_SCORE_EVENT_NAME,
    S4_S3_ORACLE_TOLERANCE_SCHEMA, S4BundleArtifactOracleAgreementInputs,
    S4InheritedS3OracleTolerances, S4OracleAgreementError, S4OracleAgreementInputs,
    S4OracleAgreementOutcome, S4OracleAgreementReport, S4OracleAgreementStreamInputs,
    S4OracleTokenBpc, s4_oracle_agreement, s4_oracle_agreement_for_bundle_and_artifact,
    s4_oracle_agreement_from_streams,
};
use gbf_foundation::{Hash256, sha256};
use gbf_workload::AcceptanceMatrix_S3;
use score_s3_support::UniformEvaluator;
use serde_json::json;

#[test]
fn s4_oracle_agreement_happy_path_uses_same_workload_and_s3_tolerances() {
    let val = val_fixture();
    let report = s4_oracle_agreement(inputs(
        &val,
        hash(40),
        hash(40),
        UniformEvaluator,
        UniformEvaluator,
        UniformEvaluator,
    ))
    .expect("S4 oracle agreement builds");

    assert_eq!(report.schema, S4_ORACLE_AGREEMENT_SCHEMA);
    assert_eq!(report.seed, 0);
    assert_eq!(report.workload_manifest_self_hash, hash(40));
    assert_eq!(report.corpus_val_sha, sha256(val.as_slice()));
    assert_eq!(report.outcome, S4OracleAgreementOutcome::Agree);
    assert_eq!(report.per_token.len(), val.len());
    assert_eq!(report.gap_live_vs_denotational, 0.0);
    assert_eq!(report.gap_live_vs_artifact, 0.0);
    assert_eq!(report.gap_denotational_vs_artifact, 0.0);
    assert_eq!(report.outcome.as_str(), "Agree");

    let tolerances = S4InheritedS3OracleTolerances::s3_pinned().expect("S3 tolerances derive");
    let acceptance = AcceptanceMatrix_S3::pinned();
    let phase_a = acceptance
        .live_phase_a_vs_bundle
        .expect("phase A tolerance")
        .max_per_token_logit_abs_diff;
    let phase_d = acceptance
        .live_phase_d_vs_artifact
        .expect("phase D tolerance")
        .max_per_token_logit_abs_diff;
    assert_eq!(tolerances.live_vs_denotational_bpc, phase_a);
    assert_eq!(tolerances.live_vs_artifact_bpc, phase_d);
    assert_eq!(
        tolerances.denotational_vs_artifact_bpc,
        phase_a.max(phase_d)
    );
    assert_eq!(
        serde_json::to_value(&tolerances).expect("tolerances serialize"),
        json!({
            "schema": S4_S3_ORACLE_TOLERANCE_SCHEMA,
            "live_vs_denotational_bpc": phase_a,
            "live_vs_artifact_bpc": phase_d,
            "denotational_vs_artifact_bpc": phase_a.max(phase_d),
        })
    );
    assert_eq!(
        report.s3_tolerance_self_hash,
        tolerances
            .compute_self_hash()
            .expect("S3 tolerance hash recomputes")
    );
    report
        .validate_canonical_write()
        .expect("oracle report self-hash validates");

    let value = serde_json::to_value(&report).expect("report serializes");
    assert_eq!(
        value,
        json!({
            "schema": S4_ORACLE_AGREEMENT_SCHEMA,
            "tinystories_manifest_self_hash": hash(1).to_string(),
            "gutenberg_manifest_self_hash": hash(2).to_string(),
            "seed": S4_ORACLE_MANDATORY_SEED,
            "checkpoint_self_hash": hash(3).to_string(),
            "corpus_val_sha": sha256(val.as_slice()).to_string(),
            "workload_manifest_self_hash": hash(40).to_string(),
            "fixture_set_self_hash": hash(5).to_string(),
            "bpc_live": report.bpc_live,
            "bpc_denotational": report.bpc_denotational,
            "bpc_artifact": report.bpc_artifact,
            "gap_live_vs_denotational": 0.0,
            "gap_live_vs_artifact": 0.0,
            "gap_denotational_vs_artifact": 0.0,
            "per_token": report.per_token.iter().map(|record| json!({
                "token": record.token,
                "target_token_id": record.target_token_id,
                "bpc_live": record.bpc_live,
                "bpc_denotational": record.bpc_denotational,
                "bpc_artifact": record.bpc_artifact,
                "gap_live_vs_denotational": record.gap_live_vs_denotational,
                "gap_live_vs_artifact": record.gap_live_vs_artifact,
                "gap_denotational_vs_artifact": record.gap_denotational_vs_artifact,
            })).collect::<Vec<_>>(),
            "s3_tolerance_self_hash": report.s3_tolerance_self_hash.to_string(),
            "outcome": {"kind": "Agree"},
            "oracle_agreement_self_hash": report.oracle_agreement_self_hash.to_string(),
        })
    );
}

#[test]
fn s4_oracle_agreement_reports_first_failing_token_and_max_gap() {
    let val = val_fixture();
    let report = s4_oracle_agreement(inputs(
        &val,
        hash(41),
        hash(41),
        DivergesAtPrefixLenOne,
        UniformEvaluator,
        UniformEvaluator,
    ))
    .expect("divergent oracle report builds");

    match report.outcome {
        S4OracleAgreementOutcome::Disagree {
            failing_token,
            max_gap,
        } => {
            assert_eq!(failing_token, 1);
            assert!(max_gap > 0.0);
        }
        S4OracleAgreementOutcome::Agree => panic!("divergent scorer should disagree"),
    }
    assert!(report.gap_live_vs_denotational > 0.0);
    assert!(report.gap_live_vs_artifact > 0.0);
}

#[test]
fn s4_oracle_agreement_rejects_workload_hash_mismatch_before_scoring() {
    let val = val_fixture();
    let error = s4_oracle_agreement(inputs(
        &val,
        hash(42),
        hash(43),
        PanicEvaluator,
        PanicEvaluator,
        PanicEvaluator,
    ))
    .expect_err("workload mismatch is rejected before any scorer runs");

    assert!(matches!(
        error,
        S4OracleAgreementError::WorkloadManifestMismatch { .. }
    ));
}

#[test]
fn s4_oracle_agreement_rejects_gutenberg_val_hash_mismatch() {
    let val = val_fixture();
    let mut inputs = inputs(
        &val,
        hash(44),
        hash(44),
        PanicEvaluator,
        PanicEvaluator,
        PanicEvaluator,
    );
    inputs.corpus_val_sha = hash(99);

    let error = s4_oracle_agreement(inputs).expect_err("val hash mismatch rejected before scoring");

    assert!(matches!(
        error,
        S4OracleAgreementError::HashMismatch {
            field: "corpus_val_sha",
            ..
        }
    ));
}

#[test]
fn s4_oracle_agreement_canonical_round_trip_is_byte_identical() {
    let val = val_fixture();
    let report = s4_oracle_agreement(inputs(
        &val,
        hash(45),
        hash(45),
        UniformEvaluator,
        UniformEvaluator,
        UniformEvaluator,
    ))
    .expect("S4 oracle agreement builds");
    let bytes = report.canonical_bytes().expect("canonical bytes");
    let decoded: gbf_experiments::s4::oracle::S4OracleAgreementReport =
        serde_json::from_slice(&bytes).expect("canonical report decodes");

    assert_eq!(
        decoded.oracle_agreement_self_hash,
        report.oracle_agreement_self_hash
    );
    assert_eq!(
        decoded.canonical_bytes().expect("decoded canonical bytes"),
        bytes
    );
}

#[test]
fn s4_oracle_agreement_events_are_subscriber_captured() {
    let val = val_fixture();
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || {
        s4_oracle_agreement(inputs(
            &val,
            hash(46),
            hash(46),
            UniformEvaluator,
            UniformEvaluator,
            UniformEvaluator,
        ))
        .expect("S4 oracle agreement builds")
    });

    let events = captured_events(&capture);
    let started = captured_event(&events, S4_ORACLE_AGREEMENT_STARTED_EVENT_NAME);
    assert_eq!(
        started.fields.get("schema"),
        Some(&json!(S4_ORACLE_AGREEMENT_SCHEMA))
    );
    assert_eq!(started.fields.get("seed"), Some(&json!(0_u64)));
    assert_eq!(started.fields.get("token_count"), Some(&json!(4_u64)));
    assert_eq!(
        started.fields.get("workload_manifest_self_hash"),
        Some(&json!(hash(46).to_string()))
    );
    assert_eq!(
        started.fields.get("expected_workload_manifest_self_hash"),
        Some(&json!(hash(46).to_string()))
    );
    assert_eq!(
        started.fields.get("corpus_val_sha"),
        Some(&json!(sha256(val.as_slice()).to_string()))
    );

    let score_events = events
        .iter()
        .filter(|event| event.name == S4_ORACLE_SCORE_EVENT_NAME)
        .collect::<Vec<_>>();
    assert_eq!(score_events.len(), val.len() * 3);
    assert_eq!(
        score_events[0].fields.get("scorer"),
        Some(&json!("live_training"))
    );
    assert_eq!(score_events[0].fields.get("token"), Some(&json!(0_u64)));
    assert_eq!(
        score_events[0].fields.get("target_token_id"),
        Some(&json!(1_u64))
    );
    assert_eq!(
        score_events[0].fields.get("bpc"),
        Some(&json!(report.per_token[0].bpc_live))
    );

    let finalized = captured_event(&events, S4_ORACLE_AGREEMENT_FINALIZED_EVENT_NAME);
    assert_eq!(
        finalized.fields.get("schema"),
        Some(&json!(S4_ORACLE_AGREEMENT_SCHEMA))
    );
    assert_eq!(finalized.fields.get("seed"), Some(&json!(0_u64)));
    assert_eq!(finalized.fields.get("token_count"), Some(&json!(4_u64)));
    assert_eq!(finalized.fields.get("outcome"), Some(&json!("Agree")));
    assert_eq!(
        finalized.fields.get("s3_tolerance_self_hash"),
        Some(&json!(report.s3_tolerance_self_hash.to_string()))
    );
    assert_eq!(
        finalized.fields.get("oracle_agreement_self_hash"),
        Some(&json!(report.oracle_agreement_self_hash.to_string()))
    );
}

#[test]
fn s4_oracle_agreement_bundle_artifact_wrapper_has_agree_path() {
    let val = val_fixture();
    let bundle = zero_reference_bundle();
    let artifact = no_weight_artifact();

    let report =
        s4_oracle_agreement_for_bundle_and_artifact(S4BundleArtifactOracleAgreementInputs {
            tinystories_manifest_self_hash: hash(1),
            gutenberg_manifest_self_hash: hash(2),
            seed: 0,
            checkpoint_self_hash: hash(3),
            corpus_val_sha: sha256(val.as_slice()),
            expected_workload_manifest_self_hash: hash(47),
            workload_manifest_self_hash: hash(47),
            fixture_set_self_hash: hash(5),
            gutenberg_val: &val,
            live_training_scorer: UniformEvaluator,
            reference_model_bundle: &bundle,
            artifact: &artifact,
        })
        .expect("fixture-local bundle/artifact agreement builds");

    assert_eq!(report.outcome, S4OracleAgreementOutcome::Agree);
    assert_eq!(report.gap_live_vs_denotational, 0.0);
    assert_eq!(report.gap_live_vs_artifact, 0.0);
    assert_eq!(report.gap_denotational_vs_artifact, 0.0);
}

#[test]
fn s4_oracle_agreement_rejects_non_mandatory_invalid_and_empty_seed_inputs() {
    let val = val_fixture();
    let mut non_mandatory = inputs(
        &val,
        hash(48),
        hash(48),
        UniformEvaluator,
        UniformEvaluator,
        UniformEvaluator,
    );
    non_mandatory.seed = 1;
    assert!(matches!(
        s4_oracle_agreement(non_mandatory).unwrap_err(),
        S4OracleAgreementError::NonMandatorySeed { seed: 1 }
    ));

    let mut invalid = inputs(
        &val,
        hash(49),
        hash(49),
        UniformEvaluator,
        UniformEvaluator,
        UniformEvaluator,
    );
    invalid.seed = 99;
    assert!(matches!(
        s4_oracle_agreement(invalid).unwrap_err(),
        S4OracleAgreementError::Schema(_)
    ));

    let empty = TextCharSeq::new(Vec::new()).expect("empty text sequence can be constructed");
    assert!(matches!(
        s4_oracle_agreement(inputs(
            &empty,
            hash(50),
            hash(50),
            UniformEvaluator,
            UniformEvaluator,
            UniformEvaluator,
        ))
        .unwrap_err(),
        S4OracleAgreementError::EmptyValidation
    ));
}

#[test]
fn s4_oracle_agreement_rejects_tolerance_summary_and_self_hash_mismatch() {
    let report = stream_report();

    let mut tolerance_mismatch = report.clone();
    tolerance_mismatch.s3_tolerance_self_hash = hash(222);
    assert!(matches!(
        tolerance_mismatch.validate_canonical_write().unwrap_err(),
        S4OracleAgreementError::ToleranceHashMismatch { .. }
    ));

    let mut summary_mismatch = report.clone();
    summary_mismatch.bpc_live += 1.0;
    assert!(matches!(
        summary_mismatch.validate_canonical_write().unwrap_err(),
        S4OracleAgreementError::SummaryMismatch
    ));

    let mut self_hash_mismatch = report;
    self_hash_mismatch.oracle_agreement_self_hash = hash(223);
    assert!(matches!(
        self_hash_mismatch.validate_canonical_write().unwrap_err(),
        S4OracleAgreementError::SelfHashMismatch { .. }
    ));
}

#[test]
fn s4_oracle_agreement_rejects_token_stream_mismatch() {
    let length_error = s4_oracle_agreement_from_streams(stream_inputs(
        vec![token_bpc(0, 1, 2.0)],
        Vec::new(),
        vec![token_bpc(0, 1, 2.0)],
    ))
    .unwrap_err();
    assert!(matches!(
        length_error,
        S4OracleAgreementError::TokenStreamLengthMismatch { .. }
    ));

    let identity_error = s4_oracle_agreement_from_streams(stream_inputs(
        vec![token_bpc(0, 1, 2.0)],
        vec![token_bpc(0, 2, 2.0)],
        vec![token_bpc(0, 1, 2.0)],
    ))
    .unwrap_err();
    assert!(matches!(
        identity_error,
        S4OracleAgreementError::TokenStreamIdentityMismatch { token: 0 }
    ));
}

#[test]
fn s4_oracle_agreement_rejects_bad_scorer_outputs() {
    let val = val_fixture();

    assert!(matches!(
        s4_oracle_agreement(inputs(
            &val,
            hash(51),
            hash(51),
            ShortLogitsEvaluator,
            UniformEvaluator,
            UniformEvaluator,
        ))
        .unwrap_err(),
        S4OracleAgreementError::LogitsWrongLength { .. }
    ));
    assert!(matches!(
        s4_oracle_agreement(inputs(
            &val,
            hash(52),
            hash(52),
            NonFiniteLogitEvaluator,
            UniformEvaluator,
            UniformEvaluator,
        ))
        .unwrap_err(),
        S4OracleAgreementError::NonFiniteLogit { .. }
    ));
    assert!(matches!(
        s4_oracle_agreement(inputs(
            &val,
            hash(53),
            hash(53),
            InvalidTargetLogprobEvaluator,
            UniformEvaluator,
            UniformEvaluator,
        ))
        .unwrap_err(),
        S4OracleAgreementError::InvalidTargetLogprob { .. }
    ));
}

fn inputs<'a, L, R, A>(
    val: &'a TextCharSeq,
    expected_workload_manifest_self_hash: Hash256,
    workload_manifest_self_hash: Hash256,
    live_training_scorer: L,
    reference_model_bundle_scorer: R,
    artifact_oracle_scorer: A,
) -> S4OracleAgreementInputs<'a, L, R, A>
where
    L: Evaluator,
    R: Evaluator,
    A: Evaluator,
{
    S4OracleAgreementInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: hash(2),
        seed: 0,
        checkpoint_self_hash: hash(3),
        corpus_val_sha: sha256(val.as_slice()),
        expected_workload_manifest_self_hash,
        workload_manifest_self_hash,
        fixture_set_self_hash: hash(5),
        gutenberg_val: val,
        live_training_scorer,
        reference_model_bundle_scorer,
        artifact_oracle_scorer,
    }
}

fn val_fixture() -> TextCharSeq {
    TextCharSeq::new(vec![1, 2, 3, 4]).expect("fixture val ids are valid")
}

fn stream_report() -> S4OracleAgreementReport {
    s4_oracle_agreement_from_streams(stream_inputs(
        vec![token_bpc(0, 1, 2.0), token_bpc(1, 2, 2.25)],
        vec![token_bpc(0, 1, 2.0), token_bpc(1, 2, 2.25)],
        vec![token_bpc(0, 1, 2.0), token_bpc(1, 2, 2.25)],
    ))
    .expect("stream report builds")
}

fn stream_inputs(
    live: Vec<S4OracleTokenBpc>,
    denotational: Vec<S4OracleTokenBpc>,
    artifact: Vec<S4OracleTokenBpc>,
) -> S4OracleAgreementStreamInputs {
    S4OracleAgreementStreamInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: hash(2),
        seed: 0,
        checkpoint_self_hash: hash(3),
        corpus_val_sha: hash(4),
        workload_manifest_self_hash: hash(40),
        fixture_set_self_hash: hash(5),
        live,
        denotational,
        artifact,
    }
}

fn token_bpc(token: u64, target_token_id: u8, bpc: f64) -> S4OracleTokenBpc {
    S4OracleTokenBpc {
        token,
        target_token_id,
        bpc,
    }
}

fn captured_event<'a>(events: &'a [TracingEvent], name: &str) -> &'a TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| panic!("missing {name}; saw {events:#?}"))
}

fn zero_reference_bundle() -> ReferenceModelBundle {
    let embedding = ReferenceTensor::new(
        tensor_ref("tensor.embedding"),
        ReferenceTensorRole::Embedding,
        vec![VOCAB_SIZE as u32, 1],
        vec![0.0; VOCAB_SIZE],
    )
    .expect("embedding tensor builds");
    let classifier = ReferenceTensor::new(
        tensor_ref("tensor.classifier.weight"),
        ReferenceTensorRole::Classifier,
        vec![VOCAB_SIZE as u32, 1],
        vec![0.0; VOCAB_SIZE],
    )
    .expect("classifier tensor builds");
    let graph = ReferenceEvalGraph::new(
        vec![
            ReferenceNode::new(
                tensor_ref("op.embedding"),
                ReferenceOp::Embedding,
                vec![tensor_ref("tensor.embedding")],
                vec![tensor_ref("runtime.embedding")],
            ),
            ReferenceNode::new(
                tensor_ref("op.classifier"),
                ReferenceOp::Classifier,
                vec![
                    tensor_ref("runtime.embedding"),
                    tensor_ref("tensor.classifier.weight"),
                ],
                vec![tensor_ref("runtime.logits")],
            ),
        ],
        vec![ReferenceEdge::new(
            tensor_ref("op.embedding"),
            tensor_ref("op.classifier"),
            tensor_ref("runtime.embedding"),
        )],
    )
    .expect("reference graph builds");

    ReferenceModelBundle::new(
        ReferenceManifest::new(
            0,
            sha256("s4-zero-reference-teacher"),
            sha256("s4-zero-reference-sequence"),
            "s4-oracle-agreement-test",
            sha256("s4-oracle-agreement-test"),
        ),
        ReferenceNumericProfile::pinned(),
        LexicalSpec_v1::pinned(),
        ReferenceModelSpec::toy0(),
        ReferenceProgram::new(graph, sha256("s4-zero-reference-program"))
            .expect("reference program builds"),
        vec![embedding, classifier],
        DecodeSpec::argmax(),
        None,
    )
    .expect("zero reference bundle builds")
}

fn no_weight_artifact() -> ModelArtifact {
    let core = ArtifactCore::new(
        ModelArtifact::fixture_manifest(0, hash(7)),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("s4-oracle-no-weight-artifact"),
        QuantSpec_S3::new(BTreeMap::new()),
        SequenceSemanticsSpec::linear_state(1).expect("sequence semantics builds"),
        Vec::new(),
        Vec::new(),
        DecodeCapabilitySet::argmax_only(),
        None,
    )
    .expect("no-weight artifact core builds");
    ModelArtifact::new(core, Vec::new(), ArtifactAux::sparse(), None)
        .expect("no-weight artifact builds")
}

fn tensor_ref(value: &str) -> gbf_artifact::TensorRef {
    gbf_artifact::TensorRef::new(value).expect("tensor ref builds")
}

#[derive(Debug, Clone, Copy)]
struct DivergesAtPrefixLenOne;

impl Evaluator for DivergesAtPrefixLenOne {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, prefix: &[u8], target_ix: usize) -> EvaluatorOutput {
        let mut logits = vec![0.0; VOCAB_SIZE];
        if prefix.len() == 1 {
            logits[target_ix] = -1.0;
        }
        EvaluatorOutput::from_logits(logits, target_ix).expect("divergent logits valid")
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
struct PanicEvaluator;

impl Evaluator for PanicEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], _target_ix: usize) -> EvaluatorOutput {
        panic!("PanicEvaluator should not be called")
    }

    fn reset_state(&mut self) {
        panic!("PanicEvaluator should not be reset")
    }
}

#[derive(Debug, Clone, Copy)]
struct ShortLogitsEvaluator;

impl Evaluator for ShortLogitsEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], _target_ix: usize) -> EvaluatorOutput {
        EvaluatorOutput {
            logits: vec![0.0; VOCAB_SIZE - 1],
            target_logprob: 0.0,
        }
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
struct NonFiniteLogitEvaluator;

impl Evaluator for NonFiniteLogitEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], target_ix: usize) -> EvaluatorOutput {
        let mut logits = vec![0.0; VOCAB_SIZE];
        logits[target_ix] = f32::NAN;
        EvaluatorOutput {
            logits,
            target_logprob: -1.0,
        }
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
struct InvalidTargetLogprobEvaluator;

impl Evaluator for InvalidTargetLogprobEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], _target_ix: usize) -> EvaluatorOutput {
        EvaluatorOutput {
            logits: vec![0.0; VOCAB_SIZE],
            target_logprob: 0.5,
        }
    }

    fn reset_state(&mut self) {}
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
