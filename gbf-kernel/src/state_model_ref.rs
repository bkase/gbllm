//! Host-side evaluators for the LinearState stateful ROM bring-up
//! (stateful deployment of the bd-29ai4 arm-B checkpoint).
//!
//! Two evaluators over the same committed S5 arm-B export
//! (`f_s5_state_checkpoint_export.v1`):
//!
//! 1. [`f32_state_forward`] — a faithful port of the trainer's hard-ternary
//!    f32 forward pass (`gbf-experiments/src/bin/s5_state_ab.rs`
//!    `forward_seq`), including the exact recurrence
//!    `h_t = decay (.) h_{t-1} + W_in(actq(rms_norm(x_t)))`,
//!    `y_t = actq(W_out(h_t))`, `x'_t = x_t + y_t`, followed by the same
//!    4-block pre-norm residual FFN stack and tied f32 head.
//! 2. [`IntStateLoweredModel::forward`] — the **canonical integer semantics**
//!    for the deployed stateful ROM, extending the pinned v0 integer
//!    conventions (u8 zero-point-128 activations, i16 matvec accumulators
//!    with `-128 * sum(row)` seeds, per-row Q8.8 integer scale multiplies,
//!    GELU LUT, integer RMS norm) to the recurrence. The ROM built by
//!    [`crate::asm_impl_state`] must reproduce this function byte-exactly,
//!    including the state vector carried in WRAM across tokens.
//!
//! Integer recurrence representation (measured on the real checkpoint over
//! the val stream, then pinned):
//! - The state slot `h[s]` is held as a **saturating i24 in an i32 word**,
//!   in units of `(ACT_RANGE / QMAX) / 256` real (the "m unit"), so the
//!   in-projection delta `m = scale_raw * acc` lands **exactly** (no
//!   rounding on accumulate). Measured max |h| on the val stream is ~1.8e5,
//!   45x inside the 2^23 - 1 saturation bound.
//! - The decay multiply is `sign(h) * ((|h| * decay_raw + 128) >> 8)` — a
//!   per-slot Q8.8 integer multiply with round-half-away-from-zero. All MT4
//!   decay raws {128, 192, 224, 240} fit u8, which the loader validates.
//! - The residual stream is **i24 Q19.5** (resolution 1/32, range
//!   +/-262143.97): the trained checkpoint's residual measurably reaches
//!   |x_raw| ~ 8.4e4 at 1/32 resolution, which overflows the dense
//!   bring-up's i16 Q11.5, so the format is widened honestly to three bytes
//!   per lane (still byte-serial adds on device).
//!
//! Every documented divergence between (2) and (1) is listed in
//! [`STATE_INT_SEMANTIC_DIVERGENCES`] and measured by the fidelity phase in
//! `gbf-bench`.

use std::fmt;

use crate::model_ref::{
    ACT_RANGE, BlockWeights, D_FF, D_MODEL, IntForwardStats, LoweredLayer, ModelRefError, N_BLOCKS,
    NORM_EPS, QMAX, TernaryLayer, f32_act_fake_quant, gelu_approx_f32, int_down_delta, int_matvec,
    int_scale_to_grid, rte_i64,
};

/// charset_v1 vocabulary (ids 0..=79).
pub const STATE_VOCAB: usize = 80;
/// Recurrent state width pinned by the manifest.
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

