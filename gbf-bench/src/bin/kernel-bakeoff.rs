//! Emit the kernel bake-off report (bd-rzq5n) as JSON and Markdown.
//!
//! Usage: `cargo run -p gbf-bench --bin kernel-bakeoff -- [out_dir]`
//! Writes `kernel_bakeoff.json` and `kernel_bakeoff.md` to `out_dir`
//! (default `docs/experiments/kernel-bakeoff`).

use gbf_bench::kernel_bakeoff::{report_to_markdown, run_kernel_bakeoff};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args().nth(1).map_or_else(
        || PathBuf::from("docs/experiments/kernel-bakeoff"),
        PathBuf::from,
    );
    let report = run_kernel_bakeoff()?;
    std::fs::create_dir_all(&out_dir)?;
    let json_path = out_dir.join("kernel_bakeoff.json");
    let md_path = out_dir.join("kernel_bakeoff.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    std::fs::write(&md_path, report_to_markdown(&report))?;
    println!("{}", report_to_markdown(&report));
    println!("wrote {} and {}", json_path.display(), md_path.display());
    Ok(())
}
