//! Independent verifier for `certs/range.cert.json`.
//!
//! This module deliberately consumes only the shared report schema and
//! re-implements the accumulator proof predicates locally. It must not depend
//! on `gbf-codegen`: the trade-off is Cargo independence for verifier
//! deployment at the cost of source-mirrored proof equations that are pinned by
//! verifier-owned tests rather than by calling Stage 5 code.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use gbf_report::report_schemas::f_b6_f_b7_common::AccumulatorDomain;
use gbf_report::report_schemas::quant_graph_v1::DeterminismClassTag;
use gbf_report::report_schemas::range_plan_v1::{
    AccumulatorCertificate, AccumulatorFailureWitness, AccumulatorProofState, CertOutcome,
    CertifiedReduction, LengthField, RangeCertBody, ReductionPlan, ReductionSiteFacts,
    RenormRecurrence, RenormRounding,
};
use gbf_report::{ReportEnvelope, ReportOutcome, ReportSelfHashError, round_trip_self_hash};

const I16_ENVELOPE_U64: u64 = i16::MAX as u64;
const I32_ENVELOPE_U64: u64 = i32::MAX as u64;

const PARSE_EVENT: &str = "range_cert.independent_verify.parse";
const REPORT_SELF_HASH_EVENT: &str = "range_cert.independent_verify.report_self_hash_check";
const SINGLE_I16_EVENT: &str = "range_cert.independent_verify.certified_reduction.single_i16";
const CHUNKED_I16_EVENT: &str = "range_cert.independent_verify.certified_reduction.chunked_i16";
const RENORM_LOOP_EVENT: &str = "range_cert.independent_verify.certified_reduction.renorm_loop";
const FAILED_EVENT: &str = "range_cert.independent_verify.failed";
const MALFORMED_EVENT: &str = "range_cert.independent_verify.failed.malformed";
const REPORT_SELF_HASH_MISMATCH_EVENT: &str =
    "range_cert.independent_verify.failed.report_self_hash_mismatch";
const UNSUPPORTED_PLAN_FAMILY_EVENT: &str =
    "range_cert.independent_verify.failed.unsupported_plan_family";
const WITNESS_MISMATCH_EVENT: &str = "range_cert.independent_verify.failed.witness_mismatch";
const TAMPER_DETECTED_EVENT: &str = "range_cert.independent_verify.tamper_detected";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndependentVerifyReport {
    pub status: IndependentVerifyStatus,
    pub certificate_count: usize,
    pub failed_evidence_count: usize,
    pub events: Vec<IndependentVerifyEvent>,
}

impl IndependentVerifyReport {
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.status, IndependentVerifyStatus::Verified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndependentVerifyStatus {
    Verified,
    FailedEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndependentVerifyEvent {
    pub event: &'static str,
    pub site: Option<String>,
    pub ok: bool,
    pub detail: Option<String>,
}

impl IndependentVerifyEvent {
    #[must_use]
    pub fn passed(event: &'static str) -> Self {
        Self {
            event,
            site: None,
            ok: true,
            detail: None,
        }
    }

    #[must_use]
    pub fn failed(event: &'static str, detail: impl Into<String>) -> Self {
        Self {
            event,
            site: None,
            ok: false,
            detail: Some(detail.into()),
        }
    }

    fn with_site(mut self, site: impl Into<String>) -> Self {
        self.site = Some(site.into());
        self
    }
}

#[derive(Debug)]
pub enum IndependentVerifyError {
    Io(std::io::Error),
    Json {
        message: String,
        event: &'static str,
        events: Vec<IndependentVerifyEvent>,
    },
    SelfHash {
        source: ReportSelfHashError,
        events: Vec<IndependentVerifyEvent>,
    },
    Invariant {
        message: String,
        event: &'static str,
        events: Vec<IndependentVerifyEvent>,
    },
}

impl IndependentVerifyError {
    #[must_use]
    pub fn event(&self) -> &'static str {
        match self {
            Self::Io(_) => MALFORMED_EVENT,
            Self::Json { event, .. } | Self::Invariant { event, .. } => event,
            Self::SelfHash { .. } => REPORT_SELF_HASH_MISMATCH_EVENT,
        }
    }

    #[must_use]
    pub fn events(&self) -> &[IndependentVerifyEvent] {
        match self {
            Self::Io(_) => &[],
            Self::Json { events, .. }
            | Self::SelfHash { events, .. }
            | Self::Invariant { events, .. } => events,
        }
    }
}

impl fmt::Display for IndependentVerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "failed to read range certificate: {error}"),
            Self::Json { message, .. } => write!(f, "failed to parse range certificate: {message}"),
            Self::SelfHash { source, .. } => {
                write!(f, "range certificate self-hash failed: {source}")
            }
            Self::Invariant { message, .. } => {
                write!(f, "range certificate verification failed: {message}")
            }
        }
    }
}

