use std::env;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gbf_verify::range_cert::independent::{IndependentVerifyEvent, independent_verify_path};
use serde_json::json;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        usage()?;
        return Ok(());
    };
    let Some(subcommand) = args.next() else {
        usage()?;
        return Ok(());
    };
    if command != "range-cert" || subcommand != "verify" {
        usage()?;
        return Ok(());
    }

    let Some(cert_path) = args.next().map(PathBuf::from) else {
        usage()?;
        return Ok(());
    };
    let mut ndjson_path = None;
    let mut build_id = String::from("manual-range-cert-verify");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ndjson" => {
                ndjson_path = args.next().map(PathBuf::from);
            }
            "--build-id" => {
                if let Some(value) = args.next() {
                    build_id = value;
                }
            }
            _ => {
                return Err(format!("unknown argument: {arg}").into());
            }
        }
    }

    match independent_verify_path(&cert_path) {
        Ok(report) if report.is_verified() => {
            if let Some(path) = ndjson_path {
                write_events(&path, &build_id, &cert_path, &report.events)?;
            }
            println!(
                "range certificate verified: cert={} certificates={}",
                cert_path.display(),
                report.certificate_count
            );
            Ok(())
        }
        Ok(report) => {
            if let Some(path) = ndjson_path {
                write_events(&path, &build_id, &cert_path, &report.events)?;
            }
            Err(format!(
                "range certificate contains failed evidence: cert={} failed_evidence={}",
                cert_path.display(),
                report.failed_evidence_count
            )
            .into())
        }
        Err(error) => {
            if let Some(path) = ndjson_path {
                let events = if error.events().is_empty() {
                    vec![IndependentVerifyEvent {
                        event: error.event(),
                        site: None,
                        ok: false,
                        detail: Some(error.to_string()),
                    }]
                } else {
                    error.events().to_vec()
                };
                write_events(&path, &build_id, &cert_path, &events)?;
            }
            Err(Box::new(error))
        }
    }
}

fn usage() -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "usage: gbf-verify range-cert verify <cert-path> [--ndjson <path>] [--build-id <id>]"
            .into(),
    )
}

fn write_events(
    path: &Path,
    build_id: &str,
    cert_path: &Path,
    events: &[IndependentVerifyEvent],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for (index, event) in events.iter().enumerate() {
        let seq = index + 1;
        let payload = json!({
            "ts": format!("unix:{:.9}", unix_timestamp()),
            "event": event.event,
            "level": if event.ok { "INFO" } else { "ERROR" },
            "target": "gbf_verify::range_cert",
            "fields": {
                "site_id": event.site.as_deref().unwrap_or("not-applicable:range-cert"),
                "checkpoint_id": "not-applicable:range-cert",
                "compact_checkpoint_id": 0,
                "stratum": "not-applicable:range-cert",
                "probe_instance_id": "not-applicable:range-cert",
                "runtime_probe_id": 0,
                "importance_class": "not-applicable:range-cert",
                "build_id": build_id,
                "k4_hash": "not-applicable:range-cert",
                "k5_hash": "not-applicable:range-cert",
                "outcome": if event.ok { "passed" } else { "failed" },
                "diag_code": event.detail.as_deref().unwrap_or("none"),
                "elapsed_ns": seq,
                "event_seq": seq,
                "cert_path": cert_path.display().to_string(),
                "substrate_note": "independent gbf-verify range certificate verifier",
            },
            "span": null,
        });
        writeln!(file, "{}", serde_json::to_string(&payload)?)?;
    }
    Ok(())
}

fn unix_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}
