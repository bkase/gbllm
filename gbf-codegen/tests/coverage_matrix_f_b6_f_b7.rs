mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gbf_codegen::s4::observation_plan::{ObservationPlanInputs, SemanticCheckpointKind};
use gbf_codegen::s5::range_plan::RangePlanInputs;
use gbf_foundation::LayerId;
use serde_json::Value;
use support::f_b6_f_b7::{
    F_B6_F_B7_COMMON_EVENT_FIELDS, Fb6Fb7NdjsonSink, GbInferIRFixture,
    ObservationPlanInputsFixture, RANGE_CERT_TRACE_TARGET, RANGE_CERT_VERIFY_EVENT_NAMES,
    RangePlanInputsFixture, STAGE4_EVENT_NAMES, STAGE4_TRACE_TARGET, STAGE5_EVENT_NAMES,
    STAGE5_TRACE_TARGET, StaticBudgetReductionSiteFactsFixture, abi_probe_id,
    build_stage4_inputs_for, build_stage5_inputs_for, canonical_json_bytes, ck_id,
    closed_event_names, is_closed_event_name, policy_probe_id, timestamp_string, to_abi,
};
use tracing_subscriber::prelude::*;

const STAGE4_DIAGNOSTIC_CODES: &[&str] = &[
    "OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE",
    "OBSERVATION-CHECKPOINT-NOT-ATTACHABLE",
    "OBSERVATION-CHECKPOINT-AMBIGUOUS",
    "OBSERVATION-PROBE-ID-UNKNOWN",
    "OBSERVATION-REQUIRED-PROBE-DISABLED",
    "OBSERVATION-METRIC-SOURCE-RESERVED-V1",
    "OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO",
    "OBSERVATION-PROBE-SOURCE-INVALID",
    "OBSERVATION-RESERVED-EFFECT-PROBE",
    "OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED",
    "OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED",
    "OBSERVATION-PROBE-CLASS-CAP-EXCEEDED",
    "OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED",
    "OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT",
    "OBSERVATION-DETERMINISM-MISMATCH",
    "OBSERVATION-SC-HASH-MISMATCH",
];

const STAGE4_DIAGNOSTIC_ORIGIN: &str = "ObservationPlanConstruction";

const STAGE4_PRODUCER_EVIDENCE: &[(&str, &str)] = &[
    (
        "OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE",
        "semantic_selection_mandatory_not_feasible_fails",
    ),
    (
        "OBSERVATION-CHECKPOINT-NOT-ATTACHABLE",
        "semantic_anchor_binding_missing_anchor_fails",
    ),
    (
        "OBSERVATION-CHECKPOINT-AMBIGUOUS",
        "bind_semantic_observations_v1",
    ),
    (
        "OBSERVATION-PROBE-ID-UNKNOWN",
        "disabled_unknown_probe_rejected",
    ),
    (
        "OBSERVATION-REQUIRED-PROBE-DISABLED",
        "disabled_required_probe_rejected",
    ),
    (
        "OBSERVATION-METRIC-SOURCE-RESERVED-V1",
        "metric_registry_filter_per_slice_reserved_rejected",
    ),
    (
        "OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO",
        "metric_aggregation_histogram_bucket_count_zero_rejected",
    ),
    (
        "OBSERVATION-PROBE-SOURCE-INVALID",
        "probe_instance_id_collision_rejected_at_canonical_sort",
    ),
    (
        "OBSERVATION-RESERVED-EFFECT-PROBE",
        "effect_class_diagnostic_precedence",
    ),
    (
        "OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED",
        "sequence_state_and_fault_boundary_effect_probe_rejections",
    ),
    (
        "OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED",
        "sequence_state_and_fault_boundary_effect_probe_rejections",
    ),
    (
        "OBSERVATION-PROBE-CLASS-CAP-EXCEEDED",
        "probe_class_cap_exceeded_for_non_required_classes",
    ),
    (
        "OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED",
        "invariant_budget_check_under_invariant_fails_when_over",
    ),
    (
        "OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT",
        "encoding_for_invalid_override_fails_without_panicking",
    ),
    (
        "OBSERVATION-DETERMINISM-MISMATCH",
        "op_pre_3a_determinism_class_mismatch_rejected",
    ),
    (
        "OBSERVATION-SC-HASH-MISMATCH",
        "op_pre_2_artifact_declared_hash_mismatch_rejected",
    ),
];

