//! Down-projection delta magnitude measurement on the REAL d192 checkpoint
//! (bd-2vkqt): records the **unclamped** delta magnitude distribution over
//! the committed val pair set so the delta carrier width is chosen from
//! data and structural weight bounds, not guesses.
//!
//! Usage: `cargo run --release -p gbf-bench --bin d192-down-delta [-- <positions-per-lane>]`
//! (default 0 = the full committed pair set). Prints the
//! `down_delta_probe.v1` report as JSON on stdout.

use std::path::PathBuf;

use gbf_bench::d192_real::run_down_delta_probe;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let positions_per_lane: usize = std::env::args()
        .nth(1)
        .map(|a| a.parse())
        .transpose()?
        .unwrap_or(0);

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    eprintln!(
        "probing down-delta magnitudes on the real d192 checkpoint ({} positions/lane; 0 = full)...",
        positions_per_lane
    );
    let report = run_down_delta_probe(&repo_root, positions_per_lane)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
