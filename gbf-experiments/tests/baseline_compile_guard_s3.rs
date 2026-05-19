#![cfg(feature = "s3")]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn baseline_prob_cannot_be_constructed_from_f32() {
    let outputs = rustc_probe_outputs();
    assert!(
        outputs.iter().any(|output| {
            !output.status.success() && has_expected_f32_diagnostic(&command_output(output))
        }),
        "f32 probe did not fail with the expected type diagnostic:\n{}",
        outputs
            .iter()
            .map(command_output)
            .collect::<Vec<_>>()
            .join("\n---\n")
    );
}

fn rustc_probe_outputs() -> Vec<Output> {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s3-baseline-prob-probe-")
        .tempdir()
        .expect("probe tempdir");
    let source = tempdir.path().join("probe.rs");
    fs::write(
        &source,
        r#"
use gbf_experiments::s3::baseline::BaselineProb;

fn main() {
    let _ = BaselineProb::try_new(0.5_f32);
}
"#,
    )
    .expect("probe main");

    let deps_dir = cargo_target_dir().join("debug/deps");
    let mut candidates = gbf_experiments_rlibs(&deps_dir);
    candidates.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    candidates.reverse();

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    candidates
        .into_iter()
        .map(|candidate| {
            Command::new(&rustc)
                .arg("--edition=2024")
                .arg("--crate-name")
                .arg("gbf_s3_baseline_prob_probe")
                .arg("--crate-type")
                .arg("bin")
                .arg(&source)
                .arg("-L")
                .arg(format!("dependency={}", deps_dir.display()))
                .arg("--extern")
                .arg(format!("gbf_experiments={}", candidate.display()))
                .arg("-o")
                .arg(tempdir.path().join("probe"))
                .output()
                .expect("rustc probe runs")
        })
        .collect()
}

fn gbf_experiments_rlibs(deps_dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(deps_dir)
        .expect("target debug deps dir reads")
        .filter_map(|entry| {
            let path = entry.expect("target dir entry reads").path();
            let name = path.file_name()?.to_str()?;
            (name.starts_with("libgbf_experiments-") && name.ends_with(".rlib")).then_some(path)
        })
        .collect()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments lives under workspace")
        .to_path_buf()
}

fn cargo_target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
}

fn has_expected_f32_diagnostic(combined: &str) -> bool {
    combined.contains("expected `f64`, found `f32`")
        || combined.contains("the trait bound")
        || combined.contains("mismatched types")
}

fn command_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
