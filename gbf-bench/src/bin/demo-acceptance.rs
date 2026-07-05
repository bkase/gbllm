//! End-to-end demo acceptance evidence runner (bd-do7sq): boot the deployed
//! d192 shell ROM, type the prompt via injected joypad frames, START,
//! sustained sampled generation — screenshots, transcript, determinism
//! proof, host byte-identity, and the honest quality section, written to
//! `docs/experiments/demo-acceptance/`.
//!
//! Usage: `cargo run --release -p gbf-bench --bin demo-acceptance [-- quick]`
//! (`quick` runs development smoke sizes, marks the report as such, and
//! writes to a temp directory instead).

use std::fs;
use std::path::PathBuf;

use gbf_bench::demo::{demo_report_to_markdown, run_demo_acceptance};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let quick = std::env::args().any(|a| a == "quick");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let out_dir = if quick {
        std::env::temp_dir().join("gbf-demo-acceptance-quick")
    } else {
        repo_root.join("docs/experiments/demo-acceptance")
    };
    fs::create_dir_all(&out_dir)?;

    eprintln!(
        "running demo acceptance ({} mode; two full scripted shell sessions)...",
        if quick { "quick" } else { "full" }
    );
    let report = run_demo_acceptance(&repo_root, quick)?;

    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(out_dir.join("README.md"), demo_report_to_markdown(&report))?;
    for (name, bytes) in &report.screenshot_pgms {
        fs::write(out_dir.join(name), bytes)?;
    }
    fs::write(out_dir.join(report.transcript_file), &report.transcript)?;

    for g in &report.gates {
        eprintln!(
            "  {:80} {}",
            g.gate,
            if g.pass { "PASS" } else { "OPEN/FAIL" }
        );
    }
    eprintln!("evidence: {}", out_dir.display());
    if !report.all_demo_gates_pass {
        return Err("demo acceptance scripted gates FAILED (see report)".into());
    }
    Ok(())
}
