#![cfg(feature = "s4")]

mod score_s3_support;

use gbf_artifact::VOCAB_SIZE;
use gbf_experiments::s4::score::{
    ReferenceModelBundle, S4_GUTENBERG_SCORE_SCHEMA, S4_MAX_TERNARY_QAT_GAP,
    S4_MIN_BPC_MARGIN_VS_KN5, S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA,
    S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS, S4_SCORE_CHUNK_SIZE, S4BpcValue,
    S4InheritedV0SuccessBits, S4ReferenceModelBundleReexportEvidence,
    S4ReferenceModelBundleReexportEvidenceInputs, S4ScoreBpcInputs, S4ScoreError,
    S4StrictV0SuccessOutcome, S4V0SuccessAcceptanceBits, S4V0SuccessInputs,
    s4_reexport_fixture_reference_model_bundle, s4_reference_model_bundle_reexport_evidence,
    s4_score_bpc, s4_v0_success_gutenberg, strict_v0_success_on_gutenberg,
};
use gbf_foundation::{Hash256, sha256};
use score_s3_support::{UniformEvaluator, repeated_a};
use serde_json::json;

#[test]
fn s4_score_bpc_binds_gutenberg_val_hash_and_reuses_reset_chunk() {
    let val = repeated_a(S4_SCORE_CHUNK_SIZE + 1);
    let corpus_val_sha = sha256(val.as_slice());

    let bpc = s4_score_bpc(S4ScoreBpcInputs {
        evaluator: UniformEvaluator,
        gutenberg_val: &val,
        corpus_val_sha,
    })
    .expect("Gutenberg score computes");

    assert!(bpc.get().is_finite());
    assert!((bpc.get() - (VOCAB_SIZE as f64).log2()).abs() < 1.0e-12);

    let error = s4_score_bpc(S4ScoreBpcInputs {
        evaluator: UniformEvaluator,
        gutenberg_val: &val,
        corpus_val_sha: hash(99),
    })
    .expect_err("Gutenberg val hash mismatch is rejected");

    assert!(matches!(
        error,
        S4ScoreError::HashMismatch {
            field: "corpus_val_sha",
            ..
        }
    ));
}

#[test]
fn s4_gutenberg_score_product_is_canonical_and_pins_json_shape() {
    let product = product_for_seed(0, passing_inputs(0)).expect("score product builds");

    assert_eq!(product.schema, S4_GUTENBERG_SCORE_SCHEMA);
    assert_eq!(product.bpc_margin, 0.25);
    assert!(product.v0_success_acceptance.all_set());
    assert!(product.pass);
    assert_eq!(
        product.score_self_hash,
        product.compute_self_hash().expect("self-hash recomputes")
    );

    let value = serde_json::to_value(&product).expect("score product serializes");
    assert_eq!(value["schema"], json!("s4_gutenberg_score.v1"));
    assert_eq!(value["seed"], json!(0));
    assert_eq!(value["bpc_ternary"], json!(1.0));
    assert_eq!(value["bpc_kn5"], json!(1.25));
    assert_eq!(value["bpc_margin"], json!(0.25));
    assert_eq!(
        value["v0_success_acceptance"],
        json!({
            "prompt_length_ok": true,
            "generation_length_ok": true,
            "no_repetition_collapse": true,
            "only_charset_v1_ids": true,
            "beats_kn5_baseline": true,
            "ternary_qat_gap_ok": true,
            "runtime_chrome_budget_ok": true,
            "emulator_smoke_ok": true
        })
    );
    assert_eq!(value["pass"], json!(true));
    assert!(value["score_self_hash"].as_str().is_some());

    let decoded: gbf_experiments::s4::score::S4V0SuccessProduct =
        serde_json::from_slice(&product.canonical_bytes().expect("canonical bytes"))
            .expect("canonical product decodes");
    decoded
        .validate_canonical_write()
        .expect("decoded product validates");
}

