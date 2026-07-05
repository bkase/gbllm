//! Regression gate (bd-pp43d): the **committed real arm-B checkpoint**
//! (`experiments/S5/state-ab/checkpoint-export`, manifest
//! `f_s5_state_checkpoint_export.v1`) must still load through the new
//! topology-driven loader, lower with i16 down accumulators, and reproduce
//! the host integer evaluator byte-exactly on the parameterized ROM
//! builders. Read-only: nothing under `experiments/` is written.

use std::path::PathBuf;

use gbf_bench::stateful::{load_state_checkpoint, run_state_rom_gate};
use gbf_kernel::state_model_ref::{AccWidth, IntStateLoweredModel, StateTopology};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn committed_arm_b_export_loads_and_matches_the_rom_byte_exactly() {
    let export_dir = repo_root().join("experiments/S5/state-ab/checkpoint-export");
    if !export_dir.join("manifest.json").exists() {
        eprintln!("committed arm-B export missing; skipping (fresh clone without experiments)");
        return;
    }
    let bundle = load_state_checkpoint(&export_dir).expect("loads via manifest topology");
    assert_eq!(bundle.topology, StateTopology::ARM_B);
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint).expect("lowers");
    assert_eq!(
        lowered.down_width,
        AccWidth::I16,
        "arm-B fan-in 128 keeps i16 down accumulators"
    );

    // Zero state plus a carried nonzero state (deterministic warmup).
    let mut state = lowered.zero_state();
    let t0 = lowered.forward(19, &mut state);
    let _ = lowered.forward(t0.argmax, &mut state);
    let cases = vec![
        (0usize, 19u8, lowered.zero_state()),
        (2usize, t0.argmax, state),
    ];
    let report = run_state_rom_gate(&lowered, &cases).expect("gate runs");
    assert!(
        report.all_byte_exact,
        "real arm-B checkpoint diverged on the parameterized builders: {:?}",
        report
            .runs
            .iter()
            .flat_map(|r| r.mismatches.iter())
            .collect::<Vec<_>>()
    );
}
