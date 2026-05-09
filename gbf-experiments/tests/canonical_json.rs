mod common;

use common::strategies::arb_canonical_json_value;
use common::tracing_capture::{TraceCapture, TracingEvent, captured_events, with_trace_capture};
use gbf_experiments::s1::schema::{
    AblationReport, BaselineReport, CheckpointMetadata, CountsSummary, DomainHash, GitCommitId,
    GradNormSummary, NegativeTestReport, PerSeedArtifacts, ReportFrontMatter, RfcRevisionRef,
    RunLog, S1BuildKind, S1CanonicalJson, S1Completion, S1Decision, S1Outcome, S1SchemaError,
    ScoreReport, SmoothingScheme, TensorMismatch, self_hash_for_value, value_with_self_hash,
    value_without_self_hash,
};
use gbf_foundation::{Hash256, SemVer};
use proptest::prelude::*;
use serde_json::{Value, json};

fn domain() -> DomainHash<'static> {
    DomainHash::new("gbf-experiments", "s1_test", "s1_test.v1", "1.0.0")
}

#[test]
fn canonical_json_sorts_keys_and_has_no_insignificant_whitespace() {
    let shuffled = json!({"b": 2, "a": 1, "nested": {"z": true, "m": null}});
    let ordered = json!({"a": 1, "b": 2, "nested": {"m": null, "z": true}});

    let shuffled_bytes = S1CanonicalJson::value_to_vec(&shuffled).expect("canonical JSON");
    let ordered_bytes = S1CanonicalJson::value_to_vec(&ordered).expect("canonical JSON");

    assert_eq!(shuffled_bytes, ordered_bytes);
    assert_eq!(
        shuffled_bytes,
        br#"{"a":1,"b":2,"nested":{"m":null,"z":true}}"#
    );
    assert_eq!(
        to_hex(&shuffled_bytes),
        "7b2261223a312c2262223a322c226e6573746564223a7b226d223a6e756c6c2c227a223a747275657d7d"
    );
}

#[test]
fn canonical_json_normalizes_negative_zero_float() {
    let negative_zero = serde_json::Number::from_f64(-0.0).expect("finite f64");
    let positive_zero = serde_json::Number::from_f64(0.0).expect("finite f64");

    assert_eq!(
        S1CanonicalJson::value_to_vec(&Value::Number(negative_zero)).expect("canonical JSON"),
        b"0.0"
    );
    assert_eq!(
        S1CanonicalJson::value_to_vec(&Value::Number(positive_zero)).expect("canonical JSON"),
        b"0.0"
    );
}

#[test]
fn canonical_json_uses_shortest_round_trip_float_decimals() {
    let tricky = (0.1_f64 + 0.2_f64, 1e308_f64, f64::from_bits(1));

    assert_eq!(
        S1CanonicalJson::to_vec(&tricky).expect("canonical JSON"),
        b"[0.30000000000000004,1e308,5e-324]"
    );
}

#[test]
fn canonical_json_rejects_non_finite_floats() {
    assert!(matches!(
        S1CanonicalJson::to_vec(&f64::NAN),
        Err(S1SchemaError::NonFiniteFloat)
    ));
    assert!(matches!(
        S1CanonicalJson::to_vec(&f64::INFINITY),
        Err(S1SchemaError::NonFiniteFloat)
    ));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn canonical_json_parse_reserialize_is_byte_stable(value in arb_canonical_json_value()) {
        let bytes = S1CanonicalJson::value_to_vec(&value).expect("canonical JSON");
        let parsed: Value = serde_json::from_slice(&bytes).expect("canonical JSON parses");
        let reparsed_bytes = S1CanonicalJson::value_to_vec(&parsed).expect("canonical JSON reserializes");

        prop_assert_eq!(reparsed_bytes, bytes);
    }

    #[test]
    fn finite_f64_canonical_json_round_trips(value in (-1_000_000_i32..=1_000_000).prop_map(f64::from)) {
        let bytes = S1CanonicalJson::to_vec(&value).expect("finite f64 canonicalizes");
        let parsed: f64 = serde_json::from_slice(&bytes).expect("finite f64 canonical JSON parses");
        let parsed_value: Value = serde_json::from_slice(&bytes).expect("finite f64 canonical JSON parses as Value");
        let reparsed_bytes = S1CanonicalJson::value_to_vec(&parsed_value).expect("finite f64 canonical JSON reserializes");

        prop_assert_eq!(parsed, if value == 0.0 { 0.0 } else { value });
        prop_assert!(parsed_value.as_f64().expect("JSON number is an f64").is_finite());
        prop_assert_eq!(reparsed_bytes, bytes);
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn arb_checkpoint_metadata_round_trips(artifact in arb_checkpoint_metadata()) {
        assert_artifact_round_trip(&artifact, CheckpointMetadata::computed_self_hash);
    }

    #[test]
    fn arb_run_log_round_trips(artifact in arb_run_log()) {
        assert_artifact_round_trip(&artifact, RunLog::computed_self_hash);
    }

    #[test]
    fn arb_score_report_round_trips(artifact in arb_score_report()) {
        assert_artifact_round_trip(&artifact, ScoreReport::computed_self_hash);
    }

    #[test]
    fn arb_negative_test_report_round_trips(artifact in arb_negative_test_report()) {
        assert_artifact_round_trip(&artifact, NegativeTestReport::computed_self_hash);
    }

    #[test]
    fn arb_ablation_report_round_trips(artifact in arb_ablation_report()) {
        assert_artifact_round_trip(&artifact, AblationReport::computed_self_hash);
    }

    #[test]
    fn arb_baseline_report_round_trips(artifact in arb_baseline_report()) {
        assert_artifact_round_trip(&artifact, BaselineReport::computed_self_hash);
    }

    #[test]
    fn arb_report_front_matter_round_trips(artifact in arb_report_front_matter()) {
        assert_artifact_round_trip(&artifact, ReportFrontMatter::computed_self_hash);
    }
}

#[test]
fn domain_hash_uses_nul_separator_and_rejects_embedded_self_hash() {
    let payload = json!({"a": 1});
    let hash = domain().hash(&payload).expect("domain hash");

    assert_eq!(
        hash.to_string(),
        "sha256:b79888197edba52cecaff5889ea3190e36bee6f5093c74660c221a2ca18178d5"
    );
    assert_ne!(
        hash,
        DomainHash::new("gbf-experiments", "s1_test", "s1_test.v1", "1.0.0 ")
            .hash(&payload)
            .expect("domain hash with changed separator neighborhood")
    );

    let with_self_hash = json!({"payload": true, "test_self_hash": hash.to_string()});
    assert!(matches!(
        domain().hash(&with_self_hash),
        Err(S1SchemaError::SelfHashFieldMustBeOmitted(field)) if field == "test_self_hash"
    ));
}

#[test]
fn a_domain_hash_trace_fields_exclude_payload_bytes() {
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        let _ = domain()
            .hash(&json!({"payload": "not present in trace fields"}))
            .expect("domain hash");
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "s1.domain_hash")
        .expect("domain hash trace event");
    assert_eq!(event.fields.get("schema_id"), Some(&json!("s1_test.v1")));
    assert_eq!(
        event.fields.get("crate_name"),
        Some(&json!("gbf-experiments"))
    );
    assert_eq!(event.fields.get("type_name"), Some(&json!("s1_test")));
    assert_eq!(event.fields.get("schema_version"), Some(&json!("1.0.0")));
    assert!(
        !event.fields.contains_key("payload") && !event.fields.contains_key("canonical_json"),
        "DomainHash trace event must not record payload bytes: {event:?}"
    );
}