const STAGE5_DIAGNOSTIC_CODES: &[&str] = &[
    "RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1",
    "RANGE-TERM-COUNT-ZERO",
    "RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY",
    "RANGE-CEILING-VIOLATED-NO-RENORM-LOOP",
    "RANGE-NO-PROVEN-PLAN-WITHIN-CEILING",
    "RANGE-SITE-MISSING-FROM-STATIC-BUDGET",
    "RANGE-STATIC-BUDGET-SITE-ORPHANED",
    "RANGE-DUPLICATE-REDUCTION-SITE-ID",
    "RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES",
    "RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1",
    "RANGE-DETERMINISM-MISMATCH",
    "RANGE-CEILING-OVERRIDE-INVALID-SELECTOR",
    "RANGE-CEILING-OVERRIDE-AMBIGUOUS",
    "RANGE-SITE-FACTS-INCONSISTENT",
    "RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX",
    "RANGE-TILE-LEN-BELOW-PROFILE-MIN",
    "RANGE-TILE-LEN-EXCEEDS-PROFILE-MAX",
];

const STAGE5_DIAGNOSTIC_ORIGIN: &str = "RangePlanConstruction";

const EXCLUDED_DIAGNOSTIC_CODES: &[(&str, &str)] = &[
    (
        "OBSERVATION-OPTIONAL-CHECKPOINT-NOT-FEASIBLE",
        "reserved by the RFC for future optional checkpoint policy",
    ),
    (
        "OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE",
        "covered as a Stage 4 input feasibility guard by a later driver bead",
    ),
    (
        "OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA",
        "covered as a Stage 4 schema-input guard by a later driver bead",
    ),
    (
        "OBSERVATION-METRIC-ID-UNKNOWN",
        "reserved in v1 because metrics are registry-owned and Stage 4 emits metric-source reserved diagnostics instead",
    ),
    (
        "OBSERVATION-LOCKED-KNOB-DRIFT",
        "input/projection validation guard; no Stage 4 construction fixture",
    ),
    (
        "OBSERVATION-COMPARE-DOMAIN-MISMATCH",
        "landed workload-vs-policy input validation before Stage 4 construction",
    ),
    (
        "OBSERVATION-WORKLOAD-DETERMINISM-MISMATCH",
        "landed workload-vs-manifest input validation before Stage 4 construction",
    ),
    (
        "OBSERVATION-POLICY-WORKLOAD-DETERMINISM-MISMATCH",
        "landed policy-vs-workload input validation before Stage 4 construction",
    ),
    (
        "RANGE-LOCKED-KNOB-DRIFT",
        "input/projection validation guard; no Stage 5 construction fixture",
    ),
    (
        "RANGE-CERT-MALFORMED",
        "owned by gbf-verify/tampered certificate validation rather than Stage 5 construction",
    ),
    (
        "RANGE-CHUNK-LEN-ZERO",
        "v1 Stage 5 candidate generation never constructs zero chunk_len; malformed external plan validation is a later schema/verifier owner",
    ),
    (
        "RANGE-TILE-LEN-ZERO",
        "v1 Stage 5 candidate generation never constructs zero tile_len; malformed external plan validation is a later schema/verifier owner",
    ),
    (
        "RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN",
        "enforced as BitExact RenormLoop reservation in Stage 5; no separate mid-reduction saturation producer exists in v1",
    ),
    (
        "RANGE-RENORM-STRATEGY-UNSUPPORTED-V1",
        "range profile parsing rejects unsupported renorm strategy before Stage 5 construction",
    ),
    (
        "RANGE-CAPS-INVALID",
        "compile profile validation rejects invalid range caps before Stage 5 construction",
    ),
    (
        "RANGE-INTEGER-OVERFLOW-DURING-PROOF",
        "v1 public maxima are u32/u64-bounded and construction uses Failed proof evidence for arithmetic overflow rather than a Stage 5 diagnostic",
    ),
    (
        "RANGE-TILE-LEN-EXCEEDS-U16",
        "v1 non-BitExact RenormLoop supports term_count above u16 via tile_count; malformed proof-state validation is verifier-owned",
    ),
];

const STAGE5_PRODUCER_EVIDENCE: &[(&str, &str)] = &[
    (
        "RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1",
        "accumulator_domain_unsupported_v1_rejected",
    ),
    ("RANGE-TERM-COUNT-ZERO", "term_count_zero_rejected"),
    (
        "RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY",
        "plan_choice_ceiling_violation_precedence_single_i16_only",
    ),
    (
        "RANGE-CEILING-VIOLATED-NO-RENORM-LOOP",
        "plan_choice_ceiling_violation_precedence_no_renorm_loop",
    ),
    (
        "RANGE-NO-PROVEN-PLAN-WITHIN-CEILING",
        "plan_choice_no_proven_plan_within_ceiling",
    ),
    (
        "RANGE-SITE-MISSING-FROM-STATIC-BUDGET",
        "site_missing_from_static_budget_rejected",
    ),
    (
        "RANGE-STATIC-BUDGET-SITE-ORPHANED",
        "static_budget_site_orphan_rejected",
    ),
    (
        "RANGE-DUPLICATE-REDUCTION-SITE-ID",
        "rp_pre_7_duplicate_reduction_site_id_in_g_rejected",
    ),
    (
        "RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES",
        "choose_chunk_len_bitexact_no_divisor_rejected",
    ),
    (
        "RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1",
        "choose_tile_len_bitexact_renorm_loop_reserved_v1",
    ),
    (
        "RANGE-DETERMINISM-MISMATCH",
        "rp_pre_4_determinism_class_mismatch",
    ),
    (
        "RANGE-CEILING-OVERRIDE-INVALID-SELECTOR",
        "rp_pre_6_override_selector_invalid",
    ),
    (
        "RANGE-CEILING-OVERRIDE-AMBIGUOUS",
        "effective_ceiling_ambiguous_overrides_rejected",
    ),
    (
        "RANGE-SITE-FACTS-INCONSISTENT",
        "site_facts_inconsistent_optional_maxima_rejected",
    ),
    (
        "RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX",
        "choose_chunk_len_above_profile_chunk_max_rejected",
    ),
    (
        "RANGE-TILE-LEN-BELOW-PROFILE-MIN",
        "choose_tile_len_below_profile_tile_min_rejected",
    ),
    (
        "RANGE-TILE-LEN-EXCEEDS-PROFILE-MAX",
        "choose_tile_len_min_above_profile_tile_max_rejected",
    ),
];

