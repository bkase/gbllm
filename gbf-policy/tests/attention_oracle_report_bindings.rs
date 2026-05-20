use gbf_foundation::{BlobCodec, BlobRef, Hash256};
use gbf_policy::compile::{
    CanonicalProjectionTensor, CanonicalProjectionTensorSet, ProjectionTensorEncoding,
    ProjectionTensorName, ProjectionTensorSource, S5_ATTENTION_ORACLE_REPORT_PRODUCER_OWNER,
    S5_ATTENTION_ORACLE_REPORT_SCHEMA_SCOPE, S5AttentionOracleAggregateMismatch,
    S5AttentionOracleBindingHashes, S5AttentionOracleInputs, S5AttentionOracleQuantSpecBinding,
    S5AttentionOracleReport, S5AttentionOracleReportPayload, S5AttentionOracleResult,
    s5_attention_oracle_canonical_json_bytes, verify_s5_attention_oracle_report_aggregates,
    verify_s5_attention_oracle_report_bindings,
};

#[test]
fn report_round_trip_includes_four_binding_hashes() {
    let report = report_fixture();
    let value = serde_json::to_value(&report).expect("report serializes");

    for field in [
        "phase_a_checkpoint_sha",
        "projection_tensors_sha",
        "quant_spec_sha",
        "activation_clip_sha",
    ] {
        assert!(
            value.get(field).is_some(),
            "AttentionOracleReport must include {field}"
        );
    }

    let decoded: S5AttentionOracleReport =
        serde_json::from_value(value).expect("report deserializes");
    assert_eq!(decoded, report);
    assert_eq!(
        decoded.computed_oracle_self_hash().unwrap(),
        decoded.oracle_self_hash
    );
}

#[test]
fn mutating_binding_hashes_breaks_verifier_with_field_names() {
    let expected = expected_bindings();
    for (field, mutate) in [
        (
            "phase_a_checkpoint_sha",
            mutate_phase_a_checkpoint_sha as fn(&mut S5AttentionOracleReport),
        ),
        (
            "projection_tensors_sha",
            mutate_projection_tensors_sha as fn(&mut S5AttentionOracleReport),
        ),
        (
            "quant_spec_sha",
            mutate_quant_spec_sha as fn(&mut S5AttentionOracleReport),
        ),
        (
            "activation_clip_sha",
            mutate_activation_clip_sha as fn(&mut S5AttentionOracleReport),
        ),
    ] {
        let mut report = report_fixture();
        mutate(&mut report);
        let error = verify_s5_attention_oracle_report_bindings(&report, &expected)
            .expect_err("mutated binding must be rejected");
        assert_eq!(error.field, field);
    }
}

#[test]
fn oracle_self_hash_includes_all_four_binding_hashes() {
    let baseline = report_fixture().oracle_self_hash;
    for mutate in [
        mutate_phase_a_checkpoint_sha as fn(&mut S5AttentionOracleReport),
        mutate_projection_tensors_sha as fn(&mut S5AttentionOracleReport),
        mutate_quant_spec_sha as fn(&mut S5AttentionOracleReport),
        mutate_activation_clip_sha as fn(&mut S5AttentionOracleReport),
    ] {
        let mut report = report_fixture();
        mutate(&mut report);
        assert_ne!(
            report.computed_oracle_self_hash().unwrap(),
            baseline,
            "oracle_self_hash must change when a binding hash changes"
        );
    }
}

#[test]
fn binding_hashes_are_deterministic_from_canonical_inputs() {
    let first = S5AttentionOracleBindingHashes::from_inputs(&inputs_fixture()).unwrap();
    let second = S5AttentionOracleBindingHashes::from_inputs(&inputs_fixture()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first, expected_bindings());
    assert_eq!(
        s5_attention_oracle_canonical_json_bytes(&inputs_fixture()).unwrap(),
        s5_attention_oracle_canonical_json_bytes(&inputs_fixture()).unwrap()
    );
}

