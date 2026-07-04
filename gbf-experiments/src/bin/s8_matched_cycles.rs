//! F-S8 matched-cycles sizing sweep (bd-3771m): dense-vs-MoE under the
//! quality-first UX budget (30 s/char => ~22M M-cycles/token at 70% CPU,
//! ~4.1M MACs/token under the measured V3 weights-as-code kernel at
//! 5.385 cy/MAC, ~2.1M under V2 dispatch at 10.261 cy/MAC; constants from
//! docs/experiments/kernel-bakeoff).
//!
//! Every arm reuses the F-S5 winner substrate (charset_v1 80-id vocab,
//! LinearState MT4 recurrent state block with residual, pre-norm residual
//! ternary FFN stack, tied head, warmup Off -> Hard QAT recipe, seed 0,
//! truncated BPTT with detached state carry) from `s5_state_ab.rs`, and adds:
//!
//!   * matched-CYCLES arms: A1 dense d128/ff256/4blk, A2 dense d160/ff320/6blk
//!     (~2x A1 per-token MACs), A3 top-1 MoE d128/4blk with 4 experts of
//!     ff256 per block (per-token MACs ~= A1, stored FFN params ~= 4x A1);
//!   * a simplified learned top-1 router per MoE block (single fp linear
//!     [n_experts, d_model] over the quantized normed block input, softmax,
//!     HOST argmax top-1 dispatch). The hard top-1 assignment is
//!     stop-gradient dispatch provenance; router gradients flow through the
//!     top-1 probability that scales the selected expert's output and
//!     through the Switch-style load-balance term P_e. This is NOT the
//!     gbf-model `Top1RouterQat` low-rank phase-machinery core - wiring that
//!     into the batched Burn trainer was out of the time budget and is
//!     documented as such in the report;
//!   * logit distillation: an fp teacher (same family, wider, trained with
//!     QAT hardness Off + activation fake-quant passthrough) supplies soft
//!     targets softmax(t/T); student loss = CE + w * T^2 *
//!     CE(softmax(t/T) || softmax(s/T)) with T and w from the CLI;
//!   * deployment accounting per arm: per-token matvec MACs, deployable ROM
//!     bytes under BOTH lowerings (V3 weights-as-code 4.401 B/weight = the
//!     measured 9013 B / 2048 weights @40% zeros; V2 dispatch 0.25 B/weight
//!     packed Ternary2 data + 2699 B shared handler code), projected
//!     M-cycles/token and s/char at 70% CPU.
//!
//! Evaluation is identical to s5_state_ab: lane-parallel full-val bpc in both
//! Soft (fp STE ceiling) and Hard (deployable ternary) semantics, re-expressed
//! per raw val byte with the same total-bits-over-raw-byte-count method as the
//! KN-5 artifact, plus a 256-char greedy sample. KN-5 reference numbers are
//! copied verbatim from experiments/S4/baseline/.
//!
//! Integrity: every number in the emitted JSON is produced by this program
//! from the actual runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use gbf_data::charset_v1::normalize_raw;
use gbf_foundation::sha256;
use gbf_model::qat::{
    ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
    ActivationRangeMode, MatrixShape, QatHardnessControl, QuantHardness, TernaryLinearQat,
    TernaryThreshold,
};
use gbf_model::sequence::DecayPolicy;
use gbf_train::adapter::burn::{
    BurnBackend, BurnDevice, BurnFloatTensor, BurnGradientsParams, BurnInt, BurnModule,
    BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnOptimizer, BurnParam, BurnTensor,
    adamw_config, burn_gelu_approximate, burn_log_softmax, burn_softmax, float_tensor_from_vec,
    float_tensor_into_vec,
};
use gbf_train::qat::{ActFakeQuantBurnQat, TernaryLinearBurnQat};
use serde_json::json;

const VOCAB: usize = 80;
const NORM_EPS: f32 = 1.0e-5;
const NORM_CLIP: f32 = 8.0;
const ACT_RANGE: f32 = 8.0;
/// S5 RFC D3 variant L_MT4 decay rates, one per contiguous slot band.
const MT4_DECAYS: [f32; 4] = [0.5, 0.75, 0.875, 0.9375];

// --- measured kernel constants (docs/experiments/kernel-bakeoff, 40% zeros) ---
/// V3 weights-as-code: measured M-cycles per MAC (5385/1000).
const V3_CY_PER_MAC: f64 = 5.385;
/// V3 weights-as-code: measured program bytes per stored weight (9013/2048).
const V3_BYTES_PER_WEIGHT: f64 = 9013.0 / 2048.0;
/// V2 threaded dispatch: measured M-cycles per MAC (10261/1000).
const V2_CY_PER_MAC: f64 = 10.261;
/// V2 dispatch: packed Ternary2 data bytes per stored weight.
const V2_BYTES_PER_WEIGHT: f64 = 0.25;
/// V2 dispatch: shared 81-handler code (bytes, once per ROM).
const V2_SHARED_CODE_BYTES: f64 = 2699.0;
/// Game Boy M-cycles per second (4.194304 MHz T-cycles / 4).
const GB_MCYCLES_PER_SEC: f64 = 1_048_576.0;
/// Fraction of CPU available to inference after the UI reserve.
const CPU_FRACTION: f64 = 0.70;
/// MBC5 ROM ceiling minus ~1 MiB runtime/UI reserve.
const ROM_BUDGET_BYTES: f64 = 7.0 * 1024.0 * 1024.0;

type Adiff = BurnNdArrayAutodiffBackend;
type Plain = BurnNdArrayBackend;

#[derive(Parser, Debug, Clone)]
#[command(about = "F-S8 matched-cycles dense-vs-MoE sizing sweep (bd-3771m)")]
struct Args {
    /// Phase: "arm" (train+eval one arm), "distill" (teacher + distilled
    /// student for one arm spec), "report" (merge arm jsons into report.json).
    #[arg(long, default_value = "arm")]
    phase: String,
    /// Arm name: A1 | A2 | A3 | CUSTOM (with --d-model etc. overrides).
    #[arg(long, default_value = "A1")]
    arm: String,
    /// Repo root (corpus + experiments paths resolve against this).
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    #[arg(
        long,
        default_value = "corpus/gutenberg/gutenberg_train_concatenated.bin"
    )]
    train_bin: String,
    #[arg(long, default_value_t = 64 * 1024 * 1024)]
    train_cap_bytes: usize,
    #[arg(long, default_value_t = 1024 * 1024)]
    val_cap_bytes: usize,
    /// Cap on scored (context,target) pairs per eval pass (0 = full stream).
    #[arg(long, default_value_t = 0)]
    eval_pairs: usize,
    #[arg(long, default_value_t = 8)]
    eval_lanes: usize,
    #[arg(long, default_value_t = 256)]
    eval_chunk: usize,
    /// Optimizer steps (matched across arms; also the distilled-student steps).
    #[arg(long, default_value_t = 40000)]
    steps: u64,
    /// fp teacher steps (distill phase only).
    #[arg(long, default_value_t = 20000)]
    teacher_steps: u64,
    /// Teacher width multiplier over the student spec (dense fp teacher).
    #[arg(long, default_value_t = 2)]
    teacher_mult: usize,
    /// Distillation temperature.
    #[arg(long, default_value_t = 2.0)]
    distill_temperature: f64,
    /// Distillation weight on the T^2-scaled soft cross-entropy term.
    #[arg(long, default_value_t = 0.5)]
    distill_weight: f64,
    /// Switch-style router load-balance aux-loss weight (MoE arms).
    #[arg(long, default_value_t = 0.01)]
    router_aux_weight: f64,
    /// BPTT chunk length (tokens per lane per step).
    #[arg(long, default_value_t = 128)]
    seq_len: usize,
    /// Parallel BPTT lanes (seq_len * lanes = tokens per step).
    #[arg(long, default_value_t = 4)]
    lanes: usize,
    #[arg(long, default_value_t = 0.01)]
    lr: f64,
    /// Fraction of steps trained with QAT hardness OFF before Hard.
    #[arg(long, default_value_t = 0.25)]
    warmup_frac: f64,
    #[arg(long, default_value_t = 0)]
    seed: u64,
    // CUSTOM arm topology overrides.
    #[arg(long, default_value_t = 128)]
    d_model: usize,
    #[arg(long, default_value_t = 256)]
    d_ff: usize,
    #[arg(long, default_value_t = 4)]
    n_blocks: usize,
    #[arg(long, default_value_t = 128)]
    state_slots: usize,
    /// Experts per block (1 = dense).
    #[arg(long, default_value_t = 1)]
    n_experts: usize,
    #[arg(long, default_value_t = 500)]
    log_every: u64,
    #[arg(long, default_value_t = 256)]
    sample_chars: usize,
    #[arg(
        long,
        default_value = "The children walked down to the river in the morning, and "
    )]
    sample_prompt: String,
    /// Arm result jsons to merge in the report phase (comma-separated names).
    #[arg(long, default_value = "A1,A2,A3,A3_distill")]
    report_arms: String,
    /// Scale-plan topology for the report phase, as
    /// "d_model,d_ff,n_blocks,state_slots,n_experts" (empty = omit).
    #[arg(long, default_value = "")]
    scale_plan: String,
    #[arg(long, default_value = "experiments/S8/sweep")]
    out_dir: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("s8_matched_cycles FAILED: {err}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// arm specs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ArmSpec {
    name: String,
    description: String,
    d_model: usize,
    d_ff: usize,
    n_blocks: usize,
    state_slots: usize,
    n_experts: usize,
}

