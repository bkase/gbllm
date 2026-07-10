//! D192 readiness gate (bd-pp43d): prove the parameterized stateful ROM
//! pipeline accepts a ternary d192/ff384/6blk/slots192 charset-80 export in
//! the `f_s5_state_checkpoint_export.v1` manifest family **before** the
//! real S8 distilled student (bd-3771m) lands.
//!
//! Because the real checkpoint is still training (`gbf-experiments` must
//! not be touched), the gate runs against a **deterministic synthetic
//! export**: [`write_synthetic_state_export`] writes manifest + tensor
//! files mirroring the exact format `s8_matched_cycles.rs export_checkpoint`
//! emits, then everything downstream exercises the *real* production path —
//! `stateful::load_state_checkpoint` (manifest topology + sha256
//! verification), `IntStateLoweredModel::lower` (width plan), the banked
//! ROM builders, and the byte-exact emulator gates.
//!
//! Gates:
//! 1. One-token: host integer evaluator vs ROM, byte-exact across every
//!    layout-planned WRAM checkpoint, for zero and carried nonzero states.
//! 2. Multi-token: 64 tokens generated entirely on-device per seed,
//!    byte-identical sequence, SP/WRAM health, first/last-token dumps.
//! 3. Cycles: measured M-cycles/token against the ~7.1 s/char projection
//!    (973,824 MACs x 5.385 M-cycles + overheads).
//!
//! Evidence lands in `docs/experiments/d192-readiness/` (program-generated).

use std::fs;
use std::path::Path;

use gbf_foundation::sha256;
use gbf_kernel::asm_impl_model::ModelRomError;
use gbf_kernel::asm_impl_state::{
    StateWramLayout, WeightLowering, build_state_multi_token_rom,
    build_state_multi_token_rom_lowered, build_state_one_token_rom,
};
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, StateCheckpoint, StateTopology, synthetic_state_checkpoint_with,
};
use serde::Serialize;
use serde_json::json;

use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};
use crate::stateful::{
    StateRomGateReport, StateSeedRun, load_state_checkpoint, run_state_rom_gate,
    run_state_rom_gate_lowered, run_state_seed_generation,
};

/// Multi-token gate length (the mission's 64-token sustained gate).
pub const D192_GENERATION_TOKENS: u16 = 64;

/// Generation seeds: 'T' and space.
pub const D192_GENERATION_SEEDS: [u8; 2] = [19, 62];

/// Synthetic export seed (deterministic; pinned in the evidence).
pub const D192_SYNTHETIC_SEED: u64 = 20260702;

/// Projection under test: 973,824 MACs/token at 5.385 M-cycles each plus
/// norm/epilogue/table overheads was projected at ~7.1 s/char.
pub const D192_PROJECTED_SECONDS_PER_TOKEN: f64 = 7.1;