impl std::error::Error for IndependentVerifyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::SelfHash { source, .. } => Some(source),
            Self::Json { .. } | Self::Invariant { .. } => None,
        }
    }
}

pub fn independent_verify_path(
    path: impl AsRef<Path>,
) -> Result<IndependentVerifyReport, IndependentVerifyError> {
    let bytes = fs::read(path).map_err(IndependentVerifyError::Io)?;
    independent_verify_bytes(&bytes)
}

pub fn independent_verify_bytes(
    bytes: &[u8],
) -> Result<IndependentVerifyReport, IndependentVerifyError> {
    let mut events = vec![IndependentVerifyEvent::passed(PARSE_EVENT)];
    tracing::info!(
        target: "gbf_verify::range_cert",
        event = PARSE_EVENT,
        body_size = bytes.len() as u64,
        "range_cert.independent_verify.parse"
    );

    let report: ReportEnvelope<RangeCertBody> = match serde_json::from_slice(bytes) {
        Ok(report) => report,
        Err(error) => {
            let message = error.to_string();
            let event = classify_json_error_event(&message);
            events.push(IndependentVerifyEvent::failed(event, message.clone()));
            return Err(IndependentVerifyError::Json {
                message,
                event,
                events,
            });
        }
    };

    round_trip_self_hash(&report).map_err(|source| {
        let mut events = events.clone();
        events.push(IndependentVerifyEvent::failed(
            REPORT_SELF_HASH_MISMATCH_EVENT,
            source.to_string(),
        ));
        IndependentVerifyError::SelfHash { source, events }
    })?;
    events.push(IndependentVerifyEvent::passed(REPORT_SELF_HASH_EVENT));
    tracing::info!(
        target: "gbf_verify::range_cert",
        event = REPORT_SELF_HASH_EVENT,
        ok = true,
        "range_cert.independent_verify.report_self_hash_check"
    );

    independent_verify_report(report, events)
}

pub fn independent_verify_report(
    report: ReportEnvelope<RangeCertBody>,
    mut events: Vec<IndependentVerifyEvent>,
) -> Result<IndependentVerifyReport, IndependentVerifyError> {
    let body = &report.body;
    verify_certificate_index(body, &mut events)?;

    let mut failed_evidence_count = 0;
    for certified in &body.certificates {
        match &certified.proof {
            AccumulatorCertificate::Failed { .. } => {
                verify_failed_evidence(certified, body.identity.determinism, &mut events)?;
                failed_evidence_count += 1;
            }
            proof => {
                let ok = verifies(
                    proof,
                    &certified.plan,
                    &certified.facts,
                    body.identity.determinism,
                );
                let event = event_for_plan(&certified.plan).ok_or_else(|| {
                    invariant_error(
                        UNSUPPORTED_PLAN_FAMILY_EVENT,
                        format!("unsupported plan family for site {}", certified.site.0),
                        events.clone(),
                    )
                })?;
                events.push(IndependentVerifyEvent {
                    event,
                    site: Some(certified.site.0.clone()),
                    ok,
                    detail: None,
                });
                tracing::info!(
                    target: "gbf_verify::range_cert",
                    event,
                    site = certified.site.0.as_str(),
                    ok,
                    "range_cert.independent_verify.certified_reduction"
                );
                if !ok {
                    return Err(invariant_error(
                        TAMPER_DETECTED_EVENT,
                        format!("proof does not verify for site {}", certified.site.0),
                        events,
                    ));
                }
            }
        }
    }

    let status = match (report.outcome, body.cert_outcome, failed_evidence_count) {
        (ReportOutcome::Passed, CertOutcome::Verified, 0) => IndependentVerifyStatus::Verified,
        (ReportOutcome::Failed, CertOutcome::Failed, failed_evidence_count)
            if failed_evidence_count > 0 =>
        {
            IndependentVerifyStatus::FailedEvidence
        }
        _ => {
            return Err(invariant_error(
                TAMPER_DETECTED_EVENT,
                "report outcome, cert outcome, and failed proof set disagree",
                events,
            ));
        }
    };

    Ok(IndependentVerifyReport {
        status,
        certificate_count: body.certificates.len(),
        failed_evidence_count,
        events,
    })
}

