use std::fs;
use std::path::{Path, PathBuf};

use gbf_foundation::Hash256;
use gbf_policy::ReductionSiteId;
use gbf_report::report_schemas::range_plan_v1::{
    AccumulatorCertificate, AccumulatorFailureWitness, AccumulatorProofState, CertOutcome,
    RangeCertBody, ReductionPlan,
};
use gbf_report::{ReportEnvelope, ReportOutcome};
use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: f_b6_f_b7_tamper_fixtures <source-cert> <out-dir>")?;
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: f_b6_f_b7_tamper_fixtures <source-cert> <out-dir>")?;
    if args.next().is_some() {
        return Err("usage: f_b6_f_b7_tamper_fixtures <source-cert> <out-dir>".into());
    }

    let source_bytes = fs::read(&source)?;
    let source_report: ReportEnvelope<RangeCertBody> = serde_json::from_slice(&source_bytes)?;

    write_raw(
        &out_dir,
        "malformed_json",
        b"{ not valid range certificate json\n",
    )?;
    write_report_self_hash_mismatch(&out_dir, &source_bytes)?;
    write_unsupported_plan_family(&out_dir, &source_bytes)?;
    write_lowered_slack(&out_dir, source_report.clone())?;
    write_wrong_plan_family(&out_dir, source_report.clone())?;
    write_inconsistent_term_count(&out_dir, source_report.clone())?;
    write_failed_witness_mismatch(&out_dir, source_report)?;

    Ok(())
}

fn write_report_self_hash_mismatch(
    out_dir: &Path,
    source_bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_slice(source_bytes)?;
    value["report_self_hash"] = Value::String(Hash256::ZERO.to_string());
    write_value(out_dir, "report_self_hash_mismatch", value)
}

fn write_unsupported_plan_family(
    out_dir: &Path,
    source_bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_slice(source_bytes)?;
    value["certificates"][0]["plan"]["kind"] = Value::String("UnsupportedPlanFamilyV99".into());
    write_value(out_dir, "unsupported_plan_family", value)
}

fn write_lowered_slack(
    out_dir: &Path,
    mut report: ReportEnvelope<RangeCertBody>,
) -> Result<(), Box<dyn std::error::Error>> {
    let certified = first_cert_mut(&mut report)?;
    match &mut certified.proof {
        AccumulatorCertificate::ChunkedI16Proof { slack, .. }
        | AccumulatorCertificate::SingleI16Proof { slack, .. }
        | AccumulatorCertificate::RenormLoopProof { slack, .. } => {
            *slack = slack.saturating_sub(1);
        }
        AccumulatorCertificate::Failed { .. } => {
            return Err("source certificate unexpectedly contains failed evidence".into());
        }
    }
    write_rehashed(out_dir, "cert_lowered_slack", report)
}

fn write_wrong_plan_family(
    out_dir: &Path,
    mut report: ReportEnvelope<RangeCertBody>,
) -> Result<(), Box<dyn std::error::Error>> {
    first_cert_mut(&mut report)?.plan = ReductionPlan::SingleI16;
    write_rehashed(out_dir, "cert_wrong_plan_family", report)
}

fn write_inconsistent_term_count(
    out_dir: &Path,
    mut report: ReportEnvelope<RangeCertBody>,
) -> Result<(), Box<dyn std::error::Error>> {
    let certified = first_cert_mut(&mut report)?;
    certified.facts.term_count = certified.facts.term_count.saturating_add(1);
    write_rehashed(out_dir, "cert_inconsistent_term_count", report)
}

fn write_failed_witness_mismatch(
    out_dir: &Path,
    mut report: ReportEnvelope<RangeCertBody>,
) -> Result<(), Box<dyn std::error::Error>> {
    report.outcome = ReportOutcome::Failed;
    report.body.cert_outcome = CertOutcome::Failed;
    let certified = first_cert_mut(&mut report)?;
    let site: ReductionSiteId = certified.site.clone();
    let facts = certified.facts.clone();
    certified.plan = ReductionPlan::SingleI16;
    certified.proof = AccumulatorCertificate::Failed {
        site,
        attempted_plan: ReductionPlan::SingleI16,
        proof_state: AccumulatorProofState::SumExceedsI16Envelope {
            sum_bound: 40_000,
            envelope: i16::MAX as u64,
        },
        witness: AccumulatorFailureWitness::BoundCalculation {
            input_max_abs_q: facts.input_max_abs_q,
            weight_max_abs_q: facts.weight_max_abs_q,
            term_count: facts.term_count.saturating_add(1),
            bias: facts.bias_max_abs_q.unwrap_or(0),
        },
    };
    write_rehashed(out_dir, "cert_failed_witness_mismatch", report)
}

fn first_cert_mut(
    report: &mut ReportEnvelope<RangeCertBody>,
) -> Result<
    &mut gbf_report::report_schemas::range_plan_v1::CertifiedReduction,
    Box<dyn std::error::Error>,
> {
    report
        .body
        .certificates
        .first_mut()
        .ok_or_else(|| "source certificate has no certified reductions".into())
}

fn write_rehashed(
    out_dir: &Path,
    name: &str,
    report: ReportEnvelope<RangeCertBody>,
) -> Result<(), Box<dyn std::error::Error>> {
    let report = report.with_computed_self_hash()?;
    let bytes = serde_json::to_vec(&report)?;
    write_raw(out_dir, name, &bytes)
}

fn write_value(out_dir: &Path, name: &str, value: Value) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(&value)?;
    write_raw(out_dir, name, &bytes)
}

fn write_raw(out_dir: &Path, name: &str, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = out_dir.join("tampered").join(name);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("range.cert.json"), bytes)?;
    Ok(())
}