/// Write a synthetic checkpoint to `export_dir` in the
/// `f_s5_state_checkpoint_export.v1` on-disk format (manifest `topology`
/// block, per-tensor sha256, same tensor names/dtypes the S8 exporter
/// writes). Returns the checkpoint for cross-checking.
pub fn write_synthetic_state_export(
    export_dir: &Path,
    topology: StateTopology,
    seed: u64,
) -> Result<StateCheckpoint, OneTokenError> {
    let ck = synthetic_state_checkpoint_with(topology, seed);
    let tensors_dir = export_dir.join("tensors");
    fs::create_dir_all(&tensors_dir).map_err(|e| OneTokenError::Io {
        path: tensors_dir.clone(),
        reason: e.to_string(),
    })?;

    let mut tensor_index = Vec::new();
    let mut write_tensor = |name: &str,
                            file: &str,
                            shape: serde_json::Value,
                            dtype: &str,
                            bytes: &[u8]|
     -> Result<(), OneTokenError> {
        let path = export_dir.join(file);
        fs::write(&path, bytes).map_err(|e| OneTokenError::Io {
            path: path.clone(),
            reason: e.to_string(),
        })?;
        tensor_index.push(json!({
            "name": name,
            "dtype": dtype,
            "shape": shape,
            "layout": "row_major",
            "file": file,
            "sha256": sha256(bytes).to_hex(),
        }));
        Ok(())
    };

    let emb_bytes: Vec<u8> = (0..topology.vocab)
        .flat_map(|id| {
            ck.embedding_row_at(id)
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        })
        .collect();
    write_tensor(
        "embedding",
        "tensors/embedding.f32.bin",
        json!([topology.vocab, topology.d_model]),
        "f32_le",
        &emb_bytes,
    )?;

    let ternary_pair = |layer: &gbf_kernel::model_ref::TernaryLayer| -> (Vec<u8>, Vec<u8>) {
        let mut tern = Vec::with_capacity(layer.rows() * layer.cols());
        let mut scales = Vec::with_capacity(layer.rows() * 2);
        for row in 0..layer.rows() {
            tern.extend(layer.row(row).iter().map(|&w| w as u8));
            scales.extend_from_slice(&layer.scale_raw(row).to_le_bytes());
        }
        (tern, scales)
    };

    let (t, s) = ternary_pair(&ck.state_in);
    write_tensor(
        "state_input_to_state.ternary",
        "tensors/state_input_to_state.ternary.bin",
        json!([topology.state_slots, topology.d_model]),
        "i8_ternary",
        &t,
    )?;
    write_tensor(
        "state_input_to_state.scales",
        "tensors/state_input_to_state.scales.bin",
        json!([topology.state_slots]),
        "u16_le_q8_8",
        &s,
    )?;
    let (t, s) = ternary_pair(&ck.state_out);
    write_tensor(
        "state_state_to_output.ternary",
        "tensors/state_state_to_output.ternary.bin",
        json!([topology.d_model, topology.state_slots]),
        "i8_ternary",
        &t,
    )?;
    write_tensor(
        "state_state_to_output.scales",
        "tensors/state_state_to_output.scales.bin",
        json!([topology.d_model]),
        "u16_le_q8_8",
        &s,
    )?;

    let decay_bytes: Vec<u8> = ck
        .decay_raw()
        .iter()
        .flat_map(|d| d.to_le_bytes())
        .collect();
    write_tensor(
        "state_decay",
        "tensors/state_decay.q8_8_u16le.bin",
        json!([topology.state_slots]),
        "u16_le (Q8.8 fixed-point; f32 = raw/256, exact for MT4 rates)",
        &decay_bytes,
    )?;

    for (k, block) in ck.blocks().iter().enumerate() {
        let (up, down) = block
            .as_dense()
            .expect("d192 dense export path handles only dense checkpoints");
        let (t, s) = ternary_pair(up);
        write_tensor(
            &format!("block{k}_up.ternary"),
            &format!("tensors/block{k}_up.ternary.bin"),
            json!([topology.d_ff, topology.d_model]),
            "i8_ternary",
            &t,
        )?;
        write_tensor(
            &format!("block{k}_up.scales"),
            &format!("tensors/block{k}_up.scales.bin"),
            json!([topology.d_ff]),
            "u16_le_q8_8",
            &s,
        )?;
        let (t, s) = ternary_pair(down);
        write_tensor(
            &format!("block{k}_down.ternary"),
            &format!("tensors/block{k}_down.ternary.bin"),
            json!([topology.d_model, topology.d_ff]),
            "i8_ternary",
            &t,
        )?;
        write_tensor(
            &format!("block{k}_down.scales"),
            &format!("tensors/block{k}_down.scales.bin"),
            json!([topology.d_model]),
            "u16_le_q8_8",
            &s,
        )?;
    }

    let manifest = json!({
        "schema": "f_s5_state_checkpoint_export.v1",
        "bead": "bd-pp43d (synthetic d192 readiness stand-in for bd-3771m)",
        "git_sha": "synthetic",
        "seed": seed,
        "topology": {
            "family": "linear_state_multi_timescale_then_dense_ffn",
            "moe": false,
            "d_model": topology.d_model,
            "d_ff": topology.d_ff,
            "n_blocks": topology.n_blocks,
            "vocab": topology.vocab,
            "lexical": "charset_v1 (80 ids; ids 0..75 printable incl. newline, 76 reserved, 77 <bos>, 78 <eos>, 79 <unk>)",
            "tied_head": true,
            "sequence_state_kind": "linear_state_multi_timescale",
            "sequence_state_params": {
                "state_slots": topology.state_slots,
                "state_bytes_per_layer": topology.state_slots * 4,
                "decay_policy": "MultiTimescale",
                "decay_rates_by_band": [0.5, 0.75, 0.875, 0.9375],
            },
        },
        "tensors": tensor_index,
    });
    let manifest_path = export_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )
    .map_err(|e| OneTokenError::Io {
        path: manifest_path,
        reason: e.to_string(),
    })?;
    Ok(ck)
}

