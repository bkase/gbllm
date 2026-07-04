//! Worst-case simulation anchor for range certificates.
//!
//! The independent verifier ([`super::independent`]) re-checks a
//! certificate's closed-form bound algebra. That is structurally independent
//! of `gbf-codegen`'s producer, but it re-derives the *same equations*, so a
//! conceptual error shared by both derivations would pass both checkers
//! (2026-07-02 audit, gbf-verify finding). This module adds a third path with
//! a different derivation: it *simulates* the certified reduction's worst
//! case term by term in the claimed accumulator widths and checks every
//! intermediate partial against the envelope directly, then requires the
//! simulated worst-case magnitude to equal the certificate's claimed
//! `total_abs_max` exactly.
//!
//! Scope: `SingleI16Proof` and `ChunkedI16Proof`. `RenormLoopProof` embeds a
//! scaling recurrence whose faithful simulation would re-implement the same
//! closed form and add no independence; it reports [`BruteVerdict::NotSimulated`].
//! This is a library-level anchor consumed by tests and future tooling; it
//! deliberately emits no tracing events so the pinned review-packet event
//! contracts stay untouched.

use gbf_report::report_schemas::range_plan_v1::{
    AccumulatorCertificate, CertifiedReduction, ReductionSiteFacts,
};

/// Refuse to simulate absurd term counts rather than loop for minutes; real
/// reduction fan-ins are a few hundred.
const MAX_SIMULATED_TERMS: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BruteVerdict {
    /// Stepwise worst-case simulation stayed inside every claimed envelope
    /// and reproduced the certificate's `total_abs_max` exactly.
    Confirmed,
    /// Simulation contradicted the certificate.
    Refuted(String),
    /// Certificate kind or size is outside this anchor's scope.
    NotSimulated(&'static str),
}

/// Simulate the worst case of a certified reduction.
#[must_use]
pub fn worst_case_simulation(certified: &CertifiedReduction) -> BruteVerdict {
    simulate(&certified.proof, &certified.facts)
}

fn simulate(cert: &AccumulatorCertificate, facts: &ReductionSiteFacts) -> BruteVerdict {
    match cert {
        AccumulatorCertificate::SingleI16Proof {
            per_term_abs_max,
            bias_abs_max,
            total_abs_max,
            i16_envelope,
            ..
        } => simulate_single(
            u64::from(facts.term_count),
            *per_term_abs_max,
            *bias_abs_max,
            *total_abs_max,
            *i16_envelope,
        ),
        AccumulatorCertificate::ChunkedI16Proof {
            chunk_len,
            per_term_abs_max,
            bias_abs_max,
            total_abs_max,
            i32_envelope,
            ..
        } => simulate_chunked(
            u64::from(facts.term_count),
            u64::from(*chunk_len),
            *per_term_abs_max,
            *bias_abs_max,
            *total_abs_max,
            *i32_envelope,
        ),
        AccumulatorCertificate::RenormLoopProof { .. } => {
            BruteVerdict::NotSimulated("renorm recurrence adds no independent derivation")
        }
        AccumulatorCertificate::Failed { .. } => {
            BruteVerdict::NotSimulated("failed certificates carry no bound claim")
        }
    }
}

fn simulate_single(
    term_count: u64,
    per_term_abs_max: u64,
    bias_abs_max: u64,
    claimed_total_abs_max: u64,
    envelope: u64,
) -> BruteVerdict {
    if term_count > MAX_SIMULATED_TERMS {
        return BruteVerdict::NotSimulated("term count exceeds simulation cap");
    }
    let mut acc: u128 = 0;
    for index in 0..term_count {
        acc += u128::from(per_term_abs_max);
        if acc > u128::from(envelope) {
            return BruteVerdict::Refuted(format!(
                "partial sum after term {} reaches {acc}, past envelope {envelope}",
                index + 1
            ));
        }
    }
    acc += u128::from(bias_abs_max);
    if acc > u128::from(envelope) {
        return BruteVerdict::Refuted(format!(
            "bias pushes worst case to {acc}, past envelope {envelope}"
        ));
    }
    if acc != u128::from(claimed_total_abs_max) {
        return BruteVerdict::Refuted(format!(
            "simulated worst case {acc} != claimed total_abs_max {claimed_total_abs_max}"
        ));
    }
    BruteVerdict::Confirmed
}

fn simulate_chunked(
    term_count: u64,
    chunk_len: u64,
    per_term_abs_max: u64,
    bias_abs_max: u64,
    claimed_total_abs_max: u64,
    i32_envelope: u64,
) -> BruteVerdict {
    if chunk_len == 0 {
        return BruteVerdict::Refuted("chunk_len is zero".to_owned());
    }
    if term_count > MAX_SIMULATED_TERMS {
        return BruteVerdict::NotSimulated("term count exceeds simulation cap");
    }
    let i16_envelope = u128::from(i16::MAX.unsigned_abs());
    let mut cross: u128 = 0;
    let mut remaining = term_count;
    while remaining > 0 {
        let this_chunk = remaining.min(chunk_len);
        let mut chunk_acc: u128 = 0;
        for index in 0..this_chunk {
            chunk_acc += u128::from(per_term_abs_max);
            if chunk_acc > i16_envelope {
                return BruteVerdict::Refuted(format!(
                    "chunk partial after term {} reaches {chunk_acc}, past i16 envelope",
                    index + 1
                ));
            }
        }
        cross += chunk_acc;
        if cross > u128::from(i32_envelope) {
            return BruteVerdict::Refuted(format!(
                "cross-chunk partial reaches {cross}, past envelope {i32_envelope}"
            ));
        }
        remaining -= this_chunk;
    }
    cross += u128::from(bias_abs_max);
    if cross > u128::from(i32_envelope) {
        return BruteVerdict::Refuted(format!(
            "bias pushes cross-chunk worst case to {cross}, past envelope {i32_envelope}"
        ));
    }
    if cross != u128::from(claimed_total_abs_max) {
        return BruteVerdict::Refuted(format!(
            "simulated worst case {cross} != claimed total_abs_max {claimed_total_abs_max}"
        ));
    }
    BruteVerdict::Confirmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::Hash256;
    use gbf_policy::{InferOpTag, ReductionSiteId};
    use gbf_report::report_schemas::f_b6_f_b7_common::AccumulatorDomain;
    use gbf_report::report_schemas::quant_graph_v1::DeterminismClassTag;
    use gbf_report::report_schemas::range_plan_v1::ReductionPlan;

    fn facts(term_count: u32, per_term: u64, bias: Option<u32>) -> ReductionSiteFacts {
        ReductionSiteFacts {
            site: ReductionSiteId("brute.site".to_owned()),
            layer: None,
            expert: None,
            slot: None,
            norm_site: None,
            term_count,
            input_max_abs_q: 1,
            weight_max_abs_q: u32::try_from(per_term).unwrap_or(u32::MAX),
            per_term_abs_max_q: per_term,
            bias_max_abs_q: bias,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            op_tag: InferOpTag::Classify,
        }
    }

    fn single(term_count: u32, per_term: u64, bias: u64, total: u64) -> CertifiedReduction {
        let facts = facts(term_count, per_term, u32::try_from(bias).ok());
        CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::SingleI16,
            facts,
            proof: AccumulatorCertificate::SingleI16Proof {
                site: ReductionSiteId("brute.site".to_owned()),
                term_count: u64::from(term_count),
                per_term_abs_max: per_term,
                sum_bound: u64::from(term_count) * per_term,
                bias_abs_max: bias,
                total_abs_max: total,
                i16_envelope: i16::MAX as u64,
                slack: (i16::MAX as u64).saturating_sub(total),
            },
        }
    }

    #[test]
    fn confirms_valid_single_i16_certificate_and_agrees_with_algebraic_verifier() {
        let certified = single(8, 256, 4, 2_052);
        assert_eq!(worst_case_simulation(&certified), BruteVerdict::Confirmed);
        assert!(crate::range_cert::independent::verifies(
            &certified.proof,
            &certified.plan,
            &certified.facts,
            DeterminismClassTag::BitExact,
        ));
    }

    #[test]
    fn refutes_single_certificate_whose_worst_case_overflows_the_envelope() {
        // 130 terms x 256 = 33,280 > 32,767: partials overflow i16.
        let certified = single(130, 256, 0, 33_280);
        assert!(matches!(
            worst_case_simulation(&certified),
            BruteVerdict::Refuted(_)
        ));
    }

    #[test]
    fn refutes_single_certificate_with_understated_total_abs_max() {
        let certified = single(8, 256, 4, 2_048);
        assert!(matches!(
            worst_case_simulation(&certified),
            BruteVerdict::Refuted(detail) if detail.contains("claimed total_abs_max")
        ));
    }

    #[test]
    fn confirms_valid_chunked_certificate_including_short_final_chunk() {
        let facts = facts(70, 512, Some(7));
        let certified = CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::ChunkedI16 { chunk_len: 32 },
            facts,
            proof: AccumulatorCertificate::ChunkedI16Proof {
                site: ReductionSiteId("brute.site".to_owned()),
                chunk_len: 32,
                chunk_count: 3,
                per_term_abs_max: 512,
                per_chunk_sum_bound: 16_384,
                per_chunk_i16_slack: 16_383,
                cross_chunk_sum_bound: 35_840,
                bias_abs_max: 7,
                total_abs_max: 35_847,
                i32_envelope: i32::MAX as u64,
                slack: (i32::MAX as u64) - 35_847,
            },
        };
        assert_eq!(worst_case_simulation(&certified), BruteVerdict::Confirmed);
    }

    #[test]
    fn refutes_chunked_certificate_whose_chunk_overflows_i16() {
        let facts = facts(64, 1_024, None);
        let certified = CertifiedReduction {
            site: facts.site.clone(),
            plan: ReductionPlan::ChunkedI16 { chunk_len: 64 },
            facts,
            proof: AccumulatorCertificate::ChunkedI16Proof {
                site: ReductionSiteId("brute.site".to_owned()),
                chunk_len: 64,
                chunk_count: 1,
                per_term_abs_max: 1_024,
                per_chunk_sum_bound: 65_536,
                per_chunk_i16_slack: 0,
                cross_chunk_sum_bound: 65_536,
                bias_abs_max: 0,
                total_abs_max: 65_536,
                i32_envelope: i32::MAX as u64,
                slack: (i32::MAX as u64) - 65_536,
            },
        };
        assert!(matches!(
            worst_case_simulation(&certified),
            BruteVerdict::Refuted(detail) if detail.contains("i16 envelope")
        ));
    }
}