impl ArmSpec {
    fn resolve(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let spec = match args.arm.to_uppercase().as_str() {
            "A1" => Self {
                name: "A1".into(),
                description:
                    "dense d128/ff256/4blk + LinearState MT4 slots128 (~4x S5 arm-B per-token MACs)"
                        .into(),
                d_model: 128,
                d_ff: 256,
                n_blocks: 4,
                state_slots: 128,
                n_experts: 1,
            },
            "A2" => Self {
                name: "A2".into(),
                description:
                    "dense d160/ff320/6blk + LinearState MT4 slots160 (~2.2x A1 per-token MACs; cycles-vs-quality slope probe)"
                        .into(),
                d_model: 160,
                d_ff: 320,
                n_blocks: 6,
                state_slots: 160,
                n_experts: 1,
            },
            "A3" => Self {
                name: "A3".into(),
                description:
                    "top-1 MoE d128/4blk, 4 experts of ff256 per block + LinearState MT4 slots128 (per-token MACs ~= A1, stored FFN params ~= 4x A1)"
                        .into(),
                d_model: 128,
                d_ff: 256,
                n_blocks: 4,
                state_slots: 128,
                n_experts: 4,
            },
            "CUSTOM" => Self {
                name: "CUSTOM".into(),
                description: "custom topology from CLI flags".into(),
                d_model: args.d_model,
                d_ff: args.d_ff,
                n_blocks: args.n_blocks,
                state_slots: args.state_slots,
                n_experts: args.n_experts,
            },
            other => return Err(format!("unknown arm {other}").into()),
        };
        if !spec.state_slots.is_multiple_of(MT4_DECAYS.len()) {
            return Err("state_slots must be divisible by 4 for the MT4 bands".into());
        }
        Ok(spec)
    }

    fn is_moe(&self) -> bool {
        self.n_experts > 1
    }

    /// Per-token matvec MACs actually spent at inference (top-1: one expert).
    fn macs_per_token(&self) -> usize {
        let state = 2 * self.state_slots * self.d_model;
        let ffn_active = self.n_blocks * 2 * self.d_ff * self.d_model;
        let router = if self.is_moe() {
            self.n_blocks * self.n_experts * self.d_model
        } else {
            0
        };
        let head = VOCAB * self.d_model;
        state + ffn_active + router + head
    }

    /// Ternary weights stored in ROM (all experts stored, one executed).
    fn stored_ternary_weights(&self) -> usize {
        let state = 2 * self.state_slots * self.d_model;
        let ffn_stored = self.n_blocks * self.n_experts * 2 * self.d_ff * self.d_model;
        state + ffn_stored
    }

    /// Per-output-row Q8.8 scale entries across all stored ternary matrices.
    fn scale_entries(&self) -> usize {
        let state = self.state_slots + self.d_model;
        let ffn = self.n_blocks * self.n_experts * (self.d_ff + self.d_model);
        state + ffn
    }

    /// fp router weights (deployed as int8 data, 1 B/weight assumption).
    fn router_weights(&self) -> usize {
        if self.is_moe() {
            self.n_blocks * self.n_experts * self.d_model
        } else {
            0
        }
    }

    fn deployment_json(&self) -> serde_json::Value {
        let macs = self.macs_per_token() as f64;
        let stored = self.stored_ternary_weights() as f64;
        let embedding_bytes = (VOCAB * self.d_model * 4) as f64; // f32_le as exported
        let scale_bytes = (self.scale_entries() * 2) as f64; // Q8.8 u16
        let decay_bytes = (self.state_slots * 2) as f64; // Q8.8 u16
        let router_bytes = self.router_weights() as f64; // int8 assumption
        let overhead = embedding_bytes + scale_bytes + decay_bytes + router_bytes;

        let lowering = |bytes_per_w: f64, shared_code: f64, cy_per_mac: f64| {
            let rom = stored * bytes_per_w + shared_code + overhead;
            let cycles = macs * cy_per_mac;
            let s_per_char = cycles / (GB_MCYCLES_PER_SEC * CPU_FRACTION);
            json!({
                "rom_bytes": rom.round() as u64,
                "rom_mib": rom / (1024.0 * 1024.0),
                "cycles_per_token": cycles.round() as u64,
                "s_per_char_at_70pct_cpu": s_per_char,
                "fits_rom_budget_7mib": rom <= ROM_BUDGET_BYTES,
            })
        };
        json!({
            "macs_per_token": self.macs_per_token(),
            "stored_ternary_weights": self.stored_ternary_weights(),
            "embedding_bytes_f32": embedding_bytes as u64,
            "scale_bytes_q8_8": scale_bytes as u64,
            "decay_bytes_q8_8": decay_bytes as u64,
            "router_bytes_int8_assumed": router_bytes as u64,
            "v3_weights_as_code": lowering(V3_BYTES_PER_WEIGHT, 0.0, V3_CY_PER_MAC),
            "v2_dispatch_data": lowering(V2_BYTES_PER_WEIGHT, V2_SHARED_CODE_BYTES, V2_CY_PER_MAC),
            "constants": {
                "v3_cy_per_mac": V3_CY_PER_MAC,
                "v3_bytes_per_weight": V3_BYTES_PER_WEIGHT,
                "v2_cy_per_mac": V2_CY_PER_MAC,
                "v2_bytes_per_weight": V2_BYTES_PER_WEIGHT,
                "v2_shared_code_bytes": V2_SHARED_CODE_BYTES,
                "source": "docs/experiments/kernel-bakeoff/kernel_bakeoff.json @ 40% zeros",
                "cpu_fraction": CPU_FRACTION,
                "rom_budget_bytes": ROM_BUDGET_BYTES as u64,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// deterministic init (identical scheme to s5_state_ab.rs)
// ---------------------------------------------------------------------------

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn init_weights(seed: u64, salt: u64, len: usize, scale: f32) -> Vec<f32> {
    let mut state = seed ^ salt.wrapping_mul(0xd1b5_4a32_d192_ed03);
    (0..len)
        .map(|_| {
            let bits = splitmix64(&mut state);
            let unit = (bits >> 40) as f32 / (1u64 << 24) as f32;
            (unit * 2.0 - 1.0) * scale
        })
        .collect()
}

// ---------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------

#[derive(BurnModule, Debug)]
struct FfnExpert<B: BurnBackend> {
    up: TernaryLinearBurnQat<B>,
    down: TernaryLinearBurnQat<B>,
}

#[derive(BurnModule, Debug)]
struct MixBlock<B: BurnBackend> {
    experts: Vec<FfnExpert<B>>,
    /// fp router [n_experts, d_model]; unused (len-0 experts never happen;
    /// dense blocks carry a 1-expert Vec and a zero-sized router is avoided
    /// by keeping a [1, d_model] tensor that is simply never used).
    router: Option<BurnParam<BurnFloatTensor<B, 2>>>,
}

#[derive(BurnModule, Debug)]
struct StateBlock<B: BurnBackend> {
    input_to_state: TernaryLinearBurnQat<B>,
    state_to_output: TernaryLinearBurnQat<B>,
}

#[derive(BurnModule, Debug)]
struct ArmModel<B: BurnBackend> {
    embedding: BurnParam<BurnFloatTensor<B, 2>>,
    state: StateBlock<B>,
    blocks: Vec<MixBlock<B>>,
}

fn make_ternary_core(out_rows: usize, in_cols: usize, weights: Vec<f32>) -> TernaryLinearQat {
    let shape = MatrixShape::new(out_rows, in_cols).expect("nonzero shape");
    let thresholds = vec![TernaryThreshold::new(0.0).expect("zero threshold"); out_rows];
    TernaryLinearQat::with_derived_per_row_scales(shape, weights, None, thresholds)
        .expect("valid ternary core")
}

impl ArmModel<Adiff> {
    fn init(spec: &ArmSpec, seed: u64, device: &BurnDevice<Adiff>) -> Self {
        let d_model = spec.d_model;
        let d_ff = spec.d_ff;
        let embedding = float_tensor_from_vec::<Adiff, 2>(
            init_weights(seed, 0x0001, VOCAB * d_model, 0.05),
            [VOCAB, d_model],
            device,
        )
        .expect("embedding init");

        let in_core = make_ternary_core(
            spec.state_slots,
            d_model,
            init_weights(seed, 0x2001, spec.state_slots * d_model, 0.08),
        );
        let out_core = make_ternary_core(
            d_model,
            spec.state_slots,
            init_weights(seed, 0x2002, d_model * spec.state_slots, 0.08),
        );
        let state = StateBlock {
            input_to_state: TernaryLinearBurnQat::from_core(in_core, device)
                .expect("state in-proj wrapper"),
            state_to_output: TernaryLinearBurnQat::from_core(out_core, device)
                .expect("state out-proj wrapper"),
        };

        let blocks = (0..spec.n_blocks)
            .map(|layer| {
                let experts = (0..spec.n_experts)
                    .map(|expert| {
                        let salt = 0x1000 + (layer as u64) * 64 + expert as u64;
                        let up_core = make_ternary_core(
                            d_ff,
                            d_model,
                            init_weights(seed, salt * 7 + 1, d_ff * d_model, 0.08),
                        );
                        let down_core = make_ternary_core(
                            d_model,
                            d_ff,
                            init_weights(seed, salt * 7 + 2, d_model * d_ff, 0.08),
                        );
                        FfnExpert {
                            up: TernaryLinearBurnQat::from_core(up_core, device)
                                .expect("up wrapper"),
                            down: TernaryLinearBurnQat::from_core(down_core, device)
                                .expect("down wrapper"),
                        }
                    })
                    .collect();
                let router = (spec.n_experts > 1).then(|| {
                    let salt = 0x9000 + layer as u64;
                    BurnParam::from_tensor(
                        float_tensor_from_vec::<Adiff, 2>(
                            init_weights(seed, salt, spec.n_experts * d_model, 0.02),
                            [spec.n_experts, d_model],
                            device,
                        )
                        .expect("router init"),
                    )
                });
                MixBlock { experts, router }
            })
            .collect();

        Self {
            embedding: BurnParam::from_tensor(embedding),
            state,
            blocks,
        }
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        self.state.input_to_state.set_hardness(hardness);
        self.state.state_to_output.set_hardness(hardness);
        for block in &mut self.blocks {
            for expert in &mut block.experts {
                expert.up.set_hardness(hardness);
                expert.down.set_hardness(hardness);
            }
        }
    }
}

fn activation() -> ActFakeQuantBurnQat {
    let core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-ACT_RANGE, ACT_RANGE).expect("range")),
        ActivationQuantFormat::Int8,
    )
    .expect("act");
    ActFakeQuantBurnQat::from_core(core).expect("act wrapper")
}

fn rms_norm<B: BurnBackend>(x: BurnFloatTensor<B, 2>) -> BurnFloatTensor<B, 2> {
    let d_model = x.dims()[1];
    let mean_sq = (x.clone() * x.clone()).mean_dim(1);
    let rms = (mean_sq + NORM_EPS).sqrt();
    let normed = x / rms.repeat_dim(1, d_model);
    normed.clamp(-NORM_CLIP, NORM_CLIP)
}

fn mt4_decay_per_slot(state_slots: usize) -> Vec<f32> {
    let policy =
        DecayPolicy::multi_timescale(MT4_DECAYS.to_vec()).expect("MT4 decay policy is valid");
    (0..state_slots)
        .map(|slot| policy.decay_for_slot(slot, state_slots))
        .collect()
}

// ---------------------------------------------------------------------------
// shared forward (dense + MoE)
// ---------------------------------------------------------------------------

enum BlockRefs<'a, B: BurnBackend> {
    Dense(&'a TernaryLinearBurnQat<B>, &'a TernaryLinearBurnQat<B>),
    Moe {
        experts: Vec<(&'a TernaryLinearBurnQat<B>, &'a TernaryLinearBurnQat<B>)>,
        router: BurnFloatTensor<B, 2>,
    },
}

struct ForwardRefs<'a, B: BurnBackend> {
    state: (&'a TernaryLinearBurnQat<B>, &'a TernaryLinearBurnQat<B>),
    blocks: Vec<BlockRefs<'a, B>>,
}

