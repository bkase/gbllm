//! Host-side evaluators for the LinearState stateful ROM bring-up
//! (bd-x5l2s), generalized to a **parameterized topology** so the same
//! canonical integer semantics serve both the arm-B d64/ff128/4blk/slots64
//! checkpoint and the S8 distilled d192/ff384/6blk/slots192 student
//! (`f_s5_state_checkpoint_export.v1` manifest family; topology is read
//! from the manifest's `topology` block).
//!
//! Two evaluators over the same export:
//!
//! 1. [`f32_state_forward`] — a faithful port of the trainer's hard-ternary
//!    f32 forward pass (`gbf-experiments/src/bin/s5_state_ab.rs`
//!    `forward_seq`), including the exact recurrence
//!    `h_t = decay (.) h_{t-1} + W_in(actq(rms_norm(x_t)))`,
//!    `y_t = actq(W_out(h_t))`, `x'_t = x_t + y_t`, followed by the same
//!    pre-norm residual FFN stack and tied f32 head.
//! 2. [`IntStateLoweredModel::forward`] — the **canonical integer semantics**
//!    for the deployed stateful ROM, extending the pinned v0 integer
//!    conventions (u8 zero-point-128 activations, i16 matvec accumulators
//!    with `-128 * sum(row)` seeds, per-row Q8.8 integer scale multiplies,
//!    GELU LUT, integer RMS norm) to the recurrence. The ROM built by
//!    [`crate::asm_impl_state`] must reproduce this function byte-exactly,
//!    including the state vector carried in WRAM across tokens.
//!
//! # Accumulator widths at large fan-in (RangePlan-style)
//!
//! The v0 i16 matvec accumulator is structurally safe only when every row's
//! worst-case value `[-(128*pos + 127*neg), 127*pos + 128*neg]` fits i16
//! (`pos`/`neg` = count of +1/-1 weights). At fan-in 128 this always holds;
//! at fan-in 384 (the d192 down projection) a dense row reaches ~49k. The
//! `f_s5_state_checkpoint_export.v1` manifest declares no measured
//! activation ranges, so lowering computes the **exact structural per-row
//! bound from the actual ternary weights** and widens the down-projection
//! accumulator to i24 (3 bytes on device, column-segmented weight code)
//! whenever any row of any block requires it ([`AccWidth`]). This is
//! conservative: it never relies on unmeasured activation statistics.
//!
//! # Integer semantics version
//!
//! [`STATE_INT_SEMANTICS_VERSION`] = `state-int-semantics.v2`.
//!
//! - **v1** carried the down-projection residual delta in a u16 with a
//!   canonical clamp at 65535 raw (2047.97 units) on *both* accumulator
//!   paths — a Q11.5-era carrier width that was never re-proven when the
//!   residual widened to i24 Q19.5. On the real d192 student the unclamped
//!   delta reaches 308,033 raw (9,626 units; measured over all 1.2e9 deltas
//!   of the committed val pair set, bd-2vkqt), so ~0.087% of deltas
//!   saturated (~1 per position) and cost +1.90 bpc vs the trainer.
//! - **v2** keeps the u16 delta (with its canonical, counted 65535 clamp)
//!   on the i16-accumulator path — the committed arm-B lowering and ROM are
//!   bit-identical — and carries the delta **exactly in a signed i24** on
//!   the wide i24-accumulator path. Lowering proves the structural per-row
//!   bound `max_row floor((2*scale_raw*acc_bound + 127)/254)` from the
//!   actual ternary weights fits [`DOWN_DELTA_WIDE_BOUND`] (else
//!   [`StateModelError::DownDeltaEscapesI24`]), so the wide path has **no
//!   reachable clamp**: carrier widths are proven from real checkpoints,
//!   not assumed.
//!
//! Integer recurrence representation (measured on the real arm-B checkpoint
//! over the val stream, then pinned; the format is topology-independent):
//! - The state slot `h[s]` is held as a **saturating i24 in an i32 word**,
//!   in units of `(ACT_RANGE / QMAX) / 256` real (the "m unit"), so the
//!   in-projection delta `m = scale_raw * acc` lands **exactly** (no
//!   rounding on accumulate).
//! - The decay multiply is `sign(h) * ((|h| * decay_raw + 128) >> 8)` — a
//!   per-slot Q8.8 integer multiply with round-half-away-from-zero. All MT4
//!   decay raws {128, 192, 224, 240} fit u8, which the loader validates.
//! - The residual stream is **i24 Q19.5** (resolution 1/32, range
//!   +/-262143.97), three bytes per lane with byte-serial adds on device.
//!
//! Every documented divergence between (2) and (1) is listed in
//! [`STATE_INT_SEMANTIC_DIVERGENCES`] and measured by the fidelity phase in
//! `gbf-bench`.

use std::fmt;

use crate::model_ref::{
    ACT_RANGE, BlockWeights, IntForwardStats, LoweredLayer, ModelRefError, NORM_EPS, QMAX,
    TernaryLayer, f32_act_fake_quant, gelu_approx_f32, int_down_delta, int_matvec,
    int_scale_to_grid, rte_i64,
};

/// charset_v1 vocabulary (ids 0..=79) — the arm-B/D192 lexical space.
pub const STATE_VOCAB: usize = 80;
/// Host-evaluator vocab ceiling. The host integer forward holds all `vocab`
/// i32 logits in RAM (no 256-byte page limit), so it accepts the V=1024
/// subword MoE student ahead of on-device logit paging (deploy step 2). Sized
/// generously above 1024; the head still fits `u16` id loops.
pub const HOST_VOCAB_CAP: usize = 4096;
/// Recurrent state width of the committed arm-B checkpoint.
pub const STATE_SLOTS: usize = 64;
/// Residual fixed point: i24 Q19.5 (fractional bits shared with the dense
/// bring-up's Q11.5; only the integer width is widened).
pub const STATE_RESID_FRAC_BITS: u32 = 5;
/// `2^STATE_RESID_FRAC_BITS`.
pub const STATE_RESID_ONE: i32 = 1 << STATE_RESID_FRAC_BITS;
/// Saturation bound for the state slots (i24 magnitude ceiling).
pub const STATE_CLAMP: i32 = (1 << 23) - 1;
/// i24 residual wrap bound (adds wrap mod 2^24, mirrored by 3-byte adds on
/// device; events are counted, expected 0 on real data).
const RESID_I24_MIN: i32 = -(1 << 23);
const RESID_I24_MAX: i32 = (1 << 23) - 1;

/// Version of the canonical stateful integer semantics (see the module docs
/// for the changelog). Bumped whenever the deployed numeric contract
/// changes; reproduced in the evidence reports.
pub const STATE_INT_SEMANTICS_VERSION: &str = "state-int-semantics.v2";

/// Signed-i24 ceiling of the wide-path down-projection delta carrier.
/// Lowering proves the structural per-row delta bound fits this (it is a
/// carrier width, not a canonical clamp: v2 has no clamp on the wide path).
pub const DOWN_DELTA_WIDE_BOUND: u64 = (1 << 23) - 1;

/// Documented places where the canonical integer semantics diverge from the
/// trainer's f32 semantics. Reproduced verbatim in the evidence report.
pub const STATE_INT_SEMANTIC_DIVERGENCES: [&str; 13] = [
    "Residual stream is i24 Q19.5 (resolution 1/32, range +/-262143.97) with mod-2^24 wrapping adds (trainer: f32). i24 replaces the dense bring-up's i16 Q11.5 because the arm-B checkpoint's residual measurably reaches |x| ~ 2,600 real (8.4e4 raw at 1/32), which no 16-bit split of range/resolution can hold without wrapping. Embedding rows are quantized to Q19.5 at lowering time (round-half-even).",
    "State slots are saturating-i24 integers in units of (ACT_RANGE/QMAX)/256 real, so the in-projection delta m = scale_raw * acc accumulates exactly; the trainer carries f32 state. Saturation at +/-(2^23 - 1) is canonical and counted.",
    "The per-token decay multiply is sign(h) * ((|h| * decay_raw + 128) >> 8) (round-half-away on the Q8.8 product); the trainer multiplies f32 state by decay_raw/256 exactly. Rounding error is at most 0.5 state units (~1.2e-4 real) per slot per token.",
    "The state out-projection epilogue quantizes y = clamp(round_half_away(scale_raw * acc2 / 65536), -127, 127) onto the activation grid via integer multiply (trainer: f32 matvec then Int8 fake-quant with round-ties-even). acc2 is the i32 dot product of ternary out-projection rows with the i24 state.",
    "The state residual add applies y through a 255-entry i16 LUT y_resid[p] = round_ties_even(p * 256 / 127) on the Q19.5 grid (trainer adds the unquantized f32 fake-quant output).",
    "RMS norm: integer sum of squares over the d_model i24 lanes (u64 accumulator, 7 bytes on device), mean = floor(sum / d_model) (a shift when d_model is a power of two, otherwise shift-then-odd-constant division on device), epsilon = +1 raw mean unit, rms = floor(isqrt48(mean+1)); norm output and Int8 activation fake-quant collapse into one rounded division q = clamp(round_half_away(|x|*127 / (8*rms)), 0, 127) * sign(x) (trainer: f32 divide, clamp to +/-8, round-ties-even quantization).",
    "GELU is a 255-entry LUT indexed by the pre-activation value quantized to the same [-8,8]/127 grid (round-half-away on the Q8.8 scale product); the trainer applies tanh-approximate GELU to the unquantized f32 matvec output before fake-quant.",
    "Up/down epilogues apply the Q8.8 row scale as an integer multiply with round-half-away rounding (round-ties-even in the trainer's f32 path).",
    "Down-projection output is quantized to the Q19.5 residual grid as sign(m) * round_half_away(|m| / 127) before the residual add; the trainer adds unquantized f32 deltas. On the i16-accumulator path the delta is carried in a u16 with a canonical, counted clamp at 65535 raw (2047.97 units; the committed arm-B run measured zero clamp events). On the wide i24-accumulator path (state-int-semantics.v2) the delta is carried EXACTLY in a signed i24 with no clamp: lowering proves the structural per-row bound floor((2*scale_raw*acc_bound + 127)/254) <= 2^23 - 1 from the actual weights (DownDeltaEscapesI24 otherwise). v1 clamped the wide path at 65535 too, which saturated ~0.087% of real d192 deltas (measured max 308,033 raw = 9,626 units over the full committed pair set) and cost +1.90 bpc (bd-2vkqt).",
    "Down-projection matvec accumulators widen from i16 to i24 (3 bytes on device) when the structural per-row worst case over the actual ternary weights exceeds i16 (possible from fan-in 257 up; certain worst-case at the d192 student's fan-in 384). The manifest declares no measured activation ranges, so the width decision uses the exact structural bound, never unmeasured statistics. The value is exact either way; only the carrier width changes. The wide path also widens the residual-delta carrier from u16 to i24 (see the down-projection delta entry).",
    "The final norm output is activation-quantized to the [-8,8]/127 grid before the tied head; tied-head weights are the embedding quantized per-tensor to i8 (scale = max|emb|/127, round-ties-even); logits are integer dot products in i32 (i24 on device).",
    "Integer rounding is round-half-away-from-zero throughout the runtime path; the trainer/Burn rounds ties to even. Because the recurrence carries state across tokens, per-token rounding differences accumulate: fidelity (bpc delta, argmax agreement) is therefore measured over long sequential streams, not per-context.",
    "MoE routing (router-fx.v1) is PURELY INTEGER on the deployed path: no f32 enters the forward. The raw pre-norm residual i24 Q19.5 x is viewed as Q16.16 via xr = i64(x_i24) << 11 (exact; 5 frac bits + 11 = 16). Weight tables are built once at lowering time (round-ties-even, f64 only there): win_q = rte(w_input_projection * 2^16) and wout_q = rte(w_expert_projection * 2^16), each asserted to fit i32 (RouterWeightEscapesI32 otherwise). hidden_acc[k] = bin_q[k] + sum_c win_q[k,c] * xr[c] accumulates at Q32.32 in an i64 (structural bound proven <= i62, RouterHiddenEscapesI62 otherwise), then shifts back to Q16.16 with round-half-away-from-zero: hidden_q[k] = sign * ((|hidden_acc| + 2^15) >> 16). raw_acc[e] = bout_q[e] + sum_k wout_q[e,k] * hidden_q[k] accumulates at Q32.32 in an i64 (structural bound proven <= i62, RouterRawLogitEscapesI62 otherwise). expert = argmax_e raw_acc[e] with a strict `>` scan from index 0 (lowest-index tiebreak). bin_q = rte(input_bias * 2^32), bout_q = rte(expert_bias * 2^32) (Option -> 0). The router output is ONLY the selected expert index; it never re-enters the integer stream, so the per-expert FFN math is byte-identical to the dense block. The f32 LowRankRouter::route_f32 stays the reference the fixed-point router is gated against (0 divergences required on the real d192x8 student across all blocks/positions). NOTE: the design named i48 for the hidden carrier, but the real student's structural hidden bound under the mandated Q16.16 xr and the full i24 saturation bound is ~1.92e16, so the proven carrier was widened to i62 (still an exact i64 accumulator); the numeric result is width-independent.",
];

// ---------------------------------------------------------------------------
// logit paging capability (deploy step 2)
// ---------------------------------------------------------------------------

/// Ids that fit one 256-byte i24 logit page (`floor(255 / 3) = 85`).
pub const LOGIT_PAGE_IDS: usize = 85;
/// Paged-logit vocab ceiling: 255 pages of [`LOGIT_PAGE_IDS`] ids. Sized so the
/// per-page id count fits a u8 loop counter AND the page count fits a u8 loop
/// counter, which is what the ROM's paged epilogue needs.
pub const LOGIT_PAGED_VOCAB_MAX: usize = 255 * LOGIT_PAGE_IDS;
/// Maximum running top-k heap size the paged sampler supports (heap tables are
/// held in one WRAM sampler page alongside the candidate id/weight arrays).
pub const HEAP_K_MAX: usize = 40;

/// How the head/argmax/sampler epilogue reads the tied-head logits.
///
/// [`LogitPaging::SinglePage`] is the pinned, byte-identical legacy path: the
/// whole `vocab` i24 logit vector materializes in one 256-byte WRAM page, so
/// `vocab <= 85`. Every existing dense ROM (arm-B, D192, D192-real) is
/// `SinglePage` and its ROM bytes are unchanged.
///
/// [`LogitPaging::Paged`] streams the head in `ceil(vocab / 85)` pages of
/// `<= 85` ids, folding each page into a running top-1 argmax (u16) and a
/// running top-k heap, so V=1024 subword MoE students need only one logit page
/// plus the heap resident at once. Gated behind this flag: `SinglePage` emits
/// the current epilogue verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogitPaging {
    /// One resident 256-byte logit page; `vocab <= 85` (the pinned legacy path).
    #[default]
    SinglePage,
    /// Streamed `<= 85`-id pages with a running top-1 + top-k heap fold.
    Paged,
}

// ---------------------------------------------------------------------------
// topology
// ---------------------------------------------------------------------------

/// Model topology as declared by the export manifest's `topology` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateTopology {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub state_slots: usize,
    pub vocab: usize,
    /// Number of MoE experts per FFN block. 1 = dense (back-compat: the whole
    /// existing dense pipeline is exactly `n_experts == 1`). Top-1 routing runs
    /// exactly one expert per block per token, so active MACs are unchanged.
    pub n_experts: usize,
    /// How the head/argmax/sampler epilogue reads the logits. `SinglePage`
    /// (default) is the byte-identical legacy path (`vocab <= 85`); `Paged`
    /// streams `<= 85`-id pages for wide-vocab subword students.
    pub logit_paging: LogitPaging,
}

impl StateTopology {
    /// The committed arm-B checkpoint (S5 state A/B).
    pub const ARM_B: Self = Self {
        d_model: 64,
        d_ff: 128,
        n_blocks: 4,
        state_slots: 64,
        vocab: STATE_VOCAB,
        n_experts: 1,
        logit_paging: LogitPaging::SinglePage,
    };

    /// Tonight's S8 distilled student (bd-3771m).
    pub const D192: Self = Self {
        d_model: 192,
        d_ff: 384,
        n_blocks: 6,
        state_slots: 192,
        vocab: STATE_VOCAB,
        n_experts: 1,
        ..Self::ARM_B
    };

    /// The trained subword MoE student (student_moe_d192x8): 8 experts, subword
    /// vocab 1024, streamed logit paging (deploy step 2).
    pub const D192_MOE: Self = Self {
        vocab: 1024,
        n_experts: 8,
        logit_paging: LogitPaging::Paged,
        ..Self::D192
    };

    /// A `validate()`-clean MoE topology for the host evaluator unit tests:
    /// the d192 shape with charset vocab (single-page logits) and a small
    /// expert bank. Distinct from [`Self::D192_MOE`] (whose 1024 vocab needs
    /// logit paging, deploy step 2).
    pub const D192_MOE_TEST: Self = Self {
        n_experts: 4,
        ..Self::D192
    };

    /// A synthetic DENSE vocab-1024 topology (orthogonal to MoE) that gates the
    /// V=1024 logit paging path: `Paged` epilogue, `n_experts = 1`, so the head/
    /// argmax/sampler paging is exercised without any MoE routing. Smaller
    /// d_model/d_ff than D192 keeps the paged ROM banks and cycle budget modest.
    pub const D1024_DENSE: Self = Self {
        d_model: 64,
        d_ff: 128,
        n_blocks: 2,
        state_slots: 64,
        vocab: 1024,
        n_experts: 1,
        logit_paging: LogitPaging::Paged,
    };

    /// Validate the device structural limits this pipeline supports. These
    /// are ROM-code constraints (8-bit loop counters, single-page tables),
    /// not arbitrary caps; each names the device structure that pins it.
    ///
    /// The vocab cap is keyed off [`Self::logit_paging`]:
    /// - [`LogitPaging::SinglePage`]: one 256-byte i24 logit page, so
    ///   `vocab <= LOGIT_PAGE_IDS` (85). This is the pinned legacy path.
    /// - [`LogitPaging::Paged`]: streamed `<= 85`-id pages folded into a
    ///   running top-1/top-k, so `vocab <= LOGIT_PAGED_VOCAB_MAX` (255 pages).
    pub fn validate(&self) -> Result<(), StateModelError> {
        let vocab_cap = match self.logit_paging {
            LogitPaging::SinglePage => LOGIT_PAGE_IDS,
            LogitPaging::Paged => LOGIT_PAGED_VOCAB_MAX,
        };
        self.validate_with_vocab_cap(vocab_cap)
    }