#[test]
fn binding_hashes_reject_non_finite_activation_clip_before_hashing() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut inputs = inputs_fixture();
        inputs.activation_fake_quant_clip = value;

        let error = S5AttentionOracleBindingHashes::from_inputs(&inputs)
            .expect_err("non-finite activation clip must be rejected");

        assert!(
            error
                .to_string()
                .contains("activation_fake_quant_clip must be finite")
        );
    }
}

#[test]
fn activation_clip_hash_is_always_present_for_reported_paths() {
    for source in [
        ProjectionTensorSource::PhaseAFpProjectionTensors,
        ProjectionTensorSource::QuantSpecWeightQuant,
    ] {
        let report = report_for_inputs(inputs_fixture_with_source(source));
        let value = serde_json::to_value(&report).expect("report serializes");

        assert!(
            value.get("activation_clip_sha").is_some(),
            "activation_clip_sha must be present for {source:?}"
        );
    }
}

#[test]
fn report_schema_scope_is_policy_only_until_producer_adoption() {
    assert!(S5_ATTENTION_ORACLE_REPORT_SCHEMA_SCOPE.contains("gbf-policy policy-only"));
    assert!(
        S5_ATTENTION_ORACLE_REPORT_SCHEMA_SCOPE.contains("gbf-artifact exposure is not claimed")
    );
    assert!(S5_ATTENTION_ORACLE_REPORT_PRODUCER_OWNER.contains("gbf-experiments::s5"));
    assert!(S5_ATTENTION_ORACLE_REPORT_PRODUCER_OWNER.contains("OracleReportEmitted"));
    assert!(S5_ATTENTION_ORACLE_REPORT_PRODUCER_OWNER.contains("seed-0 e2e"));
    assert!(S5_ATTENTION_ORACLE_REPORT_PRODUCER_OWNER.contains("on-disk mutation protocol"));
}

#[test]
fn aggregate_invariants_accept_consistent_report() {
    verify_s5_attention_oracle_report_aggregates(&report_fixture())
        .expect("fixture report aggregates match per-fixture results");
}

#[test]
fn aggregate_agreement_must_match_per_fixture_results() {
    let mut report = report_fixture();
    report.per_fixture_results[0].agreement = false;

    let error = verify_s5_attention_oracle_report_aggregates(&report)
        .expect_err("aggregate agreement must mirror fixture agreement");

    assert_aggregate_error(
        error,
        "aggregate_agreement",
        "does not match per_fixture_results agreement",
    );
}

#[test]
fn aggregate_max_must_not_be_below_per_fixture_max_abs_diff() {
    let mut report = report_fixture();
    report.per_fixture_results[0].max_abs_diff = 0.20;
    report.aggregate_max_abs_diff = 0.19;
    report.aggregate_p99_max_abs_diff = 0.19;

    let error = verify_s5_attention_oracle_report_aggregates(&report)
        .expect_err("aggregate max must cover fixture max");

    assert_aggregate_error(
        error,
        "aggregate_max_abs_diff",
        "less than per_fixture_results max_abs_diff",
    );
}

#[test]
fn aggregate_p99_must_not_exceed_aggregate_max() {
    let mut report = report_fixture();
    report.aggregate_max_abs_diff = 0.20;
    report.aggregate_p99_max_abs_diff = 0.21;

    let error = verify_s5_attention_oracle_report_aggregates(&report)
        .expect_err("aggregate p99 cannot exceed aggregate max");

    assert_aggregate_error(
        error,
        "aggregate_p99_max_abs_diff",
        "greater than aggregate_max_abs_diff",
    );
}