/// Documented places where the canonical integer semantics diverge from the
/// trainer's f32 semantics. Reproduced verbatim in the evidence report.
pub const STATE_INT_SEMANTIC_DIVERGENCES: [&str; 11] = [
    "Residual stream is i24 Q19.5 (resolution 1/32, range +/-262143.97) with mod-2^24 wrapping adds (trainer: f32). i24 replaces the dense bring-up's i16 Q11.5 because this checkpoint's residual measurably reaches |x| ~ 2,600 real (8.4e4 raw at 1/32), which no 16-bit split of range/resolution can hold without wrapping (measured: Q13.3 and Q12.4 both wrap tens of thousands of times over 120k val positions). Embedding rows are quantized to Q19.5 at lowering time (round-half-even).",
    "State slots are saturating-i24 integers in units of (ACT_RANGE/QMAX)/256 real, so the in-projection delta m = scale_raw * acc accumulates exactly; the trainer carries f32 state. Saturation at +/-(2^23 - 1) is canonical and counted (measured max |h| ~ 1.8e5, 45x inside the bound).",
    "The per-token decay multiply is sign(h) * ((|h| * decay_raw + 128) >> 8) (round-half-away on the Q8.8 product); the trainer multiplies f32 state by decay_raw/256 exactly. Rounding error is at most 0.5 state units (~1.2e-4 real) per slot per token; adding 8 fractional state bits was measured to change val bpc by < 1e-4, so the narrower exact-delta format is pinned.",
    "The state out-projection epilogue quantizes y = clamp(round_half_away(scale_raw * acc2 / 65536), -127, 127) onto the activation grid via integer multiply (trainer: f32 matvec then Int8 fake-quant with round-ties-even). acc2 is the i32 dot product of ternary out-projection rows with the i24 state (measured max |acc2| ~ 1.6e6, structural bound 2^29).",
    "The state residual add applies y through a 255-entry i16 LUT y_resid[p] = round_ties_even(p * 256 / 127) on the Q19.5 grid (trainer adds the unquantized f32 fake-quant output).",
    "RMS norm: integer sum of squares over 64 i24 lanes (u64 accumulator, 7 bytes on device), mean = floor(sum/64), epsilon = +1 raw mean unit, rms = floor(isqrt48(mean+1)); norm output and Int8 activation fake-quant collapse into one rounded division q = clamp(round_half_away(|x|*127 / (8*rms)), 0, 127) * sign(x) (trainer: f32 divide, clamp to +/-8, round-ties-even quantization).",
    "GELU is a 255-entry LUT indexed by the pre-activation value quantized to the same [-8,8]/127 grid (round-half-away on the Q8.8 scale product); the trainer applies tanh-approximate GELU to the unquantized f32 matvec output before fake-quant.",
    "Up/down epilogues apply the Q8.8 row scale as an integer multiply with round-half-away rounding (round-ties-even in the trainer's f32 path).",
    "Down-projection output is quantized to the Q19.5 residual grid as sign(m) * min(65535, round_half_away(|m| / 127)) before the residual add (identical formula to the dense Q11.5 epilogue because the fractional bits are unchanged); the trainer adds unquantized f32 deltas. The 65535 clamp is canonical and counted.",
    "The final norm output is activation-quantized to the [-8,8]/127 grid before the tied head; tied-head weights are the embedding quantized per-tensor to i8 (scale = max|emb|/127, round-ties-even); logits are integer dot products in i32 (i24 on device).",
    "Integer rounding is round-half-away-from-zero throughout the runtime path; the trainer/Burn rounds ties to even. Because the recurrence carries state across tokens, per-token rounding differences accumulate: fidelity (bpc delta, argmax agreement) is therefore measured over long sequential streams, not per-context.",
];

// ---------------------------------------------------------------------------
// checkpoint container
// ---------------------------------------------------------------------------

/// The raw exported arm-B checkpoint: f32 embedding, state in/out ternary
/// projections, per-slot Q8.8 decay raws, and four ternary FFN blocks.
#[derive(Debug, Clone)]
pub struct StateCheckpoint {
    embedding: Vec<f32>,
    pub state_in: TernaryLayer,
    pub state_out: TernaryLayer,
    decay_raw: Vec<u16>,
    blocks: Vec<BlockWeights>,
}

