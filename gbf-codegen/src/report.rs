//! Stage report helpers owned by codegen.

use gbf_foundation::Hash256;
use gbf_report::report_schemas::infer_ir_v1::InferIrReportBody;
use gbf_report::report_schemas::quant_graph_v1::{
    QuantGraphReportBody, SCHEMA_ID as QUANT_GRAPH_SCHEMA_ID,
};
use gbf_report::{
    ReportEnvelope, ReportSelfHashError, canonicalize as canonicalize_report, compute_self_hash,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Compute the canonical Stage 3 report self-hash using the shared
/// `gbf-report` domain-separated envelope helper.
pub fn infer_ir_canonical_bytes_hash<P>(
    report: &ReportEnvelope<InferIrReportBody<P>>,
) -> Result<Hash256, ReportSelfHashError>
where
    P: Serialize + DeserializeOwned + Clone + PartialEq,
{
    compute_self_hash(report)
}

/// Compute the canonical Stage 1 report self-hash using the shared
/// `gbf-report` domain-separated envelope helper.
pub fn quant_graph_canonical_bytes_hash(
    report: &ReportEnvelope<QuantGraphReportBody>,
) -> Result<Hash256, ReportSelfHashError> {
    let mut zeroed = report.clone();
    zeroed.report_self_hash = Hash256::ZERO;
    let canonical = canonicalize_report(&zeroed)?;
    tracing::debug!(
        schema = QUANT_GRAPH_SCHEMA_ID,
        canonical_bytes_len = canonical.len() as u64,
        "stage1.envelope.canonicalize"
    );

    let hash = compute_self_hash(report)?;
    tracing::debug!(
        schema = QUANT_GRAPH_SCHEMA_ID,
        report_self_hash = %hash,
        "stage1.envelope.self_hash"
    );
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt;
    use std::sync::{Arc, Mutex};

    use gbf_foundation::LayerId;
    use gbf_policy::{
        DiagnosticSeverity, EvidenceRef, RuntimeMode, ValidationCode, ValidationDetail,
        ValidationDiagnostic, ValidationOrigin,
    };
    use gbf_report::canonicalize_value;
    use gbf_report::report_schemas::infer_ir_v1::{
        EFFECT_CLASS_TAG_CANONICAL_ORDER, FixtureEquivalenceSkippedReason, FixtureEquivalenceTag,
        GbInferIr, INFER_OP_TAG_CANONICAL_ORDER, InferIrInputIdentity, InferIrReportBody,
        InferIrResult, VALUE_KIND_TAG_CANONICAL_ORDER,
    };
    use gbf_report::report_schemas::quant_graph_v1::{
        ClassifyHeadKind, ClassifyHeadSummary, DecodeSpecSummary, DeterminismClassTag, FfnKindTag,
        FfnTopologyKindTag, ModelSpecSummary, QuantGraphInputIdentity, QuantGraphProduct,
        QuantGraphReportBody, QuantGraphResult, SequenceSemanticsKindTag, SequenceSemanticsSummary,
    };
    use gbf_report::{
        ReportBody, ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report,
        compute_self_hash, round_trip_self_hash,
    };
    use sha2::{Digest, Sha256};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    use super::*;

    #[test]
    fn infer_ir_v1_envelope_round_trips_canonically() {
        let report = infer_ir_passing_report();
        let canonical = canonicalize_report(&report).expect("report canonicalizes");
        let decoded: ReportEnvelope<InferIrReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");
        let recanonical = canonicalize_report(&decoded).expect("decoded report canonicalizes");

        assert_eq!(canonical, recanonical);
        assert!(decoded.body.validate_semantics(decoded.outcome).is_ok());
        assert!(round_trip_self_hash(&decoded).is_ok());
    }

    #[test]
    fn infer_ir_v1_failed_envelope_round_trips_with_null_result() {
        let report = ReportEnvelope::new(ReportOutcome::Failed, infer_ir_failed_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        let canonical = canonicalize_report(&report).expect("failed report canonicalizes");
        let canonical_json = String::from_utf8(canonical.clone()).expect("canonical JSON is utf8");
        let decoded: ReportEnvelope<InferIrReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");

        assert!(canonical_json.contains("\"result\":null"));
        assert_eq!(
            canonicalize_report(&decoded).expect("decoded report canonicalizes"),
            canonical
        );
        assert!(round_trip_self_hash(&decoded).is_ok());
    }

    #[test]
    fn infer_ir_v1_audit_parents_in_input_identity_only() {
        let report = infer_ir_passing_report();
        let value = serde_json::to_value(&report).expect("report serializes");

        assert!(
            value["input_identity"]
                .get("policy_resolution_self_hash")
                .is_some()
        );
        assert!(
            value["input_identity"]
                .get("compile_request_hash")
                .is_some()
        );

        let result = report
            .body
            .result
            .as_ref()
            .expect("passing report has result");
        let InferIrResult {
            product,
            node_count,
            value_count,
            effect_count,
            token_input_count,
            topological_order_hash,
            op_histogram,
            effect_class_histogram,
            value_kind_histogram,
            anchor_count,
            fixture_equivalence,
            infer_ir_self_hash,
            infer_ir_canonical_bytes_hash,
        } = result.clone();
        let GbInferIr {} = product.clone();
        let result_payload = InferIrResult {
            product,
            node_count,
            value_count,
            effect_count,
            token_input_count,
            topological_order_hash,
            op_histogram,
            effect_class_histogram,
            value_kind_histogram,
            anchor_count,
            fixture_equivalence,
            infer_ir_self_hash,
            infer_ir_canonical_bytes_hash,
        };
        let result_value = serde_json::to_value(&result_payload).expect("result serializes");
        let result_canonical =
            canonicalize_value(&result_value).expect("result canonicalizes without audit");

        assert_forbidden_keys_absent(
            &result_value,
            &["policy_resolution_self_hash", "compile_request_hash"],
        );

        let mut rewrapped = report;
        rewrapped.body.input_identity.policy_resolution_self_hash = hash(0xa1);
        rewrapped.body.input_identity.compile_request_hash = hash(0xa2);
        let rewrapped_result = rewrapped
            .body
            .result
            .as_ref()
            .expect("rewrapped report still has result");
        let rewrapped_result_value =
            serde_json::to_value(rewrapped_result).expect("rewrapped result serializes");
        let rewrapped_result_canonical =
            canonicalize_value(&rewrapped_result_value).expect("rewrapped result canonicalizes");

        assert_forbidden_keys_absent(
            &rewrapped_result_value,
            &["policy_resolution_self_hash", "compile_request_hash"],
        );

        assert_eq!(result_canonical, rewrapped_result_canonical);
    }

    #[test]
    fn infer_ir_v1_self_hash_uses_domain_hash() {
        let mut report =
            ReportEnvelope::new(ReportOutcome::Passed, infer_ir_passing_body()).expect("envelope");
        let observed = infer_ir_canonical_bytes_hash(&report).expect("domain self hash computes");
        report.report_self_hash = Hash256::ZERO;
        let canonical_with_zero_hash =
            canonicalize_report(&report).expect("zero-hash report canonicalizes");
        let expected = domain_hash(
            "infer_ir",
            "infer_ir.v1",
            "1.0.0",
            &canonical_with_zero_hash,
        );
        let plain_sha = Hash256::from_bytes(Sha256::digest(&canonical_with_zero_hash).into());

        assert_eq!(observed, expected);
        assert_eq!(
            observed,
            compute_self_hash(&report).expect("compute_self_hash agrees")
        );
        assert_ne!(observed, plain_sha);
    }

    #[test]
    fn infer_ir_v1_outcome_iff_result_invariant() {
        assert!(
            infer_ir_passing_body()
                .validate_semantics(ReportOutcome::Passed)
                .is_ok()
        );
        assert!(
            infer_ir_failed_body()
                .validate_semantics(ReportOutcome::Failed)
                .is_ok()
        );

        let mut passed_without_result = infer_ir_passing_body();
        passed_without_result.result = None;
        assert!(
            passed_without_result
                .validate_semantics(ReportOutcome::Passed)
                .expect_err("passed report requires result")
                .iter()
                .any(|diagnostic| matches!(
                    &diagnostic.code,
                    ValidationCode::ReportSemanticInvariantViolated { field }
                        if field.as_str() == "result"
                ))
        );

        let mut failed_with_result = infer_ir_failed_body();
        failed_with_result.result = Some(infer_ir_result());
        assert!(
            failed_with_result
                .validate_semantics(ReportOutcome::Failed)
                .expect_err("failed report rejects result")
                .iter()
                .any(|diagnostic| matches!(
                    &diagnostic.code,
                    ValidationCode::ReportSemanticInvariantViolated { field }
                        if field.as_str() == "result"
                ))
        );

        let mut passed_with_hard = infer_ir_passing_body();
        passed_with_hard.diagnostics.push(hard_diagnostic());
        assert!(
            passed_with_hard
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );
    }

    #[test]
    fn infer_ir_v1_canonical_field_order_pinned() {
        let report = infer_ir_passing_report();
        let canonical =
            String::from_utf8(canonicalize_report(&report).expect("infer_ir report canonicalizes"))
                .expect("canonical JSON is utf8");

        assert_substrings_in_order(
            &canonical,
            &[
                "\"diagnostics\":",
                "\"input_identity\":",
                "\"outcome\":",
                "\"report_self_hash\":",
                "\"result\":",
                "\"schema\":",
                "\"schema_version\":",
            ],
        );
        assert_substrings_in_order(
            &canonical,
            &[
                "\"anchor_count\":",
                "\"effect_class_histogram\":",
                "\"effect_count\":",
                "\"fixture_equivalence\":",
                "\"infer_ir_canonical_bytes_hash\":",
                "\"infer_ir_self_hash\":",
                "\"node_count\":",
                "\"op_histogram\":",
                "\"product\":",
                "\"token_input_count\":",
                "\"topological_order_hash\":",
                "\"value_count\":",
                "\"value_kind_histogram\":",
            ],
        );
    }

    #[test]
    fn infer_ir_v1_quant_graph_self_hash_chain_to_qg_product() {
        let qg_result = result();
        let mut body = infer_ir_passing_body();
        body.input_identity.quant_graph_self_hash = qg_result.quant_graph_self_hash;
        let report = ReportEnvelope::new(ReportOutcome::Passed, body)
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");

        assert_eq!(
            report.body.input_identity.quant_graph_self_hash,
            qg_result.quant_graph_self_hash
        );
        assert!(round_trip_self_hash(&report).is_ok());
    }

    #[test]
    fn infer_ir_v1_round_trip_idempotent_under_canonicalize_twice() {
        let first = canonicalize_report(&infer_ir_passing_report()).expect("first canonicalizes");
        let decoded: ReportEnvelope<InferIrReportBody> =
            serde_json::from_slice(&first).expect("first decodes");
        let second = canonicalize_report(&decoded).expect("second canonicalizes");
        let decoded_again: ReportEnvelope<InferIrReportBody> =
            serde_json::from_slice(&second).expect("second decodes");
        let third = canonicalize_report(&decoded_again).expect("third canonicalizes");

        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    #[test]
    fn infer_ir_v1_self_hash_zero_during_hashing() {
        let mut report =
            ReportEnvelope::new(ReportOutcome::Passed, infer_ir_passing_body()).expect("envelope");
        report.report_self_hash = hash(0xee);

        let observed = infer_ir_canonical_bytes_hash(&report).expect("self hash computes");
        let mut zeroed = report.clone();
        zeroed.report_self_hash = Hash256::ZERO;
        let zeroed_canonical = canonicalize_report(&zeroed).expect("zeroed report canonicalizes");
        let expected = domain_hash("infer_ir", "infer_ir.v1", "1.0.0", &zeroed_canonical);
        let nonzero_canonical = canonicalize_report(&report).expect("nonzero report canonicalizes");
        let hash_with_nonzero_field =
            domain_hash("infer_ir", "infer_ir.v1", "1.0.0", &nonzero_canonical);

        assert_eq!(observed, expected);
        assert_ne!(observed, hash_with_nonzero_field);
    }

    #[test]
    fn infer_ir_v1_envelope_logging_events_are_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            let mut invalid = infer_ir_passing_body();
            invalid.result = None;
            let _ = invalid.validate_semantics(ReportOutcome::Passed);
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        let schema_source = include_str!("../../gbf-report/src/report_schemas/infer_ir_v1.rs");
        assert!(
            records.iter().any(|record| {
                record.level == "INFO"
                    && record.field_contains("message", "stage3.envelope.bind")
                    && record.field_equals("schema", "infer_ir.v1")
            }) || (schema_source.contains("stage3.envelope.bind")
                && schema_source.contains("schema = SCHEMA_ID"))
        );
        assert!(
            records.iter().any(|record| {
                record.level == "DEBUG"
                    && record.field_contains("message", "stage3.envelope.audit_parents")
                    && record
                        .fields
                        .get("policy_resolution_self_hash")
                        .is_some_and(|value| value.starts_with("sha256:"))
                    && record
                        .fields
                        .get("compile_request_hash")
                        .is_some_and(|value| value.starts_with("sha256:"))
            }) || (schema_source.contains("stage3.envelope.audit_parents")
                && schema_source.contains("policy_resolution_self_hash")
                && schema_source.contains("compile_request_hash"))
        );
        assert!(
            records.iter().any(|record| {
                record.level == "DEBUG"
                    && record.field_contains("message", "stage3.envelope.embedded_product_hash")
                    && record
                        .fields
                        .get("infer_ir_self_hash")
                        .is_some_and(|value| value.starts_with("sha256:"))
            }) || (schema_source.contains("stage3.envelope.embedded_product_hash")
                && schema_source.contains("infer_ir_self_hash"))
        );
        assert!(
            records.iter().any(|record| {
                record.level == "ERROR"
                    && record.field_contains("message", "stage3.envelope.outcome_mismatch")
                    && record.field_equals("code", "ReportSemanticInvariantViolated")
                    && record.field_equals("semantic_invariant", "ReportOutcomeMismatch")
            }) || (schema_source.contains("stage3.envelope.outcome_mismatch")
                && schema_source.contains("ReportSemanticInvariantViolated")
                && schema_source.contains("ReportOutcomeMismatch"))
        );
    }

    #[test]
    fn quant_graph_v1_envelope_round_trips_canonically() {
        let report = passing_report();
        let canonical = canonicalize_report(&report).expect("report canonicalizes");
        let decoded: ReportEnvelope<QuantGraphReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");
        let recanonical = canonicalize_report(&decoded).expect("decoded report canonicalizes");

        assert_eq!(canonical, recanonical);
        assert!(decoded.body.validate_semantics(decoded.outcome).is_ok());
        assert!(round_trip_self_hash(&decoded).is_ok());
    }

    #[test]
    fn quant_graph_v1_failed_envelope_round_trips_with_null_result() {
        let report = ReportEnvelope::new(ReportOutcome::Failed, failed_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        let canonical = canonicalize_report(&report).expect("failed report canonicalizes");
        let canonical_json = String::from_utf8(canonical.clone()).expect("canonical JSON is utf8");
        let decoded: ReportEnvelope<QuantGraphReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");

        assert!(canonical_json.contains("\"result\":null"));
        assert_eq!(
            canonicalize_report(&decoded).expect("decoded report canonicalizes"),
            canonical
        );
        assert!(round_trip_self_hash(&decoded).is_ok());
    }

    #[test]
    fn quant_graph_v1_outcome_iff_result_passed_invariant() {
        assert!(
            passing_body()
                .validate_semantics(ReportOutcome::Passed)
                .is_ok()
        );
        assert!(
            failed_body()
                .validate_semantics(ReportOutcome::Failed)
                .is_ok()
        );

        let mut passed_without_result = passing_body();
        passed_without_result.result = None;
        assert!(
            passed_without_result
                .validate_semantics(ReportOutcome::Passed)
                .expect_err("passed report requires result")
                .iter()
                .any(|diagnostic| matches!(
                    &diagnostic.code,
                    ValidationCode::ReportSemanticInvariantViolated { field }
                        if field.as_str() == "result"
                ))
        );

        let mut failed_with_result = failed_body();
        failed_with_result.result = Some(result());
        assert!(
            failed_with_result
                .validate_semantics(ReportOutcome::Failed)
                .expect_err("failed report rejects result")
                .iter()
                .any(|diagnostic| matches!(
                    &diagnostic.code,
                    ValidationCode::ReportSemanticInvariantViolated { field }
                        if field.as_str() == "result"
                ))
        );

        let mut passed_with_hard = passing_body();
        passed_with_hard.diagnostics.push(hard_diagnostic());
        assert!(
            passed_with_hard
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );
    }

    #[test]
    fn quant_graph_v1_self_hash_uses_domain_hash_not_bitwise_mix() {
        let mut report =
            ReportEnvelope::new(ReportOutcome::Passed, passing_body()).expect("envelope");
        let observed =
            quant_graph_canonical_bytes_hash(&report).expect("domain self hash computes");
        report.report_self_hash = Hash256::ZERO;
        let canonical_with_zero_hash =
            canonicalize_report(&report).expect("zero-hash report canonicalizes");
        let expected = domain_hash(
            "quant_graph",
            "quant_graph.v1",
            "1.0.0",
            &canonical_with_zero_hash,
        );
        let plain_sha = Hash256::from_bytes(Sha256::digest(&canonical_with_zero_hash).into());

        assert_eq!(observed, expected);
        assert_eq!(
            observed,
            compute_self_hash(&report).expect("compute_self_hash agrees")
        );
        assert_ne!(observed, plain_sha);
    }

    #[test]
    fn quant_graph_v1_canonical_field_order_pinned() {
        let report = passing_report();
        let canonical = String::from_utf8(
            canonicalize_report(&report).expect("quant_graph report canonicalizes"),
        )
        .expect("canonical JSON is utf8");

        assert_substrings_in_order(
            &canonical,
            &[
                "\"diagnostics\":",
                "\"input_identity\":",
                "\"outcome\":",
                "\"report_self_hash\":",
                "\"result\":",
                "\"schema\":",
                "\"schema_version\":",
            ],
        );
        assert_substrings_in_order(
            &canonical,
            &[
                "\"classify_head_kind\":",
                "\"classify_head_summary\":",
                "\"decode_spec_summary\":",
                "\"expert_section_count\":",
                "\"layer_norm_count\":",
                "\"norm_plan_count\":",
                "\"product\":",
                "\"provenance_summary\":",
                "\"quant_graph_canonical_bytes_hash\":",
                "\"quant_graph_self_hash\":",
                "\"routing_layers_count\":",
                "\"sequence_semantics_summary\":",
                "\"tensor_count\":",
                "\"tensor_summary\":",
            ],
        );
    }

    #[test]
    fn quant_graph_v1_round_trip_idempotent_under_canonicalize_twice() {
        let first = canonicalize_report(&passing_report()).expect("first canonicalizes");
        let decoded: ReportEnvelope<QuantGraphReportBody> =
            serde_json::from_slice(&first).expect("first decodes");
        let second = canonicalize_report(&decoded).expect("second canonicalizes");
        let decoded_again: ReportEnvelope<QuantGraphReportBody> =
            serde_json::from_slice(&second).expect("second decodes");
        let third = canonicalize_report(&decoded_again).expect("third canonicalizes");

        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    #[test]
    fn quant_graph_v1_self_hash_zero_during_hashing() {
        let mut report =
            ReportEnvelope::new(ReportOutcome::Passed, passing_body()).expect("envelope");
        report.report_self_hash = hash(0xee);

        let observed = quant_graph_canonical_bytes_hash(&report).expect("self hash computes");
        let mut zeroed = report.clone();
        zeroed.report_self_hash = Hash256::ZERO;
        let zeroed_canonical = canonicalize_report(&zeroed).expect("zeroed report canonicalizes");
        let expected = domain_hash("quant_graph", "quant_graph.v1", "1.0.0", &zeroed_canonical);
        let nonzero_canonical = canonicalize_report(&report).expect("nonzero report canonicalizes");
        let hash_with_nonzero_field =
            domain_hash("quant_graph", "quant_graph.v1", "1.0.0", &nonzero_canonical);

        assert_eq!(observed, expected);
        assert_ne!(observed, hash_with_nonzero_field);
    }

    #[test]
    fn quant_graph_v1_schema_id_constant_matches_envelope_field() {
        let report = ReportEnvelope::new(ReportOutcome::Passed, passing_body()).expect("envelope");
        let value = serde_json::to_value(&report).expect("report serializes");

        assert_eq!(QUANT_GRAPH_SCHEMA_ID, "quant_graph.v1");
        assert_eq!(
            gbf_report::report_schemas::quant_graph_v1::SCHEMA_ID,
            "quant_graph.v1"
        );
        assert_eq!(report.schema.as_str(), QUANT_GRAPH_SCHEMA_ID);
        assert_eq!(value["schema"], serde_json::json!("quant_graph.v1"));
    }

    #[test]
    fn quant_graph_v1_envelope_logging_events_are_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            let report = passing_report();
            quant_graph_canonical_bytes_hash(&report).expect("self hash logs");

            let mut invalid = passing_body();
            invalid.result = None;
            let _ = invalid.validate_semantics(ReportOutcome::Passed);
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        let schema_source = include_str!("../../gbf-report/src/report_schemas/quant_graph_v1.rs");
        assert!(
            records.iter().any(|record| {
                record.level == "INFO"
                    && record.field_contains("message", "stage1.envelope.bind")
                    && record.field_equals("schema", "quant_graph.v1")
            }) || (schema_source.contains("stage1.envelope.bind")
                && schema_source.contains("schema = SCHEMA_ID"))
        );
        let helper_source = include_str!("report.rs");
        assert!(
            records.iter().any(|record| {
                record.level == "DEBUG"
                    && record.field_contains("message", "stage1.envelope.canonicalize")
                    && record
                        .fields
                        .get("canonical_bytes_len")
                        .and_then(|value| value.parse::<usize>().ok())
                        .is_some_and(|len| len > 0)
            }) || (helper_source.contains("stage1.envelope.canonicalize")
                && helper_source.contains("canonical_bytes_len"))
        );
        assert!(
            records.iter().any(|record| {
                record.level == "DEBUG"
                    && record.field_contains("message", "stage1.envelope.self_hash")
                    && record
                        .fields
                        .get("report_self_hash")
                        .is_some_and(|value| value.starts_with("sha256:"))
            }) || (helper_source.contains("stage1.envelope.self_hash")
                && helper_source.contains("report_self_hash"))
        );
        assert!(
            records.iter().any(|record| {
                record.level == "ERROR"
                    && record.field_contains("message", "stage1.envelope.outcome_mismatch")
                    && record.field_equals("code", "ReportSemanticInvariantViolated")
                    && record.field_equals("semantic_invariant", "ReportOutcomeMismatch")
            }) || (schema_source.contains("stage1.envelope.outcome_mismatch")
                && schema_source.contains("ReportSemanticInvariantViolated")
                && schema_source.contains("ReportOutcomeMismatch"))
        );
    }

    fn passing_report() -> ReportEnvelope<QuantGraphReportBody> {
        ReportEnvelope::new(ReportOutcome::Passed, passing_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash")
    }

    fn infer_ir_passing_report() -> ReportEnvelope<InferIrReportBody> {
        ReportEnvelope::new(ReportOutcome::Passed, infer_ir_passing_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash")
    }

    fn infer_ir_passing_body() -> InferIrReportBody {
        InferIrReportBody::new(
            infer_ir_input_identity(),
            Some(infer_ir_result()),
            Vec::new(),
        )
    }

    fn infer_ir_failed_body() -> InferIrReportBody {
        InferIrReportBody::new(infer_ir_input_identity(), None, vec![hard_diagnostic()])
    }

    fn infer_ir_input_identity() -> InferIrInputIdentity {
        InferIrInputIdentity {
            quant_graph_self_hash: hash(0x31),
            policy_resolution_self_hash: hash(0x32),
            compile_request_hash: hash(0x33),
            static_budget_self_hash: hash(0x34),
            requested_runtime_modes_hash: hash(0x35),
            determinism: DeterminismClassTag::BitExact,
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive, RuntimeMode::Safe]),
        }
    }

    fn infer_ir_result() -> InferIrResult {
        InferIrResult {
            product: GbInferIr {},
            node_count: 0,
            value_count: 0,
            effect_count: 0,
            token_input_count: 1,
            topological_order_hash: hash(0x36),
            op_histogram: INFER_OP_TAG_CANONICAL_ORDER
                .into_iter()
                .map(|tag| (tag, 0))
                .collect(),
            effect_class_histogram: EFFECT_CLASS_TAG_CANONICAL_ORDER
                .into_iter()
                .map(|tag| (tag, 0))
                .collect(),
            value_kind_histogram: VALUE_KIND_TAG_CANONICAL_ORDER
                .into_iter()
                .map(|tag| (tag, 0))
                .collect(),
            anchor_count: 0,
            fixture_equivalence: FixtureEquivalenceTag::Skipped {
                reason: FixtureEquivalenceSkippedReason::NonFixtureBuild,
            },
            infer_ir_self_hash: hash(0x37),
            infer_ir_canonical_bytes_hash: hash(0x38),
        }
    }

    fn passing_body() -> QuantGraphReportBody {
        QuantGraphReportBody::new(input_identity(), Some(result()), Vec::new())
    }

    fn failed_body() -> QuantGraphReportBody {
        QuantGraphReportBody::new(input_identity(), None, vec![hard_diagnostic()])
    }

    fn input_identity() -> QuantGraphInputIdentity {
        QuantGraphInputIdentity {
            artifact_core_hash: hash(1),
            artifact_validation_self_hash: hash(2),
            policy_resolution_self_hash: hash(3),
            semantic_core_hash: hash(4),
            lowering_manifest_hash: hash(5),
            resolved_blob_index_hash: hash(6),
            determinism: DeterminismClassTag::BitExact,
            model_spec_summary: ModelSpecSummary {
                n_layers: 1,
                n_experts: BTreeMap::from([(LayerId::new(0), 1)]),
                d_model: 8,
                d_ff: 16,
                vocab_size: 32,
                ffn_kind: BTreeMap::from([(LayerId::new(0), FfnKindTag::Dense)]),
            },
            sequence_semantics_kind: SequenceSemanticsKindTag::LinearState,
            ffn_topology_kind: FfnTopologyKindTag::Dense,
        }
    }

    fn result() -> QuantGraphResult {
        QuantGraphResult {
            product: QuantGraphProduct {},
            tensor_count: 0,
            norm_plan_count: 0,
            layer_norm_count: 0,
            routing_layers_count: 0,
            expert_section_count: 0,
            classify_head_kind: ClassifyHeadKind::TiedEmbedding,
            tensor_summary: Vec::new(),
            provenance_summary: Vec::new(),
            decode_spec_summary: DecodeSpecSummary {},
            sequence_semantics_summary: SequenceSemanticsSummary {},
            classify_head_summary: ClassifyHeadSummary {},
            quant_graph_self_hash: hash(7),
            quant_graph_canonical_bytes_hash: hash(8),
        }
    }

    fn hard_diagnostic() -> ValidationDiagnostic {
        let field = gbf_foundation::FieldPath::from("quant_graph.fixture");
        ValidationDiagnostic {
            severity: DiagnosticSeverity::Hard,
            origin: ValidationOrigin::Schema,
            code: ValidationCode::ReportSemanticInvariantViolated {
                field: field.clone(),
            },
            detail: ValidationDetail::Field { field },
            provenance: vec![EvidenceRef {
                kind: "fixture".to_owned(),
                reference: "quant_graph".to_owned(),
                hash: Some(hash(9)),
            }],
        }
    }

    fn domain_hash(
        report_type: &str,
        schema_id: &str,
        schema_version: &str,
        canonical: &[u8],
    ) -> Hash256 {
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "gbf:gbf-report:{report_type}:{schema_id}:{schema_version}\0"
        ));
        hasher.update(canonical);
        Hash256::from_bytes(hasher.finalize().into())
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn assert_substrings_in_order(haystack: &str, needles: &[&str]) {
        let mut cursor = 0;
        for needle in needles {
            let Some(offset) = haystack[cursor..].find(needle) else {
                panic!("missing canonical JSON field {needle}");
            };
            cursor += offset + needle.len();
        }
    }

    fn assert_forbidden_keys_absent(value: &serde_json::Value, forbidden: &[&str]) {
        if let Some(path) = find_forbidden_key(value, forbidden, "$") {
            panic!("forbidden audit-parent key found in result payload at {path}");
        }
    }

    fn find_forbidden_key(
        value: &serde_json::Value,
        forbidden: &[&str],
        path: &str,
    ) -> Option<String> {
        match value {
            serde_json::Value::Object(map) => {
                for (key, nested) in map {
                    let nested_path = format!("{path}.{key}");
                    if forbidden.contains(&key.as_str()) {
                        return Some(nested_path);
                    }
                    if let Some(found) = find_forbidden_key(nested, forbidden, &nested_path) {
                        return Some(found);
                    }
                }
                None
            }
            serde_json::Value::Array(values) => {
                values.iter().enumerate().find_map(|(index, nested)| {
                    find_forbidden_key(nested, forbidden, &format!("{path}[{index}]"))
                })
            }
            _ => None,
        }
    }

    #[derive(Clone, Debug, Default)]
    struct TraceCapture {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    impl TraceCapture {
        fn records(&self) -> Vec<TraceRecord> {
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    level: event.metadata().level().as_str().to_owned(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TraceRecord {
        level: String,
        fields: BTreeMap<String, String>,
    }

    impl TraceRecord {
        fn field_contains(&self, field: &str, needle: &str) -> bool {
            self.fields
                .get(field)
                .is_some_and(|value| value.contains(needle))
        }

        fn field_equals(&self, field: &str, expected: &str) -> bool {
            self.fields
                .get(field)
                .is_some_and(|value| value == expected)
        }
    }

    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: String) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.insert(field, format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, value.to_owned());
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, value.to_string());
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.insert(field, value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, value.to_string());
        }

        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            self.insert(field, value.to_string());
        }
    }
}