#[test]
fn schema_hash_start_and_complete_trace_events_are_captured() {
    let score = score_report();
    let report = report_front_matter();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        assert_eq!(
            score.computed_self_hash().expect("score hash"),
            score.score_self_hash
        );
        assert_eq!(
            report.computed_self_hash().expect("report hash"),
            report.report_self_hash
        );
    });

    let events = captured_events(&capture);
    assert_schema_hash_events(
        &events,
        "s1_score.v1",
        score.score_self_hash.to_string().as_str(),
    );
    assert_schema_hash_events(
        &events,
        "s1_report.v1",
        report.report_self_hash.to_string().as_str(),
    );
}

#[test]
fn self_hash_helpers_exclude_the_self_hash_field() {
    let unhashed = json!({"payload": {"stable": true}});
    let dummy_hashed = json!({
        "payload": {"stable": true},
        "test_self_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    });

    let from_unhashed =
        self_hash_for_value(domain(), &unhashed, "test_self_hash").expect("self hash");
    let from_dummy =
        self_hash_for_value(domain(), &dummy_hashed, "test_self_hash").expect("self hash");

    assert_eq!(from_unhashed, from_dummy);
    assert_eq!(
        value_without_self_hash(&dummy_hashed, "test_self_hash").expect("stripped"),
        unhashed
    );

    let with_hash =
        value_with_self_hash(domain(), &dummy_hashed, "test_self_hash").expect("attached hash");
    assert_eq!(
        with_hash["test_self_hash"],
        json!(from_unhashed.to_string())
    );
    assert_eq!(with_hash["payload"], unhashed["payload"]);
}

#[test]
fn self_hash_helpers_reject_stray_self_hash_fields() {
    let two_self_hashes = json!({
        "payload": {"stable": true},
        "test_self_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "other_self_hash": "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    });

    assert!(matches!(
        self_hash_for_value(domain(), &two_self_hashes, "test_self_hash"),
        Err(S1SchemaError::SelfHashFieldMustBeOmitted(field)) if field == "other_self_hash"
    ));
    assert!(matches!(
        value_with_self_hash(domain(), &two_self_hashes, "test_self_hash"),
        Err(S1SchemaError::SelfHashFieldMustBeOmitted(field)) if field == "other_self_hash"
    ));
}

#[test]
fn artifact_self_hash_paths_reject_programmatic_non_finite_floats() {
    let mut checkpoint = checkpoint_metadata();
    checkpoint.final_train_loss = f32::NAN;
    assert_rejects_non_finite(checkpoint.canonical_json_bytes());
    assert_rejects_non_finite(checkpoint.computed_self_hash());

    let mut loss_run_log = run_log();
    loss_run_log.losses[0].1 = f32::INFINITY;
    assert_rejects_non_finite(loss_run_log.canonical_json_bytes());
    assert_rejects_non_finite(loss_run_log.computed_self_hash());

    let mut grad_run_log = run_log();
    grad_run_log.final_grad_norms.global_l2 = f32::NEG_INFINITY;
    assert_rejects_non_finite(grad_run_log.canonical_json_bytes());
    assert_rejects_non_finite(grad_run_log.computed_self_hash());

    let mut score = score_report();
    score.bpc = f64::NAN;
    assert_rejects_non_finite(score.canonical_json_bytes());
    assert_rejects_non_finite(score.computed_self_hash());

    let mut negative = negative_test_report();
    negative.delta = f64::INFINITY;
    assert_rejects_non_finite(negative.canonical_json_bytes());
    assert_rejects_non_finite(negative.computed_self_hash());

    let mut baseline = baseline_report();
    baseline.smoothing.lambdas[1] = f64::NEG_INFINITY;
    assert_rejects_non_finite(baseline.canonical_json_bytes());
    assert_rejects_non_finite(baseline.computed_self_hash());
}

#[test]
fn checkpoint_metadata_canonical_round_trip_and_self_hash() {
    let artifact = checkpoint_metadata();

    assert_artifact_round_trip(&artifact, CheckpointMetadata::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_checkpoint.v1",
            "seed": 0,
            "corpus_train_sha": hash(1).to_string(),
            "corpus_val_sha": hash(2).to_string(),
            "model_config_hash": hash(3).to_string(),
            "train_config_hash": hash(4).to_string(),
            "build_kind": "phase_a",
            "checkpoint_safetensors_sha256": hash(13).to_string(),
            "build_config_hash": hash(8).to_string(),
            "dependency_lockfile_sha": hash(9).to_string(),
            "rust_toolchain_hash": hash(10).to_string(),
            "device_profile_hash": hash(11).to_string(),
            "rng_stream_def_hash": hash(12).to_string(),
            "pass_version": {"major": 0, "minor": 1, "patch": 0},
            "budget_profile": "production",
            "final_step": 10000,
            "final_train_loss": 1.25,
            "completion": {"kind": "completed"},
            "checkpoint_self_hash": artifact.checkpoint_self_hash.to_string(),
        })
    );
    assert_eq!(
        artifact.checkpoint_self_hash,
        artifact.computed_self_hash().expect("checkpoint self hash")
    );
    assert_eq!(
        artifact.canonical_json_bytes().expect("checkpoint bytes"),
        br#"{"budget_profile":"production","build_config_hash":"sha256:0808080808080808080808080808080808080808080808080808080808080808","build_kind":"phase_a","checkpoint_safetensors_sha256":"sha256:0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d","completion":{"kind":"completed"},"corpus_train_sha":"sha256:0101010101010101010101010101010101010101010101010101010101010101","corpus_val_sha":"sha256:0202020202020202020202020202020202020202020202020202020202020202","dependency_lockfile_sha":"sha256:0909090909090909090909090909090909090909090909090909090909090909","device_profile_hash":"sha256:0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b","final_step":10000,"final_train_loss":1.25,"model_config_hash":"sha256:0303030303030303030303030303030303030303030303030303030303030303","pass_version":{"major":0,"minor":1,"patch":0},"rng_stream_def_hash":"sha256:0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c","rust_toolchain_hash":"sha256:0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a","schema":"s1_checkpoint.v1","seed":0,"train_config_hash":"sha256:0404040404040404040404040404040404040404040404040404040404040404"}"#
    );

    let mut mutated = artifact.clone();
    mutated.final_step ^= 1;
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn remaining_artifact_canonical_preimage_bytes_are_pinned() {
    assert_eq!(
        run_log().canonical_json_bytes().expect("run-log bytes"),
        br#"{"eval_points":[[0,2.75],[1000,2.25]],"final_grad_norms":{"global_l2":3.0,"max_l2":2.0,"mean_l2":1.0},"losses":[[1,1.5],[2,1.25]],"schema":"s1_run_log.v1","seed":0,"train_config_hash":"sha256:0404040404040404040404040404040404040404040404040404040404040404"}"#
    );
    assert_eq!(
        score_report().canonical_json_bytes().expect("score bytes"),
        br#"{"bpc":2.000244140625,"checkpoint_sha":"sha256:0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c","chunk_size":128,"corpus_val_sha":"sha256:0202020202020202020202020202020202020202020202020202020202020202","log2_sum":4096.5,"schema":"s1_score.v1","seed":0,"token_count":2048}"#
    );
    assert_eq!(
        negative_test_report()
            .canonical_json_bytes()
            .expect("negative bytes"),
        br#"{"bpc_original":2.0,"bpc_shuffled":7.5,"checkpoint_sha":"sha256:0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c","corpus_val_sha":"sha256:0202020202020202020202020202020202020202020202020202020202020202","delta":5.5,"schema":"s1_negative_test.v1","seed":0,"sensitive":true,"shuffle_seed":3735928559,"shuffled_val_sha256":"sha256:0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d"}"#
    );
    assert_eq!(
        ablation_report()
            .canonical_json_bytes()
            .expect("ablation bytes"),
        br#"{"ablation_checkpoint_sha":"sha256:0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e","ablation_tensor_payload_sha":"sha256:1010101010101010101010101010101010101010101010101010101010101010","first_mismatch":{"byte_offset":17,"tensor":"toy.blocks.0.weight"},"phase_a_checkpoint_sha":"sha256:0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c","phase_a_eq_ablation":false,"phase_a_tensor_payload_sha":"sha256:0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f","schema":"s1_ablation.v1","seed":0}"#
    );
    assert_eq!(
        baseline_report()
            .canonical_json_bytes()
            .expect("baseline bytes"),
        br#"{"bpc_2gram":3.0,"bpc_3gram":2.5,"bpc_unigram":4.0,"corpus_train_sha":"sha256:0101010101010101010101010101010101010101010101010101010101010101","corpus_val_sha":"sha256:0202020202020202020202020202020202020202020202020202020202020202","counts_blob_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","counts_summary":{"distinct_bigrams":11,"distinct_trigrams":10,"distinct_unigrams":8,"train_bytes":12},"schema":"s1_baseline.v1","smoothing":{"alpha":0.25,"lambdas":[0.2,0.3,0.5]}}"#
    );
    assert_eq!(
        report_front_matter()
            .canonical_json_bytes()
            .expect("report bytes"),
        br#"{"baseline_self_hash":"sha256:e5f2181d8e5dd0b6e3d016bfcd5cdeee016d8606764b27f4fd1f68ffcd55e4a6","decision":{"kind":"ProceedToS2"},"first_result_commit":"2222222222222222222222222222222222222222","per_seed_artifacts":[{"ablation_self_hash":"sha256:edca8bf700d33eeadd669b063e24d6d2caa28b21b314c050cc9068ae075a5e01","checkpoint_self_hash":"sha256:4a114aa464ad370e67497d7dff177619f15b59d3d3061f81b65b9c05abe3a2ea","completion":{"kind":"completed"},"negative_self_hash":"sha256:5e8b9d44b4b0b2bd8f23c928e3bfb49f99c0f97035581a1cb5c0ca1b94947a0a","run_log_self_hash":"sha256:62af639aa186b47048e7f70feadad260942820cf081e1cc307f83570df09d4ff","score_self_hash":"sha256:87eaa1cc70c5eb06d65015d1ebc955724c4720a11473fdec296908f837909abc","seed":0}],"predictions_commit":"1111111111111111111111111111111111111111","predictions_section_hash":"sha256:1212121212121212121212121212121212121212121212121212121212121212","rfc_revision":"0123456789abcdef0123456789abcdef01234567","s1_outcome":"Pass-clean","schema":"s1_report.v1"}"#
    );
}

#[test]
fn run_log_canonical_round_trip_and_self_hash() {
    let artifact = run_log();

    assert_artifact_round_trip(&artifact, RunLog::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_run_log.v1",
            "seed": 0,
            "train_config_hash": hash(4).to_string(),
            "losses": [[1, 1.5], [2, 1.25]],
            "eval_points": [[0, 2.75], [1000, 2.25]],
            "final_grad_norms": {"global_l2": 3.0, "max_l2": 2.0, "mean_l2": 1.0},
            "run_log_self_hash": artifact.run_log_self_hash.to_string(),
        })
    );
    assert_eq!(
        artifact.run_log_self_hash,
        artifact.computed_self_hash().expect("run log self hash")
    );

    let mut mutated = artifact.clone();
    mutated.losses[0].1 = f32::from_bits(mutated.losses[0].1.to_bits() ^ 1);
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn score_report_canonical_round_trip_and_self_hash() {
    let artifact = score_report();

    assert_artifact_round_trip(&artifact, ScoreReport::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_score.v1",
            "seed": 0,
            "checkpoint_sha": hash(12).to_string(),
            "corpus_val_sha": hash(2).to_string(),
            "chunk_size": 128,
            "token_count": 2048,
            "log2_sum": 4096.5,
            "bpc": 2.000244140625_f64,
            "score_self_hash": artifact.score_self_hash.to_string(),
        })
    );

    let mut mutated = artifact.clone();
    mutated.token_count ^= 1;
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn negative_test_canonical_round_trip_and_self_hash() {
    let artifact = negative_test_report();

    assert_artifact_round_trip(&artifact, NegativeTestReport::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_negative_test.v1",
            "seed": 0,
            "checkpoint_sha": hash(12).to_string(),
            "corpus_val_sha": hash(2).to_string(),
            "shuffle_seed": 0xDEADBEEF_u64,
            "bpc_original": 2.0,
            "bpc_shuffled": 7.5,
            "shuffled_val_sha256": hash(13).to_string(),
            "delta": 5.5,
            "sensitive": true,
            "negative_self_hash": artifact.negative_self_hash.to_string(),
        })
    );

    let mut mutated = artifact.clone();
    mutated.sensitive = !mutated.sensitive;
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn ablation_report_canonical_round_trip_and_self_hash() {
    let artifact = ablation_report();

    assert_artifact_round_trip(&artifact, AblationReport::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_ablation.v1",
            "seed": 0,
            "phase_a_checkpoint_sha": hash(12).to_string(),
            "ablation_checkpoint_sha": hash(14).to_string(),
            "phase_a_tensor_payload_sha": hash(15).to_string(),
            "ablation_tensor_payload_sha": hash(16).to_string(),
            "phase_a_eq_ablation": false,
            "first_mismatch": {"tensor": "toy.blocks.0.weight", "byte_offset": 17},
            "ablation_self_hash": artifact.ablation_self_hash.to_string(),
        })
    );

    let mut mutated = artifact.clone();
    mutated.phase_a_eq_ablation = !mutated.phase_a_eq_ablation;
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn baseline_report_canonical_round_trip_and_self_hash() {
    let artifact = baseline_report();

    assert_artifact_round_trip(&artifact, BaselineReport::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_baseline.v1",
            "corpus_train_sha": hash(1).to_string(),
            "corpus_val_sha": hash(2).to_string(),
            "smoothing": {"alpha": 0.25, "lambdas": [0.2, 0.3, 0.5]},
            "bpc_3gram": 2.5,
            "bpc_2gram": 3.0,
            "bpc_unigram": 4.0,
            "counts_summary": {
                "train_bytes": 12,
                "distinct_unigrams": 8,
                "distinct_bigrams": 11,
                "distinct_trigrams": 10,
            },
            "counts_blob_sha256": hash(17).to_string(),
            "baseline_self_hash": artifact.baseline_self_hash.to_string(),
        })
    );

    let mut mutated = artifact.clone();
    mutated.smoothing.lambdas[2] = f64::from_bits(mutated.smoothing.lambdas[2].to_bits() ^ 1);
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );
}