impl<'a> ForwardRefs<'a, Adiff> {
    fn from_model(model: &'a ArmModel<Adiff>) -> Self {
        Self {
            state: (&model.state.input_to_state, &model.state.state_to_output),
            blocks: model
                .blocks
                .iter()
                .map(|b| match &b.router {
                    Some(router) => BlockRefs::Moe {
                        experts: b.experts.iter().map(|e| (&e.up, &e.down)).collect(),
                        router: router.val(),
                    },
                    None => BlockRefs::Dense(&b.experts[0].up, &b.experts[0].down),
                })
                .collect(),
        }
    }
}

struct SeqOut<B: BurnBackend> {
    /// [batch*seq_len, VOCAB]; row t*batch + b = lane b at chunk position t.
    logits: BurnFloatTensor<B, 2>,
    final_state: BurnFloatTensor<B, 2>,
    /// Switch-style load-balance aux loss, summed over MoE blocks.
    router_aux: Option<BurnFloatTensor<B, 1>>,
    /// Per-MoE-block hard top-1 dispatch counts (host, stop-gradient).
    expert_counts: Vec<Vec<usize>>,
}

#[allow(clippy::too_many_arguments)]
fn forward_seq<B: BurnBackend>(
    embedding: BurnFloatTensor<B, 2>,
    refs: &ForwardRefs<'_, B>,
    act: &ActFakeQuantBurnQat,
    act_enabled: bool,
    ctx_ids: BurnTensor<B, 1, BurnInt>,
    batch: usize,
    seq_len: usize,
    init_state: BurnFloatTensor<B, 2>,
    decay_slots: &[f32],
    device: &BurnDevice<B>,
) -> Result<SeqOut<B>, Box<dyn std::error::Error>> {
    let mode = if act_enabled {
        ActivationForwardMode::Train
    } else {
        ActivationForwardMode::Passthrough
    };
    let mut x = embedding.clone().select(0, ctx_ids); // [batch*seq_len, d_model]
    let n_rows = batch * seq_len;
    let d_model = x.dims()[1];

    // --- LinearState MT4 block (identical semantics to s5_state_ab) ---
    let (in_proj, out_proj) = refs.state;
    let slots = decay_slots.len();
    let normed = rms_norm(x.clone());
    let normed = act.fake_quant_forward(normed, mode);
    let delta_all = in_proj.fake_quant_forward(normed)?; // [batch*seq_len, slots]
    let decay = float_tensor_from_vec::<B, 2>(decay_slots.to_vec(), [1, slots], device)?
        .repeat_dim(0, batch);
    let mut state = init_state;
    let mut rows = Vec::with_capacity(seq_len);
    for t in 0..seq_len {
        let delta_t = delta_all
            .clone()
            .slice([t * batch..(t + 1) * batch, 0..slots]);
        state = state * decay.clone() + delta_t;
        rows.push(state.clone());
    }
    let states_all = BurnFloatTensor::<B, 2>::cat(rows, 0);
    let projected = out_proj.fake_quant_forward(states_all)?;
    let y = act.fake_quant_forward(projected, mode);
    x = x + y; // residual around the state block
    let final_state = state;

    // --- FFN / MoE blocks ---
    let mut router_aux: Option<BurnFloatTensor<B, 1>> = None;
    let mut expert_counts = Vec::new();
    for block in &refs.blocks {
        match block {
            BlockRefs::Dense(up, down) => {
                let normed = rms_norm(x.clone());
                let normed = act.fake_quant_forward(normed, mode);
                let hidden = up.fake_quant_forward(normed)?;
                let hidden = burn_gelu_approximate(hidden);
                let hidden = act.fake_quant_forward(hidden, mode);
                let delta = down.fake_quant_forward(hidden)?;
                x = x + delta;
            }
            BlockRefs::Moe { experts, router } => {
                let n_experts = experts.len();
                let normed = rms_norm(x.clone());
                let normed = act.fake_quant_forward(normed, mode);
                // fp router over the quantized normed block input.
                let logits = normed.clone().matmul(router.clone().transpose()); // [n, E]
                let probs = burn_softmax(logits, 1);
                // HOST argmax: hard top-1 dispatch (stop-gradient provenance).
                let probs_host = float_tensor_into_vec(probs.clone().detach())?;
                let mut assign: Vec<Vec<i32>> = vec![Vec::new(); n_experts];
                for row in 0..n_rows {
                    let p = &probs_host[row * n_experts..(row + 1) * n_experts];
                    let mut best = 0usize;
                    for (e, v) in p.iter().enumerate() {
                        if *v > p[best] {
                            best = e;
                        }
                    }
                    assign[best].push(row as i32);
                }
                let counts: Vec<usize> = assign.iter().map(Vec::len).collect();
                // Dispatch: gather rows per expert, forward, scale by the
                // selected expert's routing probability (router gradient
                // path), scatter back.
                let mut out = BurnFloatTensor::<B, 2>::zeros([n_rows, d_model], device);
                for (e, (up, down)) in experts.iter().enumerate() {
                    if assign[e].is_empty() {
                        continue;
                    }
                    let ne = assign[e].len();
                    let idx = BurnTensor::<B, 1, BurnInt>::from_ints(assign[e].as_slice(), device);
                    let xe = normed.clone().select(0, idx.clone());
                    let he = up.fake_quant_forward(xe)?;
                    let he = burn_gelu_approximate(he);
                    let he = act.fake_quant_forward(he, mode);
                    let ye = down.fake_quant_forward(he)?;
                    let pe = probs
                        .clone()
                        .select(0, idx.clone())
                        .slice([0..ne, e..e + 1]); // [ne, 1]
                    let ye = ye * pe.repeat_dim(1, d_model);
                    // Add into a zeros tensor with disjoint row sets == assign.
                    out = out.select_assign(0, idx, ye, burn::tensor::IndexingUpdateOp::Add);
                }
                x = x + out;
                // Switch load-balance: aux = E * sum_e f_e * P_e, where f_e is
                // the (host, stop-gradient) hard dispatch fraction and P_e the
                // mean routing probability (gradient reaches routing
                // probabilities and router weights through P_e and the top-1
                // probability scaling above; NOT through the argmax itself).
                let f: Vec<f32> = counts.iter().map(|&c| c as f32 / n_rows as f32).collect();
                let f_t = float_tensor_from_vec::<B, 2>(f, [1, n_experts], device)?;
                let p_mean = probs.mean_dim(0); // [1, E]
                let aux = (p_mean * f_t).sum() * (n_experts as f32);
                router_aux = Some(match router_aux {
                    Some(prev) => prev + aux,
                    None => aux,
                });
                expert_counts.push(counts);
            }
        }
    }
    let normed = rms_norm(x);
    Ok(SeqOut {
        logits: normed.matmul(embedding.transpose()),
        final_state,
        router_aux,
        expert_counts,
    })
}