const EXPECTED_COMMON_EVENT_FIELDS: &[&str] = &[
    "site_id",
    "checkpoint_id",
    "compact_checkpoint_id",
    "stratum",
    "probe_instance_id",
    "runtime_probe_id",
    "importance_class",
    "build_id",
    "k4_hash",
    "k5_hash",
    "outcome",
    "diag_code",
    "elapsed_ns",
    "event_seq",
];

const STAGE4_CONDITIONAL_EVENTS: &[&str] = &[
    "stage4.driver.cache_hit",
    "stage4.driver.failure_memo",
    "stage4.driver.audit_parent_rewrap",
];

const STAGE5_CONDITIONAL_EVENTS: &[&str] = &[
    "stage5.driver.cache_hit",
    "stage5.driver.failure_memo",
    "stage5.driver.audit_parent_rewrap",
    "range_cert.verifies.single_i16",
    "range_cert.verifies.renorm_loop",
    "range_cert.verifies.failed",
    "range_cert.renorm_recurrence_verifies",
];

#[test]
fn coverage_matrix_fixture_corpus_covers_non_reserved_rfc_codes() {
    for code in STAGE4_DIAGNOSTIC_CODES {
        assert_reject_fixture("stage4", code);
    }
    for code in STAGE5_DIAGNOSTIC_CODES {
        assert_reject_fixture("stage5", code);
    }
}

#[test]
fn coverage_matrix_excluded_codes_are_documented_but_excluded() {
    let root = fixture_root().join("reject");
    let reserved_doc =
        fs::read_to_string(root.join("RESERVED.md")).expect("reserved diagnostics doc reads");
    for (code, reason) in EXCLUDED_DIAGNOSTIC_CODES {
        assert!(
            !reason.is_empty(),
            "excluded diagnostic {code} needs a reason"
        );
        assert!(
            reserved_doc.contains(code),
            "excluded diagnostic {code} is missing from RESERVED.md"
        );
        assert!(
            reserved_doc.contains(reason),
            "excluded diagnostic {code} reason is missing from RESERVED.md"
        );
        assert!(
            !root.join("stage4").join(code).exists() && !root.join("stage5").join(code).exists(),
            "excluded diagnostic {code} must not have a reject fixture"
        );
    }

    for stage in ["stage4", "stage5"] {
        for entry in fs::read_dir(root.join(stage)).expect("reject stage dir exists") {
            let entry = entry.expect("reject fixture dir is readable");
            let expected = entry.path().join("expected_diag.json");
            if expected.exists() {
                let value: Value = serde_json::from_slice(
                    &fs::read(&expected).expect("expected diagnostic is readable"),
                )
                .expect("expected diagnostic is valid JSON");
                let observed = value["code"]
                    .as_str()
                    .expect("expected diag code is string");
                assert!(
                    !EXCLUDED_DIAGNOSTIC_CODES
                        .iter()
                        .any(|(code, _reason)| *code == observed),
                    "excluded diagnostic {observed} appeared in {expected:?}"
                );
                assert_eq!(
                    value["reserved"], false,
                    "active diagnostic {observed} must not be marked reserved in {expected:?}"
                );
                let expected_origin = match stage {
                    "stage4" => STAGE4_DIAGNOSTIC_ORIGIN,
                    "stage5" => STAGE5_DIAGNOSTIC_ORIGIN,
                    other => panic!("unexpected stage {other}"),
                };
                assert_eq!(
                    value["origin"], expected_origin,
                    "{expected:?} must pin the RFC construction origin"
                );
                if stage == "stage5" {
                    assert_eq!(
                        value["wire_code_kind"], "ReportSemanticInvariantViolated",
                        "{expected:?} must pin the actual Stage 5 wire diagnostic shape"
                    );
                    assert_eq!(
                        value["rfc_code_location"], "evidence.reference",
                        "{expected:?} must say where the RANGE code lives on the wire"
                    );
                    assert!(
                        value["producer_evidence"].as_str().is_some_and(|text| {
                            STAGE5_PRODUCER_EVIDENCE.iter().any(|(code, evidence)| {
                                *code == observed && text.contains(evidence)
                            })
                        }),
                        "{expected:?} must cite a producer test for {observed}"
                    );
                } else {
                    assert!(
                        value["producer_evidence"].as_str().is_some_and(|text| {
                            STAGE4_PRODUCER_EVIDENCE.iter().any(|(code, evidence)| {
                                *code == observed && text.contains(evidence)
                            })
                        }),
                        "{expected:?} must cite a producer test or path for {observed}"
                    );
                }
            }
        }
    }
}

