//! `gbf compile` subcommand surface tests (bd-1skgm): the CLI must drive the
//! full gbf-codegen pipeline from a checkpoint-export directory to `rom.gb`
//! + `build_report.json`, and must fail loudly on bad inputs.

#![cfg(feature = "compile")]

use assert_cmd::Command;
use gbf_codegen::import_checkpoint_export::write_synthetic_checkpoint_export;
use predicates::prelude::*;

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary builds")
}

#[test]
fn compile_writes_rom_and_build_report() {
    let export = tempfile::tempdir().expect("export tempdir");
    write_synthetic_checkpoint_export(export.path(), 31).expect("writes export");
    let out = tempfile::tempdir().expect("out tempdir");

    gbf()
        .args([
            "compile",
            "--checkpoint-export",
            export.path().to_str().expect("utf8 path"),
            "--out",
            out.path().to_str().expect("utf8 path"),
            "--tokens",
            "16",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("compiled"))
        .stdout(predicate::str::contains("build_report.json"));

    let rom = std::fs::read(out.path().join("rom.gb")).expect("rom.gb exists");
    assert!(!rom.is_empty());
    // MBC5 cartridge type byte, as the banked model backend emits.
    assert_eq!(rom[0x0147], 0x19);

    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(out.path().join("build_report.json")).expect("build_report.json exists"),
    )
    .expect("report parses");
    assert_eq!(report["schema"], "gbf_compile_build_report.v1");
    assert_eq!(report["rom"]["n_tokens"], 16);
    assert_eq!(report["program"]["n_blocks"], 4);
}

#[test]
fn compile_fails_on_missing_export_dir() {
    let out = tempfile::tempdir().expect("out tempdir");
    gbf()
        .args([
            "compile",
            "--checkpoint-export",
            "/nonexistent/gbf-export-dir",
            "--out",
            out.path().to_str().expect("utf8 path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("import"));
}

#[test]
fn compile_rejects_out_of_range_token_counts() {
    let export = tempfile::tempdir().expect("export tempdir");
    write_synthetic_checkpoint_export(export.path(), 31).expect("writes export");
    let out = tempfile::tempdir().expect("out tempdir");
    gbf()
        .args([
            "compile",
            "--checkpoint-export",
            export.path().to_str().expect("utf8 path"),
            "--out",
            out.path().to_str().expect("utf8 path"),
            "--tokens",
            "300",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("1..=256"));
}