// ---------------------------------------------------------------------------
// data (identical to s5_state_ab)
// ---------------------------------------------------------------------------

fn utf8_prefix(bytes: &[u8], cap: usize) -> Result<(&[u8], usize), Box<dyn std::error::Error>> {
    let take = bytes.len().min(cap);
    let slice = &bytes[..take];
    match std::str::from_utf8(slice) {
        Ok(_) => Ok((slice, 0)),
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            Ok((&slice[..valid], take - valid))
        }
        Err(error) => Err(format!("stream is not valid UTF-8: {error}").into()),
    }
}

fn build_val_bytes(
    repo_root: &Path,
    cap: usize,
) -> Result<(Vec<u8>, Vec<u64>), Box<dyn std::error::Error>> {
    let splits_path = repo_root.join("corpus/gutenberg/splits.json");
    let splits: serde_json::Value = serde_json::from_slice(
        &fs::read(&splits_path).map_err(|e| format!("read {}: {e}", splits_path.display()))?,
    )?;
    let val_ids = splits["val"]
        .as_array()
        .ok_or("splits.json missing val array")?
        .iter()
        .filter_map(|v| v.as_u64())
        .collect::<Vec<_>>();

    let mut bytes = Vec::with_capacity(cap.min(4 * 1024 * 1024));
    let mut used_ids = Vec::new();
    for id in &val_ids {
        if bytes.len() >= cap {
            break;
        }
        let body_path = repo_root
            .join("corpus/gutenberg/bodies")
            .join(id.to_string())
            .join("body.txt");
        let Ok(body) = fs::read(&body_path) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }
        used_ids.push(*id);
        let remaining = cap - bytes.len();
        bytes.extend_from_slice(&body[..body.len().min(remaining)]);
    }
    if bytes.len() < 2 {
        return Err("assembled validation stream is too small".into());
    }
    Ok((bytes, used_ids))
}

fn id_to_char(id: u8) -> char {
    match id {
        0..=25 => (b'A' + id) as char,
        26..=51 => (b'a' + (id - 26)) as char,
        52..=61 => (b'0' + (id - 52)) as char,
        62 => ' ',
        63 => '.',
        64 => ',',
        65 => '!',
        66 => '?',
        67 => '-',
        68 => '\'',
        69 => ':',
        70 => ';',
        71 => '(',
        72 => ')',
        73 => '"',
        74 => '/',
        75 => '\n',
        _ => '\u{FFFD}',
    }
}

// ---------------------------------------------------------------------------
// snapshots + eval
// ---------------------------------------------------------------------------

struct SnapBlock {
    experts: Vec<(TernaryLinearQat, TernaryLinearQat)>,
    router: Option<Vec<f32>>,
}

struct ArmSnapshot {
    spec: ArmSpec,
    embedding: Vec<f32>,
    state_cores: (TernaryLinearQat, TernaryLinearQat),
    blocks: Vec<SnapBlock>,
}

fn extract_snapshot(
    spec: &ArmSpec,
    model: &ArmModel<Adiff>,
) -> Result<ArmSnapshot, Box<dyn std::error::Error>> {
    let embedding = float_tensor_into_vec(model.embedding.val().inner().detach())?;
    let state_cores = (
        model.state.input_to_state.to_core_from_trained_state()?,
        model.state.state_to_output.to_core_from_trained_state()?,
    );
    let mut blocks = Vec::new();
    for block in &model.blocks {
        let mut experts = Vec::new();
        for expert in &block.experts {
            experts.push((
                expert.up.to_core_from_trained_state()?,
                expert.down.to_core_from_trained_state()?,
            ));
        }
        let router = match &block.router {
            Some(r) => Some(float_tensor_into_vec(r.val().inner().detach())?),
            None => None,
        };
        blocks.push(SnapBlock { experts, router });
    }
    Ok(ArmSnapshot {
        spec: spec.clone(),
        embedding,
        state_cores,
        blocks,
    })
}

struct PlainBlock {
    experts: Vec<(TernaryLinearBurnQat<Plain>, TernaryLinearBurnQat<Plain>)>,
    router: Option<BurnFloatTensor<Plain, 2>>,
}

struct PlainLayers {
    state: (TernaryLinearBurnQat<Plain>, TernaryLinearBurnQat<Plain>),
    blocks: Vec<PlainBlock>,
}

impl PlainLayers {
    fn build(
        snapshot: &ArmSnapshot,
        hardness: QuantHardness,
        device: &BurnDevice<Plain>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let harden = |core: &TernaryLinearQat| -> Result<
            TernaryLinearBurnQat<Plain>,
            Box<dyn std::error::Error>,
        > {
            let mut c = core.clone();
            c.set_hardness(hardness);
            Ok(TernaryLinearBurnQat::<Plain>::from_core(c, device)?)
        };
        let state = (
            harden(&snapshot.state_cores.0)?,
            harden(&snapshot.state_cores.1)?,
        );
        let mut blocks = Vec::new();
        for block in &snapshot.blocks {
            let mut experts = Vec::new();
            for (up, down) in &block.experts {
                experts.push((harden(up)?, harden(down)?));
            }
            let router = match &block.router {
                Some(r) => Some(float_tensor_from_vec::<Plain, 2>(
                    r.clone(),
                    [snapshot.spec.n_experts, snapshot.spec.d_model],
                    device,
                )?),
                None => None,
            };
            blocks.push(PlainBlock { experts, router });
        }
        Ok(Self { state, blocks })
    }

    fn refs(&self) -> ForwardRefs<'_, Plain> {
        ForwardRefs {
            state: (&self.state.0, &self.state.1),
            blocks: self
                .blocks
                .iter()
                .map(|b| match &b.router {
                    Some(router) => BlockRefs::Moe {
                        experts: b.experts.iter().map(|(u, d)| (u, d)).collect(),
                        router: router.clone(),
                    },
                    None => BlockRefs::Dense(&b.experts[0].0, &b.experts[0].1),
                })
                .collect(),
        }
    }
}

/// Lane-parallel validation bpc (identical layout to s5_state_ab).
#[allow(clippy::too_many_arguments)]
fn eval_bpc_lanes(
    snapshot: &ArmSnapshot,
    decay_slots: &[f32],
    hardness: QuantHardness,
    act_enabled: bool,
    val_ids: &[u8],
    max_pairs: usize,
    lanes: usize,
    chunk: usize,
    device: &BurnDevice<Plain>,
) -> Result<(f64, usize), Box<dyn std::error::Error>> {
    let layers = PlainLayers::build(snapshot, hardness, device)?;
    let refs = layers.refs();
    let act = activation();
    let d_model = snapshot.spec.d_model;
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;

    let usable = if max_pairs == 0 {
        val_ids.len()
    } else {
        val_ids.len().min(max_pairs + lanes)
    };
    let lane_len = usable / lanes;
    if lane_len < 2 {
        return Err("validation stream too small for the lane layout".into());
    }
    let pairs_per_lane = lane_len - 1;

    let slots = decay_slots.len();
    let mut state = BurnFloatTensor::<Plain, 2>::zeros([lanes, slots], device);

    let ln2 = std::f64::consts::LN_2;
    let mut total_bits = 0.0_f64;
    let mut done = 0usize;
    while done < pairs_per_lane {
        let this = chunk.min(pairs_per_lane - done);
        let mut ctx = Vec::with_capacity(this * lanes);
        let mut tgt = Vec::with_capacity(this * lanes);
        for t in 0..this {
            for lane in 0..lanes {
                let base = lane * lane_len + done + t;
                ctx.push(val_ids[base] as i32);
                tgt.push(val_ids[base + 1] as i32);
            }
        }
        let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
        let out = forward_seq(
            embed.clone(),
            &refs,
            &act,
            act_enabled,
            ctx_ids,
            lanes,
            this,
            state,
            decay_slots,
            device,
        )?;
        state = out.final_state;
        let logp = burn_log_softmax(out.logits, 1);
        let tgt_idx = BurnTensor::<Plain, 1, BurnInt>::from_ints(tgt.as_slice(), device)
            .reshape([this * lanes, 1]);
        let picked = logp.gather(1, tgt_idx).reshape([this * lanes]);
        for v in float_tensor_into_vec(picked)? {
            total_bits += -(v as f64) / ln2;
        }
        done += this;
    }
    Ok((total_bits, pairs_per_lane * lanes))
}