#[test]
fn stage4_active_diagnostics_have_producer_evidence() {
    for code in STAGE4_DIAGNOSTIC_CODES {
        let evidence = STAGE4_PRODUCER_EVIDENCE
            .iter()
            .find_map(|(mapped, evidence)| (*mapped == *code).then_some(*evidence))
            .unwrap_or_else(|| panic!("missing Stage 4 producer evidence for {code}"));
        assert!(
            fs::read_to_string(repo_root().join("gbf-codegen/src/s4/observation_plan.rs"))
                .expect("observation_plan.rs reads")
                .contains(evidence),
            "Stage 4 producer evidence {evidence} for {code} is not present"
        );
    }
}

#[test]
fn stage5_active_diagnostics_have_producer_evidence() {
    for code in STAGE5_DIAGNOSTIC_CODES {
        let evidence = STAGE5_PRODUCER_EVIDENCE
            .iter()
            .find_map(|(mapped, evidence)| (*mapped == *code).then_some(*evidence))
            .unwrap_or_else(|| panic!("missing Stage 5 producer evidence for {code}"));
        assert!(
            fs::read_to_string(repo_root().join("gbf-codegen/src/s5/range_plan.rs"))
                .expect("range_plan.rs reads")
                .contains(evidence),
            "Stage 5 producer evidence {evidence} for {code} is not present"
        );
    }
}

#[test]
fn fixture_builders_emit_byte_identical_canonical_json() {
    let first = ObservationPlanInputsFixture::dense_default();
    let second = ObservationPlanInputsFixture::dense_default();
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());

    let first = RangePlanInputsFixture::chunked_i16();
    let second = RangePlanInputsFixture::chunked_i16();
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());

    let temp = tempfile::tempdir().expect("temp dir");
    let op_a = temp.path().join("op_a");
    let op_b = temp.path().join("op_b");
    let path_a = ObservationPlanInputsFixture::dense_default().write_to(&op_a);
    let path_b = ObservationPlanInputsFixture::dense_default().write_to(&op_b);
    assert_eq!(
        fs::read(path_a).expect("first emitted op"),
        fs::read(path_b).expect("second emitted op")
    );

    let rp_a = temp.path().join("rp_a");
    let rp_b = temp.path().join("rp_b");
    let path_a = RangePlanInputsFixture::chunked_i16().write_to(&rp_a);
    let path_b = RangePlanInputsFixture::chunked_i16().write_to(&rp_b);
    assert_eq!(
        fs::read(path_a).expect("first emitted rp"),
        fs::read(path_b).expect("second emitted rp")
    );
}

#[test]
fn fixture_builders_compile_against_landed_bridge_types() {
    let semantic = ck_id(SemanticCheckpointKind::PostEmbedding {
        layer: LayerId::new(0),
    });
    assert_eq!(semantic.as_str(), "layer.0.post_embedding");

    let policy = policy_probe_id(4095);
    let abi = abi_probe_id(4095);
    assert_eq!(to_abi(policy), abi);

    let ir = GbInferIRFixture::dense_default();
    let stage4 = build_stage4_inputs_for(&ir);
    let static_budget = StaticBudgetReductionSiteFactsFixture::single_i16().build_report();
    let stage5 = build_stage5_inputs_for(&ir, &static_budget);

    assert_eq!(
        stage4.quant_graph_self_hash, stage5.quant_graph_self_hash,
        "stage4 and stage5 fixtures share the tiny IR source of truth"
    );
    assert!(!canonical_json_bytes(&stage4).is_empty());
    assert!(!canonical_json_bytes(&stage5).is_empty());
}

