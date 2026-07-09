//! Host integer MoE evaluator on the REAL bridged subword student
//! (deploy step 1, docs/design/integer-moe-deploy.md).
//!
//! Env-gated exactly like the real-geometry `moe_parity` external test: set
//! `MOE_INT_DIR` (or reuse `MOE_PARITY_DIR`) to a directory that contains a
//! `ckpt/` subdirectory with the `f_s8_moe_state_checkpoint_export.v2`
//! manifest + `tensors/` (produced by `training/run_realparity.py`). The test
//! loads that MoE student through the production `load_state_checkpoint`
//! loader, lowers it to the canonical integer evaluator, and runs the integer
//! MoE forward for N tokens.
//!
//! This step's contract is narrow: prove the real student LOADS (router f32
//! tensors + per-expert ternary up/down + MoE topology) and the integer MoE
//! forward RUNS deterministically without panicking. Byte-exact-vs-ROM
//! agreement is a later deploy step; here we only exercise the router dispatch
//! and the reused integer FFN kernel on real weights.

use std::path::PathBuf;

use gbf_bench::stateful::load_state_checkpoint;
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LoweredBlockFfn};

fn moe_int_dir() -> Option<PathBuf> {
    std::env::var("MOE_INT_DIR")
        .or_else(|_| std::env::var("MOE_PARITY_DIR"))
        .ok()
        .map(PathBuf::from)
}

/// Run `n` greedy tokens from `seed`, threading the FULL argmax id back in
/// (subword V=1024 exceeds the u8 charset space). Returns the produced id
/// sequence.
fn generate(lowered: &IntStateLoweredModel, seed: usize, n: usize) -> Vec<usize> {
    let mut state = lowered.zero_state();
    let mut input = seed;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let trace = lowered.forward_at(input, &mut state);
        input = trace.argmax_full;
        out.push(trace.argmax_full);
    }
    out
}

#[test]
#[ignore = "requires MOE_INT_DIR / MOE_PARITY_DIR pointing at a real bridged MoE student"]
fn real_moe_student_loads_and_forwards_deterministically() {
    let Some(root) = moe_int_dir() else {
        eprintln!("MOE_INT_DIR / MOE_PARITY_DIR unset; skipping");
        return;
    };
    let ckpt = root.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)
        .unwrap_or_else(|e| panic!("load real MoE student at {}: {e}", ckpt.display()));

    let topo = bundle.topology;
    assert!(
        topo.is_moe(),
        "expected a MoE checkpoint (n_experts = {})",
        topo.n_experts
    );
    println!(
        "moe_int_eval: loaded {} d_model={} d_ff={} n_blocks={} state_slots={} vocab={} n_experts={} ({} tensors sha256-verified)",
        bundle.manifest_schema,
        topo.d_model,
        topo.d_ff,
        topo.n_blocks,
        topo.state_slots,
        topo.vocab,
        topo.n_experts,
        bundle.tensors_verified,
    );

    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .unwrap_or_else(|e| panic!("lower real MoE student: {e}"));

    // Every block must be a real MoE block with the declared expert count.
    assert_eq!(lowered.block_ffns.len(), topo.n_blocks);
    for (bi, b) in lowered.block_ffns.iter().enumerate() {
        match b {
            LoweredBlockFfn::Moe { experts, router } => {
                assert_eq!(experts.len(), topo.n_experts, "block {bi} expert count");
                assert_eq!(router.n_experts(), topo.n_experts, "block {bi} router");
            }
            LoweredBlockFfn::Dense { .. } => panic!("block {bi} lowered to Dense on a MoE student"),
        }
    }

    // Forward N tokens without panicking, then replay for a determinism check.
    let seed = 3usize.min(topo.vocab - 1);
    let n = 64usize;
    let seq_a = generate(&lowered, seed, n);
    let seq_b = generate(&lowered, seed, n);
    assert_eq!(seq_a, seq_b, "integer MoE forward is non-deterministic");
    assert!(seq_a.iter().all(|&id| id < topo.vocab), "id out of vocab");

    // Tally which experts fire in block 0 over the run (evidence the router
    // dispatch is actually exercised, not stuck on one expert).
    let LoweredBlockFfn::Moe { router, .. } = &lowered.block_ffns[0] else {
        unreachable!("block 0 is MoE");
    };
    let mut state = lowered.zero_state();
    let mut input = seed;
    let mut experts_fired = std::collections::BTreeSet::new();
    for _ in 0..n {
        let x: Vec<f32> = lowered
            .emb_resid_row_at(input)
            .iter()
            .map(|&v| v as f32 / 32.0) // Q19.5 dequant (STATE_RESID_ONE = 32)
            .collect();
        experts_fired.insert(router.route_f32(&x));
        let trace = lowered.forward_at(input, &mut state);
        input = trace.argmax_full;
    }
    println!(
        "moe_int_eval: {} tokens, block0 experts fired = {:?} (of {}), first ids = {:?}",
        n,
        experts_fired,
        topo.n_experts,
        &seq_a[..seq_a.len().min(12)],
    );
}