fn greedy_sample(
    snapshot: &ArmSnapshot,
    decay_slots: &[f32],
    prompt_ids: &[u8],
    sample_chars: usize,
    device: &BurnDevice<Plain>,
) -> Result<String, Box<dyn std::error::Error>> {
    let layers = PlainLayers::build(snapshot, QuantHardness::Hard, device)?;
    let refs = layers.refs();
    let act = activation();
    let d_model = snapshot.spec.d_model;
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;
    let slots = decay_slots.len();
    let mut state = BurnFloatTensor::<Plain, 2>::zeros([1, slots], device);

    let ctx: Vec<i32> = prompt_ids.iter().map(|&id| id as i32).collect();
    let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
    let out = forward_seq(
        embed.clone(),
        &refs,
        &act,
        true,
        ctx_ids,
        1,
        prompt_ids.len(),
        state,
        decay_slots,
        device,
    )?;
    state = out.final_state;
    let mut last = argmax_last_row(out.logits)?;

    let mut text = String::with_capacity(sample_chars);
    for _ in 0..sample_chars {
        text.push(id_to_char(last));
        let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints([last as i32].as_slice(), device);
        let out = forward_seq(
            embed.clone(),
            &refs,
            &act,
            true,
            ctx_ids,
            1,
            1,
            state,
            decay_slots,
            device,
        )?;
        state = out.final_state;
        last = argmax_last_row(out.logits)?;
    }
    Ok(text)
}

fn argmax_last_row(logits: BurnFloatTensor<Plain, 2>) -> Result<u8, Box<dyn std::error::Error>> {
    let dims = logits.dims();
    let last = logits.slice([dims[0] - 1..dims[0], 0..dims[1]]);
    let values = float_tensor_into_vec(last)?;
    let (best, _) =
        values
            .iter()
            .enumerate()
            .fold((0usize, f32::NEG_INFINITY), |(bi, bv), (i, &v)| {
                if v > bv { (i, v) } else { (bi, bv) }
            });
    Ok(best as u8)
}

/// Batched-Burn vs canonical-scalar-kernel parity on the state block
/// (identical methodology to s5_state_ab).
fn state_block_parity(
    snapshot: &ArmSnapshot,
    decay_slots: &[f32],
    token_ids: &[u8],
    device: &BurnDevice<Plain>,
) -> Result<f64, Box<dyn std::error::Error>> {
    let d_model = snapshot.spec.d_model;
    let state_slots = snapshot.spec.state_slots;
    let (in_core, out_core) = (&snapshot.state_cores.0, &snapshot.state_cores.1);
    let mut in_hard = in_core.clone();
    let mut out_hard = out_core.clone();
    in_hard.set_hardness(QuantHardness::Hard);
    out_hard.set_hardness(QuantHardness::Hard);
    let act_core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-ACT_RANGE, ACT_RANGE)?),
        ActivationQuantFormat::Int8,
    )?;

    let mut scalar_state = vec![0.0_f32; state_slots];
    let mut scalar_outputs = Vec::new();
    for &id in token_ids {
        let x: Vec<f32> =
            snapshot.embedding[id as usize * d_model..(id as usize + 1) * d_model].to_vec();
        let mean_sq = x.iter().map(|v| v * v).sum::<f32>() / d_model as f32;
        let rms = (mean_sq + NORM_EPS).sqrt();
        let normed: Vec<f32> = x
            .iter()
            .map(|v| (v / rms).clamp(-NORM_CLIP, NORM_CLIP))
            .collect();
        let activated = act_core.inference_forward(&normed, ActivationForwardMode::Train)?;
        let delta = in_hard.inference_forward(&activated)?;
        for (slot, (s, d)) in scalar_state.iter_mut().zip(delta.iter()).enumerate() {
            *s = *s * decay_slots[slot] + *d;
        }
        let projected = out_hard.inference_forward(&scalar_state)?;
        scalar_outputs
            .extend(act_core.inference_forward(&projected, ActivationForwardMode::Train)?);
    }

    let layers = PlainLayers::build(snapshot, QuantHardness::Hard, device)?;
    let refs = layers.refs();
    let (in_proj, out_proj) = refs.state;
    let act = activation();
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;
    let ctx: Vec<i32> = token_ids.iter().map(|&id| id as i32).collect();
    let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
    let x = embed.select(0, ctx_ids);
    let normed = rms_norm(x);
    let normed = act.fake_quant_forward(normed, ActivationForwardMode::Train);
    let delta_all = in_proj.fake_quant_forward(normed)?;
    let decay = float_tensor_from_vec::<Plain, 2>(decay_slots.to_vec(), [1, state_slots], device)?;
    let mut state = BurnFloatTensor::<Plain, 2>::zeros([1, state_slots], device);
    let mut rows = Vec::new();
    for t in 0..token_ids.len() {
        let delta_t = delta_all.clone().slice([t..t + 1, 0..state_slots]);
        state = state * decay.clone() + delta_t;
        rows.push(state.clone());
    }
    let states_all = BurnFloatTensor::<Plain, 2>::cat(rows, 0);
    let projected = out_proj.fake_quant_forward(states_all)?;
    let burn_outputs =
        float_tensor_into_vec(act.fake_quant_forward(projected, ActivationForwardMode::Train))?;
    let burn_state = float_tensor_into_vec(state)?;

    let mut max_diff = 0.0_f64;
    for (a, b) in scalar_outputs.iter().zip(burn_outputs.iter()) {
        max_diff = max_diff.max((f64::from(*a) - f64::from(*b)).abs());
    }
    for (a, b) in scalar_state.iter().zip(burn_state.iter()) {
        max_diff = max_diff.max((f64::from(*a) - f64::from(*b)).abs());
    }
    if scalar_outputs.len() != burn_outputs.len() {
        return Err("parity output length mismatch".into());
    }
    Ok(max_diff)
}

// ---------------------------------------------------------------------------
// training
// ---------------------------------------------------------------------------

/// fp teacher context for distillation (Plain backend, hardness Off,
/// activation fake-quant passthrough: a genuine full-precision model).
struct TeacherCtx {
    spec: ArmSpec,
    layers: PlainLayers,
    embed: BurnFloatTensor<Plain, 2>,
    decay_slots: Vec<f32>,
    carried_state: Vec<f32>,
    temperature: f64,
    weight: f64,
}

struct TrainOutcome {
    snapshot: ArmSnapshot,
    steps_per_second: f64,
    train_wall_seconds: f64,
    final_train_loss_bpc: f64,
    tokens_per_step: usize,
    expert_usage_last_window: Option<Vec<Vec<f64>>>,
}

