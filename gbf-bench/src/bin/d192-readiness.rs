//! D192 readiness evidence generator (bd-pp43d): synthetic
//! d192/ff384/6blk/slots192 export -> production loader -> banked ROM ->
//! byte-exact emulator gates -> `docs/experiments/d192-readiness/`.
//!
//! Usage: `cargo run --release -p gbf-bench --bin d192-readiness`

use std::fs;
use std::path::PathBuf;

use gbf_bench::d192::{
    d192_report_to_markdown, run_d192_readiness, run_d192_v2_gate, run_d256_v2_gate,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let out_dir = repo_root.join("docs/experiments/d192-readiness");
    fs::create_dir_all(&out_dir)?;
    let work_dir = std::env::temp_dir().join("gbf-d192-readiness");
    let _ = fs::remove_dir_all(&work_dir);

    eprintln!("running d192 readiness gates (synthetic export; emulated ROM runs)...");
    let report = run_d192_readiness(&work_dir)?;

    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(out_dir.join("README.md"), d192_report_to_markdown(&report))?;

    // V2 dispatch-lowering byte-exact gate on the same synthetic d192 model
    // (bd-1tuql, docs/design/v2-dispatch-stateful.md).
    eprintln!("running V2 dispatch-lowering byte-exact gate...");
    let v2 = run_d192_v2_gate(&work_dir)?;
    fs::write(
        out_dir.join("v2_gate.json"),
        serde_json::to_vec_pretty(&v2)?,
    )?;
    let _ = fs::remove_dir_all(&work_dir);

    // Step 4: d256-class fit + byte-exact gate — a model V3 weights-as-code
    // cannot build (> 512 banks) but V2 dispatch packs into ~45 banks / 1 MiB.
    eprintln!("running d256-class V2 fit + byte-exact gate (slow: full-model emulation)...");
    let d256 = run_d256_v2_gate()?;
    fs::write(
        out_dir.join("d256_v2_gate.json"),
        serde_json::to_vec_pretty(&d256)?,
    )?;

    let base_pass = report.one_token_gate.all_byte_exact
        && report.multi_token.all_sequences_match
        && report.multi_token.all_health_checks_pass;
    eprintln!(
        "d192 readiness (V3): one-token {} | multi-token sequences {} | health {} | {:.3} s/token",
        report.one_token_gate.all_byte_exact,
        report.multi_token.all_sequences_match,
        report.multi_token.all_health_checks_pass,
        report.cycles.seconds_per_token_dmg
    );
    eprintln!(
        "d192 V2 dispatch: one-token {} | generation {} | checkpoints {} | \
         weight banks {}->{} ({:.1}x denser) | driver {} B (window 0x150..0x4000)",
        v2.one_token_byte_exact,
        v2.multi_token_sequences_match,
        v2.multi_token_checkpoints_byte_exact,
        v2.v3_weight_banks,
        v2.v2_weight_banks,
        v2.density_ratio(),
        v2.v2_driver_bytes,
    );
    eprintln!(
        "d256-class V2 ({}d/{}ff/{}blk, {} FFN weights): one-token {} | generation {} | \
         checkpoints {} | V3 needs {} banks (>512 = unbuildable) | V2 {} banks / {:.2} MiB | \
         driver {}/{} B (one/multi, window 0x150..0x4000)",
        d256.d_model,
        d256.d_ff,
        d256.n_blocks,
        d256.ffn_weights,
        d256.one_token_byte_exact,
        d256.multi_token_sequences_match,
        d256.multi_token_checkpoints_byte_exact,
        d256.v3_banks_needed,
        d256.v2_bank_count,
        d256.v2_rom_mib,
        d256.v2_driver_bytes,
        d256.v2_multi_driver_bytes,
    );
    eprintln!("evidence: {}", out_dir.display());
    if !base_pass {
        return Err("d192 readiness gates FAILED".into());
    }
    if !v2.pass() {
        return Err("d192 V2 dispatch byte-exact gate FAILED".into());
    }
    if !d256.pass() {
        return Err("d256-class V2 fit + byte-exact gate FAILED".into());
    }
    Ok(())
}