#[test]
fn report_front_matter_canonical_round_trip_and_self_hash() {
    let artifact = report_front_matter();

    assert_artifact_round_trip(&artifact, ReportFrontMatter::computed_self_hash);
    assert_eq!(
        json_value(&artifact),
        json!({
            "schema": "s1_report.v1",
            "s1_outcome": "Pass-clean",
            "decision": {"kind": "ProceedToS2"},
            "baseline_self_hash": baseline_report().baseline_self_hash.to_string(),
            "per_seed_artifacts": [{
                "seed": 0,
                "completion": {"kind": "completed"},
                "checkpoint_self_hash": checkpoint_metadata().checkpoint_self_hash.to_string(),
                "run_log_self_hash": run_log().run_log_self_hash.to_string(),
                "score_self_hash": score_report().score_self_hash.to_string(),
                "negative_self_hash": negative_test_report().negative_self_hash.to_string(),
                "ablation_self_hash": ablation_report().ablation_self_hash.to_string(),
            }],
            "generated_at": "2026-05-09T12:00:00Z",
            "rfc_revision": "0123456789abcdef0123456789abcdef01234567",
            "predictions_section_hash": hash(18).to_string(),
            "predictions_commit": "1111111111111111111111111111111111111111",
            "first_result_commit": "2222222222222222222222222222222222222222",
            "report_self_hash": artifact.report_self_hash.to_string(),
        })
    );

    let mut mutated = artifact.clone();
    mutated.s1_outcome = S1Outcome::PassWithWarning;
    assert_ne!(
        artifact.computed_self_hash().expect("original hash"),
        mutated.computed_self_hash().expect("mutated hash")
    );

    let mut generated_at_only = artifact.clone();
    generated_at_only.generated_at = "2030-01-01T00:00:00Z".to_owned();
    assert_eq!(
        artifact.computed_self_hash().expect("original hash"),
        generated_at_only
            .computed_self_hash()
            .expect("generated_at-only hash")
    );
}