#[test]
fn telemetry_closed_event_vocabulary_and_common_fields_are_pinned() {
    assert_eq!(F_B6_F_B7_COMMON_EVENT_FIELDS, EXPECTED_COMMON_EVENT_FIELDS);
    let common_set: BTreeSet<_> = F_B6_F_B7_COMMON_EVENT_FIELDS.iter().copied().collect();
    assert_eq!(
        common_set.len(),
        F_B6_F_B7_COMMON_EVENT_FIELDS.len(),
        "common fields have no duplicates"
    );

    assert!(
        STAGE4_EVENT_NAMES
            .iter()
            .all(|name| name.starts_with("stage4."))
    );
    assert!(
        STAGE5_EVENT_NAMES
            .iter()
            .all(|name| name.starts_with("stage5.") || name.starts_with("range_cert."))
    );
    for name in closed_event_names() {
        assert!(
            is_closed_event_name(name),
            "closed event {name} is recognized"
        );
    }
    let all_events: Vec<_> = closed_event_names().collect();
    let all_events_set: BTreeSet<_> = all_events.iter().copied().collect();
    assert_eq!(
        all_events.len(),
        all_events_set.len(),
        "closed event names have no duplicates"
    );
    assert!(
        RANGE_CERT_VERIFY_EVENT_NAMES
            .iter()
            .all(|name| name.starts_with("range_cert.independent_verify."))
    );
    for target in [
        STAGE4_TRACE_TARGET,
        STAGE5_TRACE_TARGET,
        RANGE_CERT_TRACE_TARGET,
    ] {
        assert!(
            !target.is_empty(),
            "documented telemetry target is non-empty"
        );
    }
}

#[test]
fn ndjson_sink_captures_subscriber_event_shape() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("events.ndjson");
    let sink = Fb6Fb7NdjsonSink::new(&path).expect("sink opens");
    let subscriber = tracing_subscriber::registry().with(sink);

    tracing::subscriber::with_default(subscriber, || {
        f_b6_f_b7_stage4_trace_event!(
            STAGE4_EVENT_NAMES[0],
            site_id = "dense.matmul.0",
            checkpoint_id = "post_embedding.l0",
            compact_checkpoint_id = 1_u64,
            stratum = "denotation",
            probe_instance_id = "probe.7",
            runtime_probe_id = 7_u64,
            importance_class = "Required",
            build_id = "00000000-0000-0000-0000-000000000000",
            k4_hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444",
            k5_hash = "not-applicable:stage4",
            outcome = "passed",
            diag_code = "none",
            elapsed_ns = 12_u64,
            event_seq = 1_u64,
        );
    });

    let line = fs::read_to_string(path)
        .expect("ndjson file is readable")
        .lines()
        .next()
        .expect("one ndjson line exists")
        .to_owned();
    let value: Value = serde_json::from_str(&line).expect("ndjson line is JSON");
    assert_eq!(value["event"], STAGE4_EVENT_NAMES[0]);
    assert_eq!(value["level"], "INFO");
    assert_eq!(value["target"], STAGE4_TRACE_TARGET);
    assert_eq!(value["fields"]["site_id"], "dense.matmul.0");
    assert_eq!(value["fields"]["compact_checkpoint_id"], 1);
    for field in F_B6_F_B7_COMMON_EVENT_FIELDS {
        assert!(
            value["fields"].get(*field).is_some(),
            "ndjson line pins common field {field}"
        );
    }
    assert_timestamp_shape(value["ts"].as_str().expect("timestamp is string"));
    assert_timestamp_shape(&timestamp_string());
}

#[test]
fn stage5_script_emits_one_mutually_exclusive_cert_event_per_fixture() {
    for fixture in [
        "single_i16",
        "chunked_i16",
        "renorm_loop_non_bitexact",
        "ceiling_override_layer_site",
    ] {
        let temp = tempfile::tempdir().expect("temp packet dir");
        let output = Command::new(repo_root().join("scripts/review/f-b6-f-b7/stage5-run.sh"))
            .arg(fixture)
            .current_dir(repo_root())
            .env("F_B6_F_B7_BUILD_ID", format!("cargo-test-{fixture}"))
            .env("F_B6_F_B7_OUT_DIR", temp.path())
            .output()
            .expect("stage5-run.sh executes");
        assert!(
            output.status.success(),
            "stage5-run.sh failed for {fixture}: {}",
            output_text(&output)
        );

        let payloads = read_ndjson_values(&temp.path().join("stage5-run.ndjson"));
        let cert_events: Vec<_> = payloads
            .iter()
            .filter_map(|payload| payload["event"].as_str())
            .filter(|event| event.starts_with("range_cert.verifies."))
            .collect();
        assert_eq!(
            cert_events.len(),
            1,
            "{fixture} emitted exactly one certificate verification event"
        );
        for payload in payloads {
            let event = payload["event"].as_str().expect("event string");
            assert_eq!(
                payload["target"],
                target_for_event(event),
                "{fixture} event {event} uses documented telemetry target"
            );
        }
    }
}