/// State-checkpoint validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateModelError {
    Shape {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteEmbedding,
    DecayRawTooWide {
        slot: usize,
        raw: u16,
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
            Self::NonFiniteEmbedding => write!(f, "embedding contains non-finite values"),
            Self::DecayRawTooWide { slot, raw } => write!(
                f,
                "decay slot {slot} raw {raw} exceeds the u8 device table (MT4 rates are <= 240)"
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
    pub fn new(
        embedding: Vec<f32>,
        state_in: TernaryLayer,
        state_out: TernaryLayer,
        decay_raw: Vec<u16>,
        blocks: Vec<BlockWeights>,
    ) -> Result<Self, StateModelError> {
        if embedding.len() != STATE_VOCAB * D_MODEL {
            return Err(StateModelError::Shape {
                what: "embedding",
                expected: STATE_VOCAB * D_MODEL,
                actual: embedding.len(),
            });
        }
        if embedding.iter().any(|v| !v.is_finite()) {
            return Err(StateModelError::NonFiniteEmbedding);
        }
        if state_in.rows() != STATE_SLOTS || state_in.cols() != D_MODEL {
            return Err(StateModelError::Shape {
                what: "state in-projection",
                expected: STATE_SLOTS * D_MODEL,
                actual: state_in.rows() * state_in.cols(),
            });
        }
        if state_out.rows() != D_MODEL || state_out.cols() != STATE_SLOTS {
            return Err(StateModelError::Shape {
                what: "state out-projection",
                expected: D_MODEL * STATE_SLOTS,
                actual: state_out.rows() * state_out.cols(),
            });
        }
        if decay_raw.len() != STATE_SLOTS {
            return Err(StateModelError::Shape {
                what: "decay slots",
                expected: STATE_SLOTS,
                actual: decay_raw.len(),
            });
        }
        for (slot, &raw) in decay_raw.iter().enumerate() {
            if raw > 255 {
                return Err(StateModelError::DecayRawTooWide { slot, raw });
            }
        }
        if blocks.len() != N_BLOCKS {
            return Err(StateModelError::Shape {
                what: "blocks",
                expected: N_BLOCKS,
                actual: blocks.len(),
            });
        }
        for block in &blocks {
            if block.up.rows() != D_FF || block.up.cols() != D_MODEL {
                return Err(StateModelError::Shape {
                    what: "up projection",
                    expected: D_FF * D_MODEL,
                    actual: block.up.rows() * block.up.cols(),
                });
            }
            if block.down.rows() != D_MODEL || block.down.cols() != D_FF {
                return Err(StateModelError::Shape {
                    what: "down projection",
                    expected: D_MODEL * D_FF,
                    actual: block.down.rows() * block.down.cols(),
                });
            }
        }
        Ok(Self {
            embedding,
            state_in,
            state_out,
            decay_raw,
            blocks,
        })
    }

    #[must_use]
    pub fn embedding_row(&self, id: u8) -> &[f32] {
        let start = usize::from(id) * D_MODEL;
        &self.embedding[start..start + D_MODEL]
    }

    #[must_use]
    pub fn blocks(&self) -> &[BlockWeights] {
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

fn f32_rms_norm_clip(x: &[f32; D_MODEL]) -> [f32; D_MODEL] {
    let mut sum_sq = 0.0f32;
    for v in x {
        sum_sq += v * v;
    }
    let mean_sq = sum_sq / (D_MODEL as f32);
    let rms = (mean_sq + NORM_EPS).sqrt();
    let mut out = [0.0f32; D_MODEL];
    for (o, v) in out.iter_mut().zip(x.iter()) {
        *o = (v / rms).clamp(-ACT_RANGE, ACT_RANGE);
    }
    out
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
/// recurrent state carried in `state`. Returns the 80 tied-head logits.
/// `state` must be zeroed at stream start (trained initial-state contract).
#[must_use]
pub fn f32_state_forward(
    ck: &StateCheckpoint,
    prev: u8,
    state: &mut [f32; STATE_SLOTS],
) -> [f32; STATE_VOCAB] {
    let mut x = [0.0f32; D_MODEL];
    x.copy_from_slice(ck.embedding_row(prev));

    // State block: delta from the normed+act-quantized input, decayed state
    // update, act-quantized out-projection, residual add.
    let mut normed = f32_rms_norm_clip(&x);
    for v in &mut normed {
        *v = f32_act_fake_quant(*v);
    }
    let mut delta = [0.0f32; STATE_SLOTS];
    f32_ternary_matvec(&ck.state_in, &normed, &mut delta);
    for (slot, (h, d)) in state.iter_mut().zip(delta.iter()).enumerate() {
        let decay = f32::from(ck.decay_raw[slot]) / 256.0;
        *h = *h * decay + *d;
    }
    let mut y = [0.0f32; D_MODEL];
    f32_ternary_matvec(&ck.state_out, state, &mut y);
    for (xv, yv) in x.iter_mut().zip(y.iter()) {
        *xv += f32_act_fake_quant(*yv);
    }

    // The same 4-block pre-norm residual FFN stack as the dense export.
    let mut hidden = [0.0f32; D_FF];
    let mut ffn_delta = [0.0f32; D_MODEL];
    for block in ck.blocks() {
        let mut normed = f32_rms_norm_clip(&x);
        for v in &mut normed {
            *v = f32_act_fake_quant(*v);
        }
        f32_ternary_matvec(&block.up, &normed, &mut hidden);
        for v in &mut hidden {
            *v = f32_act_fake_quant(gelu_approx_f32(*v));
        }
        f32_ternary_matvec(&block.down, &hidden, &mut ffn_delta);
        for (xv, dv) in x.iter_mut().zip(ffn_delta.iter()) {
            *xv += dv;
        }
    }

    let normed = f32_rms_norm_clip(&x);
    let mut logits = [0.0f32; STATE_VOCAB];
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

/// The integer-lowered stateful model: every table the canonical integer
/// function (and therefore the ROM) needs.
#[derive(Debug, Clone)]
pub struct IntStateLoweredModel {
    /// Embedding rows on the Q19.5 residual grid (`[STATE_VOCAB * D_MODEL]`,
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
    /// State in-projection with `-128 * sum(row)` accumulator seeds.
    pub state_in: LoweredLayer,
    /// State out-projection (operates on the raw state, no zero-point seed).
    pub state_out: TernaryLayer,
    /// Per-slot decay raws, validated to fit the u8 device table.
    pub decay_u8: Vec<u8>,
    /// Lowered FFN blocks (up, down).
    pub blocks: Vec<(LoweredLayer, LoweredLayer)>,
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
    }
}

impl Default for StateForwardStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Full trace of one canonical stateful integer forward pass, including the
/// values the ROM gate compares byte-exactly.
#[derive(Debug, Clone)]
pub struct IntStateForwardTrace {
    /// State-block norm output on the u8 zp128 grid.
    pub state_norm_act: [u8; D_MODEL],
    /// In-projection raw accumulators (i16).
    pub state_in_acc: [i16; STATE_SLOTS],
    /// The state vector after this token's update (saturating i24 in i32).
    pub state_after: [i32; STATE_SLOTS],
    /// Out-projection raw accumulators (i32).
    pub state_out_acc: [i32; D_MODEL],
    /// y on the u8 zp128 grid.
    pub y_act: [u8; D_MODEL],
    /// Residual vector after each FFN block (i24 in i32, Q19.5).
    pub block_residuals: [[i32; D_MODEL]; N_BLOCKS],
    /// Block-0 debug checkpoints (mirrored by the ROM's debug dumps).
    pub block0_norm_act: [u8; D_MODEL],
    pub block0_up_acc: [i16; D_FF],
    pub block0_gelu_act: [u8; D_FF],
    pub block0_down_acc: [i16; D_MODEL],
    /// Final norm output on the activation grid (`[-127, 127]`).
    pub final_q: [i16; D_MODEL],
    /// Tied-head integer logits (i24-range values held in i32).
    pub logits: [i32; STATE_VOCAB],
    /// Argmax id (lowest index wins ties).
    pub argmax: u8,
    pub stats: StateForwardStats,
}

/// Norm+quant over the 64-lane i24 Q19.5 residual. Same canonical steps as
/// the dense `int_norm_quant`, widened: 7-byte sum-of-squares accumulator,
/// 48-bit floor isqrt, u32 numerators in the rounded division.
pub fn int_norm_quant24(x: &[i32; D_MODEL], stats: &mut IntForwardStats) -> [i16; D_MODEL] {
    let mut ss: u64 = 0;
    for &v in x {
        let a = u64::from(v.unsigned_abs());
        ss += a * a;
    }
    stats.max_norm_sumsq = stats.max_norm_sumsq.max(ss);
    let mean = ss >> 6;
    debug_assert!(mean < 1 << 46, "i24 lanes bound the mean below 2^46");
    let r = u64::from(isqrt_u48(mean + 1));
    stats.min_norm_rms_raw = stats.min_norm_rms_raw.min(r as u32);
    let d = 8 * r;
    let d2 = 16 * r;
    let mut q = [0i16; D_MODEL];
    for (qv, &v) in q.iter_mut().zip(x.iter()) {
        let a = u64::from(v.unsigned_abs());
        let num = a * 254 + d;
        debug_assert!(num < 1 << 32, "division numerator fits u32 for i24 lanes");
        let q_abs = (num / d2).min(i64::from(QMAX) as u64) as i16;
        *qv = if v < 0 { -q_abs } else { q_abs };
    }
    q
}

/// Wrap a residual add result to the i24 range (mod 2^24, sign-extended),
/// mirroring the device's 3-byte adds.
fn wrap_i24(v: i32) -> i32 {
    (v << 8) >> 8
}

impl IntStateLoweredModel {
    pub fn lower(ck: &StateCheckpoint) -> Result<Self, StateModelError> {
        // Embedding on the Q19.5 residual grid (round-ties-even in f64).
        let mut emb_resid = Vec::with_capacity(STATE_VOCAB * D_MODEL);
        let mut max_abs = 0.0f32;
        for id in 0..STATE_VOCAB {
            for &v in ck.embedding_row(id as u8) {
                max_abs = max_abs.max(v.abs());
                let q = rte_i64(f64::from(v) * f64::from(STATE_RESID_ONE))
                    .clamp(i64::from(RESID_I24_MIN), i64::from(RESID_I24_MAX));
                emb_resid.push(q as i32);
            }
        }

        // Head i8 (per-tensor symmetric).
        let head_step = max_abs / QMAX as f32;
        let mut head_i8 = Vec::with_capacity(STATE_VOCAB * D_MODEL);
        for id in 0..STATE_VOCAB {
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

        let mut blocks = Vec::with_capacity(N_BLOCKS);
        for block in ck.blocks() {
            blocks.push((LoweredLayer::new(&block.up), LoweredLayer::new(&block.down)));
        }

        Ok(Self {
            emb_resid,
            head_i8,
            head_step,
            gelu_lut: crate::model_ref::build_gelu_lut(),
            y_resid_lut,
            state_in: LoweredLayer::new(&ck.state_in),
            state_out: ck.state_out.clone(),
            decay_u8,
            blocks,
        })
    }

    #[must_use]
    pub fn emb_resid_row(&self, id: u8) -> &[i32] {
        let start = usize::from(id) * D_MODEL;
        &self.emb_resid[start..start + D_MODEL]
    }

    #[must_use]
    pub fn head_i8_row(&self, id: u8) -> &[i8] {
        let start = usize::from(id) * D_MODEL;
        &self.head_i8[start..start + D_MODEL]
    }

    /// Real value represented by one integer logit unit.
    #[must_use]
    pub fn logit_dequant_step(&self) -> f64 {
        f64::from(ACT_RANGE) / f64::from(QMAX) * f64::from(self.head_step)
    }

    /// The canonical integer forward pass for one token. `state` is the
    /// persistent recurrence vector (zeroed at stream start), updated in
    /// place exactly as the ROM updates its WRAM copy.
    #[must_use]
    pub fn forward(&self, prev: u8, state: &mut [i32; STATE_SLOTS]) -> IntStateForwardTrace {
        let mut stats = StateForwardStats::new();

        let mut x = [0i32; D_MODEL];
        x.copy_from_slice(self.emb_resid_row(prev));

        // --- state block ---
        let q = int_norm_quant24(&x, &mut stats.ffn);
        let mut state_norm_act = [0u8; D_MODEL];
        for (a, qv) in state_norm_act.iter_mut().zip(q.iter()) {
            *a = (qv + 128) as u8;
        }
        let mut in_acc = [0i16; STATE_SLOTS];
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
        let state_after = *state;

        // out projection over the i24 state, y quantized to the act grid
        let mut out_acc = [0i32; D_MODEL];
        let mut y_act = [0u8; D_MODEL];
        for row in 0..D_MODEL {
            let mut acc: i64 = 0;
            for (w, h) in self.state_out.row(row).iter().zip(state.iter()) {
                acc += i64::from(*w) * i64::from(*h);
            }
            debug_assert!(
                acc.unsigned_abs() < 1 << 31,
                "out-projection accumulator fits i32 (structural bound 64 * 2^23)"
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
        let mut block_residuals = [[0i32; D_MODEL]; N_BLOCKS];
        let mut block0_norm_act = [0u8; D_MODEL];
        let mut block0_up_acc = [0i16; D_FF];
        let mut block0_gelu_act = [0u8; D_FF];
        let mut block0_down_acc = [0i16; D_MODEL];
        let mut act = [0u8; D_FF];
        let mut acc = [0i16; D_FF];
        for (block_idx, (up, down)) in self.blocks.iter().enumerate() {
            let q = int_norm_quant24(&x, &mut stats.ffn);
            for (a, qv) in act.iter_mut().zip(q.iter()) {
                *a = (qv + 128) as u8;
            }
            if block_idx == 0 {
                block0_norm_act.copy_from_slice(&act[..D_MODEL]);
            }

            int_matvec(
                &up.layer,
                &up.biases,
                &act[..D_MODEL],
                &mut acc[..D_FF],
                &mut stats.ffn,
            );
            if block_idx == 0 {
                block0_up_acc.copy_from_slice(&acc[..D_FF]);
            }

            for row in 0..D_FF {
                let p = int_scale_to_grid(acc[row], up.layer.scale_raw(row), &mut stats.ffn);
                act[row] = self.gelu_lut[(p + QMAX) as usize];
            }
            if block_idx == 0 {
                block0_gelu_act.copy_from_slice(&act[..D_FF]);
            }

            int_matvec(
                &down.layer,
                &down.biases,
                &act[..D_FF],
                &mut acc[..D_MODEL],
                &mut stats.ffn,
            );
            if block_idx == 0 {
                block0_down_acc.copy_from_slice(&acc[..D_MODEL]);
            }

            for row in 0..D_MODEL {
                let d_raw = int_down_delta(acc[row], down.layer.scale_raw(row), &mut stats.ffn);
                let wide = x[row] + d_raw;
                let wrapped = wrap_i24(wide);
                if wrapped != wide {
                    stats.residual_i24_wrap_events += 1;
                }
                x[row] = wrapped;
                stats.ffn.max_abs_residual = stats.ffn.max_abs_residual.max(x[row].unsigned_abs());
            }
            block_residuals[block_idx] = x;
        }

        // --- final norm + head ---
        let final_q = int_norm_quant24(&x, &mut stats.ffn);
        let mut logits = [0i32; STATE_VOCAB];
        for (id, logit) in logits.iter_mut().enumerate() {
            let row = self.head_i8_row(id as u8);
            let mut acc32: i32 = 0;
            for (qv, ev) in final_q.iter().zip(row.iter()) {
                acc32 += i32::from(*qv) * i32::from(*ev);
            }
            stats.ffn.max_abs_logit = stats.ffn.max_abs_logit.max(acc32.unsigned_abs());
            *logit = acc32;
        }
        let mut argmax = 0u8;
        let mut best = logits[0];
        for (id, &v) in logits.iter().enumerate().skip(1) {
            if v > best {
                best = v;
                argmax = id as u8;
            }
        }

        IntStateForwardTrace {
            state_norm_act,
            state_in_acc: in_acc,
            state_after,
            state_out_acc: out_acc,
            y_act,
            block_residuals,
            block0_norm_act,
            block0_up_acc,
            block0_gelu_act,
            block0_down_acc,
            final_q,
            logits,
            argmax,
            stats,
        }
    }
}

// ---------------------------------------------------------------------------
// deterministic synthetic checkpoint (tests / smoke)
// ---------------------------------------------------------------------------

/// Deterministic synthetic stateful checkpoint for tests that must not
/// depend on the committed experiment files.
#[must_use]
pub fn synthetic_state_checkpoint(seed: u64) -> StateCheckpoint {
    let mut state = seed ^ 0x5bd1_e995_9e37_79b9;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };

    let embedding: Vec<f32> = (0..STATE_VOCAB * D_MODEL)
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

    let state_in = layer(STATE_SLOTS, D_MODEL);
    let state_out = layer(D_MODEL, STATE_SLOTS);
    // MT4 band layout: 4 equal contiguous bands.
    let decay_raw: Vec<u16> = (0..STATE_SLOTS)
        .map(|slot| [128u16, 192, 224, 240][slot / (STATE_SLOTS / 4)])
        .collect();
    let blocks = (0..N_BLOCKS)
        .map(|_| BlockWeights {
            up: layer(D_FF, D_MODEL),
            down: layer(D_MODEL, D_FF),
        })
        .collect();
    StateCheckpoint::new(embedding, state_in, state_out, decay_raw, blocks)
        .expect("synthetic state checkpoint is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(q16, q24);
    }

    #[test]
    fn state_delta_accumulates_exactly_and_decays_with_rounding() {
        let ck = synthetic_state_checkpoint(3);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut state = [0i32; STATE_SLOTS];
        let t = lowered.forward(5, &mut state);
        // From zero state, h = m exactly (decay of zero is zero).
        for slot in 0..STATE_SLOTS {
            let m =
                i32::from(lowered.state_in.layer.scale_raw(slot)) * i32::from(t.state_in_acc[slot]);
            assert_eq!(t.state_after[slot], m, "slot {slot}");
        }
        // A second token decays the carried state with round-half-away.
        let carried = state;
        let t2 = lowered.forward(6, &mut state);
        for slot in 0..STATE_SLOTS {
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
        let mut s1 = [0i32; STATE_SLOTS];
        let mut s2 = [0i32; STATE_SLOTS];
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
    fn int_and_f32_state_forward_mostly_agree_on_synthetic_model() {
        // Not a strict gate (fidelity is measured on the real checkpoint);
        // this catches gross semantic porting errors.
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut fs = [0.0f32; STATE_SLOTS];
        let mut is = [0i32; STATE_SLOTS];
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
    fn state_saturates_at_the_i24_bound() {
        let ck = synthetic_state_checkpoint(2);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut state = [STATE_CLAMP; STATE_SLOTS];
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
        let bad = StateCheckpoint::new(
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
}
