#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use gbf_debug::{
    CliError, ExecArgs, InitArgs, InspectArgs, ScriptConfig, run_exec, run_init, run_inspect,
};

#[test]
fn init_exec_inspect_tiny_rom() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s0 = dir.path().join("s0.gbsess");
    let s1 = dir.path().join("s1.gbsess");
    let script = dir.path().join("script.js");
    fs::write(
        &script,
        r#"
        gb.step(1);
        log("after-step", { pc: gb.regs.pc });
        globalThis.result = { pc: gb.regs.pc, entry: gb.symbol("gbf_runtime_dtiny_dentry") };
        "#,
    )
    .expect("write script");

    let root = workspace_root();
    let init = run_init(InitArgs {
        rom_path: root.join("gbf-emu/tests/fixtures/tiny_rom.gb"),
        sym_path: Some(root.join("docs/review/f-a1/artifacts/tiny_rom.sym")),
        out_path: s0.clone(),
        trace_capacity: 16,
        replace_existing_out: false,
    })
    .expect("init");
    assert_eq!(init.command, "init");
    assert!(s0.exists());

    let exec = run_exec(ExecArgs {
        in_path: s0,
        script_path: script,
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("exec");
    assert_eq!(exec.command, "exec");
    assert_eq!(exec.logs.len(), 1);
    assert!(exec.metrics.is_none());

    let inspect = run_inspect(InspectArgs { in_path: s1 }).expect("inspect");
    assert_eq!(inspect.command, "inspect");
    assert_eq!(inspect.schema_version, gbf_debug::SCHEMA_VERSION);
    assert!(inspect.symbols_summary.count > 0);
}

#[test]
fn string_predicate_round_trips_without_becoming_unconditional() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s0 = dir.path().join("s0.gbsess");
    let s1 = dir.path().join("s1.gbsess");
    let s2 = dir.path().join("s2.gbsess");
    init_tiny_session(&s0, 16);

    let add_script = dir.path().join("add.js");
    fs::write(
        &add_script,
        r#"
        gb.add_breakpoint(gb.regs.pc, "regs.a === 0xff");
        globalThis.result = { breakpoints: gb.list_breakpoints() };
        "#,
    )
    .expect("write add script");
    let add = run_exec(ExecArgs {
        in_path: s0,
        script_path: add_script,
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("add breakpoint");
    assert_eq!(add.warnings, Vec::new());

    let run_script = dir.path().join("run.js");
    fs::write(
        &run_script,
        r#"
        globalThis.result = gb.run_until_breakpoint(8);
        "#,
    )
    .expect("write run script");
    let run = run_exec(ExecArgs {
        in_path: s1,
        script_path: run_script,
        out_path: s2,
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("run breakpoint");
    assert_ne!(run.result["reason"], "breakpoint");
}

#[test]
fn closure_predicate_is_invocation_local_and_warned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s0 = dir.path().join("s0.gbsess");
    let s1 = dir.path().join("s1.gbsess");
    init_tiny_session(&s0, 16);

    let script = dir.path().join("closure.js");
    fs::write(
        &script,
        r#"
        const at = gb.regs.pc;
        gb.add_breakpoint(at, () => gb.regs.pc === regs.pc && regs.pc === pc);
        globalThis.result = gb.run_until_breakpoint(1);
        "#,
    )
    .expect("write closure script");
    let exec = run_exec(ExecArgs {
        in_path: s0,
        script_path: script,
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("exec");
    assert!(
        exec.warnings
            .iter()
            .any(|warning| warning.kind == "predicate_not_persisted")
    );
    assert_eq!(exec.result["reason"], "breakpoint");

    let inspect = run_inspect(InspectArgs { in_path: s1 }).expect("inspect");
    assert!(inspect.breakpoints.is_empty());
}

#[test]
fn deterministic_script_errors_write_normal_error_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s0 = dir.path().join("s0.gbsess");
    let s1 = dir.path().join("s1.gbsess");
    init_tiny_session(&s0, 16);

    let script = dir.path().join("boom.js");
    fs::write(
        &script,
        r#"
        gb.step(1);
        throw new Error("boom");
        "#,
    )
    .expect("write boom script");
    let error = run_exec(ExecArgs {
        in_path: s0,
        script_path: script,
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect_err("script should fail");
    let CliError::ScriptError {
        session_path,
        partial_session_path,
        partial_session_sha256,
        determinism,
        ..
    } = error
    else {
        panic!("expected script error");
    };
    assert_eq!(session_path, Some(s1.to_string_lossy().into_owned()));
    assert!(partial_session_path.is_none());
    assert!(partial_session_sha256.is_none());
    assert!(determinism.is_none());
    assert!(s1.exists());
}

#[test]
fn date_now_and_log_timestamps_are_relative_to_each_exec() {
    let dir = tempfile::tempdir().expect("tempdir");
    let s0 = dir.path().join("s0.gbsess");
    let s1 = dir.path().join("s1.gbsess");
    let s2 = dir.path().join("s2.gbsess");
    init_tiny_session(&s0, 16);

    let advance_script = dir.path().join("advance.js");
    fs::write(
        &advance_script,
        r#"
        gb.step(1);
        globalThis.result = { now: Date.now() };
        "#,
    )
    .expect("write advance script");
    run_exec(ExecArgs {
        in_path: s0,
        script_path: advance_script,
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("advance");

    let observe_script = dir.path().join("observe.js");
    fs::write(
        &observe_script,
        r#"
        log("start");
        globalThis.result = { now: Date.now() };
        "#,
    )
    .expect("write observe script");
    let observe = run_exec(ExecArgs {
        in_path: s1,
        script_path: observe_script,
        out_path: s2,
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: false,
        replace_existing_out: false,
    })
    .expect("observe");
    assert_eq!(observe.result["now"], 0);
    assert_eq!(observe.logs[0].ts_micros_since_script_start, 0);
}

#[test]
fn cli_arg_errors_are_json_exit_one() {
    let output = Command::new(env!("CARGO_BIN_EXE_gbf-debug"))
        .args(["exec", "--bad"])
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stderr).expect("stderr json");
    assert_eq!(json["command"], "args");
    assert_eq!(json["kind"], "cli_args");
}

#[test]
fn cli_help_is_json_exit_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_gbf-debug"))
        .arg("--help")
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(json["command"], "args");
    assert_eq!(json["kind"], "help");
}

fn init_tiny_session(path: &PathBuf, trace_capacity: u32) {
    let root = workspace_root();
    run_init(InitArgs {
        rom_path: root.join("gbf-emu/tests/fixtures/tiny_rom.gb"),
        sym_path: Some(root.join("docs/review/f-a1/artifacts/tiny_rom.sym")),
        out_path: path.clone(),
        trace_capacity,
        replace_existing_out: false,
    })
    .expect("init");
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