#[must_use]
pub fn verifies(
    cert: &AccumulatorCertificate,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClassTag,
) -> bool {
    if facts.accumulator_domain != AccumulatorDomain::RawIntegerProducts {
        return false;
    }

    match cert {
        AccumulatorCertificate::SingleI16Proof {
            site,
            term_count,
            per_term_abs_max,
            sum_bound,
            bias_abs_max,
            total_abs_max,
            i16_envelope,
            slack,
        } => {
            site == &facts.site
                && *term_count == u64::from(facts.term_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(*term_count, *per_term_abs_max) == Some(*sum_bound)
                && *bias_abs_max == facts_bias_abs_max(facts)
                && checked_add_u64(*sum_bound, *bias_abs_max) == Some(*total_abs_max)
                && *i16_envelope == I16_ENVELOPE_U64
                && *total_abs_max <= *i16_envelope
                && i16_envelope.checked_sub(*total_abs_max) == Some(*slack)
                && matches!(plan, ReductionPlan::SingleI16)
        }
        AccumulatorCertificate::ChunkedI16Proof {
            site,
            chunk_len,
            chunk_count,
            per_term_abs_max,
            per_chunk_sum_bound,
            per_chunk_i16_slack,
            cross_chunk_sum_bound,
            bias_abs_max,
            total_abs_max,
            i32_envelope,
            slack,
        } => {
            let ReductionPlan::ChunkedI16 {
                chunk_len: plan_chunk_len,
            } = plan
            else {
                return false;
            };
            *chunk_len > 0
                && site == &facts.site
                && plan_chunk_len == chunk_len
                && ceil_div_u64(u64::from(facts.term_count), u64::from(*chunk_len))
                    == Some(*chunk_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(u64::from(*chunk_len), *per_term_abs_max)
                    == Some(*per_chunk_sum_bound)
                && *per_chunk_sum_bound <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*per_chunk_sum_bound) == Some(*per_chunk_i16_slack)
                && (determinism != DeterminismClassTag::BitExact
                    || facts.term_count.is_multiple_of(u32::from(*chunk_len)))
                && checked_mul_u64(u64::from(facts.term_count), *per_term_abs_max)
                    == Some(*cross_chunk_sum_bound)
                && *bias_abs_max == facts_bias_abs_max(facts)
                && checked_add_u64(*cross_chunk_sum_bound, *bias_abs_max) == Some(*total_abs_max)
                && *i32_envelope == I32_ENVELOPE_U64
                && *total_abs_max <= *i32_envelope
                && i32_envelope.checked_sub(*total_abs_max) == Some(*slack)
        }
        AccumulatorCertificate::RenormLoopProof {
            site,
            tile_len,
            tile_count,
            per_term_abs_max,
            per_tile_sum_bound,
            per_tile_i16_slack,
            renorm,
            bias_abs_max,
            total_abs_max,
            slack,
        } => {
            if determinism == DeterminismClassTag::BitExact {
                return false;
            }
            let ReductionPlan::RenormLoop {
                tile_len: plan_tile_len,
                renorm: plan_renorm,
            } = plan
            else {
                return false;
            };
            *tile_len > 0
                && site == &facts.site
                && plan_tile_len == tile_len
                && plan_renorm == renorm
                && ceil_div_u64(u64::from(facts.term_count), u64::from(*tile_len))
                    == Some(*tile_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(u64::from(*tile_len), *per_term_abs_max)
                    == Some(*per_tile_sum_bound)
                && *per_tile_sum_bound <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*per_tile_sum_bound) == Some(*per_tile_i16_slack)
                && *bias_abs_max == facts_bias_abs_max(facts)
                // Strategy is already bound by `plan_renorm == renorm` above.
                // The recurrence fields carry the closed-form arithmetic; the
                // strategy margin is producer policy, not an extra recurrence
                // term in the v1 verifier equation.
                && renorm_recurrence_verifies(
                    facts,
                    *tile_len,
                    *tile_count,
                    renorm.recurrence,
                    *total_abs_max,
                )
                && *total_abs_max <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*total_abs_max) == Some(*slack)
        }
        AccumulatorCertificate::Failed { .. } => false,
    }
}

#[must_use]
pub fn renorm_recurrence_verifies(
    facts: &ReductionSiteFacts,
    tile_len: u16,
    tile_count: u64,
    recurrence: RenormRecurrence,
    claimed_total_abs_max: u64,
) -> bool {
    let ok = facts.accumulator_domain == AccumulatorDomain::RawIntegerProducts
        && recurrence.output_scale_q16_16 > 0
        && recurrence.max_rounding_error_q16_16 == rounding_error_q16_16(recurrence.rounding)
        && renorm_closed_form_bound(facts, tile_len, tile_count, recurrence)
            == Some(claimed_total_abs_max);
    tracing::info!(
        target: "gbf_verify::range_cert",
        event = "range_cert.renorm_recurrence_verifies",
        site = facts.site.0.as_str(),
        ok,
        "range_cert.renorm_recurrence_verifies"
    );
    ok
}