#[test]
fn s4_v0_success_uses_strict_margin_and_inherited_qat_gap() {
    let exact_margin = product_for_seed(
        0,
        inputs_with_bpcs(
            0,
            bpc(0.0),
            bpc(S4_MIN_BPC_MARGIN_VS_KN5),
            bpc(0.0),
            S4InheritedV0SuccessBits::all_pass(),
        ),
    )
    .expect("exact margin product builds");

    assert!(!exact_margin.v0_success_acceptance.beats_kn5_baseline);
    assert!(!exact_margin.pass);

    let gap_at_threshold = product_for_seed(
        1,
        inputs_with_bpcs(
            1,
            bpc(S4_MAX_TERNARY_QAT_GAP),
            bpc(1.0),
            bpc(0.0),
            S4InheritedV0SuccessBits::all_pass(),
        ),
    )
    .expect("threshold gap product builds");
    assert!(gap_at_threshold.v0_success_acceptance.ternary_qat_gap_ok);
    assert!(gap_at_threshold.pass);

    let gap_above_threshold = product_for_seed(
        2,
        inputs_with_bpcs(
            2,
            bpc(S4_MAX_TERNARY_QAT_GAP + 0.000_001),
            bpc(1.0),
            bpc(0.0),
            S4InheritedV0SuccessBits::all_pass(),
        ),
    )
    .expect("above-threshold gap product builds");
    assert!(!gap_above_threshold.v0_success_acceptance.ternary_qat_gap_ok);
    assert!(!gap_above_threshold.pass);
}

#[test]
fn s4_v0_success_q1_q6_bits_each_have_happy_and_failure_case() {
    let cases: [(
        &str,
        fn(S4V0SuccessAcceptanceBits) -> bool,
        fn(u64) -> S4V0SuccessInputs,
    ); 6] = [
        ("Q1_beats_kn5_baseline", q1_holds, q1_failure_inputs),
        ("Q2_ternary_qat_gap_ok", q2_holds, q2_failure_inputs),
        ("Q3_only_charset_v1_ids", q3_holds, q3_failure_inputs),
        ("Q4_no_repetition_collapse", q4_holds, q4_failure_inputs),
        ("Q5_generation_length_ok", q5_holds, q5_failure_inputs),
        ("Q6_runtime_chrome_budget_ok", q6_holds, q6_failure_inputs),
    ];

    for (offset, (name, bit, failure_inputs)) in cases.into_iter().enumerate() {
        let seed = offset as u64 % 5;
        let happy =
            product_for_seed(seed, passing_inputs(seed)).expect("happy score product builds");
        assert!(bit(happy.v0_success_acceptance), "{name} happy path");
        assert!(happy.pass, "{name} happy product passes");

        let failed = product_for_seed(seed, failure_inputs(seed))
            .unwrap_or_else(|error| panic!("{name} failure product should canonicalize: {error}"));
        assert!(
            !bit(failed.v0_success_acceptance),
            "{name} injected failure"
        );
        assert_eq!(
            acceptance_false_count(failed.v0_success_acceptance),
            1,
            "{name} injected failure should isolate one bit"
        );
        assert!(
            !failed.pass,
            "{name} failing bit must fail the per-seed product"
        );
    }
}