#[test]
fn schema_deserialization_rejects_missing_unknown_and_negative_fields() {
    assert_schema_rejects::<CheckpointMetadata>(
        json_value(&checkpoint_metadata()),
        "final_step",
        "s1_score.v1",
    );
    assert_schema_rejects::<RunLog>(json_value(&run_log()), "losses", "s1_score.v1");
    assert_schema_rejects::<ScoreReport>(
        json_value(&score_report()),
        "checkpoint_sha",
        "s1_checkpoint.v1",
    );
    assert_schema_rejects::<NegativeTestReport>(
        json_value(&negative_test_report()),
        "shuffle_seed",
        "s1_score.v1",
    );
    assert_schema_rejects::<AblationReport>(
        json_value(&ablation_report()),
        "phase_a_checkpoint_sha",
        "s1_score.v1",
    );
    assert_schema_rejects::<BaselineReport>(
        json_value(&baseline_report()),
        "counts_summary",
        "s1_score.v1",
    );
    assert_schema_rejects::<ReportFrontMatter>(
        json_value(&report_front_matter()),
        "per_seed_artifacts",
        "s1_score.v1",
    );

    let mut invalid_commit = json_value(&report_front_matter());
    invalid_commit["predictions_commit"] = json!("not-a-commit");
    assert!(serde_json::from_value::<ReportFrontMatter>(invalid_commit).is_err());

    let mut negative = json_value(&score_report());
    negative["bpc"] = json!(-0.25);
    assert!(serde_json::from_value::<ScoreReport>(negative).is_err());

    let mut non_finite = json_value(&score_report()).to_string();
    non_finite = non_finite.replace("\"log2_sum\":4096.5", "\"log2_sum\":1e999");
    assert!(serde_json::from_str::<ScoreReport>(&non_finite).is_err());
}