#[test]
fn verify_packet_check_existing_rejects_bad_telemetry_contracts() {
    let missing_field = temp_packet_dir("missing-field");
    write_valid_packet(missing_field.path());
    let stage4_path = missing_field.path().join("stage4-run.ndjson");
    let mut payloads = read_ndjson_values(&stage4_path);
    payloads[0]["fields"]
        .as_object_mut()
        .expect("fields object")
        .remove("site_id");
    write_ndjson_values(&stage4_path, &payloads);
    let output = run_verify_check_existing(missing_field.path(), None);
    assert!(
        !output.status.success(),
        "verify-packet accepted a missing common field"
    );
    assert_output_contains(&output, "missing common field site_id");

    let unexpected_event = temp_packet_dir("unexpected-event");
    write_valid_packet(unexpected_event.path());
    let stage4_path = unexpected_event.path().join("stage4-run.ndjson");
    let mut payloads = read_ndjson_values(&stage4_path);
    payloads.push(telemetry_payload("stage4.unexpected", payloads.len() + 1));
    write_ndjson_values(&stage4_path, &payloads);
    let output = run_verify_check_existing(unexpected_event.path(), None);
    assert!(
        !output.status.success(),
        "verify-packet accepted an unexpected event"
    );
    assert_output_contains(&output, "unexpected event stage4.unexpected");

    let bad_type = temp_packet_dir("bad-common-field-type");
    write_valid_packet(bad_type.path());
    let stage5_path = bad_type.path().join("stage5-run.ndjson");
    let mut payloads = read_ndjson_values(&stage5_path);
    payloads[0]["fields"]["compact_checkpoint_id"] =
        Value::String("not-applicable:stage5".to_owned());
    write_ndjson_values(&stage5_path, &payloads);
    let output = run_verify_check_existing(bad_type.path(), None);
    assert!(
        !output.status.success(),
        "verify-packet accepted a malformed common field type"
    );
    assert_output_contains(
        &output,
        "common field compact_checkpoint_id must be an integer",
    );
}

