//! Fixed-point MoE router parity gate (deploy step 3, `router-fx.v1`,
//! `docs/design/integer-moe-deploy.md`).
//!
//! The deployed integer forward routes purely-integer via `FixedRouter` (no
//! f32 enters the forward). This gate proves that, on the REAL bridged d192x8
//! subword MoE student, the integer router's top-1 argmax agrees with the f32
//! `LowRankRouter::route_f32` reference at EVERY block and EVERY position over
//! a greedy generation stream: **0 divergences required** (per the design, if
//! this ever failed the fixed-point width would be widened and re-derived, not
//! the gate relaxed).
//!
//! Env-gated like `moe_int_eval` / the real-geometry `moe_parity` test: point
//! `MOE_PARITY_DIR` (or `MOE_INT_DIR`) at a directory containing a `ckpt/`
//! subdirectory with the `f_s8_moe_state_checkpoint_export.v2` manifest +
//! `tensors/` (re-bridge with
//! `cd training && uv run python run_realparity.py --ckpt
//! artifacts/student_moe_d192x8 --out /private/tmp/claude-501/parity_moe
//! --tokens 64`). Skips with an eprintln when absent (like
//! `d192_generation_regression` on a fresh clone).

use std::path::PathBuf;

use gbf_bench::moe_router_gate::run_router_fixed_point_gate;
use gbf_bench::stateful::load_state_checkpoint;
use gbf_kernel::state_model_ref::IntStateLoweredModel;

fn moe_dir() -> Option<PathBuf> {
    std::env::var("MOE_PARITY_DIR")
        .or_else(|_| std::env::var("MOE_INT_DIR"))
        .ok()
        .map(PathBuf::from)
}

#[test]
fn real_student_fixed_point_router_agrees_with_f32_zero_divergences() {
    let Some(root) = moe_dir() else {
        eprintln!(
            "MOE_PARITY_DIR / MOE_INT_DIR unset; skipping fixed-point router gate \
             (re-bridge via training/run_realparity.py, see the module docs)"
        );
        return;
    };
    let ckpt = root.join("ckpt");
    if !ckpt.join("manifest.json").exists() {
        eprintln!(
            "bridged MoE student missing at {}; skipping fixed-point router gate",
            ckpt.display()
        );
        return;
    }

    let bundle = load_state_checkpoint(&ckpt)
        .unwrap_or_else(|e| panic!("load real MoE student at {}: {e}", ckpt.display()));
    let topo = bundle.topology;
    assert!(
        topo.is_moe(),
        "expected a MoE checkpoint (n_experts = {})",
        topo.n_experts
    );

    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .unwrap_or_else(|e| panic!("lower real MoE student (fixed router builds here): {e}"));

    // Greedy stream from a fixed seed, threading the full argmax id back in.
    let seed = 3usize.min(topo.vocab - 1);
    let n_positions = 64usize;
    let report = run_router_fixed_point_gate(&lowered, seed, n_positions);

    println!(
        "router-fx.v1 gate: {} comparisons ({} positions x {} MoE blocks), \
         min f32 margin = {:.6e} at (pos {}, block {}); divergences = {}",
        report.comparisons,
        report.positions,
        report.n_moe_blocks,
        report.min_f32_margin,
        report.min_f32_margin_at.0,
        report.min_f32_margin_at.1,
        report.divergences.len(),
    );

    assert_eq!(
        report.comparisons,
        n_positions * topo.n_blocks,
        "expected exactly n_positions * n_blocks router comparisons"
    );
    assert_eq!(report.n_moe_blocks, topo.n_blocks, "every block is MoE");

    if !report.zero_divergences() {
        for d in &report.divergences {
            eprintln!(
                "  DIVERGENCE pos {} block {}: f32 -> expert {} (margin {:.6e}), \
                 fixed -> expert {} (margin_q32 {})",
                d.position, d.block, d.f32_expert, d.f32_margin, d.fixed_expert, d.fixed_margin_q32,
            );
        }
    }
    assert!(
        report.zero_divergences(),
        "fixed-point router-fx.v1 diverged from the f32 router in {} of {} comparisons \
         on the real d192x8 student (widen the fixed-point width and re-derive per the design; \
         do NOT relax this gate)",
        report.divergences.len(),
        report.comparisons,
    );
}
