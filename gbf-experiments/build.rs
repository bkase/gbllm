use std::env;
use std::process::Command;

fn main() {
    validate_s1_build_selection();

    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../gbf-experiments");
    println!("cargo:rerun-if-changed=../gbf-train");

    let repo_sha = git_output(&["rev-parse", "HEAD"]).unwrap_or_else(|| "UNKNOWN".to_owned());
    println!("cargo:rustc-env=GBF_EXPERIMENTS_GIT_SHA={repo_sha}");
    println!("cargo:rustc-env=GBF_TRAIN_GIT_SHA={repo_sha}");
}

fn validate_s1_build_selection() {
    let phase_a = env::var_os("CARGO_FEATURE_PHASE_A").is_some();
    let ablation = env::var_os("CARGO_FEATURE_ABLATION").is_some();

    match (phase_a, ablation) {
        (true, false) | (false, true) => {}
        (true, true) => {
            panic!("gbf-experiments features phase-a and ablation are mutually exclusive");
        }
        (false, false) => {
            panic!("gbf-experiments requires exactly one of phase-a or ablation");
        }
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
