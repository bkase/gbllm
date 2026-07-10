//! Byte-exact MoE ROM generation regression (deploy step 4a,
//! `docs/design/integer-moe-deploy.md`): the on-device top-1 MoE expert
//! dispatch + the fixed-point router IN ASSEMBLY + the paged head must generate
//! BYTE-IDENTICALLY to the host integer evaluator (`IntStateLoweredModel` MoE
//! forward). Byte-exactness is the contract.
//!
//! Gate (synthetic d192x8):
//! - one-token WRAM checkpoints byte-exact host == ROM across carried states,
//! - a multi-token (>= 16) generated sequence byte-exact host == ROM,
//! - the ROM router `EXPERT_SEL` matches the host `FixedRouter` every token
//!   (folded into `state_expected_segments` as `expert_sel_last`, and — since a
//!   wrong expert diverges the block residual — also implied by the byte-exact
//!   residual/logit/argmax checkpoints).

use std::path::PathBuf;

use gbf_bench::stateful::{
    harvest_state_cases, load_state_checkpoint, run_state_moe_rom_gate_lowered,
};
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, StateTopology, synthetic_moe_state_checkpoint,
};

/// A synthetic d192x8 MoE checkpoint with single-page (charset) vocab, so the
/// one-token WRAM-checkpoint gate exercises the full `state_expected_segments`
/// surface (block-0 dumps + final residual + logits + argmax + expert_sel)
/// without the paged-head indirection. The paged head is covered separately by
/// `logit_paging_regression` (dense) and, on the real student, by the env-gated
/// gate below.
fn synthetic_d192x8() -> IntStateLoweredModel {
    let topo = StateTopology {
        n_experts: 8,
        ..StateTopology::D192
    };
    let ck = synthetic_moe_state_checkpoint(topo, 0x51a7);
    IntStateLoweredModel::lower(&ck).expect("synthetic MoE checkpoint lowers")
}

#[test]
fn d192x8_moe_rom_generates_byte_exactly_vs_host() {
    let lowered = synthetic_d192x8();
    assert!(lowered.topology.is_moe(), "d192x8 is a MoE topology");
    assert_eq!(lowered.topology.n_experts, 8);

    // One-token cases across carried states: zero state + several harvested from
    // a short generated stream (nonzero recurrent state, real routing).
    let seed = 3u8;
    let host = gbf_bench::stateful::state_host_generate(&lowered, seed, 24);
    let ids = host.sequence.clone();
    let positions = [0usize, 1, 5, 11, 17, 23];
    let mut cases = harvest_state_cases(&lowered, &ids, &positions);
    // Also include the pure zero-state case at the seed id.
    cases.insert(0, (0, seed, lowered.zero_state()));

    let n_tokens = 20u16;
    let report = run_state_moe_rom_gate_lowered(&lowered, &cases, seed, n_tokens)
        .expect("MoE ROM gate runs");

    // Report the divergence precisely if any (never fake or weaken).
    if !report.all_byte_exact {
        for run in &report.one_token.runs {
            for m in &run.mismatches {
                panic!(
                    "one-token divergence: input={} state_from_pos={} segment={} \
                     wram_addr={:#06x} first_bad_offset={} expected={:#04x} actual={:#04x}",
                    run.input_id,
                    run.state_from_position,
                    m.segment,
                    m.wram_addr,
                    m.first_bad_offset,
                    m.expected_byte,
                    m.actual_byte,
                );
            }
        }
        for m in &report.generation.checkpoint_mismatches {
            panic!(
                "generation checkpoint divergence: segment={} wram_addr={:#06x} \
                 first_bad_offset={} expected={:#04x} actual={:#04x}",
                m.segment, m.wram_addr, m.first_bad_offset, m.expected_byte, m.actual_byte,
            );
        }
        panic!(
            "MoE ROM not byte-exact (sequences_match={}, first_divergence={:?})",
            report.generation.sequences_match, report.generation.first_divergence_index,
        );
    }

    assert!(
        report.one_token.all_byte_exact,
        "one-token WRAM checkpoints byte-exact across carried states"
    );
    assert!(
        report.generation.sequences_match,
        "multi-token generated sequence byte-exact host == ROM"
    );
    assert!(
        report.generation.first_token_checkpoints_byte_exact
            && report.generation.last_token_checkpoints_byte_exact,
        "generation first/last token WRAM checkpoints byte-exact"
    );
    assert!(report.all_byte_exact);

    // The ROM router selection matches the host FixedRouter every one-token case
    // (the `expert_sel_last` segment in `state_expected_segments`); a byte-exact
    // run already implies this, but assert the host selects a real expert.
    for run in &report.one_token.runs {
        assert!(
            (run.host_argmax as usize) < lowered.topology.vocab,
            "argmax id in vocab"
        );
    }
    assert!(
        n_tokens >= 16,
        "sustained generation gate uses >= 16 tokens (got {n_tokens})"
    );
}

