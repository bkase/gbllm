#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;

use gbf_debug::{ExecArgs, InitArgs, ScriptConfig, run_exec, run_init};

#[test]
fn recipes_run_against_tiny_rom() {
    let recipes = [
        ".agents/skills/gbf-debug-usage/assets/recipes/run_to_entry.js",
        ".agents/skills/gbf-debug-usage/assets/recipes/dump_regs_at_pc.js",
        ".agents/skills/gbf-debug-usage/assets/recipes/memory_watchpoint.js",
        ".agents/skills/gbf-debug-usage/assets/recipes/snapshot_branch.js",
        ".agents/skills/gbf-debug-usage/assets/recipes/trace_io_writes.js",
    ];
    for recipe in recipes {
        let root = workspace_root();
        let dir = tempfile::tempdir().expect("tempdir");
        let s0 = dir.path().join("s0.gbsess");
        let s1 = dir.path().join("s1.gbsess");
        run_init(InitArgs {
            rom_path: root.join("gbf-emu/tests/fixtures/tiny_rom.gb"),
            sym_path: Some(root.join("docs/review/f-a1/artifacts/tiny_rom.sym")),
            out_path: s0.clone(),
            trace_capacity: 32,
            replace_existing_out: false,
        })
        .expect("init");
        let script = dir.path().join("recipe.js");
        fs::copy(root.join(recipe), &script).expect("copy recipe");
        let envelope = run_exec(ExecArgs {
            in_path: s0,
            script_path: script,
            out_path: s1,
            config: ScriptConfig::default(),
            emit_metrics: false,
            write_partial_on_timeout: false,
            replace_existing_out: false,
        })
        .unwrap_or_else(|error| panic!("recipe {recipe} failed: {error}"));
        assert_eq!(envelope.command, "exec");
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