#[test]
fn strict_v0_success_on_gutenberg_rejects_three_of_five_softening() {
    let passing = (0..5)
        .map(|seed| product_for_seed(seed, passing_inputs(seed)).expect("passing score product"))
        .collect::<Vec<_>>();
    let pass = strict_v0_success_on_gutenberg(&passing).expect("canonical seed set validates");
    assert_eq!(pass, S4StrictV0SuccessOutcome::Pass);
    assert!(pass.passed());

    let mut one_failed = passing.clone();
    let mut inherited = S4InheritedV0SuccessBits::all_pass();
    inherited.no_repetition_collapse = false;
    one_failed[3] = product_for_seed(
        3,
        inputs_with_bpcs(3, bpc(1.0), bpc(1.25), bpc(0.95), inherited),
    )
    .expect("failed score product still canonicalizes");

    assert_eq!(
        strict_v0_success_on_gutenberg(&one_failed)
            .expect("strict seed set validates even when one seed fails"),
        S4StrictV0SuccessOutcome::Fail { failing_seed: 3 },
        "H4 must name the failed seed instead of hiding it behind an aggregate bool"
    );

    let error = strict_v0_success_on_gutenberg(&passing[..4])
        .expect_err("missing seed cannot satisfy strict H4");
    assert!(matches!(
        error,
        S4ScoreError::NonCanonicalScoreSeedSet { .. }
    ));
}

#[test]
fn s4_gutenberg_score_canonical_validation_rejects_tampering() {
    let product = product_for_seed(0, passing_inputs(0)).expect("score product builds");

    let mut bad_schema = product.clone();
    bad_schema.schema = "not_s4_gutenberg_score.v1".to_owned();
    bad_schema.score_self_hash = bad_schema.compute_self_hash().expect("tamper self-hash");
    assert!(matches!(
        bad_schema
            .validate_canonical_write()
            .expect_err("schema tamper is rejected"),
        S4ScoreError::InvalidSchema { .. }
    ));

    let mut bad_margin = product.clone();
    bad_margin.bpc_margin += 0.125;
    bad_margin.score_self_hash = bad_margin.compute_self_hash().expect("tamper self-hash");
    assert!(matches!(
        bad_margin
            .validate_canonical_write()
            .expect_err("bpc margin tamper is rejected"),
        S4ScoreError::BpcMarginMismatch { .. }
    ));

    let mut bad_acceptance = product.clone();
    bad_acceptance.v0_success_acceptance.beats_kn5_baseline = false;
    bad_acceptance.score_self_hash = bad_acceptance
        .compute_self_hash()
        .expect("tamper self-hash");
    assert!(matches!(
        bad_acceptance
            .validate_canonical_write()
            .expect_err("acceptance-bit tamper is rejected"),
        S4ScoreError::AcceptanceBitMismatch {
            field: "v0_success_acceptance.beats_kn5_baseline",
            ..
        }
    ));

    let mut bad_pass = product.clone();
    bad_pass.pass = false;
    bad_pass.score_self_hash = bad_pass.compute_self_hash().expect("tamper self-hash");
    assert!(matches!(
        bad_pass
            .validate_canonical_write()
            .expect_err("pass-bit tamper is rejected"),
        S4ScoreError::PassBitMismatch { .. }
    ));

    let mut missing_checkpoint = product.clone();
    missing_checkpoint.checkpoint_self_hash = Hash256::ZERO;
    missing_checkpoint.score_self_hash = missing_checkpoint
        .compute_self_hash()
        .expect("tamper self-hash");
    assert!(matches!(
        missing_checkpoint
            .validate_canonical_write()
            .expect_err("checkpoint hash is required"),
        S4ScoreError::MissingHash {
            field: "checkpoint_self_hash"
        }
    ));

    let mut bad_self_hash = product.clone();
    bad_self_hash.score_self_hash = hash(250);
    assert!(matches!(
        bad_self_hash
            .validate_canonical_write()
            .expect_err("score self-hash tamper is rejected"),
        S4ScoreError::SelfHashMismatch { .. }
    ));
}

#[test]
fn s4_reference_model_bundle_reexport_uses_s3_contract_unchanged() {
    let product =
        s4_reexport_fixture_reference_model_bundle(0).expect("S4 re-export uses S3 bundle export");
    let _bundle: &ReferenceModelBundle = &product.bundle;

    assert!(product.program_validation.prompt_subset_pass());
    assert_eq!(product.bundle.bundle_self_hash, product.bundle_self_hash);
    assert_eq!(product.metadata.bundle_self_hash, product.bundle_self_hash);
    assert_eq!(
        product.metadata.canonical_bundle_payload_sha,
        product.canonical_bundle_payload_sha
    );
}