fn verify_certificate_index(
    body: &RangeCertBody,
    events: &mut Vec<IndependentVerifyEvent>,
) -> Result<(), IndependentVerifyError> {
    let expected = body
        .certificates
        .iter()
        .enumerate()
        .map(|(index, certificate)| (certificate.site.clone(), index as u32))
        .collect::<BTreeMap<_, _>>();
    if expected != body.site_to_certificate_index || expected.len() != body.certificates.len() {
        events.push(IndependentVerifyEvent::failed(
            TAMPER_DETECTED_EVENT,
            "range_cert.site_to_certificate_index does not match certificates",
        ));
        return Err(invariant_error(
            TAMPER_DETECTED_EVENT,
            "range_cert.site_to_certificate_index does not match certificates",
            events.clone(),
        ));
    }

    for certified in &body.certificates {
        if certified.site != certified.facts.site {
            events.push(
                IndependentVerifyEvent::failed(
                    TAMPER_DETECTED_EVENT,
                    "certified site does not match facts.site",
                )
                .with_site(certified.site.0.clone()),
            );
            return Err(invariant_error(
                TAMPER_DETECTED_EVENT,
                format!(
                    "certified site does not match facts.site for {}",
                    certified.site.0
                ),
                events.clone(),
            ));
        }
    }
    Ok(())
}

fn verify_failed_evidence(
    certified: &CertifiedReduction,
    determinism: DeterminismClassTag,
    events: &mut Vec<IndependentVerifyEvent>,
) -> Result<(), IndependentVerifyError> {
    let AccumulatorCertificate::Failed {
        site,
        attempted_plan,
        proof_state,
        witness,
    } = &certified.proof
    else {
        unreachable!("caller filters failed proofs")
    };

    let ok = site == &certified.site
        && attempted_plan == &certified.plan
        && failed_witness_matches_facts(
            witness,
            proof_state,
            &certified.plan,
            &certified.facts,
            determinism,
        );
    events.push(IndependentVerifyEvent {
        event: FAILED_EVENT,
        site: Some(certified.site.0.clone()),
        ok,
        detail: None,
    });
    tracing::info!(
        target: "gbf_verify::range_cert",
        event = FAILED_EVENT,
        site = certified.site.0.as_str(),
        ok,
        reason = ?proof_state,
        "range_cert.independent_verify.failed"
    );

    if ok {
        Ok(())
    } else {
        events.push(
            IndependentVerifyEvent::failed(WITNESS_MISMATCH_EVENT, "failed proof witness mismatch")
                .with_site(certified.site.0.clone()),
        );
        Err(invariant_error(
            WITNESS_MISMATCH_EVENT,
            format!(
                "failed proof witness mismatch for site {}",
                certified.site.0
            ),
            events.clone(),
        ))
    }
}

fn failed_witness_matches_facts(
    witness: &AccumulatorFailureWitness,
    proof_state: &AccumulatorProofState,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClassTag,
) -> bool {
    match witness {
        AccumulatorFailureWitness::BoundCalculation {
            input_max_abs_q,
            weight_max_abs_q,
            term_count,
            bias,
        } => {
            *input_max_abs_q == facts.input_max_abs_q
                && *weight_max_abs_q == facts.weight_max_abs_q
                && *term_count == facts.term_count
                && *bias == facts.bias_max_abs_q.unwrap_or(0)
                && proof_state_matches_plan(proof_state, plan)
        }
        AccumulatorFailureWitness::BitExactSaturationForbidden => {
            determinism == DeterminismClassTag::BitExact
                && matches!(plan, ReductionPlan::RenormLoop { .. })
                && matches!(
                    proof_state,
                    AccumulatorProofState::DeterminismRequiresEnforcedRenorm
                )
        }
    }
}