#[test]
fn verify_packet_check_existing_rejects_malformed_promoted_fixture_payload() {
    let packet = temp_packet_dir("bad-promoted-input");
    write_valid_packet(packet.path());

    let fixtures = tempfile::tempdir().expect("fixture temp dir");
    let stage4 = fixtures
        .path()
        .join("reject")
        .join("stage4")
        .join("OBSERVATION-SC-HASH-MISMATCH");
    fs::create_dir_all(&stage4).expect("stage4 fixture dir");
    fs::write(stage4.join("inputs.json"), br#"{"not":"a landed payload"}"#)
        .expect("malformed promoted payload");
    fs::create_dir_all(fixtures.path().join("reject").join("stage5")).expect("stage5 fixture dir");

    let output = run_verify_check_existing(packet.path(), Some(fixtures.path()));
    assert!(
        !output.status.success(),
        "verify-packet accepted a malformed promoted fixture"
    );
    assert_output_contains(
        &output,
        "promoted stage4 inputs do not match landed structural contract",
    );
}

#[test]
fn fixture_inputs_are_explicit_placeholders_or_landed_canonical_payloads() {
    let fixture_doc =
        fs::read_to_string(fixture_root().join("README.md")).expect("fixture corpus README reads");
    assert!(
        fixture_doc.contains("fixture_status"),
        "fixture README documents placeholder marker"
    );
    assert!(
        fixture_doc.contains("non-executable"),
        "fixture README documents that placeholders are non-executable"
    );

    for stage in ["stage4", "stage5"] {
        let stage_dir = fixture_root().join("reject").join(stage);
        for entry in fs::read_dir(&stage_dir).expect("reject stage dir exists") {
            let entry = entry.expect("reject fixture dir is readable");
            let dir = entry.path();
            let code = dir
                .file_name()
                .and_then(|name| name.to_str())
                .expect("fixture dir name is utf8");
            assert_fixture_inputs_contract(stage, code, &dir.join("inputs.json"));
        }
    }
}

#[test]
fn tampered_cert_fixtures_are_deterministic_generated_fixtures_not_readme_only() {
    let tampered_root = fixture_root().join("tampered");
    for fixture in [
        "malformed_json",
        "report_self_hash_mismatch",
        "unsupported_plan_family",
        "cert_failed_witness_mismatch",
        "cert_inconsistent_term_count",
        "cert_lowered_slack",
        "cert_wrong_plan_family",
    ] {
        let dir = tampered_root.join(fixture);
        assert_file(&dir.join("README.md"));
        let manifest = dir.join("tamper_manifest.json");
        assert_file(&manifest);
        let value: Value =
            serde_json::from_slice(&fs::read(&manifest).expect("tamper manifest reads"))
                .expect("tamper manifest is JSON");
        assert_eq!(value["fixture"], fixture);
        assert_eq!(value["materialization"], "generated_by_run_cert_verify");
        assert!(
            value["expected_event"].as_str().is_some_and(|event| {
                RANGE_CERT_VERIFY_EVENT_NAMES.contains(&event)
                    || event == "range_cert.independent_verify.certified_reduction.single_i16"
                    || event == "range_cert.independent_verify.certified_reduction.chunked_i16"
            }),
            "{fixture} manifest pins a closed verifier event"
        );
    }
}

fn assert_reject_fixture(stage: &str, code: &str) {
    let dir = fixture_root().join("reject").join(stage).join(code);
    assert!(
        dir.is_dir(),
        "{stage} reject fixture dir missing for {code}"
    );
    assert_file(&dir.join("README.md"));
    assert_file(&dir.join("inputs.json"));
    let expected_diag = dir.join("expected_diag.json");
    assert_file(&expected_diag);

    let value: Value =
        serde_json::from_slice(&fs::read(&expected_diag).expect("expected diag reads"))
            .expect("expected diag is JSON");
    assert_eq!(value["stage"], stage);
    assert_eq!(value["code"], code);
    assert_eq!(value["severity"], "Hard");
    assert_eq!(value["reserved"], false);
    assert_eq!(
        value["origin"],
        match stage {
            "stage4" => STAGE4_DIAGNOSTIC_ORIGIN,
            "stage5" => STAGE5_DIAGNOSTIC_ORIGIN,
            other => panic!("unexpected fixture stage {other}"),
        }
    );
}

fn assert_file(path: &Path) {
    assert!(path.is_file(), "missing fixture file {}", path.display());
}

fn assert_fixture_inputs_contract(stage: &str, code: &str, path: &Path) {
    let raw = fs::read(path).expect("inputs fixture reads");
    let value: Value = serde_json::from_slice(&raw).expect("inputs fixture is JSON");
    if value
        .get("fixture_status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "placeholder")
    {
        assert_eq!(value["stage"], stage);
        assert_eq!(value["code"], code);
        let keys: BTreeSet<_> = value
            .as_object()
            .expect("placeholder payload is object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["code", "fixture_status", "stage"]),
            "placeholder payload has only the non-executable marker fields"
        );
        return;
    }

    match stage {
        "stage4" => {
            let decoded: ObservationPlanInputs =
                serde_json::from_slice(&raw).expect("promoted Stage 4 inputs deserialize");
            assert_eq!(
                strip_trailing_ascii_whitespace(&raw),
                canonical_json_bytes(&decoded).as_slice(),
                "promoted Stage 4 inputs are canonical JSON"
            );
        }
        "stage5" => {
            let decoded: RangePlanInputs =
                serde_json::from_slice(&raw).expect("promoted Stage 5 inputs deserialize");
            assert_eq!(
                strip_trailing_ascii_whitespace(&raw),
                canonical_json_bytes(&decoded).as_slice(),
                "promoted Stage 5 inputs are canonical JSON"
            );
        }
        other => panic!("unexpected fixture stage {other}"),
    }
}

fn strip_trailing_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
    &bytes[..end]
}

fn assert_timestamp_shape(ts: &str) {
    let rest = ts
        .strip_prefix("unix:")
        .expect("timestamp has unix: prefix");
    let (seconds, nanos) = rest
        .split_once('.')
        .expect("timestamp separates seconds and nanos");
    assert!(
        !seconds.is_empty() && seconds.bytes().all(|byte| byte.is_ascii_digit()),
        "timestamp seconds are decimal"
    );
    assert_eq!(nanos.len(), 9, "timestamp has fixed-width nanoseconds");
    assert!(
        nanos.bytes().all(|byte| byte.is_ascii_digit()),
        "timestamp nanos are decimal"
    );
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("f_b6_f_b7")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-codegen has workspace parent")
        .to_path_buf()
}

fn temp_packet_dir(prefix: &str) -> tempfile::TempDir {
    let parent = Path::new("/tmp/f-b6-f-b7-closure");
    fs::create_dir_all(parent).expect("packet temp parent exists");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .expect("packet temp dir")
}

fn write_valid_packet(out_dir: &Path) {
    fs::create_dir_all(out_dir).expect("packet dir exists");

    let stage4: Vec<_> = STAGE4_EVENT_NAMES
        .iter()
        .copied()
        .filter(|event| !STAGE4_CONDITIONAL_EVENTS.contains(event))
        .collect();
    write_ndjson_events(&out_dir.join("stage4-run.ndjson"), &stage4);

    let stage5: Vec<_> = STAGE5_EVENT_NAMES
        .iter()
        .copied()
        .filter(|event| !STAGE5_CONDITIONAL_EVENTS.contains(event))
        .collect();
    write_ndjson_events(&out_dir.join("stage5-run.ndjson"), &stage5);

    write_valid_verify_packet(out_dir);
}

fn write_valid_verify_packet(out_dir: &Path) {
    let passing_cert = out_dir.join("reports/stage5/chunked_i16/certs/range.cert.json");
    let verify_cases = vec![
        (
            passing_cert.clone(),
            "range_cert.independent_verify.parse",
            "passed",
        ),
        (
            passing_cert.clone(),
            "range_cert.independent_verify.report_self_hash_check",
            "passed",
        ),
        (
            passing_cert,
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "passed",
        ),
        (
            out_dir.join("tampered/malformed_json/range.cert.json"),
            "range_cert.independent_verify.failed.malformed",
            "failed",
        ),
        (
            out_dir.join("tampered/report_self_hash_mismatch/range.cert.json"),
            "range_cert.independent_verify.failed.report_self_hash_mismatch",
            "failed",
        ),
        (
            out_dir.join("tampered/unsupported_plan_family/range.cert.json"),
            "range_cert.independent_verify.failed.unsupported_plan_family",
            "failed",
        ),
        (
            out_dir.join("tampered/cert_lowered_slack/range.cert.json"),
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "failed",
        ),
        (
            out_dir.join("tampered/cert_wrong_plan_family/range.cert.json"),
            "range_cert.independent_verify.certified_reduction.single_i16",
            "failed",
        ),
        (
            out_dir.join("tampered/cert_inconsistent_term_count/range.cert.json"),
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "failed",
        ),
        (
            out_dir.join("tampered/cert_failed_witness_mismatch/range.cert.json"),
            "range_cert.independent_verify.failed",
            "failed",
        ),
        (
            out_dir.join("tampered/cert_failed_witness_mismatch/range.cert.json"),
            "range_cert.independent_verify.failed.witness_mismatch",
            "failed",
        ),
    ];
    for (path, _event, _outcome) in &verify_cases {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("verify cert fixture parent");
        }
        fs::write(path, b"synthetic check-existing fixture").expect("verify cert fixture");
    }
    let payloads: Vec<_> = verify_cases
        .iter()
        .enumerate()
        .map(|(index, (path, event, outcome))| {
            telemetry_payload_for_cert(event, index + 1, path, outcome)
        })
        .collect();
    write_ndjson_values(&out_dir.join("verify-packet.ndjson"), &payloads);
}