#[test]
fn s4_reference_model_bundle_reexport_evidence_binds_checkpoint_and_gutenberg_val() {
    let val = repeated_a(8);
    let corpus_val_sha = sha256(val.as_slice());
    let bundle_export =
        s4_reexport_fixture_reference_model_bundle(0).expect("S4 re-export uses S3 bundle export");

    let evidence =
        s4_reference_model_bundle_reexport_evidence(S4ReferenceModelBundleReexportEvidenceInputs {
            seed: 0,
            checkpoint_self_hash: hash(80),
            corpus_val_sha,
            gutenberg_val: &val,
            conformance_fixture_set_self_hash: hash(81),
            bundle_export: &bundle_export,
        })
        .expect("re-export evidence binds checkpoint and Gutenberg val");

    assert_eq!(
        evidence.schema,
        S4_REFERENCE_MODEL_BUNDLE_REEXPORT_EVIDENCE_SCHEMA
    );
    assert_eq!(
        evidence.conformance_fixture_semantics,
        S4_REFERENCE_MODEL_BUNDLE_REEXPORT_FIXTURE_SEMANTICS
    );
    assert_eq!(evidence.seed, 0);
    assert_eq!(evidence.checkpoint_self_hash, hash(80));
    assert_eq!(evidence.corpus_val_sha, corpus_val_sha);
    assert_eq!(evidence.conformance_fixture_set_self_hash, hash(81));
    assert_eq!(evidence.bundle_self_hash, bundle_export.bundle_self_hash);
    assert_eq!(
        evidence.canonical_bundle_payload_sha,
        bundle_export.canonical_bundle_payload_sha
    );
    assert_eq!(
        evidence.reference_model_bundle_reexport_self_hash,
        evidence.compute_self_hash().expect("evidence self-hash")
    );

    let decoded: S4ReferenceModelBundleReexportEvidence =
        serde_json::from_slice(&evidence.canonical_bytes().expect("canonical bytes"))
            .expect("canonical evidence decodes");
    decoded
        .validate_canonical_write()
        .expect("decoded evidence validates");

    let error =
        s4_reference_model_bundle_reexport_evidence(S4ReferenceModelBundleReexportEvidenceInputs {
            seed: 0,
            checkpoint_self_hash: hash(80),
            corpus_val_sha: hash(82),
            gutenberg_val: &val,
            conformance_fixture_set_self_hash: hash(81),
            bundle_export: &bundle_export,
        })
        .expect_err("Gutenberg val hash mismatch is rejected before binding evidence");
    assert!(matches!(
        error,
        S4ScoreError::HashMismatch {
            field: "corpus_val_sha",
            ..
        }
    ));

    let mut tampered_semantics = evidence.clone();
    tampered_semantics.conformance_fixture_semantics = "tinystories_fixture".to_owned();
    tampered_semantics.reference_model_bundle_reexport_self_hash = tampered_semantics
        .compute_self_hash()
        .expect("tamper self-hash");
    assert!(matches!(
        tampered_semantics
            .validate_canonical_write()
            .expect_err("fixture semantics tamper is rejected"),
        S4ScoreError::InvalidReexportFixtureSemantics { .. }
    ));

    let mut tampered_self_hash = evidence.clone();
    tampered_self_hash.reference_model_bundle_reexport_self_hash = hash(83);
    assert!(matches!(
        tampered_self_hash
            .validate_canonical_write()
            .expect_err("evidence self-hash tamper is rejected"),
        S4ScoreError::ReferenceModelBundleReexportSelfHashMismatch { .. }
    ));
}