/// Train one arm. `fp_only` keeps QAT hardness Off and activation fake-quant
/// passthrough for ALL steps (used for the distillation teacher).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn train_arm(
    args: &Args,
    spec: &ArmSpec,
    steps: u64,
    fp_only: bool,
    mut teacher: Option<&mut TeacherCtx>,
    train_ids: &[u8],
    decay_slots: &[f32],
    device: &BurnDevice<Adiff>,
    plain_device: &BurnDevice<Plain>,
) -> Result<TrainOutcome, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut model = ArmModel::init(spec, args.seed, device);
    let mut optimizer = adamw_config()
        .with_weight_decay(0.0)
        .init::<Adiff, ArmModel<Adiff>>();
    let act = activation();
    let warmup_steps = if fp_only {
        steps + 1 // never harden
    } else {
        (steps as f64 * args.warmup_frac) as u64
    };

    let seq_len = args.seq_len;
    let lanes = args.lanes;
    let tokens_per_step = seq_len * lanes;
    println!(
        "[{}] d_model={} d_ff={} n_blocks={} experts={} slots={} steps={steps} seq_len={seq_len} lanes={lanes} tokens/step={tokens_per_step} lr={} fp_only={fp_only} distill={} seed={}",
        spec.name,
        spec.d_model,
        spec.d_ff,
        spec.n_blocks,
        spec.n_experts,
        spec.state_slots,
        args.lr,
        teacher.is_some(),
        args.seed,
    );

    let mut sampler = args.seed ^ 0xa5a5_5a5a_1234_5678;
    let mut lane_pos: Vec<usize> = (0..lanes)
        .map(|_| (splitmix64(&mut sampler) as usize) % (train_ids.len() - seq_len - 1))
        .collect();
    let mut carried_state = vec![0.0_f32; lanes * spec.state_slots];
    if let Some(t) = teacher.as_deref_mut() {
        t.carried_state = vec![0.0_f32; lanes * t.spec.state_slots];
    }

    let ln2 = std::f64::consts::LN_2;
    let mut running_loss = 0.0_f64;
    let mut running_n = 0u64;
    let mut last_logged_loss = f64::NAN;
    let mut current_hard = false;
    model.set_hardness(QuantHardness::Off);
    let mut usage_window: Vec<Vec<usize>> = vec![vec![0; spec.n_experts]; spec.n_blocks];
    let mut usage_last: Option<Vec<Vec<f64>>> = None;

    for step in 1..=steps {
        let want_hard = step > warmup_steps;
        if want_hard != current_hard {
            model.set_hardness(if want_hard {
                QuantHardness::Hard
            } else {
                QuantHardness::Off
            });
            current_hard = want_hard;
            println!(
                "[{}] step {step}: QAT hardness -> {}",
                spec.name,
                if want_hard { "Hard" } else { "Off" }
            );
        }
        let act_enabled = want_hard;

        let mut ctx = Vec::with_capacity(tokens_per_step);
        let mut tgt = Vec::with_capacity(tokens_per_step);
        for (lane_index, lane) in lane_pos.iter_mut().enumerate() {
            if *lane + seq_len + 1 > train_ids.len() {
                *lane = (splitmix64(&mut sampler) as usize) % (train_ids.len() - seq_len - 1);
                let band = lane_index * spec.state_slots..(lane_index + 1) * spec.state_slots;
                carried_state[band].fill(0.0);
                if let Some(t) = teacher.as_deref_mut() {
                    let tb = lane_index * t.spec.state_slots..(lane_index + 1) * t.spec.state_slots;
                    t.carried_state[tb].fill(0.0);
                }
            }
        }
        for t in 0..seq_len {
            for &pos in &lane_pos {
                ctx.push(train_ids[pos + t] as i32);
                tgt.push(train_ids[pos + t + 1] as i32);
            }
        }
        for pos in lane_pos.iter_mut() {
            *pos += seq_len;
        }

        let ctx_ids = BurnTensor::<Adiff, 1, BurnInt>::from_ints(ctx.as_slice(), device);
        let tgt_idx = BurnTensor::<Adiff, 1, BurnInt>::from_ints(tgt.as_slice(), device)
            .reshape([tokens_per_step, 1]);

        let refs = ForwardRefs::from_model(&model);
        let init_state = float_tensor_from_vec::<Adiff, 2>(
            carried_state.clone(),
            [lanes, spec.state_slots],
            device,
        )?;
        let out = forward_seq(
            model.embedding.val(),
            &refs,
            &act,
            act_enabled,
            ctx_ids,
            lanes,
            seq_len,
            init_state,
            decay_slots,
            device,
        )?;
        let logp = burn_log_softmax(out.logits.clone(), 1);
        let picked = logp.gather(1, tgt_idx).reshape([tokens_per_step]);
        let ce = picked.mean() * -1.0;

        let mut loss = ce.clone();
        // Router load-balance aux (MoE only).
        if let Some(aux) = out.router_aux {
            loss = loss + aux * (args.router_aux_weight as f32);
        }
        for (block, counts) in usage_window.iter_mut().zip(out.expert_counts.iter()) {
            for (acc, c) in block.iter_mut().zip(counts.iter()) {
                *acc += c;
            }
        }

        // Distillation: soft cross-entropy against fp teacher logits.
        if let Some(t) = teacher.as_deref_mut() {
            let t_ctx = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), plain_device);
            let t_init = float_tensor_from_vec::<Plain, 2>(
                t.carried_state.clone(),
                [lanes, t.spec.state_slots],
                plain_device,
            )?;
            let t_refs = t.layers.refs();
            let t_out = forward_seq(
                t.embed.clone(),
                &t_refs,
                &act,
                false, // fp teacher: activation fake-quant passthrough
                t_ctx,
                lanes,
                seq_len,
                t_init,
                &t.decay_slots,
                plain_device,
            )?;
            t.carried_state = float_tensor_into_vec(t_out.final_state)?;
            let t_logits_host = float_tensor_into_vec(t_out.logits)?;
            let temp = t.temperature as f32;
            let t_logits =
                float_tensor_from_vec::<Adiff, 2>(t_logits_host, [tokens_per_step, VOCAB], device)?;
            let q = burn_softmax(t_logits / temp, 1); // constant (from data)
            let s_logp = burn_log_softmax(out.logits / temp, 1);
            let soft_ce = (q * s_logp).sum_dim(1).mean() * -1.0;
            let distill = soft_ce * (temp * temp) * (t.weight as f32);
            loss = loss + distill;
        }

        let ce_nats = float_tensor_into_vec(ce.inner())?[0];
        let loss_nats = float_tensor_into_vec(loss.clone().inner())?[0];
        if !loss_nats.is_finite() {
            return Err(format!("[{}] non-finite training loss at step {step}", spec.name).into());
        }
        running_loss += f64::from(ce_nats);
        running_n += 1;

        carried_state = float_tensor_into_vec(out.final_state.inner().detach())?;

        let grads = loss.backward();
        let grads = BurnGradientsParams::from_grads(grads, &model);
        model = optimizer.step(args.lr, model, grads);

        if step % args.log_every == 0 || step == 1 {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = step as f64 / elapsed;
            let mean_loss = running_loss / running_n.max(1) as f64;
            last_logged_loss = mean_loss / ln2;
            let usage_str = if spec.is_moe() {
                let total: usize = usage_window[0].iter().sum();
                let fr: Vec<String> = usage_window
                    .iter()
                    .map(|b| {
                        let fracs: Vec<String> = b
                            .iter()
                            .map(|&c| format!("{:.2}", c as f64 / total.max(1) as f64))
                            .collect();
                        format!("[{}]", fracs.join(","))
                    })
                    .collect();
                usage_last = Some(
                    usage_window
                        .iter()
                        .map(|b| b.iter().map(|&c| c as f64 / total.max(1) as f64).collect())
                        .collect(),
                );
                usage_window = vec![vec![0; spec.n_experts]; spec.n_blocks];
                format!(" expert_use={}", fr.join(""))
            } else {
                String::new()
            };
            println!(
                "[{}] step {step}/{steps} ce_bpc~={:.4} {:.2} steps/s elapsed={:.0}s{usage_str}",
                spec.name, last_logged_loss, rate, elapsed
            );
            running_loss = 0.0;
            running_n = 0;
        }
    }

    let train_wall_seconds = started.elapsed().as_secs_f64();
    let snapshot = extract_snapshot(spec, &model)?;
    Ok(TrainOutcome {
        snapshot,
        steps_per_second: steps as f64 / train_wall_seconds,
        train_wall_seconds,
        final_train_loss_bpc: last_logged_loss,
        tokens_per_step,
        expert_usage_last_window: usage_last,
    })
}

// ---------------------------------------------------------------------------
// shared data + measurement plumbing
// ---------------------------------------------------------------------------

struct DataCtx {
    train_ids: Vec<u8>,
    val_ids: Vec<u8>,
    corpus_json: serde_json::Value,
    chars_per_raw_byte: f64,
    prompt_ids: Vec<u8>,
    prompt_text: String,
    git_sha: String,
}

fn load_data(args: &Args) -> Result<DataCtx, Box<dyn std::error::Error>> {
    let repo_root = &args.repo_root;
    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let train_path = repo_root.join(&args.train_bin);
    let train_all = fs::read(&train_path)
        .map_err(|e| format!("read train bin {}: {e}", train_path.display()))?;
    let (train_prefix, train_trimmed) = utf8_prefix(&train_all, args.train_cap_bytes)?;
    let train_raw_sha = sha256(train_prefix).to_hex();
    let train_norm = normalize_raw(train_prefix)?;
    let train_ids = train_norm.tokens.into_vec();
    let train_norm_sha = sha256(train_ids.as_slice()).to_hex();

    let (val_raw, val_book_ids) = build_val_bytes(repo_root, args.val_cap_bytes)?;
    let val_raw_sha = sha256(&val_raw).to_hex();
    let (val_prefix, val_trimmed) = utf8_prefix(&val_raw, val_raw.len())?;
    let val_norm = normalize_raw(val_prefix)?;
    let val_ids = val_norm.tokens.into_vec();
    let val_norm_sha = sha256(val_ids.as_slice()).to_hex();
    let val_chars_total = val_ids.len();
    let val_raw_bytes_total = val_prefix.len();
    let chars_per_raw_byte = val_chars_total as f64 / val_raw_bytes_total as f64;

    println!(
        "[data] train: {} raw bytes ({} trimmed) -> {} tokens, raw sha {}.. | val: {} raw bytes from books {:?} -> {} tokens, raw sha {}..",
        train_prefix.len(),
        train_trimmed,
        train_ids.len(),
        &train_raw_sha[..16],
        val_raw_bytes_total,
        val_book_ids,
        val_chars_total,
        &val_raw_sha[..16],
    );

    let prompt_norm = normalize_raw(args.sample_prompt.as_bytes())?;
    let prompt_ids = prompt_norm.tokens.into_vec();
    let prompt_text: String = prompt_ids.iter().map(|&id| id_to_char(id)).collect();

    let corpus_json = json!({
        "train_bin_path": args.train_bin,
        "train_cap_bytes": args.train_cap_bytes,
        "train_raw_bytes_used": train_prefix.len(),
        "train_bytes_trimmed_at_utf8_boundary": train_trimmed,
        "train_prefix_raw_sha256": train_raw_sha,
        "train_chars_normalized": train_ids.len(),
        "train_unk_count": train_norm.unk_count_in_example,
        "train_norm_tokens_sha256": train_norm_sha,
        "val_source": "corpus/gutenberg/splits.json val-split book bodies, same assembly as s2/s4/s5",
        "val_book_ids_used": val_book_ids,
        "val_raw_bytes_used": val_raw_bytes_total,
        "val_bytes_trimmed_at_utf8_boundary": val_trimmed,
        "val_raw_bytes_sha256": val_raw_sha,
        "val_chars_normalized": val_chars_total,
        "val_unk_count": val_norm.unk_count_in_example,
        "val_norm_tokens_sha256": val_norm_sha,
    });

    Ok(DataCtx {
        train_ids,
        val_ids,
        corpus_json,
        chars_per_raw_byte,
        prompt_ids,
        prompt_text,
        git_sha,
    })
}