fn proof_state_matches_plan(proof_state: &AccumulatorProofState, plan: &ReductionPlan) -> bool {
    match proof_state {
        AccumulatorProofState::SumExceedsI16Envelope { envelope, .. } => {
            *envelope == I16_ENVELOPE_U64
        }
        AccumulatorProofState::PerChunkExceedsI16Envelope { envelope, .. } => {
            *envelope == I16_ENVELOPE_U64 && matches!(plan, ReductionPlan::ChunkedI16 { .. })
        }
        AccumulatorProofState::CrossChunkExceedsI32Envelope { envelope, .. } => {
            *envelope == I32_ENVELOPE_U64 && matches!(plan, ReductionPlan::ChunkedI16 { .. })
        }
        AccumulatorProofState::PerTileExceedsI16Envelope { envelope, .. } => {
            *envelope == I16_ENVELOPE_U64 && matches!(plan, ReductionPlan::RenormLoop { .. })
        }
        AccumulatorProofState::LengthZero { length_field } => matches!(
            (length_field, plan),
            (LengthField::ChunkLen, ReductionPlan::ChunkedI16 { .. })
                | (LengthField::TileLen, ReductionPlan::RenormLoop { .. })
        ),
        AccumulatorProofState::ChunkLenExceedsProfileMax { .. } => {
            matches!(plan, ReductionPlan::ChunkedI16 { .. })
        }
        AccumulatorProofState::TileLenBelowProfileMin { .. }
        | AccumulatorProofState::TileLenExceedsProfileMax { .. }
        | AccumulatorProofState::TileLenExceedsU16 { .. } => {
            matches!(plan, ReductionPlan::RenormLoop { .. })
        }
        AccumulatorProofState::BitExactRequiresChunkDivides { .. } => {
            matches!(plan, ReductionPlan::ChunkedI16 { .. })
        }
        AccumulatorProofState::DeterminismRequiresEnforcedRenorm => false,
    }
}

fn event_for_plan(plan: &ReductionPlan) -> Option<&'static str> {
    match plan {
        ReductionPlan::SingleI16 => Some(SINGLE_I16_EVENT),
        ReductionPlan::ChunkedI16 { .. } => Some(CHUNKED_I16_EVENT),
        ReductionPlan::RenormLoop { .. } => Some(RENORM_LOOP_EVENT),
    }
}

fn invariant_error(
    event: &'static str,
    message: impl Into<String>,
    events: Vec<IndependentVerifyEvent>,
) -> IndependentVerifyError {
    let message = message.into();
    tracing::error!(
        target: "gbf_verify::range_cert",
        event,
        message = message.as_str(),
        "range_cert.independent_verify.tamper_detected"
    );
    IndependentVerifyError::Invariant {
        message,
        event,
        events,
    }
}

fn classify_json_error_event(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("report_self_hash") && lower.contains("does not match expected") {
        REPORT_SELF_HASH_MISMATCH_EVENT
    } else if lower.contains("unknown variant")
        && (lower.contains("singlei16")
            || lower.contains("chunkedi16")
            || lower.contains("renormloop"))
    {
        UNSUPPORTED_PLAN_FAMILY_EVENT
    } else {
        MALFORMED_EVENT
    }
}

fn renorm_closed_form_bound(
    facts: &ReductionSiteFacts,
    tile_len: u16,
    tile_count: u64,
    recurrence: RenormRecurrence,
) -> Option<u64> {
    if tile_len == 0 || recurrence.output_scale_q16_16 == 0 {
        return None;
    }
    if ceil_div_u64(u64::from(facts.term_count), u64::from(tile_len))? != tile_count {
        return None;
    }

    let per_tile_sum_bound = checked_mul_u64(u64::from(tile_len), facts.per_term_abs_max_q)?;
    let rounding_error_units =
        ceil_div_u64(u64::from(recurrence.max_rounding_error_q16_16), 65_536)?;
    let mut state = 0_u64;
    for _ in 0..tile_count {
        let pre_scale = checked_add_u64(state, per_tile_sum_bound)?;
        let scaled = ceil_div_u128(
            u128::from(pre_scale).checked_mul(u128::from(recurrence.input_scale_q16_16))?,
            u128::from(recurrence.output_scale_q16_16),
        )?;
        state = checked_add_u64(u64::try_from(scaled).ok()?, rounding_error_units)?;
    }
    checked_add_u64(state, facts_bias_abs_max(facts))
}

const fn rounding_error_q16_16(rounding: RenormRounding) -> u32 {
    match rounding {
        RenormRounding::TowardZero => 0,
        RenormRounding::NearestEven => 1,
    }
}

fn facts_bias_abs_max(facts: &ReductionSiteFacts) -> u64 {
    u64::from(facts.bias_max_abs_q.unwrap_or(0))
}

const fn checked_mul_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_mul(right)
}

const fn checked_add_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}

fn ceil_div_u64(numerator: u64, denominator: u64) -> Option<u64> {
    if denominator == 0 {
        return None;
    }
    let extra = if numerator.is_multiple_of(denominator) {
        0
    } else {
        1
    };
    Some(numerator / denominator + extra)
}