fn product_for_seed(
    seed: u64,
    mut inputs: S4V0SuccessInputs,
) -> Result<gbf_experiments::s4::score::S4V0SuccessProduct, S4ScoreError> {
    inputs.seed = seed;
    s4_v0_success_gutenberg(inputs)
}

fn q1_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.beats_kn5_baseline
}

fn q2_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.ternary_qat_gap_ok
}

fn q3_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.only_charset_v1_ids
}

fn q4_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.no_repetition_collapse
}

fn q5_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.generation_length_ok
}

fn q6_holds(bits: S4V0SuccessAcceptanceBits) -> bool {
    bits.runtime_chrome_budget_ok
}

fn q1_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    inputs_with_bpcs(
        seed,
        bpc(1.0),
        bpc(1.0),
        bpc(0.95),
        S4InheritedV0SuccessBits::all_pass(),
    )
}

fn q2_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    inputs_with_bpcs(
        seed,
        bpc(S4_MAX_TERNARY_QAT_GAP + 0.000_001),
        bpc(S4_MAX_TERNARY_QAT_GAP + 1.0),
        bpc(0.0),
        S4InheritedV0SuccessBits::all_pass(),
    )
}

fn q3_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    let mut inherited = S4InheritedV0SuccessBits::all_pass();
    inherited.only_charset_v1_ids = false;
    inputs_with_bpcs(seed, bpc(1.0), bpc(1.25), bpc(0.95), inherited)
}

fn q4_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    let mut inherited = S4InheritedV0SuccessBits::all_pass();
    inherited.no_repetition_collapse = false;
    inputs_with_bpcs(seed, bpc(1.0), bpc(1.25), bpc(0.95), inherited)
}

fn q5_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    let mut inherited = S4InheritedV0SuccessBits::all_pass();
    inherited.generation_length_ok = false;
    inputs_with_bpcs(seed, bpc(1.0), bpc(1.25), bpc(0.95), inherited)
}

fn q6_failure_inputs(seed: u64) -> S4V0SuccessInputs {
    let mut inherited = S4InheritedV0SuccessBits::all_pass();
    inherited.runtime_chrome_budget_ok = false;
    inputs_with_bpcs(seed, bpc(1.0), bpc(1.25), bpc(0.95), inherited)
}

fn acceptance_false_count(bits: S4V0SuccessAcceptanceBits) -> usize {
    [
        bits.prompt_length_ok,
        bits.generation_length_ok,
        bits.no_repetition_collapse,
        bits.only_charset_v1_ids,
        bits.beats_kn5_baseline,
        bits.ternary_qat_gap_ok,
        bits.runtime_chrome_budget_ok,
        bits.emulator_smoke_ok,
    ]
    .into_iter()
    .filter(|bit| !bit)
    .count()
}

fn passing_inputs(seed: u64) -> S4V0SuccessInputs {
    inputs_with_bpcs(
        seed,
        bpc(1.0),
        bpc(1.25),
        bpc(0.95),
        S4InheritedV0SuccessBits::all_pass(),
    )
}

fn inputs_with_bpcs(
    seed: u64,
    bpc_ternary: S4BpcValue,
    bpc_kn5: S4BpcValue,
    bpc_fp_reference: S4BpcValue,
    inherited_acceptance: S4InheritedV0SuccessBits,
) -> S4V0SuccessInputs {
    S4V0SuccessInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: hash(2),
        seed,
        checkpoint_self_hash: hash(10 + seed as u8),
        checkpoint_payload_sha: hash(20 + seed as u8),
        corpus_val_sha: hash(30),
        workload_manifest_template_self_hash: hash(40),
        workload_manifest_instance_self_hash: hash(41),
        fp_reference_self_hash: hash(50 + seed as u8),
        bpc_ternary,
        bpc_kn5,
        bpc_fp_reference,
        inherited_acceptance,
    }
}

fn bpc(value: f64) -> S4BpcValue {
    S4BpcValue::try_new(value).expect("fixture bpc is valid")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