    /// Host-evaluator validation. Identical to [`Self::validate`] except the
    /// single-page vocab cap is relaxed to the paged host ceiling: the host
    /// integer forward computes `vocab` i32 logits in RAM with no 256-byte
    /// page limit, so V=1024 subword MoE students load and forward here even
    /// though on-device logit paging (deploy step 2) has not yet relaxed the
    /// ROM's single-page cap. Every other device structural limit is enforced
    /// unchanged, so dense d192/arm-B (vocab 80) validate identically.
    pub fn validate_host(&self) -> Result<(), StateModelError> {
        // The host holds all logits in RAM, so the single-page cap never binds;
        // a Paged topology still respects the paged ceiling.
        let cap = match self.logit_paging {
            LogitPaging::SinglePage => HOST_VOCAB_CAP,
            LogitPaging::Paged => LOGIT_PAGED_VOCAB_MAX.max(HOST_VOCAB_CAP),
        };
        self.validate_with_vocab_cap(cap)
    }

    fn validate_with_vocab_cap(&self, vocab_cap: usize) -> Result<(), StateModelError> {
        let lim = |what: &'static str, value: usize, max: usize| {
            if value == 0 || value > max {
                Err(StateModelError::Topology { what, value, max })
            } else {
                Ok(())
            }
        };
        // d_model: u8 lane counters, one activation page for the head.
        lim("d_model (u8 lane loops / head act page)", self.d_model, 255)?;
        // d_ff: activation buffer must fit below the fixed scratch page.
        lim("d_ff (activation buffer below scratch)", self.d_ff, 512)?;
        lim("n_blocks", self.n_blocks, 16)?;
        // state_slots: u8 slot loop, i32 slots in WRAM.
        lim("state_slots (u8 slot loops)", self.state_slots, 255)?;
        // vocab: 3-byte logits in one page (ROM), or the host paged ceiling.
        lim("vocab (i24 logits single page)", self.vocab, vocab_cap)?;
        // n_experts: u8 expert index / one bank set per expert in ROM.
        lim("n_experts (u8 expert loop / bank set)", self.n_experts, 255)?;
        Ok(())
    }

    /// True when the FFN blocks route over more than one expert.
    #[must_use]
    pub fn is_moe(&self) -> bool {
        self.n_experts > 1
    }

    /// Multiply-accumulates per token (matvecs plus tied head).
    #[must_use]
    pub fn macs_per_token(&self) -> u64 {
        let d = self.d_model as u64;
        let ff = self.d_ff as u64;
        let s = self.state_slots as u64;
        let v = self.vocab as u64;
        s * d + d * s + (self.n_blocks as u64) * 2 * (ff * d) + v * d
    }
}

// ---------------------------------------------------------------------------
// MoE router + FFN block
// ---------------------------------------------------------------------------

/// The deployed low-rank top-1 router (`gbf-model` `Top1RouterQat`): a
/// two-stage f32 projection `hidden = input_projection @ x + input_bias`,
/// `raw = expert_projection @ hidden + expert_bias`, `expert = argmax(raw)`
/// with a lowest-index tiebreak. It runs on the **raw pre-norm residual**
/// (dequantized from the i24 Q19.5 stream at forward time) and produces ONLY
/// the selected expert index — it never re-enters the integer stream.
///
/// The `route_f32` summation reproduces `gbf-bench/src/moe_parity.rs`'s
/// `Router::route` byte-for-byte (bias-seeded folds, `input_projection`
/// iterated over `d_model` columns for the hidden vector, `expert_projection`
/// iterated over `router_rank` columns for the raw scores, `argmax` with a
/// strict `>` so ties keep the lowest expert index).
#[derive(Debug, Clone)]
pub struct LowRankRouter {
    rank: usize,
    d_model: usize,
    n_experts: usize,
    /// input_projection, shape `[rank, d_model]`, row-major.
    input_projection: Vec<f32>,
    /// input_bias, shape `[rank]`.
    input_bias: Vec<f32>,
    /// expert_projection, shape `[n_experts, rank]`, row-major.
    expert_projection: Vec<f32>,
    /// expert_bias, shape `[n_experts]`.
    expert_bias: Vec<f32>,
}

impl LowRankRouter {
    /// Build a router from its four f32 tensors. Shapes are checked against
    /// `rank`, `d_model` and `n_experts`; non-finite weights are rejected.
    pub fn new(
        rank: usize,
        d_model: usize,
        n_experts: usize,
        input_projection: Vec<f32>,
        input_bias: Vec<f32>,
        expert_projection: Vec<f32>,
        expert_bias: Vec<f32>,
    ) -> Result<Self, StateModelError> {
        let router = Self {
            rank,
            d_model,
            n_experts,
            input_projection,
            input_bias,
            expert_projection,
            expert_bias,
        };
        router.validate(usize::MAX)?;
        Ok(router)
    }

    /// Structural validation. `block` is used only for error reporting (pass
    /// `usize::MAX` when not building inside a checkpoint).
    pub fn validate(&self, block: usize) -> Result<(), StateModelError> {
        let shape = |what: &'static str, actual: usize, expected: usize| {
            if actual == expected {
                Ok(())
            } else {
                Err(StateModelError::Shape {
                    what,
                    expected,
                    actual,
                })
            }
        };
        shape(
            "router input_projection",
            self.input_projection.len(),
            self.rank * self.d_model,
        )?;
        shape("router input_bias", self.input_bias.len(), self.rank)?;
        shape(
            "router expert_projection",
            self.expert_projection.len(),
            self.n_experts * self.rank,
        )?;
        shape("router expert_bias", self.expert_bias.len(), self.n_experts)?;
        let finite = |what: &'static str, v: &[f32]| {
            if v.iter().all(|x| x.is_finite()) {
                Ok(())
            } else {
                Err(StateModelError::NonFiniteRouter { block, what })
            }
        };
        finite("input_projection", &self.input_projection)?;
        finite("input_bias", &self.input_bias)?;
        finite("expert_projection", &self.expert_projection)?;
        finite("expert_bias", &self.expert_bias)?;
        Ok(())
    }

    #[must_use]
    pub fn rank(&self) -> usize {
        self.rank
    }

    #[must_use]
    pub fn n_experts(&self) -> usize {
        self.n_experts
    }

    /// Route the raw pre-norm residual `x` (`d_model` f32) to a top-1 expert
    /// index. This reproduces `moe_parity.rs`'s `Router::route` exactly: the
    /// hidden vector folds each `input_projection` row starting from its bias
    /// over `d_model` columns, the raw scores fold each `expert_projection`
    /// row starting from its bias over `router_rank` columns, and the argmax
    /// uses a strict `>` (ties keep the lowest index).
    #[must_use]
    pub fn route_f32(&self, x: &[f32]) -> usize {
        self.route_f32_with_logits(x).0
    }

    /// [`Self::route_f32`] returning the selected expert index and the raw f32
    /// logits, so the fixed-point gate can log the `raw[top1] - raw[top2]`
    /// margin on any divergence.
    #[must_use]
    pub fn route_f32_with_logits(&self, x: &[f32]) -> (usize, Vec<f32>) {
        debug_assert_eq!(x.len(), self.d_model, "router input width");
        let hid: Vec<f32> = self
            .input_projection
            .chunks_exact(self.d_model)
            .zip(self.input_bias.iter())
            .map(|(row, &bias)| {
                row.iter()
                    .zip(x.iter())
                    .map(|(&w, &xi)| w * xi)
                    .fold(bias, |acc, p| acc + p)
            })
            .collect();
        let mut raw = vec![0f32; self.n_experts];
        let mut best_e = 0usize;
        let mut best_v = f32::NEG_INFINITY;
        for (e, (row, &bias)) in self
            .expert_projection
            .chunks_exact(self.rank)
            .zip(self.expert_bias.iter())
            .enumerate()
        {
            let acc = row
                .iter()
                .zip(hid.iter())
                .map(|(&w, &hk)| w * hk)
                .fold(bias, |a, p| a + p);
            raw[e] = acc;
            if acc > best_v {
                best_v = acc;
                best_e = e;
            }
        }
        (best_e, raw)
    }
}

// ---------------------------------------------------------------------------
// fixed-point MoE router (router-fx.v1)
// ---------------------------------------------------------------------------

/// `xr[c] = i64(x_i24[c]) << XR_SHIFT` reinterprets the i24 Q19.5 residual as
/// Q16.16 (5 frac bits + 11 = 16). Exact left shift, no rounding.
const ROUTER_XR_SHIFT: u32 = 11;
/// Fixed-point router weight scale: `win_q/wout_q = rte(w * 2^ROUTER_WEIGHT_SHIFT)`.
const ROUTER_WEIGHT_SHIFT: u32 = 16;
/// Shift that returns the Q32.32 hidden accumulator to the Q16.16 hidden grid.
const ROUTER_HIDDEN_SHIFT: u32 = 16;
/// Proven i62 carrier ceiling for both the hidden and raw-logit accumulators
/// (the design targeted i48 for the hidden carrier; the real d192x8 student's
/// structural hidden bound under the mandated Q16.16 `xr` and the full i24
/// saturation bound is ~1.92e16, so the proven width was widened to i62 — still
/// an exact i64 accumulator with >2 bits of headroom). The value is
/// width-independent; only the honesty check widens.
const ROUTER_ACC_I62_BOUND: u64 = (1 << 62) - 1;
/// Maximum magnitude of `xr[c]`: `(2^23 - 1) << ROUTER_XR_SHIFT`, from the i24
/// residual saturation bound. Used for the structural hidden-accumulator proof.
const ROUTER_XR_MAX: u64 = ((1u64 << 23) - 1) << ROUTER_XR_SHIFT;

/// The **purely integer** deployed twin of [`LowRankRouter`] (`router-fx.v1`).
///
/// It reproduces [`LowRankRouter::route_f32`]'s argmax **without any f32 in the
/// forward**, so host and ROM route identically by construction. Weight tables
/// are built once at lowering time (round-ties-even, f64 there only) and mirror
/// bit-for-bit into ROM data (deploy step 4). The forward is all `i32`/`i64`.
///
/// Fixed-point contract (see [`STATE_INT_SEMANTIC_DIVERGENCES`] entry
/// `router-fx.v1`):
/// - Input: the raw pre-norm residual `x_i24` (i24 Q19.5). `xr[c] =
///   i64(x_i24[c]) << 11` is the exact Q16.16 view.
/// - `win_q[k*d_model+c] = rte(input_projection * 2^16)` (i32),
///   `wout_q[e*rank+k] = rte(expert_projection * 2^16)` (i32).
/// - `bin_q[k] = rte(input_bias * 2^32)`, `bout_q[e] = rte(expert_bias * 2^32)`
///   at the Q32.32 accumulator scale (missing bias -> 0).
/// - `hidden_acc[k] = bin_q[k] + sum_c win_q * xr[c]` (i64, Q32.32),
///   `hidden_q[k] = sign * ((|hidden_acc| + 2^15) >> 16)` (round-half-away).
/// - `raw_acc[e] = bout_q[e] + sum_k wout_q * hidden_q[k]` (i64, Q32.32).
/// - `expert = argmax_e raw_acc[e]`, strict `>` scan from 0 (lowest index).
#[derive(Debug, Clone)]
pub struct FixedRouter {
    rank: usize,
    d_model: usize,
    n_experts: usize,
    /// `win_q`, shape `[rank, d_model]`, row-major (i32, scale `2^16`).
    win_q: Vec<i32>,
    /// `bin_q`, shape `[rank]` (i64, scale `2^32`).
    bin_q: Vec<i64>,
    /// `wout_q`, shape `[n_experts, rank]`, row-major (i32, scale `2^16`).
    wout_q: Vec<i32>,
    /// `bout_q`, shape `[n_experts]` (i64, scale `2^32`).
    bout_q: Vec<i64>,
    /// Reported structural hidden-accumulator bound (proven `<= i62`).
    hidden_structural_bound: u64,
    /// Reported structural raw-logit-accumulator bound (proven `<= i62`).
    raw_structural_bound: u64,
}

impl FixedRouter {
    /// Build the fixed-point router from the f32 [`LowRankRouter`] at lowering
    /// time. `block` is used only for error reporting. Proves the i32 weight
    /// fit and the i62 accumulator bounds structurally from the actual
    /// `|weights|` and the i24 saturation bound; fails loud (never clamps).
    pub fn lower(router: &LowRankRouter, block: usize) -> Result<Self, StateModelError> {
        let rank = router.rank;
        let d_model = router.d_model;
        let n_experts = router.n_experts;

        // Weight quantization at scale 2^16, i32 fit check.
        let quant_weight = |what: &'static str, w: &[f32]| -> Result<Vec<i32>, StateModelError> {
            w.iter()
                .enumerate()
                .map(|(index, &v)| {
                    let q = rte_i64(f64::from(v) * f64::from(1u32 << ROUTER_WEIGHT_SHIFT));
                    i32::try_from(q).map_err(|_| StateModelError::RouterWeightEscapesI32 {
                        block,
                        what,
                        index,
                        quantized: q,
                    })
                })
                .collect()
        };
        // Biases quantize at the Q32.32 accumulator scale (2^32).
        let quant_bias = |b: &[f32]| -> Vec<i64> {
            b.iter()
                .map(|&v| rte_i64(f64::from(v) * 4_294_967_296.0_f64))
                .collect()
        };

        let win_q = quant_weight("input_projection", &router.input_projection)?;
        let bin_q = quant_bias(&router.input_bias);
        let wout_q = quant_weight("expert_projection", &router.expert_projection)?;
        let bout_q = quant_bias(&router.expert_bias);

        // Structural hidden bound: |bin_q[k]| + (sum_c |win_q[k,c]|) * XR_MAX.
        let mut hidden_structural_bound: u64 = 0;
        let mut hidden_q_bound = vec![0u64; rank];
        for k in 0..rank {
            let abssum: u64 = win_q[k * d_model..(k + 1) * d_model]
                .iter()
                .map(|&w| u64::from(w.unsigned_abs()))
                .sum();
            let bound = bin_q[k]
                .unsigned_abs()
                .saturating_add(abssum.saturating_mul(ROUTER_XR_MAX));
            if bound > ROUTER_ACC_I62_BOUND {
                return Err(StateModelError::RouterHiddenEscapesI62 {
                    block,
                    row: k,
                    bound,
                });
            }
            hidden_structural_bound = hidden_structural_bound.max(bound);
            // Max |hidden_q[k]| = (bound + 2^15) >> 16 (round-half-away ceiling).
            hidden_q_bound[k] = (bound + (1 << (ROUTER_HIDDEN_SHIFT - 1))) >> ROUTER_HIDDEN_SHIFT;
        }

        // Structural raw-logit bound: |bout_q[e]| + sum_k |wout_q[e,k]|*hq_bound.
        let mut raw_structural_bound: u64 = 0;
        for e in 0..n_experts {
            let mut bound = bout_q[e].unsigned_abs();
            for k in 0..rank {
                bound = bound.saturating_add(
                    u64::from(wout_q[e * rank + k].unsigned_abs())
                        .saturating_mul(hidden_q_bound[k]),
                );
            }
            if bound > ROUTER_ACC_I62_BOUND {
                return Err(StateModelError::RouterRawLogitEscapesI62 {
                    block,
                    expert: e,
                    bound,
                });
            }
            raw_structural_bound = raw_structural_bound.max(bound);
        }

        Ok(Self {
            rank,
            d_model,
            n_experts,
            win_q,
            bin_q,
            wout_q,
            bout_q,
            hidden_structural_bound,
            raw_structural_bound,
        })
    }

    #[must_use]
    pub fn rank(&self) -> usize {
        self.rank
    }

    #[must_use]
    pub fn n_experts(&self) -> usize {
        self.n_experts
    }

    #[must_use]
    pub fn d_model(&self) -> usize {
        self.d_model
    }

    /// `win_q`, shape `[rank, d_model]`, row-major (i32, scale `2^16`).
    #[must_use]
    pub fn win_q(&self) -> &[i32] {
        &self.win_q
    }

    /// `bin_q`, shape `[rank]` (i64, scale `2^32`).
    #[must_use]
    pub fn bin_q(&self) -> &[i64] {
        &self.bin_q
    }

    /// `wout_q`, shape `[n_experts, rank]`, row-major (i32, scale `2^16`).
    #[must_use]
    pub fn wout_q(&self) -> &[i32] {
        &self.wout_q
    }

    /// `bout_q`, shape `[n_experts]` (i64, scale `2^32`).
    #[must_use]
    pub fn bout_q(&self) -> &[i64] {
        &self.bout_q
    }

    /// Serialize the fixed-point router tables into the exact byte layout the
    /// ROM router reads from bank 0, little-endian, in this order:
    ///   win_q  : `rank * d_model` i32 (4 B each)
    ///   bin_q  : `rank` i64 (8 B each)
    ///   wout_q : `n_experts * rank` i32 (4 B each)
    ///   bout_q : `n_experts` i64 (8 B each)
    /// The ROM builder writes these bytes verbatim into the params blob and the
    /// on-device router indexes them with the same strides, so host and ROM
    /// route from bit-identical tables by construction (they cannot drift).
    #[must_use]
    pub fn param_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(
            self.win_q.len() * 4
                + self.bin_q.len() * 8
                + self.wout_q.len() * 4
                + self.bout_q.len() * 8,
        );
        for &w in &self.win_q {
            b.extend_from_slice(&w.to_le_bytes());
        }
        for &w in &self.bin_q {
            b.extend_from_slice(&w.to_le_bytes());
        }
        for &w in &self.wout_q {
            b.extend_from_slice(&w.to_le_bytes());
        }
        for &w in &self.bout_q {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b
    }

    /// Reported structural hidden-accumulator bound (proven `<= i62`).
    #[must_use]
    pub fn hidden_structural_bound(&self) -> u64 {
        self.hidden_structural_bound
    }

    /// Reported structural raw-logit-accumulator bound (proven `<= i62`).
    #[must_use]
    pub fn raw_structural_bound(&self) -> u64 {
        self.raw_structural_bound
    }

    /// Route the raw pre-norm residual `x_i24` (i24 Q19.5, `d_model` lanes) to
    /// a top-1 expert index using PURELY INTEGER arithmetic. Deterministic and
    /// identical on host and ROM. See [`Self::route_with_logits`] for the raw
    /// logits (margin diagnostics).
    #[must_use]
    pub fn route(&self, x_i24: &[i32]) -> usize {
        self.route_with_logits(x_i24).0
    }

    /// [`Self::route`] returning the selected expert index and the raw i64
    /// logits, so callers can log the `raw[top1] - raw[top2]` margin.
    #[must_use]
    pub fn route_with_logits(&self, x_i24: &[i32]) -> (usize, Vec<i64>) {
        debug_assert_eq!(x_i24.len(), self.d_model, "router input width");
        // Q16.16 view of the residual (exact left shift).
        let xr: Vec<i64> = x_i24
            .iter()
            .map(|&v| i64::from(v) << ROUTER_XR_SHIFT)
            .collect();

        // hidden_q[k] = round_half_away(hidden_acc >> 16), hidden_acc at Q32.32.
        let mut hidden_q = vec![0i64; self.rank];
        for (k, hq) in hidden_q.iter_mut().enumerate() {
            let mut acc: i64 = self.bin_q[k];
            let row = &self.win_q[k * self.d_model..(k + 1) * self.d_model];
            for (&w, &xrv) in row.iter().zip(xr.iter()) {
                acc += i64::from(w) * xrv;
            }
            let mag =
                (acc.unsigned_abs() + (1 << (ROUTER_HIDDEN_SHIFT - 1))) >> ROUTER_HIDDEN_SHIFT;
            *hq = if acc < 0 { -(mag as i64) } else { mag as i64 };
        }

        // raw_acc[e] at Q32.32, argmax with strict `>` (lowest index wins ties).
        let mut raw = vec![0i64; self.n_experts];
        let mut best_e = 0usize;
        let mut best_v = i64::MIN;
        for (e, rv) in raw.iter_mut().enumerate() {
            let mut acc: i64 = self.bout_q[e];
            let row = &self.wout_q[e * self.rank..(e + 1) * self.rank];
            for (&w, &hk) in row.iter().zip(hidden_q.iter()) {
                acc += i64::from(w) * hk;
            }
            *rv = acc;
            if acc > best_v {
                best_v = acc;
                best_e = e;
            }
        }
        (best_e, raw)
    }
}

