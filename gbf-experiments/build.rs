use std::env;
use std::process::Command;

fn main() {
    validate_s1_build_selection();

    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../gbf-experiments");
    println!("cargo:rerun-if-changed=../gbf-train");
    // `GBF_RUSTC_VERSION` is part of the S2 environment hash. Cargo reruns
    // build scripts for package inputs; these env triggers cover the common
    // toolchain-selection knobs, and the file triggers cover future pinned
    // rust-toolchain manifests. Plain `rustup default` changes still require
    // a fresh Cargo invocation; this script records the rustc used by that
    // invocation.
    println!("cargo:rerun-if-changed=../rust-toolchain.toml");
    println!("cargo:rerun-if-changed=../rust-toolchain");
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
    println!("cargo:rerun-if-env-changed=RUSTUP_TOOLCHAIN");

    let repo_sha = git_output(&["rev-parse", "HEAD"]).unwrap_or_else(|| "UNKNOWN".to_owned());
    let rustc_version = command_output("rustc", &["--version"])
        .unwrap_or_else(|| format!("rustc-msrv-{}", env!("CARGO_PKG_RUST_VERSION")));
    println!("cargo:rustc-env=GBF_EXPERIMENTS_GIT_SHA={repo_sha}");
    println!("cargo:rustc-env=GBF_TRAIN_GIT_SHA={repo_sha}");
    println!("cargo:rustc-env=GBF_RUSTC_VERSION={rustc_version}");
}

fn validate_s1_build_selection() {
    let phase_a = env::var_os("CARGO_FEATURE_PHASE_A").is_some();
    let ablation = env::var_os("CARGO_FEATURE_ABLATION").is_some();
    let s2_full = env::var_os("CARGO_FEATURE_S2_FULL").is_some();
    let s2_ablation = env::var_os("CARGO_FEATURE_S2_ABLATION").is_some();

    if s2_full && s2_ablation {
        panic!("S2 feature mutex violated");
    }

    match (phase_a, ablation) {
        (true, false) | (false, true) => {}
        (true, true) => {
            panic!("gbf-experiments features phase-a and ablation are mutually exclusive");
        }
        (false, false) => {
            if !s2_full && !s2_ablation {
                panic!("gbf-experiments requires at least one S1 or S2 experiment feature");
            }
        }
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    command_output("git", args)
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