/// Evaluate a snapshot in Soft+Hard semantics, write the sample, and return
/// the full per-arm JSON block.
#[allow(clippy::too_many_arguments)]
fn measure_arm(
    args: &Args,
    data: &DataCtx,
    spec: &ArmSpec,
    outcome: &TrainOutcome,
    decay_slots: &[f32],
    label: &str,
    extra_config: serde_json::Value,
    out_dir: &Path,
    plain_device: &BurnDevice<Plain>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    println!("[{label}] eval: Soft (fp STE ceiling) and Hard (deployable ternary) ...");
    let eval_start = Instant::now();
    let (fp_bits, fp_pairs) = eval_bpc_lanes(
        &outcome.snapshot,
        decay_slots,
        QuantHardness::Soft,
        true,
        &data.val_ids,
        args.eval_pairs,
        args.eval_lanes,
        args.eval_chunk,
        plain_device,
    )?;
    let (hard_bits, hard_pairs) = eval_bpc_lanes(
        &outcome.snapshot,
        decay_slots,
        QuantHardness::Hard,
        true,
        &data.val_ids,
        args.eval_pairs,
        args.eval_lanes,
        args.eval_chunk,
        plain_device,
    )?;
    assert_eq!(fp_pairs, hard_pairs, "eval pair sets must match");
    let eval_wall = eval_start.elapsed().as_secs_f64();
    let fp_bpc = fp_bits / fp_pairs as f64;
    let hard_bpc = hard_bits / hard_pairs as f64;
    let gap = hard_bpc - fp_bpc;
    let fp_bpb = fp_bpc * data.chars_per_raw_byte;
    let hard_bpb = hard_bpc * data.chars_per_raw_byte;
    println!(
        "[{label}] fp_bpc/char={fp_bpc:.6} hard_bpc/char={hard_bpc:.6} gap={gap:.6} | per raw byte: fp={fp_bpb:.6} hard={hard_bpb:.6} ({hard_pairs} pairs, eval {eval_wall:.0}s)"
    );

    let parity = state_block_parity(
        &outcome.snapshot,
        decay_slots,
        &data.val_ids[..32.min(data.val_ids.len())],
        plain_device,
    )?;
    println!("[{label}] state-block scalar-kernel parity max_abs_diff={parity:.3e}");

    let sample = greedy_sample(
        &outcome.snapshot,
        decay_slots,
        &data.prompt_ids,
        args.sample_chars,
        plain_device,
    )?;
    let sample_path = out_dir.join(format!("sample_{label}.txt"));
    fs::write(
        &sample_path,
        format!(
            "PROMPT (charset_v1-normalized):\n{}\n\nGREEDY CONTINUATION ({} chars, hard ternary):\n{sample}\n",
            data.prompt_text, args.sample_chars
        ),
    )?;
    println!("[{label}] sample -> {}", sample_path.display());

    Ok(json!({
        "arm": label,
        "description": spec.description,
        "status": "ok",
        "git_sha": data.git_sha,
        "corpus": data.corpus_json,
        "config": {
            "d_model": spec.d_model,
            "d_ff": spec.d_ff,
            "n_blocks": spec.n_blocks,
            "n_experts_per_block": spec.n_experts,
            "state_slots": spec.state_slots,
            "vocab": VOCAB,
            "tied_head": true,
            "sequence_state_kind": "linear_state_multi_timescale",
            "decay_rates_by_band": MT4_DECAYS,
            "seq_len": args.seq_len,
            "lanes": args.lanes,
            "tokens_per_step": outcome.tokens_per_step,
            "steps": args.steps,
            "lr": args.lr,
            "warmup_frac_hardness_off": args.warmup_frac,
            "qat_recipe": "warmup Off then Hard, act Int8 fake-quant when hard (identical to s5_state_ab)",
            "tbptt": "state carried across chunks detached; lane reset to zero state on stream wrap",
            "router": spec.is_moe().then_some(
                "simplified fp linear top-1 router [n_experts, d_model] over the quantized normed block input; HOST argmax hard dispatch (stop-gradient provenance); gradients reach routing probabilities via top-1 prob output scaling + Switch load-balance P_e term; NOT the gbf-model Top1RouterQat low-rank core"),
            "router_aux_weight": spec.is_moe().then_some(args.router_aux_weight),
            "extra": extra_config,
        },
        "training": {
            "steps_per_second": outcome.steps_per_second,
            "train_wall_clock_seconds": outcome.train_wall_seconds,
            "final_logged_train_ce_bpc": outcome.final_train_loss_bpc,
            "expert_usage_fraction_last_log_window_per_block": outcome.expert_usage_last_window,
        },
        "measurement": {
            "eval_pairs": hard_pairs,
            "eval_lanes": args.eval_lanes,
            "fp_val_bpc_per_normalized_char": fp_bpc,
            "ternary_val_bpc_per_normalized_char": hard_bpc,
            "gap_bpc_per_normalized_char": gap,
            "fp_val_bits_per_raw_byte": fp_bpb,
            "ternary_val_bits_per_raw_byte": hard_bpb,
            "bits_per_raw_byte_method": "bpc_per_normalized_char * (val_chars_total / val_raw_bytes_total), the same total-bits-over-raw-byte-count re-expression as the KN-5 artifact",
            "fp_semantics": "SOFT continuous ternary relaxation with the same learned per-output-row Q8.8 scales + Int8 act fake-quant (the calibrated STE ceiling)",
            "ternary_semantics": "HARD ternary projection {-1,0,+1} with the same per-row Q8.8 scales + Int8 act fake-quant",
            "state_block_scalar_parity_max_abs_diff": parity,
            "eval_wall_clock_seconds": eval_wall,
        },
        "deployment": spec.deployment_json(),
        "sample": {
            "prompt_normalized": data.prompt_text,
            "greedy_continuation": sample,
            "sample_path": format!("{}/sample_{label}.txt", args.out_dir),
        },
    }))
}

// ---------------------------------------------------------------------------
// phases
// ---------------------------------------------------------------------------

fn phase_arm(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let spec = ArmSpec::resolve(args)?;
    let data = load_data(args)?;
    let device = BurnDevice::<Adiff>::default();
    let plain_device = BurnDevice::<Plain>::default();
    let decay_slots = mt4_decay_per_slot(spec.state_slots);
    let out_dir = args.repo_root.join(&args.out_dir);
    fs::create_dir_all(&out_dir)?;

    let label = spec.name.clone();
    let arm_json = match train_arm(
        args,
        &spec,
        args.steps,
        false,
        None,
        &data.train_ids,
        &decay_slots,
        &device,
        &plain_device,
    ) {
        Ok(outcome) => measure_arm(
            args,
            &data,
            &spec,
            &outcome,
            &decay_slots,
            &label,
            json!(null),
            &out_dir,
            &plain_device,
        )?,
        Err(err) => {
            eprintln!("[{label}] FAILED: {err}");
            json!({ "arm": label, "description": spec.description, "status": "failed", "error": err.to_string() })
        }
    };
    let path = out_dir.join(format!("arm_{label}.json"));
    fs::write(&path, serde_json::to_vec_pretty(&arm_json)?)?;
    println!("[write] {}", path.display());
    Ok(())
}