/// A single pre-norm residual FFN block: either a dense up/down pair
/// (`n_experts == 1`, the byte-exact legacy path) or a top-1 MoE bank of
/// experts fronted by a [`LowRankRouter`]. Top-1 routing runs exactly one
/// expert per block per token, so the active integer FFN math (and therefore
/// byte-exactness) is identical to the dense block.
#[derive(Debug, Clone)]
pub enum BlockFfn {
    Dense {
        up: TernaryLayer,
        down: TernaryLayer,
    },
    Moe {
        router: LowRankRouter,
        experts: Vec<(TernaryLayer, TernaryLayer)>,
    },
}

impl BlockFfn {
    #[must_use]
    pub fn is_moe(&self) -> bool {
        matches!(self, Self::Moe { .. })
    }

    /// The dense up/down pair, if this block is dense. Lets the dense export
    /// pipelines (`d192`, `d192_real`) keep reading `up`/`down` unchanged.
    #[must_use]
    pub fn as_dense(&self) -> Option<(&TernaryLayer, &TernaryLayer)> {
        match self {
            Self::Dense { up, down } => Some((up, down)),
            Self::Moe { .. } => None,
        }
    }

    /// Every (up, down) pair this block owns (one for Dense, `n_experts` for
    /// Moe). Used by the lowering's structural down-width/overflow scan.
    fn ffn_pairs(&self) -> Vec<(&TernaryLayer, &TernaryLayer)> {
        match self {
            Self::Dense { up, down } => vec![(up, down)],
            Self::Moe { experts, .. } => experts.iter().map(|(u, d)| (u, d)).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// checkpoint container
// ---------------------------------------------------------------------------

/// The raw exported checkpoint: f32 embedding, state in/out ternary
/// projections, per-slot Q8.8 decay raws, and the ternary FFN blocks.
#[derive(Debug, Clone)]
pub struct StateCheckpoint {
    topology: StateTopology,
    embedding: Vec<f32>,
    pub state_in: TernaryLayer,
    pub state_out: TernaryLayer,
    decay_raw: Vec<u16>,
    blocks: Vec<BlockFfn>,
}

/// State-checkpoint validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateModelError {
    Shape {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    Topology {
        what: &'static str,
        value: usize,
        max: usize,
    },
    NonFiniteEmbedding,
    DecayRawTooWide {
        slot: usize,
        raw: u16,
    },
    /// The state in-projection must keep i16 accumulators (the device state
    /// update reads i16); structurally guaranteed for d_model <= 255.
    StateInAccTooWide {
        row: usize,
        bound: i64,
    },
    /// A wide down-projection row's scale product overflows the u32 division
    /// numerator the device epilogue uses (`2*scale*|acc| + 127 < 2^32`).
    DownEpilogueOverflow {
        block: usize,
        row: usize,
        scale_raw: u16,
        acc_bound: i64,
    },
    /// A wide down-projection row's structural delta bound escapes the
    /// signed-i24 delta carrier (`floor((2*scale*acc_bound + 127)/254)` must
    /// fit [`DOWN_DELTA_WIDE_BOUND`]); the v2 wide path carries the delta
    /// exactly, so a checkpoint that structurally exceeds i24 needs a new
    /// carrier-width bead, not a silent clamp.
    DownDeltaEscapesI24 {
        block: usize,
        row: usize,
        scale_raw: u16,
        acc_bound: i64,
        delta_bound: u64,
    },
    /// A router tensor (input/expert projection or bias) contains a non-finite
    /// f32 value. The router runs in f32 to pick the expert index; NaN/Inf
    /// weights would make the argmax non-deterministic.
    NonFiniteRouter {
        block: usize,
        what: &'static str,
    },
    /// A fixed-point router weight (`round_ties_even(w * 2^16)`) does not fit a
    /// signed i32 (`|w| >= 2^15`). The `router-fx.v1` contract quantizes both
    /// projections at scale `2^16` into i32; a weight this large would need a
    /// wider carrier, so lowering fails loud rather than clamp.
    RouterWeightEscapesI32 {
        block: usize,
        what: &'static str,
        index: usize,
        quantized: i64,
    },
    /// The `router-fx.v1` hidden accumulator's structural worst case (from the
    /// actual `|win_q|` and the i24 saturation bound `+/-(2^23 - 1)`) escapes
    /// the proven i62 carrier. The design targeted i48, but the real d192x8
    /// student's structural hidden bound is ~1.92e16 under the mandated Q16.16
    /// `xr` view, so the proven width was widened to i62 (still an exact i64
    /// accumulator with >2 bits of headroom); a checkpoint past i62 needs a new
    /// carrier-width bead, not a silent clamp.
    RouterHiddenEscapesI62 {
        block: usize,
        row: usize,
        bound: u64,
    },
    /// The `router-fx.v1` raw-logit accumulator's structural worst case (from
    /// `|wout_q|` and the proven `|hidden_q|` bound) escapes the i62 carrier.
    /// Fails loud rather than clamp.
    RouterRawLogitEscapesI62 {
        block: usize,
        expert: usize,
        bound: u64,
    },
    /// A MoE block's expert count does not match the declared `n_experts`
    /// (`experts.len() != topology.n_experts`).
    MoeArityMismatch {
        block: usize,
        expected: usize,
        actual: usize,
    },
    /// A block's FFN kind disagrees with the topology: a `Dense` block in an
    /// `n_experts > 1` model, or a `Moe` block in an `n_experts == 1` model.
    MoeDenseMixup {
        block: usize,
        n_experts: usize,
        block_is_moe: bool,
    },
    Model(ModelRefError),
}

impl fmt::Display for StateModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape {
                what,
                expected,
                actual,
            } => write!(f, "{what}: expected {expected} elements, got {actual}"),
            Self::Topology { what, value, max } => {
                write!(
                    f,
                    "topology out of device range: {what} = {value} (1..={max})"
                )
            }
            Self::NonFiniteEmbedding => write!(f, "embedding contains non-finite values"),
            Self::DecayRawTooWide { slot, raw } => write!(
                f,
                "decay slot {slot} raw {raw} exceeds the u8 device table (MT4 rates are <= 240)"
            ),
            Self::StateInAccTooWide { row, bound } => write!(
                f,
                "state in-projection row {row} structural accumulator bound {bound} escapes i16"
            ),
            Self::DownEpilogueOverflow {
                block,
                row,
                scale_raw,
                acc_bound,
            } => write!(
                f,
                "block {block} down row {row}: 2 * scale {scale_raw} * structural acc bound \
                 {acc_bound} + 127 overflows the u32 epilogue numerator"
            ),
            Self::DownDeltaEscapesI24 {
                block,
                row,
                scale_raw,
                acc_bound,
                delta_bound,
            } => write!(
                f,
                "block {block} down row {row}: structural delta bound {delta_bound} \
                 (scale {scale_raw}, acc bound {acc_bound}) escapes the signed-i24 delta \
                 carrier ({DOWN_DELTA_WIDE_BOUND})"
            ),
            Self::NonFiniteRouter { block, what } => write!(
                f,
                "block {block} router {what} contains a non-finite f32 value"
            ),
            Self::RouterWeightEscapesI32 {
                block,
                what,
                index,
                quantized,
            } => write!(
                f,
                "block {block} router-fx.v1 {what}[{index}] quantized to {quantized} \
                 (round_ties_even(w * 2^16)) escapes signed i32"
            ),
            Self::RouterHiddenEscapesI62 { block, row, bound } => write!(
                f,
                "block {block} router-fx.v1 hidden row {row} structural bound {bound} escapes i62"
            ),
            Self::RouterRawLogitEscapesI62 {
                block,
                expert,
                bound,
            } => write!(
                f,
                "block {block} router-fx.v1 raw logit expert {expert} structural bound {bound} \
                 escapes i62"
            ),
            Self::MoeArityMismatch {
                block,
                expected,
                actual,
            } => write!(
                f,
                "block {block} MoE arity mismatch: expected {expected} experts, got {actual}"
            ),
            Self::MoeDenseMixup {
                block,
                n_experts,
                block_is_moe,
            } => write!(
                f,
                "block {block} FFN kind disagrees with topology (n_experts = {n_experts}, \
                 block is {}): dense topologies need Dense blocks and MoE topologies need \
                 Moe blocks",
                if *block_is_moe { "Moe" } else { "Dense" }
            ),
            Self::Model(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StateModelError {}

impl From<ModelRefError> for StateModelError {
    fn from(e: ModelRefError) -> Self {
        Self::Model(e)
    }
}

impl StateCheckpoint {
    /// Build a **dense** checkpoint (`n_experts == 1`). Back-compat entry
    /// point: every existing caller passes a `Vec<BlockWeights>` and gets the
    /// byte-identical dense pipeline. Each block becomes a [`BlockFfn::Dense`].
    pub fn new(
        topology: StateTopology,
        embedding: Vec<f32>,
        state_in: TernaryLayer,
        state_out: TernaryLayer,
        decay_raw: Vec<u16>,
        blocks: Vec<BlockWeights>,
    ) -> Result<Self, StateModelError> {
        let ffns = blocks
            .into_iter()
            .map(|b| BlockFfn::Dense {
                up: b.up,
                down: b.down,
            })
            .collect();
        Self::new_blocks(topology, embedding, state_in, state_out, decay_raw, ffns)
    }

    /// Build a checkpoint from already-typed [`BlockFfn`] blocks (dense or
    /// MoE). Used by the `f_s8_moe_state_checkpoint_export.v2` loader.
    pub fn new_moe(
        topology: StateTopology,
        embedding: Vec<f32>,
        state_in: TernaryLayer,
        state_out: TernaryLayer,
        decay_raw: Vec<u16>,
        blocks: Vec<BlockFfn>,
    ) -> Result<Self, StateModelError> {
        Self::new_blocks(topology, embedding, state_in, state_out, decay_raw, blocks)
    }

    fn new_blocks(
        topology: StateTopology,
        embedding: Vec<f32>,
        state_in: TernaryLayer,
        state_out: TernaryLayer,
        decay_raw: Vec<u16>,
        blocks: Vec<BlockFfn>,
    ) -> Result<Self, StateModelError> {
        // Host evaluator: relax only the single-page vocab cap (subword V=1024
        // students load here; on-device logit paging is deploy step 2). The
        // ROM planner still enforces the strict cap at build time.
        topology.validate_host()?;
        let t = &topology;
        if embedding.len() != t.vocab * t.d_model {
            return Err(StateModelError::Shape {
                what: "embedding",
                expected: t.vocab * t.d_model,
                actual: embedding.len(),
            });
        }
        if embedding.iter().any(|v| !v.is_finite()) {
            return Err(StateModelError::NonFiniteEmbedding);
        }
        if state_in.rows() != t.state_slots || state_in.cols() != t.d_model {
            return Err(StateModelError::Shape {
                what: "state in-projection",
                expected: t.state_slots * t.d_model,
                actual: state_in.rows() * state_in.cols(),
            });
        }
        if state_out.rows() != t.d_model || state_out.cols() != t.state_slots {
            return Err(StateModelError::Shape {
                what: "state out-projection",
                expected: t.d_model * t.state_slots,
                actual: state_out.rows() * state_out.cols(),
            });
        }
        if decay_raw.len() != t.state_slots {
            return Err(StateModelError::Shape {
                what: "decay slots",
                expected: t.state_slots,
                actual: decay_raw.len(),
            });
        }
        for (slot, &raw) in decay_raw.iter().enumerate() {
            if raw > 255 {
                return Err(StateModelError::DecayRawTooWide { slot, raw });
            }
        }
        if blocks.len() != t.n_blocks {
            return Err(StateModelError::Shape {
                what: "blocks",
                expected: t.n_blocks,
                actual: blocks.len(),
            });
        }
        let check_up_down =
            |up: &TernaryLayer, down: &TernaryLayer| -> Result<(), StateModelError> {
                if up.rows() != t.d_ff || up.cols() != t.d_model {
                    return Err(StateModelError::Shape {
                        what: "up projection",
                        expected: t.d_ff * t.d_model,
                        actual: up.rows() * up.cols(),
                    });
                }
                if down.rows() != t.d_model || down.cols() != t.d_ff {
                    return Err(StateModelError::Shape {
                        what: "down projection",
                        expected: t.d_model * t.d_ff,
                        actual: down.rows() * down.cols(),
                    });
                }
                Ok(())
            };
        for (bi, block) in blocks.iter().enumerate() {
            // The FFN kind must agree with the topology. A MoE topology
            // (n_experts > 1) requires Moe blocks (a Dense block would drop
            // routing). A dense topology (n_experts == 1) accepts either a
            // Dense block or a single-expert Moe block — top-1 routing over one
            // expert always picks expert 0, so it is byte-equivalent to dense
            // (this is exactly the n_experts == 1 == dense bridge).
            if t.is_moe() && !block.is_moe() {
                return Err(StateModelError::MoeDenseMixup {
                    block: bi,
                    n_experts: t.n_experts,
                    block_is_moe: block.is_moe(),
                });
            }
            match block {
                BlockFfn::Dense { up, down } => check_up_down(up, down)?,
                BlockFfn::Moe { router, experts } => {
                    if experts.len() != t.n_experts {
                        return Err(StateModelError::MoeArityMismatch {
                            block: bi,
                            expected: t.n_experts,
                            actual: experts.len(),
                        });
                    }
                    router.validate(bi)?;
                    if router.rank == 0 {
                        return Err(StateModelError::Shape {
                            what: "router rank",
                            expected: 1,
                            actual: 0,
                        });
                    }
                    if router.d_model != t.d_model || router.n_experts != t.n_experts {
                        return Err(StateModelError::Shape {
                            what: "router topology",
                            expected: t.d_model * t.n_experts,
                            actual: router.d_model * router.n_experts,
                        });
                    }
                    for (up, down) in experts {
                        check_up_down(up, down)?;
                    }
                }
            }
        }
        Ok(Self {
            topology,
            embedding,
            state_in,
            state_out,
            decay_raw,
            blocks,
        })
    }

    #[must_use]
    pub fn topology(&self) -> StateTopology {
        self.topology
    }

    #[must_use]
    pub fn embedding_row(&self, id: u8) -> &[f32] {
        let start = usize::from(id) * self.topology.d_model;
        &self.embedding[start..start + self.topology.d_model]
    }

    #[must_use]
    pub fn blocks(&self) -> &[BlockFfn] {
        &self.blocks
    }

    #[must_use]
    pub fn decay_raw(&self) -> &[u16] {
        &self.decay_raw
    }
}

// ---------------------------------------------------------------------------
// f32 reference (trainer port)
// ---------------------------------------------------------------------------

fn f32_rms_norm_clip(x: &[f32]) -> Vec<f32> {
    let mut sum_sq = 0.0f32;
    for v in x {
        sum_sq += v * v;
    }
    let mean_sq = sum_sq / (x.len() as f32);
    let rms = (mean_sq + NORM_EPS).sqrt();
    x.iter()
        .map(|v| (v / rms).clamp(-ACT_RANGE, ACT_RANGE))
        .collect()
}

fn f32_ternary_matvec(layer: &TernaryLayer, input: &[f32], out: &mut [f32]) {
    debug_assert_eq!(input.len(), layer.cols());
    debug_assert_eq!(out.len(), layer.rows());
    for (row, out_v) in out.iter_mut().enumerate() {
        let scale = f32::from(layer.scale_raw(row)) / 256.0;
        let mut acc = 0.0f32;
        for (w, v) in layer.row(row).iter().zip(input.iter()) {
            acc += (f32::from(*w) * scale) * v;
        }
        *out_v = acc;
    }
}

/// The trainer's hard-ternary f32 forward pass for one token, with the f32
/// recurrent state carried in `state` (`state_slots` long, zeroed at stream
/// start). Returns the `vocab` tied-head logits.
#[must_use]
pub fn f32_state_forward(ck: &StateCheckpoint, prev: u8, state: &mut [f32]) -> Vec<f32> {
    let t = ck.topology();
    assert_eq!(state.len(), t.state_slots, "state width");
    let mut x = ck.embedding_row(prev).to_vec();

    // State block: delta from the normed+act-quantized input, decayed state
    // update, act-quantized out-projection, residual add.
    let mut normed = f32_rms_norm_clip(&x);
    for v in &mut normed {
        *v = f32_act_fake_quant(*v);
    }
    let mut delta = vec![0.0f32; t.state_slots];
    f32_ternary_matvec(&ck.state_in, &normed, &mut delta);
    for (slot, (h, d)) in state.iter_mut().zip(delta.iter()).enumerate() {
        let decay = f32::from(ck.decay_raw[slot]) / 256.0;
        *h = *h * decay + *d;
    }
    let mut y = vec![0.0f32; t.d_model];
    f32_ternary_matvec(&ck.state_out, state, &mut y);
    for (xv, yv) in x.iter_mut().zip(y.iter()) {
        *xv += f32_act_fake_quant(*yv);
    }

    // The same pre-norm residual FFN stack as the dense export.
    let mut hidden = vec![0.0f32; t.d_ff];
    let mut ffn_delta = vec![0.0f32; t.d_model];
    for block in ck.blocks() {
        // Top-1 MoE: route on the RAW pre-norm residual, then run the selected
        // expert; dense blocks run their single up/down pair. Either way the
        // FFN math below is identical.
        let (up, down) = match block {
            BlockFfn::Dense { up, down } => (up, down),
            BlockFfn::Moe { router, experts } => {
                let e = router.route_f32(&x);
                let (u, d) = &experts[e];
                (u, d)
            }
        };
        let mut normed = f32_rms_norm_clip(&x);
        for v in &mut normed {
            *v = f32_act_fake_quant(*v);
        }
        f32_ternary_matvec(up, &normed, &mut hidden);
        for v in &mut hidden {
            *v = f32_act_fake_quant(gelu_approx_f32(*v));
        }
        f32_ternary_matvec(down, &hidden, &mut ffn_delta);
        for (xv, dv) in x.iter_mut().zip(ffn_delta.iter()) {
            *xv += dv;
        }
    }

    let normed = f32_rms_norm_clip(&x);
    let mut logits = vec![0.0f32; t.vocab];
    for (id, logit) in logits.iter_mut().enumerate() {
        let row = ck.embedding_row(id as u8);
        let mut acc = 0.0f32;
        for (n, e) in normed.iter().zip(row.iter()) {
            acc += n * e;
        }
        *logit = acc;
    }
    logits
}

// ---------------------------------------------------------------------------
// canonical integer semantics
// ---------------------------------------------------------------------------

/// Floor integer square root of a value below 2^48 (result fits u24).
#[must_use]
pub fn isqrt_u48(n: u64) -> u32 {
    debug_assert!(n < 1 << 48);
    let mut rem = n;
    let mut root: u64 = 0;
    let mut bit: u64 = 1 << 46;
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    u32::try_from(root).expect("isqrt of u48 fits u32")
}

/// Device accumulator carrier width for one matvec, decided at lowering
/// time from the exact structural per-row bound over the actual ternary
/// weights (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccWidth {
    /// 2-byte accumulators, single-pass V3 weight code (v0 convention).
    I16,
    /// 3-byte accumulators, column-segmented weight code (fan-in > i16).
    I24,
}

/// Structural worst-case accumulator range of one ternary row under u8
/// zero-point-128 activations: `acc = sum(w * (act - 128))`,
/// `act - 128 in [-128, 127]`.
#[must_use]
pub fn row_acc_bounds(row: &[i8]) -> (i64, i64) {
    let pos = row.iter().filter(|w| **w == 1).count() as i64;
    let neg = row.iter().filter(|w| **w == -1).count() as i64;
    (-(128 * pos + 127 * neg), 127 * pos + 128 * neg)
}

fn layer_needs_i24(layer: &TernaryLayer) -> bool {
    (0..layer.rows()).any(|r| {
        let (lo, hi) = row_acc_bounds(layer.row(r));
        lo < -32768 || hi > 32767
    })
}

/// A lowered FFN block: dense (one up/down `LoweredLayer` pair) or top-1 MoE
/// (an f32 [`LowRankRouter`] plus one lowered up/down pair per expert). The
/// integer FFN kernel run for the selected expert is byte-identical to the
/// dense block — only the expert *selection* is new.
#[derive(Debug, Clone)]
pub enum LoweredBlockFfn {
    Dense {
        up: LoweredLayer,
        down: LoweredLayer,
    },
    Moe {
        /// The f32 reference router (kept for the parity gate; NOT used by the
        /// deployed integer forward).
        router: LowRankRouter,
        /// The purely-integer `router-fx.v1` router the integer forward routes
        /// on (no f32 enters the deployed path).
        fixed_router: FixedRouter,
        experts: Vec<(LoweredLayer, LoweredLayer)>,
    },
}

/// The integer-lowered stateful model: every table the canonical integer
/// function (and therefore the ROM) needs.
#[derive(Debug, Clone)]
pub struct IntStateLoweredModel {
    pub topology: StateTopology,
    /// Embedding rows on the Q19.5 residual grid (`[vocab * d_model]`,
    /// row-major, values in the i24 range).
    pub emb_resid: Vec<i32>,
    /// Tied-head weights as per-tensor symmetric i8.
    pub head_i8: Vec<i8>,
    /// `max|emb| / 127`: real value of one head-weight step.
    pub head_step: f32,
    /// GELU LUT (shared construction with the dense lowering).
    pub gelu_lut: [u8; 255],
    /// State residual-add LUT: index `p + 127` for `p in [-127, 127]`;
    /// `round_ties_even(p * STATE_RESID_ONE * ACT_RANGE / QMAX)`.
    pub y_resid_lut: [i16; 255],
    /// State in-projection with `-128 * sum(row)` accumulator seeds
    /// (always i16 accumulators; validated at lowering).
    pub state_in: LoweredLayer,
    /// State out-projection (operates on the raw state, no zero-point seed).
    pub state_out: TernaryLayer,
    /// Per-slot decay raws, validated to fit the u8 device table.
    pub decay_u8: Vec<u8>,
    /// Lowered **dense** FFN blocks (up, down). For a dense checkpoint this is
    /// the whole model and the ROM builder (`asm_impl_state`) consumes it
    /// directly, byte-identical to before MoE existed. For a MoE checkpoint
    /// this holds each block's **expert 0** (dispatch-agnostic placeholder);
    /// the real per-token dispatch lives in [`Self::block_ffns`]. The MoE ROM
    /// builder (deploy step 4) reads `block_ffns`, not this field.
    pub blocks: Vec<(LoweredLayer, LoweredLayer)>,
    /// Lowered FFN blocks with dispatch (dense or top-1 MoE). The host
    /// [`Self::forward`] routes over these; `n_experts == 1` blocks are
    /// [`LoweredBlockFfn::Dense`] and forward byte-identically to `blocks`.
    pub block_ffns: Vec<LoweredBlockFfn>,
    /// Down-projection accumulator width, uniform across blocks (i24 if any
    /// block's down projection structurally requires it).
    pub down_width: AccWidth,
    /// Largest structural |acc| bound over every down row (reported).
    pub down_acc_structural_bound: i64,
    /// Largest structural delta bound `floor((2*scale*acc_bound + 127)/254)`
    /// over every down row (raw Q19.5; reported). On the wide path the
    /// lowering proves this fits [`DOWN_DELTA_WIDE_BOUND`], which is what
    /// makes the v2 clamp-free i24 delta carrier sound.
    pub down_delta_structural_bound: u64,
}

/// Range/overflow observations for the stateful integer evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateForwardStats {
    /// Dense-convention stats for the FFN blocks + head (matvec accumulators,
    /// scale products, down deltas, logits). `max_abs_residual` is on the
    /// Q19.5 grid.
    pub ffn: IntForwardStats,
    /// Max |in-projection accumulator| (i16 bound on device).
    pub max_abs_in_acc: u32,
    /// Max |m| = |scale_raw * in_acc| (exact state delta).
    pub max_abs_state_delta: u64,
    /// Max |state slot| after update (saturating i24 bound 2^23 - 1).
    pub max_abs_state: u32,
    /// State saturation events (expected 0 on real data).
    pub state_clamp_events: u64,
    /// Max |out-projection accumulator| over the i24 state (i32 on host,
    /// 4-byte accumulator on device).
    pub max_abs_out_acc: u64,
    /// Max |scale_raw * out_acc| (fits u64; the device splits the multiply).
    pub max_abs_out_scale_product: u64,
    /// Positions where the y activation saturated at +/-127.
    pub y_saturation_events: u64,
    /// i24 residual wrap events (expected 0).
    pub residual_i24_wrap_events: u64,
    /// Max |down accumulator| (only distinct from `ffn.max_abs_matvec_acc`
    /// when the down width is i24).
    pub max_abs_down_acc: u32,
}

impl StateForwardStats {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ffn: IntForwardStats::new(),
            max_abs_in_acc: 0,
            max_abs_state_delta: 0,
            max_abs_state: 0,
            state_clamp_events: 0,
            max_abs_out_acc: 0,
            max_abs_out_scale_product: 0,
            y_saturation_events: 0,
            residual_i24_wrap_events: 0,
            max_abs_down_acc: 0,
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.ffn.merge(&other.ffn);
        self.max_abs_in_acc = self.max_abs_in_acc.max(other.max_abs_in_acc);
        self.max_abs_state_delta = self.max_abs_state_delta.max(other.max_abs_state_delta);
        self.max_abs_state = self.max_abs_state.max(other.max_abs_state);
        self.state_clamp_events += other.state_clamp_events;
        self.max_abs_out_acc = self.max_abs_out_acc.max(other.max_abs_out_acc);
        self.max_abs_out_scale_product = self
            .max_abs_out_scale_product
            .max(other.max_abs_out_scale_product);
        self.y_saturation_events += other.y_saturation_events;
        self.residual_i24_wrap_events += other.residual_i24_wrap_events;
        self.max_abs_down_acc = self.max_abs_down_acc.max(other.max_abs_down_acc);
    }
}

impl Default for StateForwardStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Full trace of one canonical stateful integer forward pass, including the
/// values the ROM gate compares byte-exactly. All vectors are sized by the
/// model topology.
#[derive(Debug, Clone)]
pub struct IntStateForwardTrace {
    /// State-block norm output on the u8 zp128 grid (`d_model`).
    pub state_norm_act: Vec<u8>,
    /// In-projection raw accumulators (i16, `state_slots`).
    pub state_in_acc: Vec<i16>,
    /// The state vector after this token's update (saturating i24 in i32).
    pub state_after: Vec<i32>,
    /// Out-projection raw accumulators (i32, `d_model`).
    pub state_out_acc: Vec<i32>,
    /// y on the u8 zp128 grid (`d_model`).
    pub y_act: Vec<u8>,
    /// Residual vector after each FFN block (i24 in i32, Q19.5).
    pub block_residuals: Vec<Vec<i32>>,
    /// Top-1 expert index selected by the fixed-point router at each MoE block,
    /// in block order. Empty for a dense (`n_experts == 1`) model. The ROM MoE
    /// gate compares its per-block `EXPERT_SEL` byte against this.
    pub selected_experts: Vec<usize>,
    /// Block-0 debug checkpoints (mirrored by the ROM's debug dumps).
    pub block0_norm_act: Vec<u8>,
    pub block0_up_acc: Vec<i16>,
    pub block0_gelu_act: Vec<u8>,
    /// Block-0 down accumulators (i16 range or i24 range per `down_width`).
    pub block0_down_acc: Vec<i32>,
    /// Final norm output on the activation grid (`[-127, 127]`).
    pub final_q: Vec<i16>,
    /// Tied-head integer logits (i24-range values held in i32). Under
    /// [`LogitPaging::SinglePage`] this is the full `vocab` vector (gate-compat,
    /// exactly as before). Under [`LogitPaging::Paged`] it holds ONLY the last
    /// resident page (`<= LOGIT_PAGE_IDS` ids) — the full `3 * vocab` vector is
    /// never materialized; use [`Self::logit_pages`] for per-page views.
    pub logits: Vec<i32>,
    /// One `Vec<i32>` per streamed logit page (`ceil(vocab / 85)` pages of
    /// `<= 85` ids each), in ascending id order. Under `SinglePage` this is a
    /// single page equal to [`Self::logits`]. The ROM gate peeks the WRAM logit
    /// page against `logit_pages.last()` under `Paged`.
    pub logit_pages: Vec<Vec<i32>>,
    /// Argmax id truncated to u8 (the charset/dense pipeline token id).
    pub argmax: u8,
    /// Full argmax id (lowest index wins ties). Distinct from [`Self::argmax`]
    /// only for wide-vocab (V > 256) subword MoE students. Fits u16 for every
    /// paged vocab (`<= LOGIT_PAGED_VOCAB_MAX < 2^16`).
    pub argmax_full: usize,
    /// Finalized running top-k heap (selection order: logit desc, id asc) over
    /// all pages, sized `min(k, vocab)` where `k = HEAP_K_MAX`. Under `Paged`
    /// the ROM's WRAM heap region must equal this. Under `SinglePage` it is the
    /// top-`HEAP_K_MAX` of the single page (still exact; unused by the legacy
    /// gate but cheap to fill).
    pub topk_heap: Vec<HeapEntry>,
    pub stats: StateForwardStats,
}

/// One finalized top-k candidate from the paged head: `(logit, id)` in the
/// sampler's selection total order (logit descending, id ascending on ties).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapEntry {
    pub logit: i32,
    pub id: usize,
}