fn moe_parity_dir() -> Option<PathBuf> {
    std::env::var("MOE_PARITY_DIR")
        .or_else(|_| std::env::var("MOE_INT_DIR"))
        .ok()
        .map(PathBuf::from)
}

/// THE REAL MILESTONE (env-gated like `moe_int_eval` / `moe_router_fixed_point_gate`):
/// build the REAL bridged subword MoE student's ROM (vocab-1024 Paged, 8
/// experts) and assert byte-exact generation vs the host integer evaluator —
/// one-token WRAM checkpoints across carried states (including the paged
/// running argmax16 + top-k heap) AND a multi-token generated sequence.
///
/// Set `MOE_PARITY_DIR` (or `MOE_INT_DIR`) to a dir with a `ckpt/`
/// (`f_s8_moe_state_checkpoint_export.v2` manifest + `tensors/`), produced by
/// `training/run_realparity.py --ckpt artifacts/student_moe_d192x8 --out ...`.
#[test]
#[ignore = "requires MOE_PARITY_DIR pointing at a real bridged MoE student"]
fn real_moe_student_rom_generates_byte_exactly_vs_host() {
    let Some(root) = moe_parity_dir() else {
        eprintln!("MOE_PARITY_DIR / MOE_INT_DIR unset; skipping");
        return;
    };
    let ckpt = root.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)
        .unwrap_or_else(|e| panic!("load real MoE student at {}: {e}", ckpt.display()));
    let topo = bundle.topology;
    assert!(topo.is_moe(), "expected MoE (n_experts={})", topo.n_experts);
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .unwrap_or_else(|e| panic!("lower real MoE student: {e}"));

    println!(
        "real MoE student: d_model={} d_ff={} n_blocks={} state_slots={} vocab={} n_experts={}",
        topo.d_model, topo.d_ff, topo.n_blocks, topo.state_slots, topo.vocab, topo.n_experts,
    );

    // Report ROM bank / driver / WRAM budget (fails loud if over budget).
    let rom = gbf_kernel::asm_impl_state::build_state_one_token_rom_lowered(
        &lowered,
        gbf_kernel::asm_impl_state::WeightLowering::V2Dispatch,
    )
    .expect("real MoE ROM builds within the 512-bank / bank-0 / 8 KiB budgets");
    println!(
        "real MoE ROM: banks={} driver_bytes={} weight_code_bytes={} table_bytes={}",
        rom.bank_count, rom.driver_bytes, rom.weight_code_bytes, rom.table_bytes,
    );

    // Seed must be a valid id; feed the FULL argmax back for the host harvest.
    let seed = 3u8;
    let host = gbf_bench::stateful::state_host_generate(&lowered, seed, 24);
    let ids = host.sequence.clone();
    let positions = [0usize, 3, 7, 15, 23];
    let mut cases = harvest_state_cases(&lowered, &ids, &positions);
    cases.insert(0, (0, seed, lowered.zero_state()));

    let report =
        run_state_moe_rom_gate_lowered(&lowered, &cases, seed, 16).expect("real MoE ROM gate runs");

    if !report.all_byte_exact {
        for run in &report.one_token.runs {
            for m in &run.mismatches {
                panic!(
                    "REAL divergence: input={} pos={} segment={} addr={:#06x} \
                     off={} expected={:#04x} actual={:#04x}",
                    run.input_id,
                    run.state_from_position,
                    m.segment,
                    m.wram_addr,
                    m.first_bad_offset,
                    m.expected_byte,
                    m.actual_byte,
                );
            }
        }
        panic!(
            "real MoE ROM not byte-exact (sequences_match={} first_div={:?})",
            report.generation.sequences_match, report.generation.first_divergence_index,
        );
    }
    assert!(report.all_byte_exact, "real MoE student ROM is byte-exact");
    println!(
        "real MoE student ROM byte-exact: banks={} wram_bytes={}",
        report.bank_count, report.wram_bytes,
    );
}
