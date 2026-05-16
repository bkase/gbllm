#![allow(dead_code)]

use std::path::{Path, PathBuf};

use gbf_workload::{WorkloadManifest_v0, read_v0_success_workload_manifest};

pub fn fixture_path() -> PathBuf {
    workspace_root().join("fixtures/workloads/v0_success.toml")
}

pub fn fixture_text() -> String {
    std::fs::read_to_string(fixture_path()).expect("v0_success workload fixture reads")
}

pub fn load_v0_success() -> WorkloadManifest_v0 {
    read_v0_success_workload_manifest(fixture_path())
        .expect("v0_success workload fixture validates")
}

pub fn parse_v0_success_unverified() -> WorkloadManifest_v0 {
    toml::from_str(&fixture_text()).expect("v0_success fixture parses without validation")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-workload has workspace parent")
        .to_path_buf()
}