fn phase_distill(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let student_spec = ArmSpec::resolve(args)?;
    let data = load_data(args)?;
    let device = BurnDevice::<Adiff>::default();
    let plain_device = BurnDevice::<Plain>::default();
    let out_dir = args.repo_root.join(&args.out_dir);
    fs::create_dir_all(&out_dir)?;

    // --- fp teacher: dense, teacher_mult x width, same block count ---
    let teacher_spec = ArmSpec {
        name: format!("{}_teacher", student_spec.name),
        description: format!(
            "fp dense teacher for {}: {}x width, QAT hardness Off + act passthrough for all steps",
            student_spec.name, args.teacher_mult
        ),
        d_model: student_spec.d_model * args.teacher_mult,
        d_ff: student_spec.d_ff * args.teacher_mult,
        n_blocks: student_spec.n_blocks,
        state_slots: student_spec.state_slots * args.teacher_mult,
        n_experts: 1,
    };
    let teacher_decay = mt4_decay_per_slot(teacher_spec.state_slots);
    println!(
        "[distill] training fp teacher {} for {} steps",
        teacher_spec.description, args.teacher_steps
    );
    let teacher_outcome = train_arm(
        args,
        &teacher_spec,
        args.teacher_steps,
        true,
        None,
        &data.train_ids,
        &teacher_decay,
        &device,
        &plain_device,
    )?;
    // Teacher reference eval: fp semantics (hardness Off, act passthrough).
    let (t_bits, t_pairs) = eval_bpc_lanes(
        &teacher_outcome.snapshot,
        &teacher_decay,
        QuantHardness::Off,
        false,
        &data.val_ids,
        args.eval_pairs,
        args.eval_lanes,
        args.eval_chunk,
        &plain_device,
    )?;
    let t_bpc = t_bits / t_pairs as f64;
    let t_bpb = t_bpc * data.chars_per_raw_byte;
    println!("[distill] teacher fp val: {t_bpc:.6} bpc/char = {t_bpb:.6} bits/raw-byte");

    // --- distilled student (identical spec + steps to the Phase-A arm) ---
    let teacher_layers =
        PlainLayers::build(&teacher_outcome.snapshot, QuantHardness::Off, &plain_device)?;
    let teacher_embed = float_tensor_from_vec::<Plain, 2>(
        teacher_outcome.snapshot.embedding.clone(),
        [VOCAB, teacher_spec.d_model],
        &plain_device,
    )?;
    let mut teacher_ctx = TeacherCtx {
        spec: teacher_spec.clone(),
        layers: teacher_layers,
        embed: teacher_embed,
        decay_slots: teacher_decay.clone(),
        carried_state: Vec::new(),
        temperature: args.distill_temperature,
        weight: args.distill_weight,
    };
    let student_decay = mt4_decay_per_slot(student_spec.state_slots);
    let label = format!("{}_distill", student_spec.name);
    let student_outcome = train_arm(
        args,
        &student_spec,
        args.steps,
        false,
        Some(&mut teacher_ctx),
        &data.train_ids,
        &student_decay,
        &device,
        &plain_device,
    )?;
    let mut arm_json = measure_arm(
        args,
        &data,
        &student_spec,
        &student_outcome,
        &student_decay,
        &label,
        json!({
            "distillation": {
                "teacher": {
                    "description": teacher_spec.description,
                    "d_model": teacher_spec.d_model,
                    "d_ff": teacher_spec.d_ff,
                    "n_blocks": teacher_spec.n_blocks,
                    "state_slots": teacher_spec.state_slots,
                    "steps": args.teacher_steps,
                    "train_wall_clock_seconds": teacher_outcome.train_wall_seconds,
                    "steps_per_second": teacher_outcome.steps_per_second,
                    "fp_val_bpc_per_normalized_char": t_bpc,
                    "fp_val_bits_per_raw_byte": t_bpb,
                    "eval_pairs": t_pairs,
                },
                "temperature": args.distill_temperature,
                "weight": args.distill_weight,
                "loss": "CE + weight * T^2 * softCE(softmax(teacher/T) || softmax(student/T)), teacher logits recomputed per step on the training batch with per-lane teacher state carry",
            },
        }),
        &out_dir,
        &plain_device,
    )?;
    arm_json["arm"] = json!(label);
    let path = out_dir.join(format!("arm_{label}.json"));
    fs::write(&path, serde_json::to_vec_pretty(&arm_json)?)?;
    println!("[write] {}", path.display());
    Ok(())
}

fn phase_report(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = args.repo_root.join(&args.out_dir);
    let mut arms = Vec::new();
    for name in args.report_arms.split(',').map(str::trim) {
        if name.is_empty() {
            continue;
        }
        let path = out_dir.join(format!("arm_{name}.json"));
        match fs::read(&path) {
            Ok(raw) => arms.push(serde_json::from_slice::<serde_json::Value>(&raw)?),
            Err(err) => {
                eprintln!("[report] missing arm json {}: {err}", path.display());
                arms.push(json!({ "arm": name, "status": "missing", "error": err.to_string() }));
            }
        }
    }

    let kn_meta_path = args
        .repo_root
        .join("experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json");
    let kn_reference = match fs::read(&kn_meta_path) {
        Ok(raw) => {
            let meta: serde_json::Value = serde_json::from_slice(&raw)?;
            json!({
                "source": "experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json (copied verbatim)",
                "runs": meta["runs"].as_array().map(|runs| runs.iter().map(|r| json!({
                    "train_cap_bytes": r["train_cap_bytes"],
                    "bpc_kn5_val_per_normalized_char": r["bpc_kn5_val_per_normalized_char"],
                    "kn5_bits_per_raw_val_byte": r["kn5_bits_per_raw_val_byte"],
                })).collect::<Vec<_>>()),
                "val_raw_bytes_sha256": meta["val_stream"]["val_raw_bytes_sha256"],
            })
        }
        Err(err) => json!({ "status": format!("KN-5 run meta unavailable: {err}") }),
    };

    // Verdict: best deployable (hard ternary) bits/raw-byte among ok arms.
    let mut best: Option<(String, f64)> = None;
    for arm in &arms {
        if arm["status"] != "ok" {
            continue;
        }
        let name = arm["arm"].as_str().unwrap_or("?").to_string();
        let Some(v) = arm["measurement"]["ternary_val_bits_per_raw_byte"].as_f64() else {
            continue;
        };
        if best.as_ref().map(|(_, b)| v < *b).unwrap_or(true) {
            best = Some((name, v));
        }
    }
    let kn5_full = kn_reference["runs"]
        .as_array()
        .and_then(|runs| {
            runs.iter()
                .max_by_key(|r| r["train_cap_bytes"].as_u64().unwrap_or(0))
        })
        .and_then(|r| r["kn5_bits_per_raw_val_byte"].as_f64());

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&args.repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Scale-plan deployment projection (same measured-constant accounting as
    // the arms), from a "d,ff,blocks,slots,experts" CLI tuple.
    let scale_plan = if args.scale_plan.trim().is_empty() {
        json!(null)
    } else {
        let parts: Vec<usize> = args
            .scale_plan
            .split(',')
            .map(|p| p.trim().parse::<usize>())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("bad --scale-plan: {e}"))?;
        if parts.len() != 5 {
            return Err("--scale-plan needs d_model,d_ff,n_blocks,state_slots,n_experts".into());
        }
        let spec = ArmSpec {
            name: "SCALE".into(),
            description: "overnight scale-run topology projection".into(),
            d_model: parts[0],
            d_ff: parts[1],
            n_blocks: parts[2],
            state_slots: parts[3],
            n_experts: parts[4],
        };
        let mut plan = spec.deployment_json();
        plan["config"] = json!({
            "d_model": spec.d_model,
            "d_ff": spec.d_ff,
            "n_blocks": spec.n_blocks,
            "state_slots": spec.state_slots,
            "n_experts_per_block": spec.n_experts,
        });
        plan
    };

    let report = json!({
        "schema": "s8_matched_cycles_sweep.v1",
        "bead": "bd-3771m",
        "purpose": "matched-CYCLES dense-vs-MoE architecture signal + distillation probe at proxy scale under the 30 s/char quality-first UX budget, on the S5 LinearState MT4 charset-80 substrate",
        "git_sha": git_sha,
        "seed": args.seed,
        "backend": "burn_ndarray_autodiff",
        "ux_budget": {
            "s_per_char": 30.0,
            "cpu_fraction": CPU_FRACTION,
            "m_cycles_per_token": 30.0 * GB_MCYCLES_PER_SEC * CPU_FRACTION,
            "macs_per_token_v3": 30.0 * GB_MCYCLES_PER_SEC * CPU_FRACTION / V3_CY_PER_MAC,
            "macs_per_token_v2": 30.0 * GB_MCYCLES_PER_SEC * CPU_FRACTION / V2_CY_PER_MAC,
            "rom_budget_bytes": ROM_BUDGET_BYTES as u64,
            "kernel_constants_source": "docs/experiments/kernel-bakeoff/kernel_bakeoff.json @ 40% zeros (V3 5.385 cy/MAC, 4.401 B/w; V2 10.261 cy/MAC, 0.25 B/w + 2699 B shared code)",
        },
        "kn5_reference": kn_reference,
        "arms": arms,
        "verdict": {
            "best_arm_by_ternary_bits_per_raw_byte": best.as_ref().map(|(n, _)| n.clone()),
            "best_ternary_bits_per_raw_byte": best.as_ref().map(|(_, v)| *v),
            "kn5_full_corpus_bits_per_raw_byte": kn5_full,
            "best_beats_kn5_full_corpus": match (&best, kn5_full) {
                (Some((_, v)), Some(k)) => json!(*v < k),
                _ => json!(null),
            },
        },
        "scale_plan": scale_plan,
    });
    let path = out_dir.join("report.json");
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    println!("[write] {}", path.display());
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.phase.as_str() {
        "arm" => phase_arm(&args),
        "distill" => phase_distill(&args),
        "report" => phase_report(&args),
        other => Err(format!("unknown phase {other}").into()),
    }
}