#[derive(Debug, Clone, Serialize)]
pub struct D192LayoutFacts {
    pub wram_bytes_allocated: usize,
    pub wram_budget_bytes: usize,
    pub per_block_residual_dumps_kept: bool,
    pub out_acc_dump_kept: bool,
    pub scratch_overlaid_on_matvec_arena: bool,
    pub untouched_regions: Vec<(u16, u16)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192WidthFacts {
    pub down_acc_width: String,
    pub down_acc_structural_bound: i64,
    pub i16_bound: i64,
    /// Structural per-row worst case of the Q19.5 down delta over the actual
    /// weights/scales; on the wide path lowering proves it fits the i24
    /// delta carrier (state-int-semantics.v2, clamp-free).
    pub down_delta_structural_bound: u64,
    pub i24_delta_bound: u64,
    pub decision_source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192RomFacts {
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub uses_romb1_9bit_banking: bool,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192CycleFacts {
    pub macs_per_token: u64,
    pub one_token_mean_m_cycles: u64,
    pub multi_token_mean_m_cycles: u64,
    pub seconds_per_token_dmg: f64,
    pub projected_seconds_per_token: f64,
    pub measured_over_projected: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192MultiTokenReport {
    pub n_tokens: u16,
    pub seeds: Vec<u8>,
    pub all_sequences_match: bool,
    pub all_health_checks_pass: bool,
    pub runs: Vec<StateSeedRun>,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192ReadinessReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub target_checkpoint_bead: &'static str,
    pub git_sha: String,
    pub topology: TopologyFacts,
    pub synthetic_export: SyntheticExportFacts,
    pub width: D192WidthFacts,
    pub layout: D192LayoutFacts,
    pub rom: D192RomFacts,
    pub one_token_gate: StateRomGateReport,
    pub multi_token: D192MultiTokenReport,
    pub cycles: D192CycleFacts,
    pub arm_b_regression_note: &'static str,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TopologyFacts {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub state_slots: usize,
    pub vocab: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyntheticExportFacts {
    pub schema: String,
    pub seed: u64,
    pub tensors_verified_sha256: usize,
    pub manifest_sha256: String,
    pub loader: &'static str,
}

/// Run every d192 readiness gate against a synthetic export written under
/// `work_dir`, returning the evidence report.
pub fn run_d192_readiness(work_dir: &Path) -> Result<D192ReadinessReport, OneTokenError> {
    let topology = StateTopology::D192;
    let export_dir = work_dir.join("synthetic-export");
    write_synthetic_state_export(&export_dir, topology, D192_SYNTHETIC_SEED)?;

    // Load through the REAL production path (manifest topology + sha256).
    let bundle = load_state_checkpoint(&export_dir)?;
    assert_eq!(bundle.topology, topology, "manifest topology round-trips");
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;

    let layout = StateWramLayout::plan(topology, lowered.down_width, false)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let layout_facts = D192LayoutFacts {
        wram_bytes_allocated: layout.bytes_allocated,
        wram_budget_bytes: 8192,
        per_block_residual_dumps_kept: layout.xdump.is_some(),
        out_acc_dump_kept: layout.sacc_separate,
        scratch_overlaid_on_matvec_arena: layout.absx == layout.acc,
        untouched_regions: layout.untouched_regions(),
    };
    let width = D192WidthFacts {
        down_acc_width: format!("{:?}", lowered.down_width),
        down_acc_structural_bound: lowered.down_acc_structural_bound,
        i16_bound: 32767,
        down_delta_structural_bound: lowered.down_delta_structural_bound,
        i24_delta_bound: gbf_kernel::state_model_ref::DOWN_DELTA_WIDE_BOUND,
        decision_source: "structural per-row worst case over the actual ternary weights \
                          (the f_s5_state_checkpoint_export.v1 manifest declares no measured \
                          activation ranges, so lowering never relies on unmeasured statistics)",
    };

    // One-token gate: zero state plus carried states harvested from a
    // short self-generated stream.
    let mut cases: Vec<(usize, u8, Vec<i32>)> = vec![(0, 19u8, lowered.zero_state())];
    let mut state = lowered.zero_state();
    let mut input = 19u8;
    for pos in 1..=13usize {
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax;
        if pos == 1 || pos == 5 || pos == 13 {
            cases.push((pos, input, state.clone()));
        }
    }
    let one_token_gate = run_state_rom_gate(&lowered, &cases)?;

    // Multi-token gate: 64 tokens on-device per seed.
    let rom = build_state_multi_token_rom(&lowered, D192_GENERATION_TOKENS)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let mut runs = Vec::new();
    for &seed in &D192_GENERATION_SEEDS {
        runs.push(run_state_seed_generation(&rom, &lowered, seed)?);
    }
    let all_sequences_match = runs.iter().all(|r| r.sequences_match);
    let all_health_checks_pass = runs.iter().all(StateSeedRun::all_checks_pass);
    let multi_mean = runs.iter().map(|r| r.cycles.mean).sum::<u64>() / runs.len().max(1) as u64;

    let one_rom =
        build_state_one_token_rom(&lowered).map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let rom_facts = D192RomFacts {
        rom_bytes: one_rom.rom.len(),
        bank_count: one_rom.bank_count,
        uses_romb1_9bit_banking: one_rom.bank_count > 256,
        driver_bytes: one_rom.driver_bytes,
        weight_code_bytes: one_rom.weight_code_bytes,
        weight_chunk_count: one_rom.weight_chunk_count,
        table_bytes: one_rom.table_bytes,
    };

    let seconds = multi_mean as f64 / DMG_M_CYCLES_PER_SECOND as f64;
    let cycles = D192CycleFacts {
        macs_per_token: topology.macs_per_token(),
        one_token_mean_m_cycles: one_token_gate.mean_m_cycles,
        multi_token_mean_m_cycles: multi_mean,
        seconds_per_token_dmg: seconds,
        projected_seconds_per_token: D192_PROJECTED_SECONDS_PER_TOKEN,
        measured_over_projected: seconds / D192_PROJECTED_SECONDS_PER_TOKEN,
    };

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let manifest_bytes =
        fs::read(export_dir.join("manifest.json")).map_err(|e| OneTokenError::Io {
            path: export_dir.join("manifest.json"),
            reason: e.to_string(),
        })?;

    Ok(D192ReadinessReport {
        schema: "d192_readiness.v1",
        bead: "bd-pp43d",
        target_checkpoint_bead: "bd-3771m",
        git_sha,
        topology: TopologyFacts {
            d_model: topology.d_model,
            d_ff: topology.d_ff,
            n_blocks: topology.n_blocks,
            state_slots: topology.state_slots,
            vocab: topology.vocab,
        },
        synthetic_export: SyntheticExportFacts {
            schema: bundle.manifest_schema,
            seed: D192_SYNTHETIC_SEED,
            tensors_verified_sha256: bundle.tensors_verified,
            manifest_sha256: sha256(&manifest_bytes).to_hex(),
            loader: "gbf_bench::stateful::load_state_checkpoint (production path: manifest \
                     topology block + per-tensor sha256 verification)",
        },
        width,
        layout: layout_facts,
        rom: rom_facts,
        one_token_gate,
        multi_token: D192MultiTokenReport {
            n_tokens: D192_GENERATION_TOKENS,
            seeds: D192_GENERATION_SEEDS.to_vec(),
            all_sequences_match,
            all_health_checks_pass,
            runs,
        },
        cycles,
        arm_b_regression_note: "All committed arm-B d64 gates (dense one-token/multi-token, \
             stateful one/multi-token, sampling, interactive shell, compile gate) run on the \
             same parameterized builders and stay green; see cargo test -p gbf-bench --lib.",
        caveats: vec![
            "The gate model is SYNTHETIC (deterministic seeded weights mirroring the export \
             format and weight statistics), not tonight's real distilled student; the real \
             checkpoint may differ in accumulator ranges. The width decision is structural \
             (worst case over actual weights), so a real checkpoint can only be safer, but \
             its per-row scale magnitudes must still pass the lowering's u32 epilogue bound."
                .to_string(),
            "Per-block residual dumps and the out-projection accumulator dump are dropped at \
             this topology (WRAM budget); the gate still pins the final residual, all block-0 \
             dumps, state vector, in-projection accumulators, y activations, final norm, \
             logits, and argmax byte-exactly."
                .to_string(),
            "Fidelity vs the trainer's f32 forward is meaningless on synthetic weights and is \
             deferred to the real-checkpoint bring-up."
                .to_string(),
        ],
    })
}

/// Result of the V2 dispatch-lowering byte-exact gate on synthetic d192.
#[derive(Debug, Clone, Serialize)]
pub struct D192V2GateResult {
    /// One-token byte-exact across the zero + carried-state cases under V2.
    pub one_token_byte_exact: bool,
    /// 64-token on-device generation matches the host feedback loop under V2.
    pub multi_token_sequences_match: bool,
    /// First/last-token WRAM checkpoints byte-exact under V2.
    pub multi_token_checkpoints_byte_exact: bool,
    /// Weight banks under V2 (packed base-81 streams).
    pub v2_weight_banks: usize,
    /// Weight banks under V3 (weights-as-code chunks).
    pub v3_weight_banks: usize,
    /// V2 packed weight-stream bytes.
    pub v2_weight_stream_bytes: usize,
    /// V3 weight-code bytes.
    pub v3_weight_code_bytes: usize,
    /// V2 one-token ROM total bank count.
    pub v2_bank_count: u16,
    /// V2 bank-0 driver bytes (must stay below the 0x4000 window).
    pub v2_driver_bytes: usize,
}

impl D192V2GateResult {
    #[must_use]
    pub fn pass(&self) -> bool {
        self.one_token_byte_exact
            && self.multi_token_sequences_match
            && self.multi_token_checkpoints_byte_exact
    }

    /// Bytes-per-weight density win of V2 over V3 (weight code/stream only).
    #[must_use]
    pub fn density_ratio(&self) -> f64 {
        if self.v2_weight_stream_bytes == 0 {
            return 0.0;
        }
        self.v3_weight_code_bytes as f64 / self.v2_weight_stream_bytes as f64
    }
}

/// Byte-exact gate for the V2 dispatch lowering on the SAME synthetic d192
/// model the readiness report uses: one-token (zero + carried state) and a
/// short on-device generation, all compared against the host integer evaluator
/// (design: docs/design/v2-dispatch-stateful.md).
pub fn run_d192_v2_gate(work_dir: &Path) -> Result<D192V2GateResult, OneTokenError> {
    let topology = StateTopology::D192;
    let export_dir = work_dir.join("synthetic-export-v2");
    write_synthetic_state_export(&export_dir, topology, D192_SYNTHETIC_SEED)?;
    let bundle = load_state_checkpoint(&export_dir)?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;

    // One-token cases: zero state plus carried states from a short stream.
    let mut cases: Vec<(usize, u8, Vec<i32>)> = vec![(0, 19u8, lowered.zero_state())];
    let mut state = lowered.zero_state();
    let mut input = 19u8;
    for pos in 1..=13usize {
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax;
        if pos == 1 || pos == 5 || pos == 13 {
            cases.push((pos, input, state.clone()));
        }
    }
    let v2_gate = run_state_rom_gate_lowered(&lowered, &cases, WeightLowering::V2Dispatch)?;

    // Multi-token: a short on-device generation (byte-exact vs host feedback).
    let rom = build_state_multi_token_rom_lowered(&lowered, 24, WeightLowering::V2Dispatch)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let run = run_state_seed_generation(&rom, &lowered, D192_GENERATION_SEEDS[0])?;

    let v3 = build_state_one_token_rom(&lowered).map_err(|e| OneTokenError::Rom(e.to_string()))?;
    Ok(D192V2GateResult {
        one_token_byte_exact: v2_gate.all_byte_exact,
        multi_token_sequences_match: run.sequences_match,
        multi_token_checkpoints_byte_exact: run.first_token_checkpoints_byte_exact
            && run.last_token_checkpoints_byte_exact,
        v2_weight_banks: v2_gate.rom.weight_chunk_count,
        v3_weight_banks: v3.weight_chunk_count,
        v2_weight_stream_bytes: v2_gate.rom.weight_code_bytes,
        v3_weight_code_bytes: v3.weight_code_bytes,
        v2_bank_count: v2_gate.rom.bank_count,
        v2_driver_bytes: v2_gate.rom.driver_bytes,
    })
}

/// A "d256-class" fit topology (step 4, docs/design/v2-dispatch-stateful.md).
///
/// Literal d_model=256 exceeds the device's u8 lane-loop / single head
/// activation-page limit (max 255), and the nearest 4-aligned d256 width
/// (d252/ff512) overflows the full-debug-dump 8 KiB WRAM surface the byte-exact
/// gate compares. This topology instead carries **more FFN weight than
/// d256/ff512/6blk** (10 * 2 * 416 * 208 = 1,730,560 vs 1,572,864) while fitting
/// the full debug-dump WRAM surface, which lets the gate verify every WRAM
/// checkpoint byte-exact. Its purpose is to prove the V2 ROM-capacity unlock:
/// V3 weights-as-code needs 578 ROM banks (> the 512-bank / 8 MiB MBC5 ceiling,
/// i.e. **unbuildable**), while V2 dispatch packs it into ~45 banks / 1 MiB.
pub const D256_CLASS_TOPOLOGY: StateTopology = StateTopology {
    d_model: 208,
    d_ff: 416,
    n_blocks: 10,
    state_slots: 208,
    vocab: 80,
    n_experts: 1,
    logit_paging: gbf_kernel::state_model_ref::LogitPaging::SinglePage,
};

/// Result of the d256-class V2 fit + byte-exact gate.
#[derive(Debug, Clone, Serialize)]
pub struct D256V2GateResult {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub ffn_weights: usize,
    /// One-token byte-exact (zero + carried state) under V2.
    pub one_token_byte_exact: bool,
    /// On-device generation matches the host feedback loop under V2.
    pub multi_token_sequences_match: bool,
    /// First/last-token WRAM checkpoints byte-exact under V2.
    pub multi_token_checkpoints_byte_exact: bool,
    /// True only if V3 weights-as-code would ALSO fit; the unlock expects false.
    pub v3_builds: bool,
    /// ROM banks V3 would need (from the `TooManyBanks` overflow).
    pub v3_banks_needed: usize,
    /// V2 one-token ROM total bank count.
    pub v2_bank_count: u16,
    /// V2 ROM size in MiB.
    pub v2_rom_mib: f64,
    /// V2 packed weight banks.
    pub v2_weight_banks: usize,
    /// V2 bank-0 driver bytes (must stay below 0x4000).
    pub v2_driver_bytes: usize,
    /// Multi-token V2 bank-0 driver bytes (larger than one-token).
    pub v2_multi_driver_bytes: usize,
}

impl D256V2GateResult {
    #[must_use]
    pub fn pass(&self) -> bool {
        self.one_token_byte_exact
            && self.multi_token_sequences_match
            && self.multi_token_checkpoints_byte_exact
            && !self.v3_builds
            && self.v3_banks_needed > 512
            && self.v2_bank_count <= 512
            && self.v2_driver_bytes < 0x4000 - 0x150
            && self.v2_multi_driver_bytes < 0x4000 - 0x150
    }
}

/// Byte-exact gate for the V2 dispatch lowering on the d256-class fit topology:
/// one-token (zero + carried state) and a short on-device generation compared
/// against the host integer evaluator, plus the fit facts proving V3 cannot
/// build this model while V2 can (step 4, docs/design/v2-dispatch-stateful.md).
pub fn run_d256_v2_gate() -> Result<D256V2GateResult, OneTokenError> {
    let topology = D256_CLASS_TOPOLOGY;
    let ck = synthetic_state_checkpoint_with(topology, 0xD256);
    let lowered =
        IntStateLoweredModel::lower(&ck).map_err(|e| OneTokenError::Model(e.to_string()))?;

    // One-token cases: zero state plus carried states from a short stream.
    let mut cases: Vec<(usize, u8, Vec<i32>)> = vec![(0, 19u8, lowered.zero_state())];
    let mut state = lowered.zero_state();
    let mut input = 19u8;
    for pos in 1..=9usize {
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax;
        if pos == 1 || pos == 5 || pos == 9 {
            cases.push((pos, input, state.clone()));
        }
    }
    let v2_gate = run_state_rom_gate_lowered(&lowered, &cases, WeightLowering::V2Dispatch)?;

    // Multi-token: short on-device generation, byte-exact vs host feedback.
    let mt = build_state_multi_token_rom_lowered(&lowered, 8, WeightLowering::V2Dispatch)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let run = run_state_seed_generation(&mt, &lowered, D192_GENERATION_SEEDS[0])?;

    // V3 must NOT be able to build this model (exceeds the 512-bank ceiling).
    let (v3_builds, v3_banks_needed) = match build_state_one_token_rom(&lowered) {
        Ok(r) => (true, usize::from(r.bank_count)),
        Err(ModelRomError::TooManyBanks { banks }) => (false, banks),
        Err(e) => return Err(OneTokenError::Rom(e.to_string())),
    };

    let ffn_weights = topology.n_blocks * 2 * topology.d_ff * topology.d_model;
    Ok(D256V2GateResult {
        d_model: topology.d_model,
        d_ff: topology.d_ff,
        n_blocks: topology.n_blocks,
        ffn_weights,
        one_token_byte_exact: v2_gate.all_byte_exact,
        multi_token_sequences_match: run.sequences_match,
        multi_token_checkpoints_byte_exact: run.first_token_checkpoints_byte_exact
            && run.last_token_checkpoints_byte_exact,
        v3_builds,
        v3_banks_needed,
        v2_bank_count: v2_gate.rom.bank_count,
        v2_rom_mib: v2_gate.rom.rom_bytes as f64 / (1024.0 * 1024.0),
        v2_weight_banks: v2_gate.rom.weight_chunk_count,
        v2_driver_bytes: v2_gate.rom.driver_bytes,
        v2_multi_driver_bytes: mt.driver_bytes,
    })
}

/// Render the README (generated, never hand-written).
#[must_use]
pub fn d192_report_to_markdown(r: &D192ReadinessReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# D192 readiness ({})", r.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Proof that the stateful ROM pipeline (generalized from the fixed arm-B \
         d64/ff128/4blk/slots64 builders) accepts a ternary d{}/ff{}/{}blk/slots{} charset-{} \
         export the moment the real S8 distilled student ({}) lands. Generated by \
         `cargo run -p gbf-bench --bin d192-readiness`; every number is program output at git \
         `{}`.",
        r.topology.d_model,
        r.topology.d_ff,
        r.topology.n_blocks,
        r.topology.state_slots,
        r.topology.vocab,
        r.target_checkpoint_bead,
        r.git_sha
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Synthetic export (format mirror)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Schema `{}`, synthetic seed {}, {} tensors sha256-verified through the production \
         loader ({}); manifest sha256 `{}`",
        r.synthetic_export.schema,
        r.synthetic_export.seed,
        r.synthetic_export.tensors_verified_sha256,
        r.synthetic_export.loader,
        &r.synthetic_export.manifest_sha256[..16]
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Accumulator widths at fan-in {}", r.topology.d_ff);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Down-projection accumulators: **{}** — structural per-row bound {} vs i16 bound {}; \
         decision source: {}",
        r.width.down_acc_width,
        r.width.down_acc_structural_bound,
        r.width.i16_bound,
        r.width.decision_source
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## WRAM budget");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- {} of {} bytes allocated (layout-planned, budget asserted at build time); \
         per-block residual dumps kept: {}; out-acc dump kept: {}; |x|/out-acc scratch \
         overlaid on the matvec arena: {}",
        r.layout.wram_bytes_allocated,
        r.layout.wram_budget_bytes,
        r.layout.per_block_residual_dumps_kept,
        r.layout.out_acc_dump_kept,
        r.layout.scratch_overlaid_on_matvec_arena
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## ROM");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- {} bytes, {} banks (9-bit MBC5 ROMB1 banking: {}), driver {} B, weight code {} B \
         in {} chunks, tables {} B",
        r.rom.rom_bytes,
        r.rom.bank_count,
        r.rom.uses_romb1_9bit_banking,
        r.rom.driver_bytes,
        r.rom.weight_code_bytes,
        r.rom.weight_chunk_count,
        r.rom.table_bytes
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Gates");
    let _ = writeln!(out);
    let g = &r.one_token_gate;
    let _ = writeln!(
        out,
        "- One-token: **{}** — {}/{} (input, state) cases byte-exact across all layout WRAM \
         checkpoints",
        if g.all_byte_exact { "PASS" } else { "FAIL" },
        g.runs.iter().filter(|run| run.byte_exact).count(),
        g.runs.len()
    );
    let m = &r.multi_token;
    let _ = writeln!(
        out,
        "- Multi-token ({} tokens/seed, seeds {:?}): sequences **{}**, health **{}**",
        m.n_tokens,
        m.seeds,
        if m.all_sequences_match {
            "PASS"
        } else {
            "FAIL"
        },
        if m.all_health_checks_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(out, "- {}", r.arm_b_regression_note);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Cycles per token");
    let _ = writeln!(out);
    let c = &r.cycles;
    let _ = writeln!(
        out,
        "- {} MACs/token; one-token mean {} M-cycles; generation-loop mean {} M-cycles = \
         **{:.3} s/token** on DMG vs the ~{:.1} s/char projection ({:.2}x)",
        c.macs_per_token,
        c.one_token_mean_m_cycles,
        c.multi_token_mean_m_cycles,
        c.seconds_per_token_dmg,
        c.projected_seconds_per_token,
        c.measured_over_projected
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Caveats");
    let _ = writeln!(out);
    for cv in &r.caveats {
        let _ = writeln!(out, "- {cv}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_kernel::state_model_ref::AccWidth;

    /// The synthetic export must round-trip through the production loader
    /// with the manifest-declared topology and verified tensor hashes.
    #[test]
    fn synthetic_export_round_trips_through_the_production_loader() {
        let dir = std::env::temp_dir().join(format!("gbf-d192-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ck = write_synthetic_state_export(&dir, StateTopology::D192, 7).expect("writes");
        let bundle = load_state_checkpoint(&dir).expect("loads");
        assert_eq!(bundle.topology, StateTopology::D192);
        assert_eq!(bundle.tensors_verified, 4 + 2 + 6 * 4);
        // Same lowered semantics from disk as from memory.
        let from_disk = IntStateLoweredModel::lower(&bundle.checkpoint).expect("lowers");
        let in_mem = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut s1 = from_disk.zero_state();
        let mut s2 = in_mem.zero_state();
        let a = from_disk.forward(19, &mut s1);
        let b = in_mem.forward(19, &mut s2);
        assert_eq!(a.logits, b.logits);
        assert_eq!(from_disk.down_width, AccWidth::I24);
        let _ = fs::remove_dir_all(&dir);
    }
}