fn ceil_div_u128(numerator: u128, denominator: u128) -> Option<u128> {
    if denominator == 0 {
        return None;
    }
    let extra = if numerator.is_multiple_of(denominator) {
        0
    } else {
        1
    };
    Some(numerator / denominator + extra)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use gbf_foundation::Hash256;
    use gbf_policy::{InferOpTag, ReductionSiteId};
    use gbf_report::report_schemas::f_b6_f_b7_common::AccumulatorDomain;
    use gbf_report::report_schemas::quant_graph_v1::DeterminismClassTag;
    use gbf_report::report_schemas::range_plan_v1::{
        AccumulatorCertificate, AccumulatorFailureWitness, AccumulatorProofState, CertOutcome,
        CertifiedReduction, RangeCertBody, RangeCertIdentity, ReductionPlan, ReductionSiteFacts,
        RenormRecurrence, RenormRounding, RenormSaturationPolicy, RenormSpec, RenormStrategy,
    };
    use gbf_report::{ReportEnvelope, ReportOutcome};

    use super::{
        IndependentVerifyError, IndependentVerifyStatus, independent_verify_bytes,
        independent_verify_path, independent_verify_report, renorm_recurrence_verifies,
    };

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn site(raw: &str) -> ReductionSiteId {
        ReductionSiteId(raw.to_owned())
    }

    fn facts(
        site_id: &str,
        term_count: u32,
        per_term_abs_max_q: u64,
        bias: Option<u32>,
    ) -> ReductionSiteFacts {
        ReductionSiteFacts {
            site: site(site_id),
            layer: None,
            expert: None,
            slot: None,
            norm_site: None,
            term_count,
            input_max_abs_q: 1,
            weight_max_abs_q: per_term_abs_max_q.try_into().unwrap_or(u32::MAX),
            per_term_abs_max_q,
            bias_max_abs_q: bias,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            op_tag: InferOpTag::Classify,
        }
    }

    fn identity(determinism: DeterminismClassTag) -> RangeCertIdentity {
        RangeCertIdentity {
            range_plan_self_hash: Some(hash(9)),
            infer_ir_self_hash: hash(1),
            quant_graph_self_hash: hash(2),
            static_budget_self_hash: hash(3),
            determinism,
        }
    }

    fn body(
        cert_outcome: CertOutcome,
        determinism: DeterminismClassTag,
        certificates: Vec<CertifiedReduction>,
    ) -> RangeCertBody {
        RangeCertBody {
            identity: identity(determinism),
            cert_outcome,
            site_to_certificate_index: certificates
                .iter()
                .enumerate()
                .map(|(index, certificate)| (certificate.site.clone(), index as u32))
                .collect::<BTreeMap<_, _>>(),
            certificates,
            diagnostics: Vec::new(),
        }
    }

    fn report_bytes(body: RangeCertBody, outcome: ReportOutcome) -> Vec<u8> {
        let report = ReportEnvelope::new(outcome, body)
            .expect("report envelope")
            .with_computed_self_hash()
            .expect("self hash");
        serde_json::to_vec(&report).expect("serialize report")
    }

    fn single_i16_cert(site_id: &str) -> CertifiedReduction {
        let facts = facts(site_id, 8, 256, Some(4));
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::SingleI16,
            facts,
            proof: AccumulatorCertificate::SingleI16Proof {
                site: site(site_id),
                term_count: 8,
                per_term_abs_max: 256,
                sum_bound: 2_048,
                bias_abs_max: 4,
                total_abs_max: 2_052,
                i16_envelope: 32_767,
                slack: 30_715,
            },
        }
    }

    fn chunked_i16_cert(site_id: &str) -> CertifiedReduction {
        let facts = facts(site_id, 64, 512, Some(7));
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::ChunkedI16 { chunk_len: 32 },
            facts,
            proof: AccumulatorCertificate::ChunkedI16Proof {
                site: site(site_id),
                chunk_len: 32,
                chunk_count: 2,
                per_term_abs_max: 512,
                per_chunk_sum_bound: 16_384,
                per_chunk_i16_slack: 16_383,
                cross_chunk_sum_bound: 32_768,
                bias_abs_max: 7,
                total_abs_max: 32_775,
                i32_envelope: 2_147_483_647,
                slack: 2_147_450_872,
            },
        }
    }

    fn renorm_spec() -> RenormSpec {
        RenormSpec {
            strategy: RenormStrategy::DynamicMargin {
                margin_q16_16: 0x0000_4000,
            },
            recurrence: RenormRecurrence {
                input_scale_q16_16: 0x0001_0000,
                output_scale_q16_16: 0x0010_0000,
                rounding: RenormRounding::NearestEven,
                saturation: RenormSaturationPolicy::Forbidden,
                max_rounding_error_q16_16: 1,
            },
        }
    }

    fn renorm_loop_cert(site_id: &str) -> CertifiedReduction {
        let facts = facts(site_id, 64, 1_000, Some(3));
        let renorm = renorm_spec();
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::RenormLoop {
                tile_len: 16,
                renorm: renorm.clone(),
            },
            facts,
            proof: AccumulatorCertificate::RenormLoopProof {
                site: site(site_id),
                tile_len: 16,
                tile_count: 4,
                per_term_abs_max: 1_000,
                per_tile_sum_bound: 16_000,
                per_tile_i16_slack: 16_767,
                renorm,
                bias_abs_max: 3,
                total_abs_max: 1_071,
                slack: 31_696,
            },
        }
    }

    fn failed_cert(site_id: &str) -> CertifiedReduction {
        let facts = facts(site_id, 2, 20_000, None);
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::SingleI16,
            facts,
            proof: AccumulatorCertificate::Failed {
                site: site(site_id),
                attempted_plan: ReductionPlan::SingleI16,
                proof_state: AccumulatorProofState::SumExceedsI16Envelope {
                    sum_bound: 40_000,
                    envelope: 32_767,
                },
                witness: AccumulatorFailureWitness::BoundCalculation {
                    input_max_abs_q: 1,
                    weight_max_abs_q: 20_000,
                    term_count: 2,
                    bias: 0,
                },
            },
        }
    }

    fn bitexact_saturation_forbidden_cert(site_id: &str) -> CertifiedReduction {
        let facts = facts(site_id, 8, 1_024, None);
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::RenormLoop {
                tile_len: 4,
                renorm: renorm_spec(),
            },
            facts,
            proof: AccumulatorCertificate::Failed {
                site: site(site_id),
                attempted_plan: ReductionPlan::RenormLoop {
                    tile_len: 4,
                    renorm: renorm_spec(),
                },
                proof_state: AccumulatorProofState::DeterminismRequiresEnforcedRenorm,
                witness: AccumulatorFailureWitness::BitExactSaturationForbidden,
            },
        }
    }

    #[test]
    fn independent_verify_parses_canonical_cert_json() {
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![single_i16_cert("site.single")],
            ),
            ReportOutcome::Passed,
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("range.cert.json");
        fs::write(&path, bytes).expect("write cert");

        let report = independent_verify_path(&path).expect("verifies");

        assert_eq!(report.status, IndependentVerifyStatus::Verified);
        assert_eq!(report.certificate_count, 1);
    }

    #[test]
    fn independent_verify_report_self_hash_round_trips() {
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![single_i16_cert("site.self_hash")],
            ),
            ReportOutcome::Passed,
        );

        assert!(independent_verify_bytes(&bytes).is_ok());

        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json value");
        value["report_self_hash"] = serde_json::Value::String(hash(0).to_string());
        let tampered = serde_json::to_vec(&value).expect("tampered json");

        let error =
            independent_verify_bytes(&tampered).expect_err("self-hash mismatch is rejected");
        assert_eq!(
            error.event(),
            "range_cert.independent_verify.failed.report_self_hash_mismatch"
        );
        assert!(
            error.events().iter().any(|event| event.event
                == "range_cert.independent_verify.failed.report_self_hash_mismatch"),
            "self-hash mismatch emits the exact failure event"
        );
    }

    #[test]
    fn independent_verify_single_i16_proof() {
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![single_i16_cert("site.single.pass")],
            ),
            ReportOutcome::Passed,
        );

        assert!(
            independent_verify_bytes(&bytes)
                .expect("verifies")
                .is_verified()
        );
    }

    #[test]
    fn independent_verify_chunked_i16_proof() {
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![chunked_i16_cert("site.chunk.pass")],
            ),
            ReportOutcome::Passed,
        );

        assert!(
            independent_verify_bytes(&bytes)
                .expect("verifies")
                .is_verified()
        );
    }

    #[test]
    fn independent_verify_renorm_loop_proof_non_bitexact() {
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![renorm_loop_cert("site.renorm.pass")],
            ),
            ReportOutcome::Passed,
        );

        assert!(
            independent_verify_bytes(&bytes)
                .expect("verifies")
                .is_verified()
        );
    }

    #[test]
    fn independent_verify_failed_pass_through() {
        let bytes = report_bytes(
            body(
                CertOutcome::Failed,
                DeterminismClassTag::Deterministic,
                vec![failed_cert("site.failed")],
            ),
            ReportOutcome::Failed,
        );

        let report = independent_verify_bytes(&bytes).expect("failed evidence is accepted");

        assert_eq!(report.status, IndependentVerifyStatus::FailedEvidence);
        assert_eq!(report.failed_evidence_count, 1);
    }

    #[test]
    fn independent_verify_failed_outcome_requires_failed_evidence() {
        let body = body(
            CertOutcome::Failed,
            DeterminismClassTag::Deterministic,
            vec![single_i16_cert("site.failed_without_evidence")],
        );
        let report = ReportEnvelope::new(ReportOutcome::Failed, body).expect("report envelope");

        let error = independent_verify_report(report, Vec::new())
            .expect_err("failed envelope without failed evidence is rejected");

        assert!(matches!(error, IndependentVerifyError::Invariant { .. }));
        assert_eq!(
            error.event(),
            "range_cert.independent_verify.tamper_detected"
        );
    }

    #[test]
    fn independent_verify_bitexact_saturation_forbidden_witness() {
        let bytes = report_bytes(
            body(
                CertOutcome::Failed,
                DeterminismClassTag::BitExact,
                vec![bitexact_saturation_forbidden_cert(
                    "site.failed.bitexact_saturation",
                )],
            ),
            ReportOutcome::Failed,
        );

        let report =
            independent_verify_bytes(&bytes).expect("bitexact saturation witness is accepted");
        assert_eq!(report.status, IndependentVerifyStatus::FailedEvidence);
        assert_eq!(report.failed_evidence_count, 1);

        let bytes = report_bytes(
            body(
                CertOutcome::Failed,
                DeterminismClassTag::Deterministic,
                vec![bitexact_saturation_forbidden_cert(
                    "site.failed.bitexact_saturation.bad",
                )],
            ),
            ReportOutcome::Failed,
        );

        assert!(
            independent_verify_bytes(&bytes).is_err(),
            "bitexact saturation witness is rejected outside BitExact determinism"
        );
    }

    #[test]
    fn independent_verify_renorm_recurrence_verifies() {
        let facts = facts("site.renorm.recur", 64, 1_000, Some(3));
        let renorm = renorm_spec();

        assert!(renorm_recurrence_verifies(
            &facts,
            16,
            4,
            renorm.recurrence,
            1_071,
        ));
        assert!(!renorm_recurrence_verifies(
            &facts,
            16,
            4,
            RenormRecurrence {
                output_scale_q16_16: 0,
                ..renorm.recurrence
            },
            1_071,
        ));
    }

    #[test]
    fn renorm_closed_form_is_recurrence_bound_not_strategy_margin_bound() {
        let dynamic = renorm_loop_cert("site.renorm.dynamic_margin");
        let mut exact = dynamic.clone();
        exact.plan = ReductionPlan::RenormLoop {
            tile_len: 16,
            renorm: RenormSpec {
                strategy: RenormStrategy::ExactPostBoundary,
                recurrence: renorm_spec().recurrence,
            },
        };
        if let AccumulatorCertificate::RenormLoopProof { renorm, .. } = &mut exact.proof {
            renorm.strategy = RenormStrategy::ExactPostBoundary;
        }

        for certified in [dynamic, exact] {
            let bytes = report_bytes(
                body(
                    CertOutcome::Verified,
                    DeterminismClassTag::Deterministic,
                    vec![certified],
                ),
                ReportOutcome::Passed,
            );
            assert!(
                independent_verify_bytes(&bytes)
                    .expect("same recurrence bound verifies for either strategy")
                    .is_verified()
            );
        }
    }

    #[test]
    fn independent_verify_lowered_slack_tamper_fails() {
        let mut certified = single_i16_cert("site.tamper.slack");
        if let AccumulatorCertificate::SingleI16Proof { slack, .. } = &mut certified.proof {
            *slack -= 1;
        }
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![certified],
            ),
            ReportOutcome::Passed,
        );

        assert!(independent_verify_bytes(&bytes).is_err());
    }

    #[test]
    fn independent_verify_wrong_plan_family_tamper_fails() {
        let mut certified = single_i16_cert("site.tamper.plan");
        certified.plan = ReductionPlan::ChunkedI16 { chunk_len: 32 };
        let bytes = report_bytes(
            body(
                CertOutcome::Verified,
                DeterminismClassTag::Deterministic,
                vec![certified],
            ),
            ReportOutcome::Passed,
        );

        assert!(independent_verify_bytes(&bytes).is_err());
    }

    #[test]
    fn independent_verify_failed_witness_mismatch_fails() {
        let mut certified = failed_cert("site.tamper.failed");
        if let AccumulatorCertificate::Failed {
            witness: AccumulatorFailureWitness::BoundCalculation { term_count, .. },
            ..
        } = &mut certified.proof
        {
            *term_count += 1;
        }
        let bytes = report_bytes(
            body(
                CertOutcome::Failed,
                DeterminismClassTag::Deterministic,
                vec![certified],
            ),
            ReportOutcome::Failed,
        );

        assert!(independent_verify_bytes(&bytes).is_err());
    }
}