#[test]
fn aggregate_invariants_reject_non_finite_and_negative_fixture_diffs() {
    let mut report = report_fixture();
    report.per_fixture_results[0].max_abs_diff = f32::NAN;
    let error = verify_s5_attention_oracle_report_aggregates(&report)
        .expect_err("non-finite fixture diffs are rejected");
    assert_aggregate_error(error, "per_fixture_results.max_abs_diff", "must be finite");

    let mut report = report_fixture();
    report.per_fixture_results[0].max_abs_diff = -0.01;
    let error = verify_s5_attention_oracle_report_aggregates(&report)
        .expect_err("negative fixture diffs are rejected");
    assert_aggregate_error(
        error,
        "per_fixture_results.max_abs_diff",
        "must be non-negative",
    );
}

fn report_fixture() -> S5AttentionOracleReport {
    report_for_inputs(inputs_fixture())
}

fn report_for_inputs(inputs: S5AttentionOracleInputs) -> S5AttentionOracleReport {
    S5AttentionOracleReport::new(
        0,
        S5AttentionOracleBindingHashes::from_inputs(&inputs).unwrap(),
        S5AttentionOracleReportPayload {
            fixture_suite_sha: hash(0x51),
            spec_sha: hash(0x52),
            per_fixture_results: vec![S5AttentionOracleResult {
                fixture_id: "AOF-1".to_owned(),
                position: 0,
                oracle_logits_sha256: hash(0x61),
                boundedkv_logits_sha256: hash(0x62),
                max_abs_diff: 0.00001,
                agreement: true,
            }],
            aggregate_max_abs_diff: 0.00001,
            aggregate_p99_max_abs_diff: 0.00001,
            aggregate_agreement: true,
        },
    )
    .with_computed_oracle_self_hash()
    .unwrap()
}

fn expected_bindings() -> S5AttentionOracleBindingHashes {
    S5AttentionOracleBindingHashes::from_inputs(&inputs_fixture()).unwrap()
}

fn inputs_fixture() -> S5AttentionOracleInputs {
    inputs_fixture_with_source(ProjectionTensorSource::PhaseAFpProjectionTensors)
}

fn inputs_fixture_with_source(source: ProjectionTensorSource) -> S5AttentionOracleInputs {
    S5AttentionOracleInputs::new(
        blob(0x11),
        CanonicalProjectionTensorSet::from_checkpoint_artifact(
            source,
            vec![
                tensor(ProjectionTensorName::Query, 0x21),
                tensor(ProjectionTensorName::Key, 0x22),
                tensor(ProjectionTensorName::Value, 0x23),
                tensor(ProjectionTensorName::Output, 0x24),
            ],
        ),
        S5AttentionOracleQuantSpecBinding {
            source,
            canonical_bytes_sha256: hash(0x31),
        },
        6.5,
        vec![1, 2, 3],
    )
}

fn mutate_phase_a_checkpoint_sha(report: &mut S5AttentionOracleReport) {
    report.phase_a_checkpoint_sha = hash(0xa1);
}

fn mutate_projection_tensors_sha(report: &mut S5AttentionOracleReport) {
    report.projection_tensors_sha = hash(0xa2);
}

fn mutate_quant_spec_sha(report: &mut S5AttentionOracleReport) {
    report.quant_spec_sha = hash(0xa3);
}

fn mutate_activation_clip_sha(report: &mut S5AttentionOracleReport) {
    report.activation_clip_sha = hash(0xa4);
}

fn tensor(name: ProjectionTensorName, hash_byte: u8) -> CanonicalProjectionTensor {
    CanonicalProjectionTensor {
        name,
        rows: 8,
        cols: 8,
        encoding: ProjectionTensorEncoding::F64LeMatrix,
        canonical_bytes_sha256: hash(hash_byte),
    }
}

fn blob(byte: u8) -> BlobRef {
    BlobRef {
        hash: hash(byte),
        len: 4096,
        codec: BlobCodec::Raw,
    }
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn assert_aggregate_error(
    error: S5AttentionOracleAggregateMismatch,
    field: &'static str,
    reason: &'static str,
) {
    assert_eq!(error.field, field);
    assert_eq!(error.reason, reason);
}