fn write_ndjson_events(path: &Path, events: &[&str]) {
    let payloads: Vec<_> = events
        .iter()
        .enumerate()
        .map(|(index, event)| telemetry_payload(event, index + 1))
        .collect();
    write_ndjson_values(path, &payloads);
}

fn telemetry_payload(event: &str, seq: usize) -> Value {
    serde_json::json!({
        "ts": format!("unix:1700000000.{seq:09}"),
        "event": event,
        "level": "INFO",
        "target": target_for_event(event),
        "fields": {
            "site_id": "dense.matmul.0",
            "checkpoint_id": "layer.0.post_embedding",
            "compact_checkpoint_id": 1,
            "stratum": "denotation",
            "probe_instance_id": "0007",
            "runtime_probe_id": 7,
            "importance_class": "Required",
            "build_id": "cargo-test",
            "k4_hash": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
            "k5_hash": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
            "outcome": "passed",
            "diag_code": "none",
            "elapsed_ns": seq as u64,
            "event_seq": seq as u64,
            "fixture": "chunked_i16",
        },
        "span": null,
    })
}

fn telemetry_payload_for_cert(event: &str, seq: usize, cert_path: &Path, outcome: &str) -> Value {
    let mut payload = telemetry_payload(event, seq);
    payload["level"] = if outcome == "passed" {
        Value::String("INFO".to_owned())
    } else {
        Value::String("ERROR".to_owned())
    };
    payload["fields"]["outcome"] = Value::String(outcome.to_owned());
    payload["fields"]["cert_path"] = Value::String(cert_path.display().to_string());
    payload
}

fn target_for_event(event: &str) -> &'static str {
    if event.starts_with("stage4.") {
        STAGE4_TRACE_TARGET
    } else if event.starts_with("stage5.") {
        STAGE5_TRACE_TARGET
    } else if event.starts_with("range_cert.") {
        RANGE_CERT_TRACE_TARGET
    } else {
        "unknown"
    }
}

fn read_ndjson_values(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("ndjson reads")
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson line is JSON"))
        .collect()
}

fn write_ndjson_values(path: &Path, payloads: &[Value]) {
    let mut body = payloads
        .iter()
        .map(|payload| serde_json::to_string(payload).expect("payload serializes"))
        .collect::<Vec<_>>()
        .join("\n");
    body.push('\n');
    fs::write(path, body).expect("ndjson writes");
}

fn run_verify_check_existing(out_dir: &Path, fixture_root: Option<&Path>) -> Output {
    let mut command = Command::new(repo_root().join("scripts/review/f-b6-f-b7/verify-packet.sh"));
    command
        .arg("--check-existing")
        .arg(out_dir)
        .current_dir(repo_root());
    if let Some(fixture_root) = fixture_root {
        command.env("F_B6_F_B7_FIXTURE_ROOT", fixture_root);
    }
    command.output().expect("verify-packet.sh executes")
}

fn assert_output_contains(output: &Output, needle: &str) {
    let text = output_text(output);
    assert!(
        text.contains(needle),
        "output did not contain {needle:?}:\n{text}"
    );
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