/// A running top-k selector over streamed logit pages. It folds one `(logit,
/// id)` at a time, retaining the `k` entries with the largest logits under the
/// EXACT total order the deployed sampler selects with (logit descending; on
/// equal logits the lower id ranks higher). [`Self::finalize`] returns them in
/// that selection order, byte-identical to `decode::sample_topk`'s pass-k scan
/// over the full logit vector.
///
/// The ROM mirrors this with an in-WRAM heap of `<= HEAP_K_MAX` entries; the
/// insertion order is irrelevant because the retained SET and its final order
/// are a pure function of the `(logit, id)` multiset and `k`.
#[derive(Debug, Clone)]
pub struct RunningTopK {
    k: usize,
    /// Retained entries, kept sorted ascending in selection order (worst first,
    /// best last) so eviction pops index 0.
    entries: Vec<HeapEntry>,
}

impl RunningTopK {
    /// A running top-k with capacity `k` (`k >= 1`; `k <= HEAP_K_MAX` on the
    /// deployed path, but the host helper accepts any `k`).
    #[must_use]
    pub fn new(k: usize) -> Self {
        assert!(k >= 1, "top-k needs k >= 1");
        Self {
            k,
            entries: Vec::with_capacity(k + 1),
        }
    }

    /// True when candidate `a` ranks strictly ABOVE candidate `b` in the
    /// sampler's selection total order: higher logit wins; on equal logits the
    /// lower id wins (the sampler's strict-`>` scan keeps the lowest index).
    fn ranks_above(a: HeapEntry, b: HeapEntry) -> bool {
        a.logit > b.logit || (a.logit == b.logit && a.id < b.id)
    }

    /// Offer one `(logit, id)` to the running top-k. Retains it iff it ranks
    /// above the current worst retained entry (or the set is not yet full).
    pub fn offer(&mut self, logit: i32, id: usize) {
        let cand = HeapEntry { logit, id };
        if self.entries.len() < self.k {
            self.insert_sorted(cand);
            return;
        }
        // `entries[0]` is the current worst. Replace iff the candidate ranks
        // above it (strictly; an equal candidate cannot beat a lower-id
        // incumbent, matching the sampler's lowest-index tiebreak).
        if Self::ranks_above(cand, self.entries[0]) {
            self.entries.remove(0);
            self.insert_sorted(cand);
        }
    }

    /// Insert keeping `entries` ascending in selection order (worst at 0, best
    /// last), so eviction pops index 0.
    fn insert_sorted(&mut self, cand: HeapEntry) {
        // First index whose incumbent ranks ABOVE `cand`: `cand` slots in just
        // before it, keeping the worst-first ordering.
        let pos = self
            .entries
            .iter()
            .position(|&e| Self::ranks_above(e, cand))
            .unwrap_or(self.entries.len());
        self.entries.insert(pos, cand);
    }