fn checkpoint_metadata() -> CheckpointMetadata {
    CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed: 0,
        corpus_train_sha: hash(1),
        corpus_val_sha: hash(2),
        model_config_hash: hash(3),
        train_config_hash: hash(4),
        build_kind: S1BuildKind::PhaseA,
        build_config_hash: hash(8),
        dependency_lockfile_sha: hash(9),
        rust_toolchain_hash: hash(10),
        device_profile_hash: hash(11),
        rng_stream_def_hash: hash(12),
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: "production".to_owned(),
        final_step: 10000,
        final_train_loss: 1.25,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: hash(13),
        checkpoint_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("checkpoint self hash")
}

fn run_log() -> RunLog {
    RunLog {
        schema: "s1_run_log.v1".to_owned(),
        seed: 0,
        train_config_hash: hash(4),
        losses: vec![(1, 1.5), (2, 1.25)],
        eval_points: vec![(0, 2.75), (1000, 2.25)],
        final_grad_norms: GradNormSummary {
            global_l2: 3.0,
            max_l2: 2.0,
            mean_l2: 1.0,
        },
        run_log_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("run log self hash")
}

fn score_report() -> ScoreReport {
    ScoreReport {
        schema: "s1_score.v1".to_owned(),
        seed: 0,
        checkpoint_sha: hash(12),
        corpus_val_sha: hash(2),
        chunk_size: 128,
        token_count: 2048,
        log2_sum: 4096.5,
        bpc: 2.000244140625,
        score_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("score self hash")
}

fn negative_test_report() -> NegativeTestReport {
    NegativeTestReport {
        schema: "s1_negative_test.v1".to_owned(),
        seed: 0,
        checkpoint_sha: hash(12),
        corpus_val_sha: hash(2),
        shuffle_seed: 0xDEADBEEF,
        bpc_original: 2.0,
        bpc_shuffled: 7.5,
        shuffled_val_sha256: hash(13),
        delta: 5.5,
        sensitive: true,
        negative_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("negative self hash")
}

fn ablation_report() -> AblationReport {
    AblationReport {
        schema: "s1_ablation.v1".to_owned(),
        seed: 0,
        phase_a_checkpoint_sha: hash(12),
        ablation_checkpoint_sha: hash(14),
        phase_a_tensor_payload_sha: hash(15),
        ablation_tensor_payload_sha: hash(16),
        phase_a_eq_ablation: false,
        first_mismatch: Some(TensorMismatch {
            tensor: "toy.blocks.0.weight".to_owned(),
            byte_offset: 17,
        }),
        ablation_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("ablation self hash")
}

fn baseline_report() -> BaselineReport {
    BaselineReport {
        schema: "s1_baseline.v1".to_owned(),
        corpus_train_sha: hash(1),
        corpus_val_sha: hash(2),
        smoothing: SmoothingScheme {
            alpha: 0.25,
            lambdas: [0.2, 0.3, 0.5],
        },
        bpc_3gram: 2.5,
        bpc_2gram: 3.0,
        bpc_unigram: 4.0,
        counts_summary: CountsSummary {
            train_bytes: 12,
            distinct_unigrams: 8,
            distinct_bigrams: 11,
            distinct_trigrams: 10,
        },
        counts_blob_sha256: hash(17),
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("baseline self hash")
}

fn report_front_matter() -> ReportFrontMatter {
    ReportFrontMatter {
        schema: "s1_report.v1".to_owned(),
        s1_outcome: S1Outcome::PassClean,
        decision: S1Decision::ProceedToS2,
        baseline_self_hash: baseline_report().baseline_self_hash,
        per_seed_artifacts: vec![PerSeedArtifacts {
            seed: 0,
            completion: S1Completion::Completed,
            checkpoint_self_hash: Some(checkpoint_metadata().checkpoint_self_hash),
            run_log_self_hash: Some(run_log().run_log_self_hash),
            score_self_hash: Some(score_report().score_self_hash),
            negative_self_hash: Some(negative_test_report().negative_self_hash),
            ablation_self_hash: Some(ablation_report().ablation_self_hash),
        }],
        generated_at: "2026-05-09T12:00:00Z".to_owned(),
        rfc_revision: RfcRevisionRef::GitCommitId(
            GitCommitId::new("0123456789abcdef0123456789abcdef01234567")
                .expect("valid RFC commit id"),
        ),
        predictions_section_hash: hash(18),
        predictions_commit: GitCommitId::new("1111111111111111111111111111111111111111")
            .expect("valid predictions commit id"),
        first_result_commit: GitCommitId::new("2222222222222222222222222222222222222222")
            .expect("valid first result commit id"),
        report_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("report self hash")
}

fn arb_hash() -> impl Strategy<Value = Hash256> {
    prop::array::uniform32(any::<u8>()).prop_map(Hash256::from_bytes)
}

fn arb_finite_f32() -> impl Strategy<Value = f32> {
    (0_u16..=10_000).prop_map(|value| f32::from(value) / 4.0)
}

fn arb_finite_f64() -> impl Strategy<Value = f64> {
    (0_u16..=10_000).prop_map(|value| f64::from(value) / 8.0)
}

fn arb_signed_finite_f64() -> impl Strategy<Value = f64> {
    (-10_000_i16..=10_000).prop_map(|value| f64::from(value) / 8.0)
}

fn arb_completion() -> impl Strategy<Value = S1Completion> {
    prop_oneof![
        Just(S1Completion::Completed),
        (0_u64..=10_000).prop_map(|step| S1Completion::DivergedAt { step }),
        Just(S1Completion::NotReached),
    ]
}

fn arb_checkpoint_metadata() -> impl Strategy<Value = CheckpointMetadata> {
    (
        (0_u64..=4, arb_hash(), arb_hash(), arb_hash(), arb_hash()),
        (
            any::<bool>(),
            arb_hash(),
            arb_hash(),
            arb_hash(),
            arb_hash(),
            arb_hash(),
        ),
        (
            0_u64..=2,
            0_u64..=20,
            0_u64..=20,
            0_u64..=20_000,
            arb_finite_f32(),
            arb_completion(),
        ),
    )
        .prop_map(
            |(
                (seed, corpus_train_sha, corpus_val_sha, model_config_hash, train_config_hash),
                (
                    ablation_build,
                    build_config_hash,
                    dependency_lockfile_sha,
                    rust_toolchain_hash,
                    device_profile_hash,
                    rng_stream_def_hash,
                ),
                (major, minor, patch, final_step, final_train_loss, completion),
            )| {
                CheckpointMetadata {
                    schema: "s1_checkpoint.v1".to_owned(),
                    seed,
                    corpus_train_sha,
                    corpus_val_sha,
                    model_config_hash,
                    train_config_hash,
                    build_kind: if ablation_build {
                        S1BuildKind::Ablation
                    } else {
                        S1BuildKind::PhaseA
                    },
                    build_config_hash,
                    dependency_lockfile_sha,
                    rust_toolchain_hash,
                    device_profile_hash,
                    rng_stream_def_hash,
                    pass_version: SemVer::new(major, minor, patch),
                    budget_profile: "production".to_owned(),
                    final_step,
                    final_train_loss,
                    completion,
                    checkpoint_safetensors_sha256: Hash256::ZERO,
                    checkpoint_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("checkpoint self hash")
            },
        )
}

fn arb_run_log() -> impl Strategy<Value = RunLog> {
    (
        0_u64..=4,
        arb_hash(),
        prop::collection::vec((0_u64..=20_000, arb_finite_f32()), 0..=8),
        prop::collection::vec((0_u64..=20_000, arb_finite_f64()), 0..=8),
        arb_finite_f32(),
        arb_finite_f32(),
        arb_finite_f32(),
    )
        .prop_map(
            |(seed, train_config_hash, losses, eval_points, global_l2, max_l2, mean_l2)| {
                RunLog {
                    schema: "s1_run_log.v1".to_owned(),
                    seed,
                    train_config_hash,
                    losses,
                    eval_points,
                    final_grad_norms: GradNormSummary {
                        global_l2,
                        max_l2,
                        mean_l2,
                    },
                    run_log_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("run log self hash")
            },
        )
}

fn arb_score_report() -> impl Strategy<Value = ScoreReport> {
    (
        0_u64..=4,
        arb_hash(),
        arb_hash(),
        1_u64..=256,
        1_u64..=65_536,
        arb_finite_f64(),
        arb_finite_f64(),
    )
        .prop_map(
            |(seed, checkpoint_sha, corpus_val_sha, chunk_size, token_count, log2_sum, bpc)| {
                ScoreReport {
                    schema: "s1_score.v1".to_owned(),
                    seed,
                    checkpoint_sha,
                    corpus_val_sha,
                    chunk_size,
                    token_count,
                    log2_sum,
                    bpc,
                    score_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("score self hash")
            },
        )
}

fn arb_negative_test_report() -> impl Strategy<Value = NegativeTestReport> {
    (
        0_u64..=4,
        arb_hash(),
        arb_hash(),
        any::<u64>(),
        arb_finite_f64(),
        arb_finite_f64(),
        arb_hash(),
        arb_signed_finite_f64(),
        any::<bool>(),
    )
        .prop_map(
            |(
                seed,
                checkpoint_sha,
                corpus_val_sha,
                shuffle_seed,
                bpc_original,
                bpc_shuffled,
                shuffled_val_sha256,
                delta,
                sensitive,
            )| {
                NegativeTestReport {
                    schema: "s1_negative_test.v1".to_owned(),
                    seed,
                    checkpoint_sha,
                    corpus_val_sha,
                    shuffle_seed,
                    bpc_original,
                    bpc_shuffled,
                    shuffled_val_sha256,
                    delta,
                    sensitive,
                    negative_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("negative self hash")
            },
        )
}

fn arb_ablation_report() -> impl Strategy<Value = AblationReport> {
    (
        0_u64..=4,
        arb_hash(),
        arb_hash(),
        arb_hash(),
        arb_hash(),
        any::<bool>(),
        prop::option::of(("[a-z][a-z0-9_.]{0,24}", 0_u64..=4096)),
    )
        .prop_map(
            |(
                seed,
                phase_a_checkpoint_sha,
                ablation_checkpoint_sha,
                phase_a_tensor_payload_sha,
                ablation_tensor_payload_sha,
                phase_a_eq_ablation,
                first_mismatch,
            )| {
                AblationReport {
                    schema: "s1_ablation.v1".to_owned(),
                    seed,
                    phase_a_checkpoint_sha,
                    ablation_checkpoint_sha,
                    phase_a_tensor_payload_sha,
                    ablation_tensor_payload_sha,
                    phase_a_eq_ablation,
                    first_mismatch: first_mismatch.map(|(tensor, byte_offset)| TensorMismatch {
                        tensor,
                        byte_offset,
                    }),
                    ablation_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("ablation self hash")
            },
        )
}

fn arb_baseline_report() -> impl Strategy<Value = BaselineReport> {
    (
        arb_hash(),
        arb_hash(),
        arb_finite_f64(),
        (arb_finite_f64(), arb_finite_f64(), arb_finite_f64()),
        arb_finite_f64(),
        arb_finite_f64(),
        arb_finite_f64(),
        (0_u64..=65_536, 0_u64..=4096, 0_u64..=4096, 0_u64..=4096),
        arb_hash(),
    )
        .prop_map(
            |(
                corpus_train_sha,
                corpus_val_sha,
                alpha,
                (lambda_0, lambda_1, lambda_2),
                bpc_3gram,
                bpc_2gram,
                bpc_unigram,
                (train_bytes, distinct_unigrams, distinct_bigrams, distinct_trigrams),
                counts_blob_sha256,
            )| {
                BaselineReport {
                    schema: "s1_baseline.v1".to_owned(),
                    corpus_train_sha,
                    corpus_val_sha,
                    smoothing: SmoothingScheme {
                        alpha,
                        lambdas: [lambda_0, lambda_1, lambda_2],
                    },
                    bpc_3gram,
                    bpc_2gram,
                    bpc_unigram,
                    counts_summary: CountsSummary {
                        train_bytes,
                        distinct_unigrams,
                        distinct_bigrams,
                        distinct_trigrams,
                    },
                    counts_blob_sha256,
                    baseline_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("baseline self hash")
            },
        )
}

fn arb_report_front_matter() -> impl Strategy<Value = ReportFrontMatter> {
    (
        prop_oneof![
            Just(S1Outcome::PassClean),
            Just(S1Outcome::PassWithWarning),
            Just(S1Outcome::FailSubstrate),
            Just(S1Outcome::FailCapacity),
            Just(S1Outcome::FailSuspicious),
            Just(S1Outcome::FailPhase),
            Just(S1Outcome::FailMetric),
        ],
        prop_oneof![
            Just(S1Decision::ProceedToS2),
            Just(S1Decision::ProceedToS2WithT125Prereq),
            "[a-z_]{1,16}".prop_map(|reason| S1Decision::Investigate { reason }),
            "[a-z_]{1,16}".prop_map(|reason| S1Decision::Halt { reason }),
        ],
        arb_hash(),
        prop::collection::vec(
            (
                0_u64..=4,
                arb_completion(),
                prop::option::of(arb_hash()),
                prop::option::of(arb_hash()),
                prop::option::of(arb_hash()),
                prop::option::of(arb_hash()),
                prop::option::of(arb_hash()),
            ),
            0..=5,
        ),
        any::<bool>(),
        arb_hash(),
    )
        .prop_map(
            |(
                s1_outcome,
                decision,
                baseline_self_hash,
                artifacts,
                rfc_as_hash,
                predictions_section_hash,
            )| {
                let per_seed_artifacts = artifacts
                    .into_iter()
                    .map(
                        |(
                            seed,
                            completion,
                            checkpoint_self_hash,
                            run_log_self_hash,
                            score_self_hash,
                            negative_self_hash,
                            ablation_self_hash,
                        )| PerSeedArtifacts {
                            seed,
                            completion,
                            checkpoint_self_hash,
                            run_log_self_hash,
                            score_self_hash,
                            negative_self_hash,
                            ablation_self_hash,
                        },
                    )
                    .collect();

                ReportFrontMatter {
                    schema: "s1_report.v1".to_owned(),
                    s1_outcome,
                    decision,
                    baseline_self_hash,
                    per_seed_artifacts,
                    generated_at: "2026-05-09T12:00:00Z".to_owned(),
                    rfc_revision: if rfc_as_hash {
                        RfcRevisionRef::Hash256(hash(42))
                    } else {
                        RfcRevisionRef::GitCommitId(
                            GitCommitId::new("0123456789abcdef0123456789abcdef01234567")
                                .expect("valid RFC commit id"),
                        )
                    },
                    predictions_section_hash,
                    predictions_commit: GitCommitId::new(
                        "1111111111111111111111111111111111111111",
                    )
                    .expect("valid predictions commit id"),
                    first_result_commit: GitCommitId::new(
                        "2222222222222222222222222222222222222222",
                    )
                    .expect("valid first result commit id"),
                    report_self_hash: Hash256::ZERO,
                }
                .with_computed_self_hash()
                .expect("report self hash")
            },
        )
}

fn assert_artifact_round_trip<T>(
    artifact: &T,
    compute_hash: fn(&T) -> Result<Hash256, S1SchemaError>,
) where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let first = S1CanonicalJson::to_vec(artifact).expect("canonical JSON with self hash");
    let decoded: T = serde_json::from_slice(&first).expect("canonical JSON decodes");
    let second = S1CanonicalJson::to_vec(&decoded).expect("canonical JSON re-encodes");

    assert_eq!(decoded, *artifact);
    assert_eq!(first, second);
    assert_eq!(
        compute_hash(&decoded).expect("computed self hash"),
        compute_hash(artifact).expect("original computed self hash")
    );
}

fn json_value<T: serde::Serialize>(artifact: &T) -> Value {
    serde_json::to_value(artifact).expect("artifact serializes to JSON value")
}

fn assert_schema_rejects<T>(value: Value, missing_field: &str, wrong_schema: &str)
where
    T: serde::de::DeserializeOwned + std::fmt::Debug,
{
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .expect("object")
        .remove(missing_field);
    let missing_error = serde_json::from_value::<T>(missing)
        .expect_err("missing required field must reject")
        .to_string();
    assert!(
        missing_error.contains(missing_field),
        "missing-field error should name {missing_field:?}, got {missing_error:?}"
    );

    let mut unknown = value.clone();
    unknown
        .as_object_mut()
        .expect("object")
        .insert("surprise".to_owned(), json!(true));
    assert!(serde_json::from_value::<T>(unknown).is_err());

    let mut wrong = value;
    wrong["schema"] = json!(wrong_schema);
    assert!(serde_json::from_value::<T>(wrong).is_err());
}

fn assert_rejects_non_finite<T>(result: Result<T, S1SchemaError>) {
    assert!(matches!(result, Err(S1SchemaError::NonFiniteFloat)));
}

fn assert_schema_hash_events(events: &[TracingEvent], schema_id: &str, self_hash: &str) {
    let start = events
        .iter()
        .find(|event| {
            event.name == "s1.schema.hash.start"
                && event.fields.get("schema_id") == Some(&json!(schema_id))
        })
        .unwrap_or_else(|| panic!("missing schema hash start for {schema_id} in {events:?}"));
    assert_eq!(start.level, "DEBUG");
    assert_eq!(start.fields.get("schema_version"), Some(&json!("1")));
    assert!(!start.fields.contains_key("self_hash"));

    let complete = events
        .iter()
        .find(|event| {
            event.name == "s1.schema.hash.complete"
                && event.fields.get("schema_id") == Some(&json!(schema_id))
        })
        .unwrap_or_else(|| panic!("missing schema hash complete for {schema_id} in {events:?}"));
    assert_eq!(complete.level, "DEBUG");
    assert_eq!(complete.fields.get("schema_version"), Some(&json!("1")));
    assert_eq!(complete.fields.get("self_hash"), Some(&json!(self_hash)));
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