    /// The retained entries in selection order (best first): logit descending,
    /// id ascending on ties — exactly the order `decode::sample_topk` visits.
    #[must_use]
    pub fn finalize(&self) -> Vec<HeapEntry> {
        let mut out = self.entries.clone();
        out.reverse();
        out
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Norm+quant over `d_model` i24 Q19.5 residual lanes. Same canonical steps
/// as the dense `int_norm_quant`, widened: 7-byte sum-of-squares
/// accumulator, `mean = floor(ss / d_model)` (shift + odd-constant division
/// on device), 48-bit floor isqrt, u32 numerators in the rounded division.
pub fn int_norm_quant24(x: &[i32], stats: &mut IntForwardStats) -> Vec<i16> {
    let mut ss: u64 = 0;
    for &v in x {
        let a = u64::from(v.unsigned_abs());
        ss += a * a;
    }
    stats.max_norm_sumsq = stats.max_norm_sumsq.max(ss);
    let mean = ss / (x.len() as u64);
    debug_assert!(mean < 1 << 46, "i24 lanes bound the mean below 2^46");
    let r = u64::from(isqrt_u48(mean + 1));
    stats.min_norm_rms_raw = stats.min_norm_rms_raw.min(r as u32);
    let d = 8 * r;
    let d2 = 16 * r;
    let mut q = vec![0i16; x.len()];
    for (qv, &v) in q.iter_mut().zip(x.iter()) {
        let a = u64::from(v.unsigned_abs());
        let num = a * 254 + d;
        debug_assert!(num < 1 << 32, "division numerator fits u32 for i24 lanes");
        let q_abs = (num / d2).min(i64::from(QMAX) as u64) as i16;
        *qv = if v < 0 { -q_abs } else { q_abs };
    }
    q
}

/// Ternary matvec with u8 zero-point-128 activations and i24 (3-byte)
/// accumulators: the wide twin of `int_matvec` for fan-in past the i16
/// structural bound. The value is exact; the lowering guarantees it fits
/// i24 structurally.
pub(crate) fn int_matvec_i24(
    layer: &TernaryLayer,
    act: &[u8],
    out: &mut [i32],
    stats: &mut StateForwardStats,
) {
    debug_assert_eq!(act.len(), layer.cols());
    debug_assert_eq!(out.len(), layer.rows());
    for (row, out_v) in out.iter_mut().enumerate() {
        let mut acc: i64 = 0;
        for (w, u) in layer.row(row).iter().zip(act.iter()) {
            acc += i64::from(*w) * (i64::from(*u) - 128);
        }
        assert!(
            (-(1 << 23)..(1 << 23)).contains(&acc),
            "wide matvec accumulator {acc} escapes i24 (structurally impossible for fan-in <= 65536)"
        );
        stats.max_abs_down_acc = stats.max_abs_down_acc.max(acc.unsigned_abs() as u32);
        *out_v = acc as i32;
    }
}

/// Measurement instrument for the wide down-projection delta: a fixed-width
/// histogram of the **unclamped** delta magnitude `round_half_away(|m|/127)`
/// on the Q19.5 grid, recorded *before* any carrier clamp so carrier widths
/// are chosen from real data (bd-2vkqt). Purely observational: attaching a
/// probe never changes the canonical integer semantics.
#[derive(Debug, Clone)]
pub struct DownDeltaProbe {
    /// `counts[b]` counts deltas with `|d| in [32b, 32b + 31]` (1 real unit
    /// per bucket on the Q19.5 grid); the last bucket absorbs everything at
    /// or above `32 * (BUCKETS - 1)` (~16.7M raw, the u32/254 ceiling).
    counts: Vec<u64>,
    max: u64,
    total: u64,
}

impl DownDeltaProbe {
    /// Raw Q19.5 units per histogram bucket.
    pub const BUCKET_WIDTH: u64 = 32;
    const BUCKETS: usize = (1 << 19) + 1;

    #[must_use]
    pub fn new() -> Self {
        Self {
            counts: vec![0; Self::BUCKETS],
            max: 0,
            total: 0,
        }
    }

    fn record(&mut self, d_abs: u64) {
        let bucket = usize::try_from(d_abs / Self::BUCKET_WIDTH)
            .unwrap_or(Self::BUCKETS - 1)
            .min(Self::BUCKETS - 1);
        self.counts[bucket] += 1;
        self.max = self.max.max(d_abs);
        self.total += 1;
    }

    pub fn merge(&mut self, other: &Self) {
        for (a, b) in self.counts.iter_mut().zip(other.counts.iter()) {
            *a += b;
        }
        self.max = self.max.max(other.max);
        self.total += other.total;
    }

    /// Exact maximum recorded |delta| (raw Q19.5).
    #[must_use]
    pub fn max_abs(&self) -> u64 {
        self.max
    }

    /// Number of recorded deltas.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Exact count of deltas with `|d| >= raw`. `raw` must be a multiple of
    /// [`Self::BUCKET_WIDTH`] so the count is bucket-exact.
    #[must_use]
    pub fn count_at_or_above(&self, raw: u64) -> u64 {
        assert_eq!(
            raw % Self::BUCKET_WIDTH,
            0,
            "threshold must be bucket-aligned"
        );
        let from = usize::try_from(raw / Self::BUCKET_WIDTH)
            .unwrap_or(Self::BUCKETS)
            .min(Self::BUCKETS);
        self.counts[from..].iter().sum()
    }

    /// Upper edge (raw Q19.5) of the bucket containing quantile `q` of the
    /// recorded magnitudes: at least a fraction `q` of deltas are <= the
    /// returned value (within one 32-raw bucket; the max is exact).
    #[must_use]
    pub fn quantile_upper_bound(&self, q: f64) -> u64 {
        assert!((0.0..=1.0).contains(&q), "quantile in [0, 1]");
        if self.total == 0 {
            return 0;
        }
        #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
        let need = (q * self.total as f64).ceil() as u64;
        let mut cum = 0u64;
        for (bucket, &c) in self.counts.iter().enumerate() {
            cum += c;
            if cum >= need {
                let edge = (bucket as u64 + 1) * Self::BUCKET_WIDTH - 1;
                return edge.min(self.max);
            }
        }
        self.max
    }
}

impl Default for DownDeltaProbe {
    fn default() -> Self {
        Self::new()
    }
}

/// Wide down epilogue (state-int-semantics.v2): exact Q19.5 delta
/// `sign(m) * ((|m|*2 + 127) div 254)` with `m = scale_raw * acc`, carried
/// in a signed i24 with **no clamp** — the lowering proves the structural
/// per-row bound fits [`DOWN_DELTA_WIDE_BOUND`] (and therefore the u32
/// division numerator), so the assert is unreachable for lowered models.
pub(crate) fn int_down_delta_i24(
    acc: i32,
    scale_raw: u16,
    stats: &mut IntForwardStats,
    probe: Option<&mut DownDeltaProbe>,
) -> i32 {
    let m = i64::from(scale_raw) * i64::from(acc);
    stats.max_abs_scale_product = stats.max_abs_scale_product.max(m.unsigned_abs());
    let num = m.unsigned_abs() * 2 + 127;
    let d_abs = num / 254;
    if let Some(p) = probe {
        p.record(d_abs);
    }
    assert!(
        d_abs <= DOWN_DELTA_WIDE_BOUND,
        "wide down delta {d_abs} escapes the i24 carrier (the lowering's structural \
         per-row bound proves this cannot happen for lowered models)"
    );
    stats.max_abs_down_delta = stats.max_abs_down_delta.max(d_abs);
    if m < 0 { -(d_abs as i32) } else { d_abs as i32 }
}

/// Wrap a residual add result to the i24 range (mod 2^24, sign-extended),
/// mirroring the device's 3-byte adds.
fn wrap_i24(v: i32) -> i32 {
    (v << 8) >> 8
}

impl IntStateLoweredModel {
    pub fn lower(ck: &StateCheckpoint) -> Result<Self, StateModelError> {
        let t = ck.topology();
        // Embedding on the Q19.5 residual grid (round-ties-even in f64).
        let mut emb_resid = Vec::with_capacity(t.vocab * t.d_model);
        let mut max_abs = 0.0f32;
        for id in 0..t.vocab {
            for &v in ck.embedding_row(id as u8) {
                max_abs = max_abs.max(v.abs());
                let q = rte_i64(f64::from(v) * f64::from(STATE_RESID_ONE))
                    .clamp(i64::from(RESID_I24_MIN), i64::from(RESID_I24_MAX));
                emb_resid.push(q as i32);
            }
        }

        // Head i8 (per-tensor symmetric).
        let head_step = max_abs / QMAX as f32;
        let mut head_i8 = Vec::with_capacity(t.vocab * t.d_model);
        for id in 0..t.vocab {
            for &v in ck.embedding_row(id as u8) {
                let q = rte_i64(f64::from(v) * f64::from(QMAX) / f64::from(max_abs))
                    .clamp(-i64::from(QMAX), i64::from(QMAX));
                head_i8.push(q as i8);
            }
        }

        // y residual LUT on the Q19.5 grid.
        let mut y_resid_lut = [0i16; 255];
        for (idx, entry) in y_resid_lut.iter_mut().enumerate() {
            let p = idx as i64 - i64::from(QMAX);
            let raw = rte_i64(
                p as f64 * f64::from(STATE_RESID_ONE) * f64::from(ACT_RANGE) / f64::from(QMAX),
            );
            *entry = i16::try_from(raw).expect("|p * 256 / 127| <= 256 fits i16");
        }

        let decay_u8 = ck
            .decay_raw
            .iter()
            .enumerate()
            .map(|(slot, &raw)| {
                u8::try_from(raw).map_err(|_| StateModelError::DecayRawTooWide { slot, raw })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Width plan: the state in-projection must stay i16 (the device
        // state update reads i16 accumulators)...
        for row in 0..ck.state_in.rows() {
            let (lo, hi) = row_acc_bounds(ck.state_in.row(row));
            if lo < -32768 || hi > 32767 {
                return Err(StateModelError::StateInAccTooWide {
                    row,
                    bound: lo.abs().max(hi),
                });
            }
        }
        // ...and the down projection widens to i24 when any row of any
        // EXPERT's down of any block structurally requires it (uniform device
        // buffer format). A dense block contributes exactly one up/down pair.
        let down_needs_i24 = ck
            .blocks()
            .iter()
            .flat_map(BlockFfn::ffn_pairs)
            .any(|(_, down)| layer_needs_i24(down));
        let down_width = if down_needs_i24 {
            AccWidth::I24
        } else {
            AccWidth::I16
        };
        let mut down_acc_structural_bound: i64 = 0;
        let mut down_delta_structural_bound: u64 = 0;
        for (block, b) in ck.blocks().iter().enumerate() {
            for (up, down) in b.ffn_pairs() {
                // Up projections have fan-in d_model <= 255: always i16 (the
                // structural bound 128 * 255 = 32640 fits).
                debug_assert!(!layer_needs_i24(up), "up fan-in <= 255 always fits i16");
                for row in 0..down.rows() {
                    let (lo, hi) = row_acc_bounds(down.row(row));
                    let bound = lo.abs().max(hi);
                    down_acc_structural_bound = down_acc_structural_bound.max(bound);
                    let scale = i64::from(down.scale_raw(row));
                    let delta_bound = ((2 * scale * bound + 127) / 254).unsigned_abs();
                    down_delta_structural_bound = down_delta_structural_bound.max(delta_bound);
                    if down_width == AccWidth::I24 {
                        if 2 * scale * bound + 127 >= 1 << 32 {
                            return Err(StateModelError::DownEpilogueOverflow {
                                block,
                                row,
                                scale_raw: down.scale_raw(row),
                                acc_bound: bound,
                            });
                        }
                        // v2 wide path: the delta is carried exactly in a signed
                        // i24 with no clamp, so the structural bound must fit.
                        if delta_bound > DOWN_DELTA_WIDE_BOUND {
                            return Err(StateModelError::DownDeltaEscapesI24 {
                                block,
                                row,
                                scale_raw: down.scale_raw(row),
                                acc_bound: bound,
                                delta_bound,
                            });
                        }
                    }
                }
            }
        }

        // Lower each block. `blocks` keeps the dense ROM-facing view (dense
        // pair, or expert 0 for a MoE block); `block_ffns` carries the full
        // dispatch the host forward routes over.
        let mut blocks = Vec::with_capacity(t.n_blocks);
        let mut block_ffns = Vec::with_capacity(t.n_blocks);
        for block in ck.blocks() {
            match block {
                BlockFfn::Dense { up, down } => {
                    let up = LoweredLayer::new(up);
                    let down = LoweredLayer::new(down);
                    blocks.push((up.clone(), down.clone()));
                    block_ffns.push(LoweredBlockFfn::Dense { up, down });
                }
                BlockFfn::Moe { router, experts } => {
                    let lowered_experts: Vec<(LoweredLayer, LoweredLayer)> = experts
                        .iter()
                        .map(|(up, down)| (LoweredLayer::new(up), LoweredLayer::new(down)))
                        .collect();
                    // Build the deployed purely-integer router (router-fx.v1)
                    // once at lowering time; the structural width proofs fail
                    // loud here rather than clamp in the forward.
                    let fixed_router = FixedRouter::lower(router, block_ffns.len())?;
                    // Dense ROM-facing placeholder: expert 0 (the MoE ROM
                    // builder uses `block_ffns`, deploy step 4).
                    blocks.push(lowered_experts[0].clone());
                    block_ffns.push(LoweredBlockFfn::Moe {
                        router: router.clone(),
                        fixed_router,
                        experts: lowered_experts,
                    });
                }
            }
        }

        Ok(Self {
            topology: t,
            emb_resid,
            head_i8,
            head_step,
            gelu_lut: crate::model_ref::build_gelu_lut(),
            y_resid_lut,
            state_in: LoweredLayer::new(&ck.state_in),
            state_out: ck.state_out.clone(),
            decay_u8,
            blocks,
            block_ffns,
            down_width,
            down_acc_structural_bound,
            down_delta_structural_bound,
        })
    }

    #[must_use]
    pub fn emb_resid_row(&self, id: u8) -> &[i32] {
        self.emb_resid_row_at(usize::from(id))
    }

    /// Embedding-residual row for a full `usize` token id (V=1024 subword
    /// students exceed the u8 charset id space).
    #[must_use]
    pub fn emb_resid_row_at(&self, id: usize) -> &[i32] {
        let start = id * self.topology.d_model;
        &self.emb_resid[start..start + self.topology.d_model]
    }

    #[must_use]
    pub fn head_i8_row(&self, id: u8) -> &[i8] {
        self.head_i8_row_at(usize::from(id))
    }

    /// Tied-head row for a full `usize` token id.
    #[must_use]
    pub fn head_i8_row_at(&self, id: usize) -> &[i8] {
        let start = id * self.topology.d_model;
        &self.head_i8[start..start + self.topology.d_model]
    }

    /// Real value represented by one integer logit unit.
    #[must_use]
    pub fn logit_dequant_step(&self) -> f64 {
        f64::from(ACT_RANGE) / f64::from(QMAX) * f64::from(self.head_step)
    }

    /// Fresh zero state sized for this model.
    #[must_use]
    pub fn zero_state(&self) -> Vec<i32> {
        vec![0i32; self.topology.state_slots]
    }

    /// The canonical integer forward pass for one token. `state` is the
    /// persistent recurrence vector (`state_slots` long, zeroed at stream
    /// start), updated in place exactly as the ROM updates its WRAM copy.
    #[must_use]
    pub fn forward(&self, prev: u8, state: &mut [i32]) -> IntStateForwardTrace {
        self.forward_probed(prev, state, None)
    }

    /// [`Self::forward`] for a full `usize` token id (V > 256 subword MoE
    /// students). For `prev < 256` this is byte-identical to `forward`.
    #[must_use]
    pub fn forward_at(&self, prev: usize, state: &mut [i32]) -> IntStateForwardTrace {
        self.forward_at_probed(prev, state, None)
    }

    /// [`Self::forward`] with an optional [`DownDeltaProbe`] observing the
    /// unclamped down-projection delta magnitudes. The returned trace and the
    /// state update are byte-identical to `forward`'s regardless of the
    /// probe: probes only record, never steer.
    #[must_use]
    pub fn forward_probed(
        &self,
        prev: u8,
        state: &mut [i32],
        probe: Option<&mut DownDeltaProbe>,
    ) -> IntStateForwardTrace {
        self.forward_at_probed(usize::from(prev), state, probe)
    }

    /// Core forward over a full `usize` token id. See [`Self::forward_probed`].
    #[must_use]
    pub fn forward_at_probed(
        &self,
        prev: usize,
        state: &mut [i32],
        probe: Option<&mut DownDeltaProbe>,
    ) -> IntStateForwardTrace {
        self.forward_at_core(prev, state, probe, None)
    }

    /// [`Self::forward_at`] that additionally records, for each MoE block, the
    /// `(block_idx, pre-block raw i24 Q19.5 residual)` the router routes on.
    /// The trace and state update are byte-identical to `forward_at`; the audit
    /// only observes. Used by the fixed-point router parity gate to compare the
    /// deployed `FixedRouter` against the f32 `LowRankRouter` reference on the
    /// exact residual the forward routes on, at every block, every position.
    #[must_use]
    pub fn forward_at_route_audit(
        &self,
        prev: usize,
        state: &mut [i32],
        route_audit: &mut Vec<(usize, Vec<i32>)>,
    ) -> IntStateForwardTrace {
        self.forward_at_core(prev, state, None, Some(route_audit))
    }

    #[allow(clippy::type_complexity)]
    fn forward_at_core(
        &self,
        prev: usize,
        state: &mut [i32],
        mut probe: Option<&mut DownDeltaProbe>,
        mut route_audit: Option<&mut Vec<(usize, Vec<i32>)>>,
    ) -> IntStateForwardTrace {
        let t = self.topology;
        assert_eq!(state.len(), t.state_slots, "state width");
        let mut stats = StateForwardStats::new();

        let mut x = self.emb_resid_row_at(prev).to_vec();

        // --- state block ---
        let q = int_norm_quant24(&x, &mut stats.ffn);
        let mut state_norm_act = vec![0u8; t.d_model];
        for (a, qv) in state_norm_act.iter_mut().zip(q.iter()) {
            *a = (qv + 128) as u8;
        }
        let mut in_acc = vec![0i16; t.state_slots];
        int_matvec(
            &self.state_in.layer,
            &self.state_in.biases,
            &state_norm_act,
            &mut in_acc,
            &mut stats.ffn,
        );
        for &acc in &in_acc {
            stats.max_abs_in_acc = stats.max_abs_in_acc.max(i32::from(acc).unsigned_abs());
        }

        // decayed state update with exact integer delta
        for (slot, h) in state.iter_mut().enumerate() {
            let decayed_abs =
                (u64::from(h.unsigned_abs()) * u64::from(self.decay_u8[slot]) + 128) >> 8;
            let decayed = if *h < 0 {
                -(decayed_abs as i64)
            } else {
                decayed_abs as i64
            };
            let m = i64::from(self.state_in.layer.scale_raw(slot)) * i64::from(in_acc[slot]);
            stats.max_abs_state_delta = stats.max_abs_state_delta.max(m.unsigned_abs());
            let mut next = decayed + m;
            if next.unsigned_abs() > STATE_CLAMP as u64 {
                stats.state_clamp_events += 1;
                next = next.signum() * i64::from(STATE_CLAMP);
            }
            *h = next as i32;
            stats.max_abs_state = stats.max_abs_state.max(h.unsigned_abs());
        }
        let state_after = state.to_vec();

        // out projection over the i24 state, y quantized to the act grid
        let mut out_acc = vec![0i32; t.d_model];
        let mut y_act = vec![0u8; t.d_model];
        for row in 0..t.d_model {
            let mut acc: i64 = 0;
            for (w, h) in self.state_out.row(row).iter().zip(state.iter()) {
                acc += i64::from(*w) * i64::from(*h);
            }
            debug_assert!(
                acc.unsigned_abs() < 1 << 31,
                "out-projection accumulator fits i32 (structural bound slots * 2^23)"
            );
            stats.max_abs_out_acc = stats.max_abs_out_acc.max(acc.unsigned_abs());
            out_acc[row] = acc as i32;
            let m = i64::from(self.state_out.scale_raw(row)) * acc;
            stats.max_abs_out_scale_product = stats.max_abs_out_scale_product.max(m.unsigned_abs());
            let p_abs = ((m.unsigned_abs() + 32768) >> 16).min(i64::from(QMAX) as u64) as i32;
            if p_abs == i32::from(QMAX as u8) && (m.unsigned_abs() + 32768) >> 16 > QMAX as u64 {
                stats.y_saturation_events += 1;
            }
            let p = if m < 0 { -p_abs } else { p_abs };
            y_act[row] = (p + 128) as u8;
            let delta = i32::from(self.y_resid_lut[(p + i32::from(QMAX as u8)) as usize]);
            let wide = x[row] + delta;
            let wrapped = wrap_i24(wide);
            if wrapped != wide {
                stats.residual_i24_wrap_events += 1;
            }
            x[row] = wrapped;
            stats.ffn.max_abs_residual = stats.ffn.max_abs_residual.max(x[row].unsigned_abs());
        }

        // --- FFN blocks (dense conventions on the widened residual) ---
        let mut block_residuals: Vec<Vec<i32>> = Vec::with_capacity(t.n_blocks);
        let mut selected_experts: Vec<usize> = Vec::new();
        let mut block0_norm_act = vec![0u8; t.d_model];
        let mut block0_up_acc = vec![0i16; t.d_ff];
        let mut block0_gelu_act = vec![0u8; t.d_ff];
        let mut block0_down_acc = vec![0i32; t.d_model];
        let mut act = vec![0u8; t.d_ff];
        let mut up_acc = vec![0i16; t.d_ff];
        let mut down_acc16 = vec![0i16; t.d_model];
        let mut down_acc24 = vec![0i32; t.d_model];
        for (block_idx, block) in self.block_ffns.iter().enumerate() {
            // Select the block's up/down pair. Dense blocks use their single
            // pair (byte-identical to the pre-MoE path). MoE blocks route on the
            // RAW pre-norm residual: the router picks the expert INDEX ONLY and
            // never re-enters the integer stream, so the FFN math below is
            // unchanged.
            let (up, down) = match block {
                LoweredBlockFfn::Dense { up, down } => (up, down),
                LoweredBlockFfn::Moe {
                    fixed_router,
                    experts,
                    ..
                } => {
                    // PURELY INTEGER routing (router-fx.v1): the fixed-point
                    // router picks the expert index directly from the raw i24
                    // Q19.5 residual — no f32 enters the deployed forward. The
                    // f32 `router` field stays only as the parity reference (the
                    // gate compares the two). Record the pre-block residual for
                    // the optional routing audit.
                    if let Some(sink) = route_audit.as_deref_mut() {
                        sink.push((block_idx, x.clone()));
                    }
                    let e = fixed_router.route(&x);
                    selected_experts.push(e);
                    let (u, d) = &experts[e];
                    (u, d)
                }
            };
            let q = int_norm_quant24(&x, &mut stats.ffn);
            for (a, qv) in act.iter_mut().zip(q.iter()) {
                *a = (qv + 128) as u8;
            }
            if block_idx == 0 {
                block0_norm_act.copy_from_slice(&act[..t.d_model]);
            }

            int_matvec(
                &up.layer,
                &up.biases,
                &act[..t.d_model],
                &mut up_acc[..t.d_ff],
                &mut stats.ffn,
            );
            if block_idx == 0 {
                block0_up_acc.copy_from_slice(&up_acc[..t.d_ff]);
            }

            for row in 0..t.d_ff {
                let p = int_scale_to_grid(up_acc[row], up.layer.scale_raw(row), &mut stats.ffn);
                act[row] = self.gelu_lut[(p + QMAX) as usize];
            }
            if block_idx == 0 {
                block0_gelu_act.copy_from_slice(&act[..t.d_ff]);
            }

            match self.down_width {
                AccWidth::I16 => {
                    int_matvec(
                        &down.layer,
                        &down.biases,
                        &act[..t.d_ff],
                        &mut down_acc16[..t.d_model],
                        &mut stats.ffn,
                    );
                    for (wide, narrow) in down_acc24.iter_mut().zip(down_acc16.iter()) {
                        *wide = i32::from(*narrow);
                    }
                }
                AccWidth::I24 => {
                    int_matvec_i24(&down.layer, &act[..t.d_ff], &mut down_acc24, &mut stats);
                }
            }
            if block_idx == 0 {
                block0_down_acc.copy_from_slice(&down_acc24);
            }

            for row in 0..t.d_model {
                let d_raw = match self.down_width {
                    AccWidth::I16 => {
                        int_down_delta(down_acc16[row], down.layer.scale_raw(row), &mut stats.ffn)
                    }
                    AccWidth::I24 => int_down_delta_i24(
                        down_acc24[row],
                        down.layer.scale_raw(row),
                        &mut stats.ffn,
                        probe.as_deref_mut(),
                    ),
                };
                let wide = x[row] + d_raw;
                let wrapped = wrap_i24(wide);
                if wrapped != wide {
                    stats.residual_i24_wrap_events += 1;
                }
                x[row] = wrapped;
                stats.ffn.max_abs_residual = stats.ffn.max_abs_residual.max(x[row].unsigned_abs());
            }
            block_residuals.push(x.clone());
        }

        // --- final norm + paged head ---
        // Stream the tied head in pages of <= LOGIT_PAGE_IDS ids. Each page
        // accumulates its i32 dots into one reused page buffer, then folds into
        // (a) a running top-1 argmax (strict `>`, lowest id wins ties ACROSS
        // pages, ascending id order) and (b) a RunningTopK heap whose total
        // order matches decode::sample_topk selection exactly. Under
        // SinglePage there is exactly one page and the full logits vector is
        // produced (n_pages == 1, gate-compat). Under Paged the full 3*vocab
        // vector is never materialized.
        let final_q = int_norm_quant24(&x, &mut stats.ffn);
        let n_pages = t.vocab.div_ceil(LOGIT_PAGE_IDS).max(1);
        let mut logit_pages: Vec<Vec<i32>> = Vec::with_capacity(n_pages);
        let mut last_page: Vec<i32> = Vec::new();
        let mut argmax_full = 0usize;
        let mut best: i32 = i32::MIN;
        let mut seen_any = false;
        let mut heap = RunningTopK::new(HEAP_K_MAX.min(t.vocab.max(1)));
        let mut page_buf: Vec<i32> = Vec::with_capacity(LOGIT_PAGE_IDS);
        for page in 0..n_pages {
            let lo = page * LOGIT_PAGE_IDS;
            let hi = ((page + 1) * LOGIT_PAGE_IDS).min(t.vocab);
            page_buf.clear();
            for id in lo..hi {
                let row = self.head_i8_row_at(id);
                let mut acc32: i32 = 0;
                for (qv, ev) in final_q.iter().zip(row.iter()) {
                    acc32 += i32::from(*qv) * i32::from(*ev);
                }
                stats.ffn.max_abs_logit = stats.ffn.max_abs_logit.max(acc32.unsigned_abs());
                page_buf.push(acc32);
                // running top-1 (strict `>`; ascending id order keeps the
                // lowest index on ties, across pages)
                if !seen_any || acc32 > best {
                    best = acc32;
                    argmax_full = id;
                    seen_any = true;
                }
                heap.offer(acc32, id);
            }
            last_page = page_buf.clone();
            logit_pages.push(page_buf.clone());
        }
        let topk_heap = heap.finalize();
        // `logits` stays the full vector under SinglePage (byte-compat with
        // every existing gate); under Paged it is only the last resident page.
        let logits = match t.logit_paging {
            LogitPaging::SinglePage => logit_pages.iter().flatten().copied().collect(),
            LogitPaging::Paged => last_page,
        };
        // `argmax` keeps the u8 charset id for the dense/charset pipeline;
        // `argmax_full` carries the true id for wide-vocab (subword) models.
        let argmax = argmax_full as u8;

        IntStateForwardTrace {
            state_norm_act,
            state_in_acc: in_acc,
            state_after,
            state_out_acc: out_acc,
            y_act,
            block_residuals,
            selected_experts,
            block0_norm_act,
            block0_up_acc,
            block0_gelu_act,
            block0_down_acc,
            final_q,
            logits,
            logit_pages,
            argmax,
            argmax_full,
            topk_heap,
            stats,
        }
    }
}

// ---------------------------------------------------------------------------
// deterministic synthetic checkpoint (tests / smoke)
// ---------------------------------------------------------------------------

/// Deterministic synthetic stateful checkpoint at an arbitrary topology.
/// Mirrors the real export's weight statistics (ternary with ~30% zeros,
/// scale raws 4..84, MT4 decay bands) so device range behavior is
/// representative; used by the d192 readiness gates.
#[must_use]
pub fn synthetic_state_checkpoint_with(topology: StateTopology, seed: u64) -> StateCheckpoint {
    let mut state = seed ^ 0x5bd1_e995_9e37_79b9;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };

    let embedding: Vec<f32> = (0..topology.vocab * topology.d_model)
        .map(|_| {
            let unit = (next() >> 40) as f32 / (1u64 << 24) as f32;
            (unit * 2.0 - 1.0) * 3.0
        })
        .collect();

    let mut layer = |rows: usize, cols: usize| {
        let weights: Vec<i8> = (0..rows * cols)
            .map(|_| match next() % 10 {
                0..=2 => 0i8,
                n if n < 7 => 1,
                _ => -1,
            })
            .collect();
        let scales: Vec<u16> = (0..rows).map(|_| (next() % 80 + 4) as u16).collect();
        TernaryLayer::new(rows, cols, weights, scales).expect("synthetic layer is valid")
    };

    let state_in = layer(topology.state_slots, topology.d_model);
    let state_out = layer(topology.d_model, topology.state_slots);
    // MT4 band layout: 4 equal contiguous bands.
    let band = (topology.state_slots / 4).max(1);
    let decay_raw: Vec<u16> = (0..topology.state_slots)
        .map(|slot| [128u16, 192, 224, 240][(slot / band).min(3)])
        .collect();
    let blocks = (0..topology.n_blocks)
        .map(|_| BlockWeights {
            up: layer(topology.d_ff, topology.d_model),
            down: layer(topology.d_model, topology.d_ff),
        })
        .collect();
    StateCheckpoint::new(topology, embedding, state_in, state_out, decay_raw, blocks)
        .expect("synthetic state checkpoint is valid")
}

/// Deterministic synthetic arm-B-topology checkpoint (legacy tests).
#[must_use]
pub fn synthetic_state_checkpoint(seed: u64) -> StateCheckpoint {
    synthetic_state_checkpoint_with(StateTopology::ARM_B, seed)
}

/// Deterministic synthetic **MoE** checkpoint at a MoE topology
/// (`n_experts >= 1`). Reuses the dense synthetic for the shared non-FFN
/// tensors (embedding/state/decay) so the non-FFN path matches the dense
/// checkpoint exactly, then draws a low-rank router and `n_experts` distinct
/// per-(block, expert) up/down pairs. Used by the deploy-step-4 MoE ROM gate.
///
/// # Panics
/// Panics if the constructed checkpoint fails validation (never for a valid
/// MoE topology).
#[must_use]
pub fn synthetic_moe_state_checkpoint(topology: StateTopology, seed: u64) -> StateCheckpoint {
    assert!(topology.n_experts >= 1, "MoE topology needs >= 1 expert");
    let dense_topo = StateTopology {
        n_experts: 1,
        ..topology
    };
    let dense = synthetic_state_checkpoint_with(dense_topo, seed);
    let embedding: Vec<f32> = (0..topology.vocab)
        .flat_map(|id| dense.embedding_row(id as u8).to_vec())
        .collect();

    let mut rng = seed ^ 0xa5a5_5a5a_1234_9876;
    let mut next = move || {
        rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut unit = move || (next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;

    let rank = 2usize;
    let mut blocks = Vec::with_capacity(topology.n_blocks);
    for bi in 0..topology.n_blocks {
        let router = LowRankRouter::new(
            rank,
            topology.d_model,
            topology.n_experts,
            (0..rank * topology.d_model).map(|_| unit()).collect(),
            (0..rank).map(|_| unit()).collect(),
            (0..topology.n_experts * rank).map(|_| unit()).collect(),
            (0..topology.n_experts).map(|_| unit()).collect(),
        )
        .expect("router valid");
        let experts: Vec<(TernaryLayer, TernaryLayer)> = (0..topology.n_experts)
            .map(|ei| {
                let ck = synthetic_state_checkpoint_with(
                    dense_topo,
                    seed ^ ((bi as u64) << 32) ^ ((ei as u64 + 1) << 8),
                );
                let (u, d) = ck.blocks()[0].as_dense().expect("synthetic block is dense");
                (u.clone(), d.clone())
            })
            .collect();
        blocks.push(BlockFfn::Moe { router, experts });
    }
    StateCheckpoint::new_moe(
        topology,
        embedding,
        dense.state_in.clone(),
        dense.state_out.clone(),
        dense.decay_raw().to_vec(),
        blocks,
    )
    .expect("synthetic MoE checkpoint is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_ref::D_MODEL;

    #[test]
    fn isqrt48_matches_f64_sqrt_floor_over_samples() {
        let samples: [u64; 12] = [
            0,
            1,
            2,
            3,
            15,
            16,
            65535,
            65536,
            (1 << 32) - 1,
            1 << 40,
            (1 << 46) + 12345,
            (1 << 48) - 1,
        ];
        for &n in &samples {
            let expected = (n as f64).sqrt().floor() as u64;
            // f64 sqrt is exact for these magnitudes (< 2^48 << 2^52).
            assert_eq!(u64::from(isqrt_u48(n)), expected, "isqrt48({n})");
        }
        for k in (0u64..1 << 24).step_by(65537) {
            let sq = k * k;
            assert_eq!(u64::from(isqrt_u48(sq)), k);
            if sq > 0 {
                assert_eq!(u64::from(isqrt_u48(sq - 1)), k - 1);
            }
        }
    }

    #[test]
    fn int_norm_quant24_agrees_with_dense_norm_on_i16_range_inputs() {
        // On inputs that fit the dense i16 residual, the widened norm must
        // reduce to the dense canonical norm (same formula, wider registers).
        let ck = crate::model_ref::synthetic_checkpoint(9);
        let lowered = crate::model_ref::IntLoweredModel::lower(&ck).expect("lowers");
        let x16: Vec<i16> = lowered.emb_resid_row(0x41).to_vec();
        let mut x16a = [0i16; D_MODEL];
        x16a.copy_from_slice(&x16);
        let mut x24 = [0i32; D_MODEL];
        for (o, v) in x24.iter_mut().zip(x16.iter()) {
            *o = i32::from(*v);
        }
        let mut s16 = IntForwardStats::new();
        let mut s24 = IntForwardStats::new();
        let q16 = crate::model_ref::int_norm_quant(&x16a, &mut s16);
        let q24 = int_norm_quant24(&x24, &mut s24);
        assert_eq!(q16.to_vec(), q24);
    }

    #[test]
    fn norm24_mean_uses_floor_division_for_non_pow2_lanes() {
        // 192 lanes: mean = floor(ss / 192) must equal the shift-then-odd
        // factorization the device uses (ss >> 6, then / 3).
        let x: Vec<i32> = (0..192).map(|i| (i as i32 - 96) * 1234).collect();
        let ss: u64 = x.iter().map(|&v| u64::from(v.unsigned_abs()).pow(2)).sum();
        assert_eq!(ss / 192, (ss >> 6) / 3);
        let mut stats = IntForwardStats::new();
        let q = int_norm_quant24(&x, &mut stats);
        assert_eq!(q.len(), 192);
        assert!(q.iter().all(|&v| (-127..=127).contains(&v)));
    }

    #[test]
    fn state_delta_accumulates_exactly_and_decays_with_rounding() {
        let ck = synthetic_state_checkpoint(3);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut state = lowered.zero_state();
        let t = lowered.forward(5, &mut state);
        // From zero state, h = m exactly (decay of zero is zero).
        for slot in 0..lowered.topology.state_slots {
            let m =
                i32::from(lowered.state_in.layer.scale_raw(slot)) * i32::from(t.state_in_acc[slot]);
            assert_eq!(t.state_after[slot], m, "slot {slot}");
        }
        // A second token decays the carried state with round-half-away.
        let carried = state.clone();
        let t2 = lowered.forward(6, &mut state);
        for slot in 0..lowered.topology.state_slots {
            let decayed_abs =
                (u64::from(carried[slot].unsigned_abs()) * u64::from(lowered.decay_u8[slot]) + 128)
                    >> 8;
            let decayed = if carried[slot] < 0 {
                -(decayed_abs as i64)
            } else {
                decayed_abs as i64
            };
            let m = i64::from(lowered.state_in.layer.scale_raw(slot))
                * i64::from(t2.state_in_acc[slot]);
            let expected = (decayed + m).clamp(-i64::from(STATE_CLAMP), i64::from(STATE_CLAMP));
            assert_eq!(i64::from(t2.state_after[slot]), expected, "slot {slot}");
        }
    }

    #[test]
    fn int_state_forward_is_deterministic_and_state_dependent() {
        let ck = synthetic_state_checkpoint(7);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut s1 = lowered.zero_state();
        let mut s2 = lowered.zero_state();
        let a = lowered.forward(10, &mut s1);
        let b = lowered.forward(10, &mut s2);
        assert_eq!(a.logits, b.logits);
        assert_eq!(s1, s2);
        // Same input with different carried state must (generically) differ.
        let c = lowered.forward(10, &mut s1);
        assert_ne!(
            a.state_after, c.state_after,
            "carried state must influence the update"
        );
        for &l in &a.logits {
            assert!(
                (-(1 << 23)..(1 << 23)).contains(&l),
                "logit {l} escapes i24"
            );
        }
    }

    #[test]
    fn d192_topology_lowers_with_wide_down_accumulators() {
        let ck = synthetic_state_checkpoint_with(StateTopology::D192, 5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        // ~70% nonzero synthetic rows at fan-in 384 structurally exceed i16.
        assert_eq!(lowered.down_width, AccWidth::I24);
        assert!(lowered.down_acc_structural_bound > 32767);
        let mut state = lowered.zero_state();
        let a = lowered.forward(19, &mut state);
        let b_state = state.clone();
        let b = lowered.forward(a.argmax, &mut state);
        assert_eq!(a.logits.len(), 80);
        assert_eq!(a.state_after.len(), 192);
        assert_eq!(b.state_after.len(), 192);
        assert_ne!(b_state, state, "state evolves");
        // Deterministic replay.
        let mut s2 = lowered.zero_state();
        let a2 = lowered.forward(19, &mut s2);
        assert_eq!(a.logits, a2.logits);
    }

    #[test]
    fn arm_b_topology_keeps_i16_down_accumulators() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        assert_eq!(lowered.down_width, AccWidth::I16);
    }

    #[test]
    fn int_and_f32_state_forward_mostly_agree_on_synthetic_model() {
        // Not a strict gate (fidelity is measured on the real checkpoint);
        // this catches gross semantic porting errors.
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut fs = vec![0.0f32; STATE_SLOTS];
        let mut is = lowered.zero_state();
        let mut agree = 0usize;
        let mut input = 1u8;
        let n = 64usize;
        for _ in 0..n {
            let fl = f32_state_forward(&ck, input, &mut fs);
            let it = lowered.forward(input, &mut is);
            let f_arg = fl
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).expect("finite"))
                .map(|(i, _)| i as u8)
                .expect("nonempty");
            if f_arg == it.argmax {
                agree += 1;
            }
            input = it.argmax;
        }
        assert!(
            agree * 2 >= n,
            "int/f32 argmax agreement suspiciously low on synthetic stateful model: {agree}/{n}"
        );
    }

    #[test]
    fn y_resid_lut_endpoints() {
        let ck = synthetic_state_checkpoint(1);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        assert_eq!(lowered.y_resid_lut[127], 0); // p = 0
        assert_eq!(lowered.y_resid_lut[254], 256); // p = 127 -> exactly 8.0 * 32
        assert_eq!(lowered.y_resid_lut[0], -256);
    }

    #[test]
    fn wide_down_delta_is_exact_above_the_old_u16_cap() {
        // v1 clamped here (bd-2vkqt); v2 must carry the exact Q19.5 value.
        let mut stats = IntForwardStats::new();
        // m = 500 * 40000 = 20,000,000; delta = (2m + 127) / 254 = 157,480
        // (round_half_away(m / 127)) — 2.4x the old 65,535 cap.
        let d = int_down_delta_i24(40000, 500, &mut stats, None);
        assert_eq!(d, 157_480);
        assert_eq!(
            stats.down_delta_clamp_events, 0,
            "v2 wide path has no clamp"
        );
        assert_eq!(stats.max_abs_down_delta, 157_480);
        let dn = int_down_delta_i24(-40000, 500, &mut stats, None);
        assert_eq!(dn, -157_480);
    }

    #[test]
    fn down_delta_probe_records_unclamped_magnitudes() {
        let mut stats = IntForwardStats::new();
        let mut probe = DownDeltaProbe::new();
        let _ = int_down_delta_i24(40000, 500, &mut stats, Some(&mut probe));
        let _ = int_down_delta_i24(1, 1, &mut stats, Some(&mut probe));
        assert_eq!(probe.total(), 2);
        assert_eq!(probe.max_abs(), 157_480);
        assert_eq!(probe.count_at_or_above(65536), 1);
        assert_eq!(probe.count_at_or_above(1 << 23), 0);
        // p50 falls in the first bucket; the max quantile is exact.
        assert!(probe.quantile_upper_bound(0.5) < DownDeltaProbe::BUCKET_WIDTH);
        assert_eq!(probe.quantile_upper_bound(1.0), 157_480);
        let mut merged = DownDeltaProbe::new();
        merged.merge(&probe);
        assert_eq!(merged.total(), 2);
        assert_eq!(merged.max_abs(), 157_480);
    }

    #[test]
    fn lowering_rejects_structural_delta_bounds_past_the_i24_carrier() {
        // A dense +/-1 fan-in-384 down row at scale 30000: acc bound
        // 128 * 384 = 49,152; delta bound = (2*30000*49152 + 127)/254 =
        // 11,611,654 > 2^23 - 1, while the u32 numerator check still passes
        // (2.95e9 < 2^32). v2 must refuse to lower rather than clamp.
        let t = StateTopology::D192;
        let ck = synthetic_state_checkpoint_with(t, 5);
        let hostile_down = TernaryLayer::new(
            t.d_model,
            t.d_ff,
            vec![1i8; t.d_model * t.d_ff],
            vec![30000u16; t.d_model],
        )
        .expect("valid layer");
        let blocks: Vec<BlockWeights> = ck
            .blocks()
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (up, down) = b.as_dense().expect("synthetic checkpoint is dense");
                BlockWeights {
                    up: up.clone(),
                    down: if i == 0 {
                        hostile_down.clone()
                    } else {
                        down.clone()
                    },
                }
            })
            .collect();
        let bad = StateCheckpoint::new(
            t,
            ck.embedding.clone(),
            ck.state_in.clone(),
            ck.state_out.clone(),
            ck.decay_raw.clone(),
            blocks,
        )
        .expect("shapes are valid");
        match IntStateLoweredModel::lower(&bad) {
            Err(StateModelError::DownDeltaEscapesI24 {
                block: 0,
                delta_bound,
                ..
            }) => {
                assert!(delta_bound > DOWN_DELTA_WIDE_BOUND);
            }
            other => panic!("expected DownDeltaEscapesI24, got {other:?}"),
        }
    }

    #[test]
    fn lowering_reports_the_structural_delta_bound() {
        let ck = synthetic_state_checkpoint_with(StateTopology::D192, 5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        // Recompute independently from the raw weights.
        let mut expected = 0u64;
        for b in ck.blocks() {
            let (_, down) = b.as_dense().expect("synthetic checkpoint is dense");
            for row in 0..down.rows() {
                let (lo, hi) = row_acc_bounds(down.row(row));
                let bound = lo.unsigned_abs().max(hi.unsigned_abs());
                expected = expected.max((2 * u64::from(down.scale_raw(row)) * bound + 127) / 254);
            }
        }
        assert_eq!(lowered.down_delta_structural_bound, expected);
        assert!(lowered.down_delta_structural_bound <= DOWN_DELTA_WIDE_BOUND);
    }

    #[test]
    fn running_topk_matches_k_pass_selection_over_random_logits() {
        use crate::decode::{SamplerConfig, XorShift16, sample_topk_trace};
        // The RunningTopK finalize() order must equal decode::sample_topk's
        // pass-k candidate selection over the full vocab-1024 logit vector, for
        // k in {1, 4, 8, 40}, on 200 random vectors.
        let mut lcg: u64 = 0xDEAD_BEEF_CAFE_1234;
        let mut rand = move || {
            lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1);
            (lcg >> 33) as i32
        };
        for _ in 0..200 {
            let vocab = 1024usize;
            let logits: Vec<i32> = (0..vocab).map(|_| (rand() % 400_000) - 200_000).collect();
            for k in [1usize, 4, 8, 40] {
                let mut heap = RunningTopK::new(k);
                for (id, &l) in logits.iter().enumerate() {
                    heap.offer(l, id);
                }
                let got: Vec<(i32, usize)> =
                    heap.finalize().iter().map(|e| (e.logit, e.id)).collect();
                // Reference: decode's pass-k selection (k <= MAX_TOP_K only for
                // the config; use a large scale so weights don't matter — we
                // compare the candidate ORDER, not the draw). For k > 8 we build
                // the reference selection directly with the same rule.
                let cfg = SamplerConfig::new(k.min(8) as u8, 1000).expect("cfg");
                let mut rng = XorShift16::new(1);
                let full = sample_topk_trace(&logits, &cfg, &mut rng);
                let want_prefix: Vec<(i32, usize)> =
                    full.candidates.iter().map(|c| (c.logit, c.id)).collect();
                assert_eq!(
                    &got[..k.min(8)],
                    &want_prefix[..],
                    "top-{} prefix mismatch (k={k})",
                    k.min(8)
                );
                // Full-order cross-check via an independent sort with the exact
                // sampler total order (logit desc, id asc).
                let mut sorted: Vec<(i32, usize)> =
                    logits.iter().enumerate().map(|(i, &l)| (l, i)).collect();
                sorted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                assert_eq!(got, sorted[..k].to_vec(), "full top-{k} order");
            }
        }
    }

    #[test]
    fn paged_head_equals_single_page_on_small_vocab() {
        // On a <= 85-vocab model the Paged epilogue (one page, n_pages == 1)
        // must produce the identical logits/argmax/heap as SinglePage.
        let single = StateTopology {
            logit_paging: LogitPaging::SinglePage,
            ..StateTopology::ARM_B
        };
        let paged = StateTopology {
            logit_paging: LogitPaging::Paged,
            ..StateTopology::ARM_B
        };
        let ck_s = synthetic_state_checkpoint_with(single, 77);
        let ck_p = synthetic_state_checkpoint_with(paged, 77);
        let ls = IntStateLoweredModel::lower(&ck_s).expect("lowers");
        let lp = IntStateLoweredModel::lower(&ck_p).expect("lowers");
        let mut ss = ls.zero_state();
        let mut sp = lp.zero_state();
        let mut input = 3u8;
        for _ in 0..24 {
            let ts = ls.forward(input, &mut ss);
            let tp = lp.forward(input, &mut sp);
            assert_eq!(ts.logits, tp.logits, "logits identical");
            assert_eq!(ts.logit_pages, tp.logit_pages, "one page each");
            assert_eq!(tp.logit_pages.len(), 1, "small vocab is one page");
            assert_eq!(ts.argmax_full, tp.argmax_full);
            assert_eq!(ts.topk_heap, tp.topk_heap, "heap identical");
            input = tp.argmax;
        }
    }

    #[test]
    fn vocab_1024_paged_topology_validates() {
        // D192_MOE (Paged, vocab 1024) must validate; a SinglePage vocab=86
        // must be rejected (single page holds only 85 ids).
        StateTopology::D192_MOE
            .validate()
            .expect("Paged vocab 1024 validates");
        StateTopology::D1024_DENSE
            .validate()
            .expect("Paged dense vocab 1024 validates");
        let over_single = StateTopology {
            vocab: LOGIT_PAGE_IDS + 1,
            logit_paging: LogitPaging::SinglePage,
            ..StateTopology::ARM_B
        };
        assert!(
            matches!(
                over_single.validate(),
                Err(StateModelError::Topology { .. })
            ),
            "SinglePage vocab 86 must be rejected"
        );
        // Paged still enforces the paged ceiling.
        let over_paged = StateTopology {
            vocab: LOGIT_PAGED_VOCAB_MAX + 1,
            logit_paging: LogitPaging::Paged,
            ..StateTopology::ARM_B
        };
        assert!(matches!(
            over_paged.validate(),
            Err(StateModelError::Topology { .. })
        ));
    }

    #[test]
    fn state_saturates_at_the_i24_bound() {
        let ck = synthetic_state_checkpoint(2);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut state = vec![STATE_CLAMP; STATE_SLOTS];
        let t = lowered.forward(0, &mut state);
        for (slot, &h) in state.iter().enumerate() {
            assert!(
                h.unsigned_abs() <= STATE_CLAMP as u32,
                "slot {slot} escaped the saturation bound: {h}"
            );
        }
        assert!(t.stats.max_abs_state <= STATE_CLAMP as u32);
    }

    #[test]
    fn decay_wider_than_u8_is_rejected() {
        let ck = synthetic_state_checkpoint(4);
        let bad = StateCheckpoint::new_moe(
            StateTopology::ARM_B,
            ck.embedding.clone(),
            ck.state_in.clone(),
            ck.state_out.clone(),
            std::iter::once(256u16)
                .chain(ck.decay_raw.iter().copied().skip(1))
                .collect(),
            ck.blocks.clone(),
        );
        assert!(matches!(
            bad,
            Err(StateModelError::DecayRawTooWide { slot: 0, raw: 256 })
        ));
    }

    #[test]
    fn topology_out_of_device_range_is_rejected() {
        let too_wide = StateTopology {
            d_model: 256,
            ..StateTopology::ARM_B
        };
        assert!(matches!(
            too_wide.validate(),
            Err(StateModelError::Topology { .. })
        ));
        let too_many_ids = StateTopology {
            vocab: 86,
            ..StateTopology::ARM_B
        };
        assert!(too_many_ids.validate().is_err());
        assert!(StateTopology::D192.validate().is_ok());
    }

    #[test]
    fn row_acc_bounds_are_exact() {
        // pos=2 neg=1: hi = 127*2 + 128 = 382, lo = -(128*2 + 127) = -383.
        assert_eq!(row_acc_bounds(&[1, 1, -1, 0]), (-383, 382));
        assert_eq!(row_acc_bounds(&[0, 0]), (0, 0));
    }

    // -----------------------------------------------------------------------
    // MoE integer evaluator (deploy step 1)
    // -----------------------------------------------------------------------

    /// Build a deterministic MoE checkpoint at `topology` with `n_experts`
    /// experts per block. Experts are drawn from independent seeds so distinct
    /// experts hold distinct weights; the router tensors are seeded so the
    /// top-1 argmax exercises every expert across a token window.
    fn synthetic_moe_checkpoint(topology: StateTopology, seed: u64) -> StateCheckpoint {
        assert!(topology.n_experts >= 1);
        // Dense sibling topology (n_experts = 1) for the shared non-FFN tensors
        // and for drawing per-expert up/down weights.
        let dense_topo = StateTopology {
            n_experts: 1,
            ..topology
        };
        // Reuse the dense synthetic for the embedding + state + decay so the
        // non-FFN path matches the dense checkpoint exactly.
        let dense = synthetic_state_checkpoint_with(dense_topo, seed);
        let embedding: Vec<f32> = (0..topology.vocab)
            .flat_map(|id| dense.embedding_row(id as u8).to_vec())
            .collect();

        let mut rng = seed ^ 0xa5a5_5a5a_1234_9876;
        let mut next = move || {
            rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut unit = move || (next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;

        let rank = 2usize;
        let mut blocks = Vec::with_capacity(topology.n_blocks);
        for bi in 0..topology.n_blocks {
            let router = LowRankRouter::new(
                rank,
                topology.d_model,
                topology.n_experts,
                (0..rank * topology.d_model).map(|_| unit()).collect(),
                (0..rank).map(|_| unit()).collect(),
                (0..topology.n_experts * rank).map(|_| unit()).collect(),
                (0..topology.n_experts).map(|_| unit()).collect(),
            )
            .expect("router valid");
            let experts: Vec<(TernaryLayer, TernaryLayer)> = (0..topology.n_experts)
                .map(|ei| {
                    // Distinct per-(block,expert) seed => distinct experts.
                    let ck = synthetic_state_checkpoint_with(
                        dense_topo,
                        seed ^ ((bi as u64) << 32) ^ ((ei as u64 + 1) << 8),
                    );
                    let (u, d) = ck.blocks()[0].as_dense().expect("synthetic block is dense");
                    (u.clone(), d.clone())
                })
                .collect();
            blocks.push(BlockFfn::Moe { router, experts });
        }
        StateCheckpoint::new_moe(
            topology,
            embedding,
            dense.state_in.clone(),
            dense.state_out.clone(),
            dense.decay_raw().to_vec(),
            blocks,
        )
        .expect("synthetic MoE checkpoint is valid")
    }

    #[test]
    fn moe_n_experts_1_is_byte_identical_to_dense() {
        // A single-expert MoE checkpoint whose one expert IS the dense block
        // must lower and forward byte-for-byte identically to the dense
        // checkpoint: top-1 routing over one expert always picks expert 0 and
        // the integer FFN math is verbatim the dense kernel.
        let topo1 = StateTopology {
            n_experts: 1,
            ..StateTopology::D192
        };
        let dense = synthetic_state_checkpoint_with(topo1, 5);

        // Build the equivalent single-expert MoE: same up/down as the dense
        // block, wrapped in a Moe with one expert.
        let embedding: Vec<f32> = (0..topo1.vocab)
            .flat_map(|id| dense.embedding_row(id as u8).to_vec())
            .collect();
        let blocks: Vec<BlockFfn> = dense
            .blocks()
            .iter()
            .map(|b| {
                let (up, down) = b.as_dense().expect("dense");
                let router = LowRankRouter::new(
                    2,
                    topo1.d_model,
                    1,
                    vec![0.0f32; 2 * topo1.d_model],
                    vec![0.0f32; 2],
                    vec![0.0f32; 2],
                    vec![0.0f32; 1],
                )
                .expect("router valid");
                BlockFfn::Moe {
                    router,
                    experts: vec![(up.clone(), down.clone())],
                }
            })
            .collect();
        let moe = StateCheckpoint::new_moe(
            topo1,
            embedding,
            dense.state_in.clone(),
            dense.state_out.clone(),
            dense.decay_raw().to_vec(),
            blocks,
        )
        .expect("single-expert MoE is valid");

        let lo_dense = IntStateLoweredModel::lower(&dense).expect("dense lowers");
        let lo_moe = IntStateLoweredModel::lower(&moe).expect("moe lowers");
        assert_eq!(lo_dense.down_width, lo_moe.down_width);
        assert_eq!(
            lo_dense.down_delta_structural_bound,
            lo_moe.down_delta_structural_bound
        );

        // Drive a multi-token sequence through both and require byte-identical
        // traces and identical carried state at every step.
        let mut sd = lo_dense.zero_state();
        let mut sm = lo_moe.zero_state();
        let mut input = 7u8;
        for _ in 0..48 {
            let td = lo_dense.forward(input, &mut sd);
            let tm = lo_moe.forward(input, &mut sm);
            assert_eq!(td.logits, tm.logits, "logits diverge");
            assert_eq!(td.argmax, tm.argmax, "argmax diverges");
            assert_eq!(td.block_residuals, tm.block_residuals, "residuals diverge");
            assert_eq!(td.final_q, tm.final_q, "final norm diverges");
            assert_eq!(sd, sm, "carried state diverges");
            input = td.argmax;
        }
    }

    #[test]
    fn moe_multi_expert_forwards_deterministically_and_routes() {
        let topo = StateTopology::D192_MOE_TEST;
        let ck = synthetic_moe_checkpoint(topo, 11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("moe lowers");

        // block_ffns must actually be MoE with n_experts experts.
        assert_eq!(lowered.block_ffns.len(), topo.n_blocks);
        for b in &lowered.block_ffns {
            match b {
                LoweredBlockFfn::Moe {
                    experts,
                    router,
                    fixed_router,
                } => {
                    assert_eq!(experts.len(), topo.n_experts);
                    assert_eq!(router.n_experts(), topo.n_experts);
                    assert_eq!(fixed_router.n_experts(), topo.n_experts);
                }
                LoweredBlockFfn::Dense { .. } => panic!("expected MoE block"),
            }
        }

        // Deterministic: same input + same zero state => identical trace.
        let mut s1 = lowered.zero_state();
        let mut s2 = lowered.zero_state();
        let a = lowered.forward(9, &mut s1);
        let b = lowered.forward(9, &mut s2);
        assert_eq!(a.logits, b.logits);
        assert_eq!(s1, s2);

        // The router (running on the dequantized residual) selects an expert;
        // over a token window at least two distinct experts fire in block 0
        // (else the router is degenerate and the test would not exercise
        // dispatch).
        let LoweredBlockFfn::Moe { router, .. } = &lowered.block_ffns[0] else {
            panic!("block 0 is MoE");
        };
        let mut state = lowered.zero_state();
        let mut input = 3u8;
        let mut selected = std::collections::BTreeSet::new();
        for _ in 0..64 {
            // Recompute the exact residual the forward routes on: emb row on
            // the Q19.5 grid dequantized to f32.
            let x: Vec<f32> = lowered
                .emb_resid_row(input)
                .iter()
                .map(|&v| v as f32 / STATE_RESID_ONE as f32)
                .collect();
            selected.insert(router.route_f32(&x));
            let t = lowered.forward(input, &mut state);
            input = t.argmax;
        }
        assert!(
            selected.len() >= 2,
            "router degenerate: only experts {selected:?} ever selected"
        );

        // Different router weights select a different expert on the same input.
        // Flip the expert_bias to strongly prefer a different expert and check
        // the top-1 index changes for at least one probe input.
        let base_input: Vec<f32> = lowered
            .emb_resid_row(3)
            .iter()
            .map(|&v| v as f32 / STATE_RESID_ONE as f32)
            .collect();
        let base_e = router.route_f32(&base_input);
        let other_e = (base_e + 1) % topo.n_experts;
        let mut biased_expert_bias = vec![0.0f32; topo.n_experts];
        biased_expert_bias[other_e] = 1e6;
        let biased = LowRankRouter::new(
            router.rank(),
            topo.d_model,
            topo.n_experts,
            vec![0.0f32; router.rank() * topo.d_model],
            vec![0.0f32; router.rank()],
            vec![0.0f32; topo.n_experts * router.rank()],
            biased_expert_bias,
        )
        .expect("biased router valid");
        assert_eq!(biased.route_f32(&base_input), other_e);
        assert_ne!(
            biased.route_f32(&base_input),
            base_e,
            "biasing the router must change the selected expert"
        );
    }

    #[test]
    fn moe_arity_and_mixup_are_rejected() {
        // A MoE topology with a Dense block is rejected.
        let topo = StateTopology::D192_MOE_TEST;
        let dense = synthetic_state_checkpoint_with(
            StateTopology {
                n_experts: 1,
                ..topo
            },
            2,
        );
        let (up, down) = dense.blocks()[0].as_dense().unwrap();
        let embedding: Vec<f32> = (0..topo.vocab)
            .flat_map(|id| dense.embedding_row(id as u8).to_vec())
            .collect();
        let dense_blocks: Vec<BlockFfn> = (0..topo.n_blocks)
            .map(|_| BlockFfn::Dense {
                up: up.clone(),
                down: down.clone(),
            })
            .collect();
        let mixup = StateCheckpoint::new_moe(
            topo,
            embedding.clone(),
            dense.state_in.clone(),
            dense.state_out.clone(),
            dense.decay_raw().to_vec(),
            dense_blocks,
        );
        assert!(matches!(mixup, Err(StateModelError::MoeDenseMixup { .. })));

        // Wrong expert count is rejected.
        let good = synthetic_moe_checkpoint(topo, 3);
        let mut short_blocks = Vec::new();
        for b in good.blocks() {
            if let BlockFfn::Moe { router, experts } = b {
                let mut experts = experts.clone();
                experts.pop(); // one short
                short_blocks.push(BlockFfn::Moe {
                    router: router.clone(),
                    experts,
                });
            }
        }
        let short = StateCheckpoint::new_moe(
            topo,
            embedding,
            dense.state_in.clone(),
            dense.state_out.clone(),
            dense.decay_raw().to_vec(),
            short_blocks,
        );
        assert!(matches!(
            short,
            Err(StateModelError::MoeArityMismatch { .. })
        ));
    }

    #[test]
    fn router_matches_moe_parity_summation_order() {
        // Cross-check the route_f32 fold against a hand-rolled reference that
        // matches gbf-bench/src/moe_parity.rs's Router::route byte-for-byte.
        let rank = 3;
        let d_model = 5;
        let n_experts = 4;
        let ip: Vec<f32> = (0..rank * d_model)
            .map(|i| (i as f32 * 0.37).sin())
            .collect();
        let ib: Vec<f32> = (0..rank).map(|i| (i as f32 * 1.1).cos()).collect();
        let ep: Vec<f32> = (0..n_experts * rank)
            .map(|i| (i as f32 * 0.19).sin() * 0.5)
            .collect();
        let eb: Vec<f32> = (0..n_experts).map(|i| (i as f32 * 0.7).cos()).collect();
        let router = LowRankRouter::new(
            rank,
            d_model,
            n_experts,
            ip.clone(),
            ib.clone(),
            ep.clone(),
            eb.clone(),
        )
        .expect("router valid");
        let x: Vec<f32> = (0..d_model).map(|i| (i as f32 - 2.0) * 0.3).collect();

        // Reference (identical fold order to moe_parity Router::route).
        let hid: Vec<f32> = ip
            .chunks_exact(d_model)
            .zip(ib.iter())
            .map(|(row, &bias)| {
                row.iter()
                    .zip(x.iter())
                    .map(|(&w, &xi)| w * xi)
                    .fold(bias, |acc, p| acc + p)
            })
            .collect();
        let mut best_e = 0usize;
        let mut best_v = f32::NEG_INFINITY;
        for (e, (row, &bias)) in ep.chunks_exact(rank).zip(eb.iter()).enumerate() {
            let acc = row
                .iter()
                .zip(hid.iter())
                .map(|(&w, &hk)| w * hk)
                .fold(bias, |a, p| a + p);
            if acc > best_v {
                best_v = acc;
                best_e = e;
            }
        }
        assert_eq!(router.route_f32(&x), best_e);

        // Ties keep the lowest index: all-equal raws => expert 0.
        let flat = LowRankRouter::new(
            1,
            d_model,
            n_experts,
            vec![0.0f32; d_model],
            vec![0.0f32; 1],
            vec![0.0f32; n_experts],
            vec![2.5f32; n_experts],
        )
        .expect("flat router valid");
        assert_eq!(flat.route_f32(&x), 0);
    }

    // -----------------------------------------------------------------------
    // fixed-point MoE router (router-fx.v1)
    // -----------------------------------------------------------------------

    /// Deterministic synthetic rank-2, 8-expert router with small (real-scale)
    /// weights, plus a matching i24 Q19.5 residual generator.
    fn synthetic_fixed_router_case(
        seed: u64,
        d_model: usize,
    ) -> (LowRankRouter, FixedRouter, Vec<i32>) {
        let rank = 2usize;
        let n_experts = 8usize;
        let mut rng = seed ^ 0x1234_5678_9abc_def0;
        let mut next = move || {
            rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        // Router weights in ~[-0.5, 0.5], biases in ~[-1, 1] (real student scale).
        let mut unit = move || (next() >> 40) as f64 / (1u64 << 24) as f64 * 2.0 - 1.0;
        let ip: Vec<f32> = (0..rank * d_model).map(|_| (unit() * 0.5) as f32).collect();
        let ib: Vec<f32> = (0..rank).map(|_| unit() as f32).collect();
        let ep: Vec<f32> = (0..n_experts * rank)
            .map(|_| (unit() * 1.5) as f32)
            .collect();
        let eb: Vec<f32> = (0..n_experts).map(|_| unit() as f32).collect();
        let router = LowRankRouter::new(rank, d_model, n_experts, ip, ib, ep, eb)
            .expect("synthetic router valid");
        let fixed = FixedRouter::lower(&router, 0).expect("fixed router lowers");
        // Residual on the Q19.5 grid: |x| up to ~2000 real (64000 raw), the
        // representative range the real student reaches.
        let x_i24: Vec<i32> = (0..d_model)
            .map(|i| ((i as i64 * 6151 % 128001) - 64000) as i32)
            .collect();
        (router, fixed, x_i24)
    }

    #[test]
    fn fixed_router_matches_f32_router_on_synthetic_rank2_8experts() {
        // On clear-margin cases the integer router-fx.v1 argmax must equal the
        // f32 reference argmax for every seed (100% agreement).
        let d_model = 192usize;
        let mut checked = 0usize;
        for seed in 0..64u64 {
            let (router, fixed, x_i24) = synthetic_fixed_router_case(seed, d_model);
            let x_f32: Vec<f32> = x_i24
                .iter()
                .map(|&v| v as f32 / STATE_RESID_ONE as f32)
                .collect();
            let f_e = router.route_f32(&x_f32);
            let (fx_e, raw) = fixed.route_with_logits(&x_i24);
            // Only assert on clear-margin cases (integer vs f32 quantization can
            // legitimately flip a genuine near-tie; the real gate proves 0 such
            // ties on the deployed student).
            let mut sorted = raw.clone();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            let margin = sorted[0] - sorted[1];
            // Q32.32 units: 2^32 ~ one raw-logit real unit. Require a clear gap.
            if margin > (1i64 << 34) {
                assert_eq!(fx_e, f_e, "seed {seed}: clear-margin disagreement");
                checked += 1;
            }
        }
        assert!(
            checked >= 48,
            "too few clear-margin cases exercised: {checked}"
        );
    }

    #[test]
    fn fixed_router_is_host_deterministic_and_shift_is_round_half_away() {
        // Determinism: identical inputs => identical route and raw logits.
        let (_, fixed, x_i24) = synthetic_fixed_router_case(7, 192);
        let a = fixed.route_with_logits(&x_i24);
        let b = fixed.route_with_logits(&x_i24);
        assert_eq!(a, b, "fixed router is not host-deterministic");

        // Pin the hidden shift as ROUND-HALF-AWAY-FROM-ZERO (not truncate, not
        // round-to-even). Build a router whose single hidden accumulator lands
        // exactly on a .5 boundary in the Q16.16 shift, for both signs.
        //
        // hidden_acc = win_q * xr with rank 1, d_model 1. Choose win_q and xr so
        // that |hidden_acc| = 3 * 2^15 (i.e. 1.5 in Q16.16 -> rounds AWAY to 2).
        // win_q = rte(w * 2^16). Pick w = 1.5 -> win_q = 98304 = 3*2^15.
        // xr = x_i24 << 11. Pick x_i24 = 1 -> xr = 2048.
        // hidden_acc = 98304 * 2048 = 201,326,592 = 3 * 2^26.
        // >>16 with round-half-away: (|acc| + 2^15) >> 16 = (3*2^26 + 2^15)>>16.
        // 3*2^26 = 201326592; /2^16 = 3072.0 exactly (not a tie here); construct
        // a real .5 tie instead: |acc| = k*2^16 + 2^15.
        let d_model = 1usize;
        // Want hidden_acc magnitude = 2^16 + 2^15 = 98304 (=> 1.5 in Q16.16).
        // win_q * xr = 98304. xr = 1<<11 = 2048 (x_i24 = 1) => win_q = 48 = w*2^16
        // => w = 48/65536.
        let w = 48.0f32 / 65536.0;
        let pos = LowRankRouter::new(
            1,
            d_model,
            2,
            vec![w],
            vec![0.0],
            vec![1.0, 0.0], // expert 0 reads +hidden, expert 1 reads 0
            vec![0.0, 0.0],
        )
        .expect("router valid");
        let fpos = FixedRouter::lower(&pos, 0).expect("lowers");
        // x_i24 = 1: hidden_acc = 48 * 2048 = 98304; (98304 + 32768)>>16 = 2.
        let (_, raw_pos) = fpos.route_with_logits(&[1]);
        // raw[0] = wout_q(1.0)=65536 * hidden_q(2) = 131072; raw[1] = 0.
        assert_eq!(raw_pos[0], 131_072, "round-half-away up (+1.5 -> +2)");
        // Negative: x_i24 = -1 => hidden_acc = -98304; magnitude rounds AWAY to
        // 2 => hidden_q = -2 => raw[0] = -131072.
        let (_, raw_neg) = fpos.route_with_logits(&[-1]);
        assert_eq!(raw_neg[0], -131_072, "round-half-away down (-1.5 -> -2)");
    }

    #[test]
    fn fixed_router_lower_rejects_overwidth_weight_i32() {
        // A router weight of 2^15 quantizes to 2^31 which does not fit i32.
        let router = LowRankRouter::new(
            1,
            1,
            2,
            vec![32768.0f32], // 2^15 -> win_q = 2^31, escapes i32
            vec![0.0],
            vec![0.0, 0.0],
            vec![0.0, 0.0],
        )
        .expect("router valid (finite weights)");
        match FixedRouter::lower(&router, 3) {
            Err(StateModelError::RouterWeightEscapesI32 {
                block: 3,
                what: "input_projection",
                index: 0,
                quantized,
            }) => assert_eq!(quantized, 1i64 << 31),
            other => panic!("expected RouterWeightEscapesI32, got {other:?}"),
        }
    }

    #[test]
    fn fixed_router_lower_rejects_overwidth_hidden_i62() {
        // A wide-fan-in input projection at near-max i32 weights drives the
        // hidden accumulator's structural bound past i62. d_model such that
        // sum_c |win_q| * XR_MAX exceeds 2^62. |win_q| ~ 2^30 (w ~ 2^14), XR_MAX
        // ~ 2^34, so ~2^64 * d_model / ... a handful of columns overflow i62.
        let d_model = 64usize;
        let w = 16384.0f32; // win_q = rte(2^14 * 2^16) = 2^30, fits i32
        let router = LowRankRouter::new(
            1,
            d_model,
            2,
            vec![w; d_model],
            vec![0.0],
            vec![0.0, 0.0],
            vec![0.0, 0.0],
        )
        .expect("router valid");
        match FixedRouter::lower(&router, 5) {
            Err(StateModelError::RouterHiddenEscapesI62 {
                block: 5,
                row: 0,
                bound,
            }) => {
                assert!(
                    bound > ROUTER_ACC_I62_BOUND,
                    "bound {bound} must exceed i62"
                );
            }
            other => panic!("expected RouterHiddenEscapesI62, got {other:?}"),
        }
    }

    #[test]
    fn fixed_router_lower_rejects_overwidth_raw_logit_i62() {
        // Keep the hidden accumulator inside i62 but drive a single expert's raw
        // logit past i62 via a large expert_projection weight against a large
        // (but in-bounds) hidden_q. hidden_q_bound ~ (bound>>16); pick modest
        // input weights so hidden fits, and a near-max wout_q so raw overflows.
        let d_model = 192usize;
        // input: w ~ 2^7 over d_model=192 => win_q = rte(2^7 * 2^16) = 2^23.
        // sum_c |win_q| = 192 * 2^23 ~ 2^30.6; * XR_MAX(~2^34) ~ 2^64.6 -> hidden
        // would overflow. Reduce d_model contribution: use a single nonzero col.
        let mut ip = vec![0.0f32; d_model];
        ip[0] = 8192.0; // win_q = 2^13 * 2^16 = 2^29 (one column)
        // hidden bound ~ 2^29 * XR_MAX(~2^34) ~ 2^63 -> still too big; shrink.
        ip[0] = 64.0; // win_q = 2^6 * 2^16 = 2^22; bound ~ 2^22 * 2^34 = 2^56 (< i62)
        // hidden_q_bound ~ 2^56 >> 16 = 2^40. Then a wout_q ~ rte(30000*2^16)
        // ~ 2^31 (fits i32) times 2^40 rank-1 ~ 2^71 -> raw overflows i62.
        let ep = vec![30000.0f32, 0.0]; // expert 0 big, expert 1 zero
        let router = LowRankRouter::new(1, d_model, 2, ip, vec![0.0], ep, vec![0.0, 0.0])
            .expect("router valid");
        match FixedRouter::lower(&router, 2) {
            Err(StateModelError::RouterRawLogitEscapesI62 {
                block: 2,
                expert: 0,
                bound,
            }) => {
                assert!(
                    bound > ROUTER_ACC_I62_BOUND,
                    "bound {bound} must exceed i62"
                );
            }
            // If the hidden bound trips first, the construction is wrong for this
            // test; surface it explicitly.
            other => panic!("expected RouterRawLogitEscapesI62, got {other:?}"),
        }
    }

    #[test]
    fn fixed_router_argmax_tiebreak_is_lowest_index() {
        // All-zero weights and biases => every raw logit is 0 => the strict `>`
        // scan from 0 keeps expert 0.
        let router = LowRankRouter::new(
            2,
            192,
            8,
            vec![0.0f32; 2 * 192],
            vec![0.0f32; 2],
            vec![0.0f32; 8 * 2],
            vec![0.0f32; 8],
        )
        .expect("router valid");
        let fixed = FixedRouter::lower(&router, 0).expect("lowers");
        let x_i24: Vec<i32> = (0..192i32).map(|i| i * 37 - 3000).collect();
        let (e, raw) = fixed.route_with_logits(&x_i24);
        assert!(raw.iter().all(|&v| v == 0), "all raw logits zero");
        assert_eq!(e, 0, "lowest-index tiebreak selects expert 0");

        // Equal nonzero biases also tie to the lowest index.
        let tied = LowRankRouter::new(
            1,
            192,
            8,
            vec![0.0f32; 192],
            vec![0.0f32; 1],
            vec![0.0f32; 8],
            vec![0.75f32; 8],
        )
        .expect("router valid");
        let ftied = FixedRouter::lower(&tied, 0).expect("lowers");
        assert_eq!(ftied.route(&x_i24), 0);
    }
}
