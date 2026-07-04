//! Host-side evaluators for the one-token dense bigram bring-up (bd-59qiq).
//!
//! Two evaluators over the same committed S6 checkpoint tensors:
//!
//! 1. [`f32_forward`] — a faithful port of the trainer's hard-ternary f32
//!    forward pass (`gbf-experiments/src/bin/s2_gap_and_export.rs`): full-vector
//!    RMS norm, Int8 activation fake-quant on the fixed `[-8, 8]` range,
//!    ternary matvecs with per-output-row Q8.8 scales, tanh-approximate GELU,
//!    tied f32 embedding head.
//! 2. [`IntLoweredModel::forward`] — the **canonical integer semantics** for
//!    the deployed ROM under the pinned v0 numeric contract (`history/planv0.md`
//!    "Session amendment 2026-07-04" §3): activations as `u8` at zero point
//!    128, i16 matvec accumulators seeded with `-128 * sum(row)`, per-row Q8.8
//!    scales applied as integer multiplies, GELU as a 255-entry LUT on the
//!    activation grid, RMS norm as integer sum-of-squares + floor isqrt +
//!    per-lane rounded division. The ROM built by
//!    [`crate::asm_impl_model::build_one_token_rom`] must reproduce this
//!    function byte-exactly.
//!
//! Every documented divergence between (2) and (1) is listed in
//! [`INT_SEMANTIC_DIVERGENCES`] and measured by the fidelity phase in
//! `gbf-bench`.

use std::fmt;

/// Model dimensions pinned by the `f_s6_dense_checkpoint_export.v1` manifest.
pub const D_MODEL: usize = 64;
/// FFN hidden width.
pub const D_FF: usize = 128;
/// Residual FFN block count.
pub const N_BLOCKS: usize = 4;
/// Byte vocabulary.
pub const VOCAB: usize = 256;

/// Activation fake-quant integer grid: `q in [-QMAX, QMAX]`, real value
/// `q * ACT_RANGE / QMAX`.
pub const QMAX: i32 = 127;
/// Activation fake-quant range half-width (trainer `ACT_RANGE`).
pub const ACT_RANGE: f32 = 8.0;
/// Trainer RMS-norm epsilon (f32 semantics only; the integer path uses
/// `+1` on the Q16.16 mean, i.e. ~1.526e-5).
pub const NORM_EPS: f32 = 1.0e-5;

/// Residual-stream fixed point: i16 with `RESID_FRAC_BITS` fractional bits
/// (Q11.5, range +/-1023.97, resolution 1/32). Chosen because the trained
/// checkpoint's f32 residual reaches |x| = 564.4 over the exhaustive
/// 256-context bigram input space, which overflows the v0 Q8.8 default;
/// widening the *format* (not the register width) keeps every device
/// operation 16-bit. The integer RMS norm is scale-free, so only the
/// embedding quantization and the down-epilogue constant depend on this.
pub const RESID_FRAC_BITS: u32 = 5;
/// `2^RESID_FRAC_BITS`.
pub const RESID_ONE: i32 = 1 << RESID_FRAC_BITS;

/// Documented places where the canonical integer semantics diverge from the
/// trainer's f32 semantics. Reproduced verbatim in the evidence report.
pub const INT_SEMANTIC_DIVERGENCES: [&str; 8] = [
    "Residual stream is i16 Q11.5 (resolution 1/32, range +/-1023.97) with mod-2^16 wrapping adds (trainer: f32). Q11.5 replaces the v0 Q8.8 default because the trained checkpoint's residual measurably reaches |x| = 564.4 (exhaustive over all 256 bigram contexts), which Q8.8 cannot represent; the widening is in format, not device register width. Embedding rows are quantized to Q11.5 at lowering time (round-half-even).",
    "RMS norm: integer sum of squares over 64 lanes, mean = floor(sum/64), epsilon = +1 raw mean unit (scale-dependent: ~9.8e-4 at Q11.5 vs trainer 1e-5; both are negligible against real mean-square values), rms = floor(isqrt(mean+1)) in the residual scale; norm output and Int8 activation fake-quant collapse into one rounded division q = clamp(round_half_away(|x|*127 / (8*rms)), 0, 127) * sign(x) (trainer: f32 divide, clamp to +/-8, then round-half-even quantization).",
    "GELU is a 255-entry LUT indexed by the pre-activation value quantized to the same [-8,8]/127 grid (round-half-away on the Q8.8 scale product); the trainer applies tanh-approximate GELU to the unquantized f32 matvec output before fake-quant. Pre-GELU values are clamped to +/-8 by the grid (trainer clamps after GELU; both saturate identically for |h| >= 8 up to LUT rounding).",
    "Up/down epilogues apply the Q8.8 row scale as an integer multiply with round-half-away rounding (round-half-even in the trainer's f32 path). Scale values themselves are identical: the trainer fake-quantizes scales to the Q8.8 grid.",
    "Down-projection output is quantized to the Q11.5 residual grid (round-half-away of scale*acc*8*32/(127*256) = scale*acc/127, magnitude clamped at 65535 raw) before the residual add; the trainer adds unquantized f32 deltas.",
    "The final norm output is activation-quantized to the [-8,8]/127 grid before the tied head (the trainer feeds the unquantized f32 normed vector to the head).",
    "Tied-head weights are the embedding quantized per-tensor to i8 (scale = max|emb|/127, round-half-even); logits are integer dot products in i32 (i24 on device). This reduction structurally exceeds i16 (64*127*127 = 1,032,256), so it is honestly widened past the v0 i16 accumulator target; the artifact contract's I32 canonical semantics still hold.",
    "Integer rounding is round-half-away-from-zero throughout the runtime path; the trainer/Burn rounds ties to even.",
];

// ---------------------------------------------------------------------------
// checkpoint container
// ---------------------------------------------------------------------------

/// One ternary linear layer as exported: row-major `{-1,0,+1}` weights plus
/// per-output-row raw Q8.8 scales.
#[derive(Debug, Clone)]
pub struct TernaryLayer {
    rows: usize,
    cols: usize,
    weights: Vec<i8>,
    scales_raw: Vec<u16>,
}

impl TernaryLayer {
    pub fn new(
        rows: usize,
        cols: usize,
        weights: Vec<i8>,
        scales_raw: Vec<u16>,
    ) -> Result<Self, ModelRefError> {
        if weights.len() != rows * cols {
            return Err(ModelRefError::ShapeMismatch {
                what: "ternary weights",
                expected: rows * cols,
                actual: weights.len(),
            });
        }
        if scales_raw.len() != rows {
            return Err(ModelRefError::ShapeMismatch {
                what: "row scales",
                expected: rows,
                actual: scales_raw.len(),
            });
        }
        if let Some(&bad) = weights.iter().find(|w| !matches!(**w, -1..=1)) {
            return Err(ModelRefError::NonTernaryWeight { value: bad });
        }
        Ok(Self {
            rows,
            cols,
            weights,
            scales_raw,
        })
    }

    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub fn cols(&self) -> usize {
        self.cols
    }

    #[must_use]
    pub fn row(&self, row: usize) -> &[i8] {
        &self.weights[row * self.cols..(row + 1) * self.cols]
    }

    #[must_use]
    pub fn scale_raw(&self, row: usize) -> u16 {
        self.scales_raw[row]
    }

    /// `-128 * sum(row)` — the u8 zero-point correction folded into the
    /// accumulator seed (pinned v0 contract).
    #[must_use]
    pub fn row_zero_point_bias(&self, row: usize) -> i16 {
        let sum: i32 = self.row(row).iter().map(|&w| i32::from(w)).sum();
        i16::try_from(-128 * sum).expect("|sum(row)| <= cols <= 128 so bias fits i16")
    }

    /// Fraction of zero weights in permille (reporting only).
    #[must_use]
    pub fn zero_permille(&self) -> u32 {
        let zeros = self.weights.iter().filter(|w| **w == 0).count();
        (zeros * 1000 / self.weights.len()) as u32
    }
}

/// Up/down pair for one pre-norm residual FFN block.
#[derive(Debug, Clone)]
pub struct BlockWeights {
    pub up: TernaryLayer,
    pub down: TernaryLayer,
}

/// The raw exported checkpoint: f32 embedding plus four ternary blocks.
#[derive(Debug, Clone)]
pub struct DenseBigramCheckpoint {
    embedding: Vec<f32>,
    blocks: Vec<BlockWeights>,
}

impl DenseBigramCheckpoint {
    pub fn new(embedding: Vec<f32>, blocks: Vec<BlockWeights>) -> Result<Self, ModelRefError> {
        if embedding.len() != VOCAB * D_MODEL {
            return Err(ModelRefError::ShapeMismatch {
                what: "embedding",
                expected: VOCAB * D_MODEL,
                actual: embedding.len(),
            });
        }
        if embedding.iter().any(|v| !v.is_finite()) {
            return Err(ModelRefError::NonFiniteEmbedding);
        }
        if blocks.len() != N_BLOCKS {
            return Err(ModelRefError::ShapeMismatch {
                what: "blocks",
                expected: N_BLOCKS,
                actual: blocks.len(),
            });
        }
        for block in &blocks {
            if block.up.rows() != D_FF || block.up.cols() != D_MODEL {
                return Err(ModelRefError::ShapeMismatch {
                    what: "up projection",
                    expected: D_FF * D_MODEL,
                    actual: block.up.rows() * block.up.cols(),
                });
            }
            if block.down.rows() != D_MODEL || block.down.cols() != D_FF {
                return Err(ModelRefError::ShapeMismatch {
                    what: "down projection",
                    expected: D_MODEL * D_FF,
                    actual: block.down.rows() * block.down.cols(),
                });
            }
        }
        Ok(Self { embedding, blocks })
    }

    #[must_use]
    pub fn embedding_row(&self, byte: u8) -> &[f32] {
        let start = usize::from(byte) * D_MODEL;
        &self.embedding[start..start + D_MODEL]
    }

    #[must_use]
    pub fn blocks(&self) -> &[BlockWeights] {
        &self.blocks
    }
}

// ---------------------------------------------------------------------------
// f32 reference (trainer port)
// ---------------------------------------------------------------------------

/// Burn's `SQRT_2_OVER_PI` constant, reproduced exactly.
const SQRT_2_OVER_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI * std::f64::consts::FRAC_1_SQRT_2;

/// Tanh-approximate GELU exactly as `burn_gelu_approximate` computes it on the
/// ndarray backend (f32 elementwise; the constant is f64 truncated to f32 by
/// the elementwise multiply).
#[must_use]
pub fn gelu_approx_f32(x: f32) -> f32 {
    let inner = x + x.powf(3.0) * 0.044715;
    let inner = inner * (SQRT_2_OVER_PI as f32);
    (x * (inner.tanh() + 1.0)) * 0.5
}

/// f64 twin of [`gelu_approx_f32`], used only to build the canonical GELU LUT.
#[must_use]
pub fn gelu_approx_f64(x: f64) -> f64 {
    let inner = x + x.powi(3) * 0.044715;
    let inner = inner * SQRT_2_OVER_PI;
    (x * (inner.tanh() + 1.0)) * 0.5
}

/// Full-vector RMS norm + clip, mirroring the trainer's `rms_norm`.
fn f32_rms_norm(x: &[f32; D_MODEL]) -> [f32; D_MODEL] {
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

/// Int8 signed activation fake-quant on the fixed range, mirroring
/// `fake_quant_signed` at `QuantHardness::Hard` (round ties to even, as the
/// Burn ndarray backend rounds).
#[must_use]
pub fn f32_act_fake_quant(x: f32) -> f32 {
    let max_abs = ACT_RANGE;
    let qmax = QMAX as f32;
    let clamped = x.clamp(-max_abs, max_abs);
    let quantized = ((clamped / max_abs) * qmax)
        .round_ties_even()
        .clamp(-qmax, qmax);
    ((quantized / qmax) * max_abs).clamp(-max_abs, max_abs)
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

/// The trainer's hard-ternary f32 forward pass for one context byte.
/// Returns the 256 tied-head logits.
#[must_use]
pub fn f32_forward(ck: &DenseBigramCheckpoint, prev: u8) -> [f32; VOCAB] {
    let mut x = [0.0f32; D_MODEL];
    x.copy_from_slice(ck.embedding_row(prev));

    let mut hidden = [0.0f32; D_FF];
    let mut delta = [0.0f32; D_MODEL];
    for block in ck.blocks() {
        let mut normed = f32_rms_norm(&x);
        for v in &mut normed {
            *v = f32_act_fake_quant(*v);
        }
        f32_ternary_matvec(&block.up, &normed, &mut hidden);
        for v in &mut hidden {
            *v = f32_act_fake_quant(gelu_approx_f32(*v));
        }
        f32_ternary_matvec(&block.down, &hidden, &mut delta);
        for (xv, dv) in x.iter_mut().zip(delta.iter()) {
            *xv += dv;
        }
    }

    let normed = f32_rms_norm(&x);
    let mut logits = [0.0f32; VOCAB];
    for (byte, logit) in logits.iter_mut().enumerate() {
        let row = ck.embedding_row(byte as u8);
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

/// Round-half-even f64 -> integer used for lowering-time table construction.
pub(crate) fn rte_i64(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// Floor integer square root of a u32 (result fits u16).
#[must_use]
pub fn isqrt_u32(n: u32) -> u16 {
    let mut rem = u64::from(n);
    let mut root: u64 = 0;
    let mut bit: u64 = 1 << 30;
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    u16::try_from(root).expect("isqrt of u32 fits u16")
}

/// GELU LUT on the activation grid: index `p + 127` for `p in [-127, 127]`;
/// entries are the activation-quantized GELU output stored as `u8` at zero
/// point 128. Shared between the dense and stateful lowerings.
#[must_use]
pub(crate) fn build_gelu_lut() -> [u8; 255] {
    let mut gelu_lut = [0u8; 255];
    for (idx, entry) in gelu_lut.iter_mut().enumerate() {
        let p = idx as i64 - i64::from(QMAX);
        let x = p as f64 * f64::from(ACT_RANGE) / f64::from(QMAX);
        let g = gelu_approx_f64(x).clamp(-f64::from(ACT_RANGE), f64::from(ACT_RANGE));
        let q = rte_i64(g * f64::from(QMAX) / f64::from(ACT_RANGE))
            .clamp(-i64::from(QMAX), i64::from(QMAX));
        *entry = (q + 128) as u8;
    }
    gelu_lut
}

/// One lowered ternary layer plus its accumulator seeds.
#[derive(Debug, Clone)]
pub struct LoweredLayer {
    pub layer: TernaryLayer,
    /// Per-row `-128 * sum(row)` accumulator seeds.
    pub biases: Vec<i16>,
}

impl LoweredLayer {
    pub(crate) fn new(layer: &TernaryLayer) -> Self {
        let biases = (0..layer.rows())
            .map(|row| layer.row_zero_point_bias(row))
            .collect();
        Self {
            layer: layer.clone(),
            biases,
        }
    }
}

/// The integer-lowered model: every table the canonical integer function (and
/// therefore the ROM) needs.
#[derive(Debug, Clone)]
pub struct IntLoweredModel {
    /// Embedding rows on the Q11.5 residual grid (`[VOCAB * D_MODEL]`,
    /// row-major).
    pub emb_resid: Vec<i16>,
    /// Tied-head weights as per-tensor symmetric i8 (`[VOCAB * D_MODEL]`).
    pub head_i8: Vec<i8>,
    /// `max|emb| / 127`: real value of one head-weight step.
    pub head_step: f32,
    /// GELU LUT: index `p + 127` for `p in [-127, 127]`; entries are the
    /// activation-quantized GELU output stored as `u8` at zero point 128.
    pub gelu_lut: [u8; 255],
    /// Lowered blocks (up, down).
    pub blocks: Vec<(LoweredLayer, LoweredLayer)>,
}

/// Numeric-contract violation observed while lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRefError {
    ShapeMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    NonTernaryWeight {
        value: i8,
    },
    NonFiniteEmbedding,
}

impl fmt::Display for ModelRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeMismatch {
                what,
                expected,
                actual,
            } => write!(f, "{what}: expected {expected} elements, got {actual}"),
            Self::NonTernaryWeight { value } => {
                write!(f, "ternary weight outside {{-1,0,1}}: {value}")
            }
            Self::NonFiniteEmbedding => write!(f, "embedding contains non-finite values"),
        }
    }
}

impl std::error::Error for ModelRefError {}

impl IntLoweredModel {
    pub fn lower(ck: &DenseBigramCheckpoint) -> Result<Self, ModelRefError> {
        // Embedding on the Q11.5 residual grid (round half to even in f64;
        // max |emb| * 32 ~ 186, far inside i16).
        let mut emb_resid = Vec::with_capacity(VOCAB * D_MODEL);
        let mut max_abs = 0.0f32;
        for byte in 0..VOCAB {
            for &v in ck.embedding_row(byte as u8) {
                max_abs = max_abs.max(v.abs());
                let q = rte_i64(f64::from(v) * f64::from(RESID_ONE)).clamp(-32768, 32767);
                emb_resid.push(q as i16);
            }
        }

        // Head i8 (per-tensor symmetric).
        let head_step = max_abs / QMAX as f32;
        let mut head_i8 = Vec::with_capacity(VOCAB * D_MODEL);
        for byte in 0..VOCAB {
            for &v in ck.embedding_row(byte as u8) {
                let q = rte_i64(f64::from(v) * f64::from(QMAX) / f64::from(max_abs))
                    .clamp(-i64::from(QMAX), i64::from(QMAX));
                head_i8.push(q as i8);
            }
        }

        // GELU LUT on the activation grid.
        let gelu_lut = build_gelu_lut();

        // Down-epilogue u32 bound: |m| * 2 + 127 < 2^32 with
        // |m| <= 65535 * 16256 (structural scale/acc bounds), i.e.
        // 2.13e9 + 127 < 4.29e9 — satisfied for every representable u16
        // scale, so no per-row runtime check is needed at Q11.5. (The
        // synthetic max-scale regression test exercises this bound.)
        let mut blocks = Vec::with_capacity(N_BLOCKS);
        for block in ck.blocks() {
            blocks.push((LoweredLayer::new(&block.up), LoweredLayer::new(&block.down)));
        }

        Ok(Self {
            emb_resid,
            head_i8,
            head_step,
            gelu_lut,
            blocks,
        })
    }

    #[must_use]
    pub fn emb_resid_row(&self, byte: u8) -> &[i16] {
        let start = usize::from(byte) * D_MODEL;
        &self.emb_resid[start..start + D_MODEL]
    }

    #[must_use]
    pub fn head_i8_row(&self, byte: u8) -> &[i8] {
        let start = usize::from(byte) * D_MODEL;
        &self.head_i8[start..start + D_MODEL]
    }

    /// Real value represented by one integer logit unit:
    /// `(ACT_RANGE / QMAX) * head_step`.
    #[must_use]
    pub fn logit_dequant_step(&self) -> f64 {
        f64::from(ACT_RANGE) / f64::from(QMAX) * f64::from(self.head_step)
    }

    /// The canonical integer forward pass for one context byte.
    #[must_use]
    pub fn forward(&self, prev: u8) -> IntForwardTrace {
        let mut stats = IntForwardStats::new();

        let mut x = [0i16; D_MODEL];
        x.copy_from_slice(self.emb_resid_row(prev));

        let mut block_residuals = [[0i16; D_MODEL]; N_BLOCKS];
        let mut block0_norm_act = [0u8; D_MODEL];
        let mut block0_up_acc = [0i16; D_FF];
        let mut block0_gelu_act = [0u8; D_FF];
        let mut block0_down_acc = [0i16; D_MODEL];
        let mut act = [0u8; D_FF];
        let mut acc = [0i16; D_FF];
        for (block_idx, (up, down)) in self.blocks.iter().enumerate() {
            // norm + activation quant -> u8 zp128 in act[..D_MODEL]
            let q = int_norm_quant(&x, &mut stats);
            for (a, qv) in act.iter_mut().zip(q.iter()) {
                *a = (qv + 128) as u8;
            }
            if block_idx == 0 {
                block0_norm_act.copy_from_slice(&act[..D_MODEL]);
            }

            // up matvec
            int_matvec(
                &up.layer,
                &up.biases,
                &act[..D_MODEL],
                &mut acc[..D_FF],
                &mut stats,
            );
            if block_idx == 0 {
                block0_up_acc.copy_from_slice(&acc[..D_FF]);
            }

            // up epilogue: scale multiply, requantize to grid, GELU LUT
            for row in 0..D_FF {
                let p = int_scale_to_grid(acc[row], up.layer.scale_raw(row), &mut stats);
                act[row] = self.gelu_lut[(p + QMAX) as usize];
            }
            if block_idx == 0 {
                block0_gelu_act.copy_from_slice(&act[..D_FF]);
            }

            // down matvec
            int_matvec(
                &down.layer,
                &down.biases,
                &act[..D_FF],
                &mut acc[..D_MODEL],
                &mut stats,
            );
            if block_idx == 0 {
                block0_down_acc.copy_from_slice(&acc[..D_MODEL]);
            }

            // down epilogue: scale multiply, Q8.8 requantize, wrapping residual add
            for row in 0..D_MODEL {
                let d_raw = int_down_delta(acc[row], down.layer.scale_raw(row), &mut stats);
                let wide = i32::from(x[row]) + d_raw;
                if wide != i32::from(wide as i16) {
                    stats.residual_wrap_events += 1;
                }
                x[row] = wide as i16;
                stats.max_abs_residual =
                    stats.max_abs_residual.max(i32::from(x[row]).unsigned_abs());
            }
            block_residuals[block_idx] = x;
        }

        // final norm + head
        let final_q = int_norm_quant(&x, &mut stats);
        let mut logits = [0i32; VOCAB];
        for (byte, logit) in logits.iter_mut().enumerate() {
            let row = self.head_i8_row(byte as u8);
            let mut acc32: i32 = 0;
            for (qv, ev) in final_q.iter().zip(row.iter()) {
                acc32 += i32::from(*qv) * i32::from(*ev);
            }
            stats.max_abs_logit = stats.max_abs_logit.max(acc32.unsigned_abs());
            *logit = acc32;
        }
        let mut argmax = 0u8;
        let mut best = logits[0];
        for (byte, &v) in logits.iter().enumerate().skip(1) {
            if v > best {
                best = v;
                argmax = byte as u8;
            }
        }

        IntForwardTrace {
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

/// Norm+quant over the 64-lane fixed-point residual (scale-free: the
/// quotient `|x|*127 / (8*rms)` is invariant to the residual fixed-point
/// scale). Returns `q in [-127, 127]`.
///
/// Canonical steps (the ROM mirrors these exactly):
/// - `ss = sum(x_i^2)` (fits 2^36; 5-byte accumulator on device)
/// - `mean = ss >> 6` (Q16.16, fits u32)
/// - `r = isqrt_floor(mean + 1)` (Q8.8, fits u16)
/// - `q_i = sign(x_i) * min(127, (|x_i|*254 + 8r) div (16r))`
///   (round-half-away of `|x_i|*127 / (8r)`)
pub fn int_norm_quant(x: &[i16; D_MODEL], stats: &mut IntForwardStats) -> [i16; D_MODEL] {
    let mut ss: u64 = 0;
    for &v in x {
        let a = i64::from(v).unsigned_abs();
        ss += a * a;
    }
    stats.max_norm_sumsq = stats.max_norm_sumsq.max(ss);
    let mean = ss >> 6;
    debug_assert!(mean < u64::from(u32::MAX));
    let r = u32::from(isqrt_u32((mean + 1) as u32));
    stats.min_norm_rms_raw = stats.min_norm_rms_raw.min(r);
    let d = 8 * r;
    let d2 = 16 * r;
    let mut q = [0i16; D_MODEL];
    for (qv, &v) in q.iter_mut().zip(x.iter()) {
        let a = i64::from(v).unsigned_abs();
        let num = a * 254 + u64::from(d);
        let q_abs = (num / u64::from(d2)).min(i64::from(QMAX) as u64) as i16;
        *qv = if v < 0 { -q_abs } else { q_abs };
    }
    q
}

/// Ternary matvec with u8 zero-point-128 activations and i16 accumulators
/// seeded with the per-row zero-point bias. Asserts the (structural) i16
/// bound on the true accumulator value.
pub(crate) fn int_matvec(
    layer: &TernaryLayer,
    biases: &[i16],
    act: &[u8],
    out: &mut [i16],
    stats: &mut IntForwardStats,
) {
    debug_assert_eq!(act.len(), layer.cols());
    debug_assert_eq!(out.len(), layer.rows());
    for (row, out_v) in out.iter_mut().enumerate() {
        let mut acc: i32 = i32::from(biases[row]);
        for (w, u) in layer.row(row).iter().zip(act.iter()) {
            acc += i32::from(*w) * i32::from(*u);
        }
        assert!(
            (-32768..=32767).contains(&acc),
            "matvec accumulator {acc} escapes i16 (structurally impossible for fan-in <= 128)"
        );
        stats.max_abs_matvec_acc = stats.max_abs_matvec_acc.max(acc.unsigned_abs());
        *out_v = acc as i16;
    }
}

/// Up epilogue: requantize `scale_raw * acc` onto the activation grid.
/// `p = sign(m) * min(127, (|m| + 128) >> 8)` where `m = scale_raw * acc`.
pub(crate) fn int_scale_to_grid(acc: i16, scale_raw: u16, stats: &mut IntForwardStats) -> i32 {
    let m = i64::from(scale_raw) * i64::from(acc);
    stats.max_abs_scale_product = stats.max_abs_scale_product.max(m.unsigned_abs());
    debug_assert!(m.unsigned_abs() < 1 << 31, "u32 scale product bound");
    let p_abs = ((m.unsigned_abs() + 128) >> 8).min(i64::from(QMAX) as u64) as i32;
    if m < 0 { -p_abs } else { p_abs }
}

/// Down epilogue: Q11.5 residual delta
/// `sign(m) * min(65535, (|m|*2 + 127) div 254)` with `m = scale_raw * acc`
/// (round-half-away of `m * 8 * 32 / (127 * 256) = m / 127`).
pub(crate) fn int_down_delta(acc: i16, scale_raw: u16, stats: &mut IntForwardStats) -> i32 {
    let m = i64::from(scale_raw) * i64::from(acc);
    stats.max_abs_scale_product = stats.max_abs_scale_product.max(m.unsigned_abs());
    let num = m.unsigned_abs() * 2 + 127;
    debug_assert!(
        num < 1 << 32,
        "u32 down-epilogue bound (checked at lowering)"
    );
    let d_abs = num / 254;
    if d_abs > 65535 {
        stats.down_delta_clamp_events += 1;
    }
    let d_abs = d_abs.min(65535);
    stats.max_abs_down_delta = stats.max_abs_down_delta.max(d_abs);
    if m < 0 { -(d_abs as i32) } else { d_abs as i32 }
}

/// Full trace of one canonical integer forward pass, including the
/// checkpoint values the ROM gate compares byte-exactly.
#[derive(Debug, Clone)]
pub struct IntForwardTrace {
    /// Residual vector after each block (i16 Q11.5).
    pub block_residuals: [[i16; D_MODEL]; N_BLOCKS],
    /// Block-0 debug checkpoints (mirrored by the ROM's debug dumps).
    pub block0_norm_act: [u8; D_MODEL],
    pub block0_up_acc: [i16; D_FF],
    pub block0_gelu_act: [u8; D_FF],
    pub block0_down_acc: [i16; D_MODEL],
    /// Final norm output on the activation grid (`[-127, 127]`).
    pub final_q: [i16; D_MODEL],
    /// Tied-head integer logits (i24-range values held in i32).
    pub logits: [i32; VOCAB],
    /// Argmax byte (lowest index wins ties).
    pub argmax: u8,
    pub stats: IntForwardStats,
}

/// Range/overflow observations accumulated by the integer evaluator. The
/// fidelity phase reports the maxima over every evaluated position so the
/// i16/u32 claims are checked against real data, not just structure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntForwardStats {
    pub max_abs_matvec_acc: u32,
    pub max_abs_scale_product: u64,
    pub max_abs_down_delta: u64,
    pub max_abs_residual: u32,
    pub max_abs_logit: u32,
    pub max_norm_sumsq: u64,
    pub min_norm_rms_raw: u32,
    pub down_delta_clamp_events: u64,
    pub residual_wrap_events: u64,
}

impl IntForwardStats {
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_norm_rms_raw: u32::MAX,
            ..Self::default()
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.max_abs_matvec_acc = self.max_abs_matvec_acc.max(other.max_abs_matvec_acc);
        self.max_abs_scale_product = self.max_abs_scale_product.max(other.max_abs_scale_product);
        self.max_abs_down_delta = self.max_abs_down_delta.max(other.max_abs_down_delta);
        self.max_abs_residual = self.max_abs_residual.max(other.max_abs_residual);
        self.max_abs_logit = self.max_abs_logit.max(other.max_abs_logit);
        self.max_norm_sumsq = self.max_norm_sumsq.max(other.max_norm_sumsq);
        self.min_norm_rms_raw = self.min_norm_rms_raw.min(other.min_norm_rms_raw);
        self.down_delta_clamp_events += other.down_delta_clamp_events;
        self.residual_wrap_events += other.residual_wrap_events;
    }
}

impl Default for IntForwardTrace {
    fn default() -> Self {
        Self {
            block_residuals: [[0; D_MODEL]; N_BLOCKS],
            block0_norm_act: [0; D_MODEL],
            block0_up_acc: [0; D_FF],
            block0_gelu_act: [0; D_FF],
            block0_down_acc: [0; D_MODEL],
            final_q: [0; D_MODEL],
            logits: [0; VOCAB],
            argmax: 0,
            stats: IntForwardStats::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// deterministic synthetic checkpoint (tests / smoke)
// ---------------------------------------------------------------------------

/// Deterministic synthetic checkpoint for tests that must not depend on the
/// committed experiment files. Weight/scale/embedding distributions are picked
/// to exercise signs, zeros, and saturation paths.
#[must_use]
pub fn synthetic_checkpoint(seed: u64) -> DenseBigramCheckpoint {
    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };

    let embedding: Vec<f32> = (0..VOCAB * D_MODEL)
        .map(|_| {
            let unit = (next() >> 40) as f32 / (1u64 << 24) as f32;
            (unit * 2.0 - 1.0) * 3.0
        })
        .collect();

    let mut blocks = Vec::new();
    for _ in 0..N_BLOCKS {
        let mut layer = |rows: usize, cols: usize| {
            let weights: Vec<i8> = (0..rows * cols)
                .map(|_| match next() % 10 {
                    0 => 0i8,
                    n if n < 5 => 1,
                    _ => -1,
                })
                .collect();
            // Scale magnitudes shaped like the real S6 checkpoint (raw Q8.8
            // mostly 4..84, i.e. ~0.016..0.33): keeps the residual stream
            // bounded so the model is not chaotically sensitive to LSBs.
            let scales: Vec<u16> = (0..rows).map(|_| (next() % 80 + 4) as u16).collect();
            TernaryLayer::new(rows, cols, weights, scales).expect("synthetic layer is valid")
        };
        blocks.push(BlockWeights {
            up: layer(D_FF, D_MODEL),
            down: layer(D_MODEL, D_FF),
        });
    }
    DenseBigramCheckpoint::new(embedding, blocks).expect("synthetic checkpoint is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_matches_f64_sqrt_floor_over_samples() {
        let samples = [
            0u32,
            1,
            2,
            3,
            4,
            15,
            16,
            17,
            255,
            65535,
            65536,
            1 << 20,
            u32::MAX,
            1_073_741_824,
        ];
        for &n in &samples {
            let expected = (f64::from(n)).sqrt().floor() as u32;
            assert_eq!(u32::from(isqrt_u32(n)), expected, "isqrt({n})");
        }
        // exhaustive near squares
        for k in 0u32..2000 {
            let sq = k * k;
            assert_eq!(u32::from(isqrt_u32(sq)), k);
            if sq > 0 {
                assert_eq!(u32::from(isqrt_u32(sq - 1)), k - 1);
            }
        }
    }

    #[test]
    fn act_fake_quant_matches_grid() {
        // exact grid values survive
        for p in -127i32..=127 {
            let v = p as f32 * ACT_RANGE / QMAX as f32;
            let fq = f32_act_fake_quant(v);
            assert!((fq - v).abs() < 1e-6, "{p}: {fq} vs {v}");
        }
        assert_eq!(f32_act_fake_quant(100.0), ACT_RANGE);
        assert_eq!(f32_act_fake_quant(-100.0), -ACT_RANGE);
    }

    #[test]
    fn int_forward_runs_on_synthetic_checkpoint_and_is_deterministic() {
        let ck = synthetic_checkpoint(7);
        let lowered = IntLoweredModel::lower(&ck).expect("lowers");
        let a = lowered.forward(0x41);
        let b = lowered.forward(0x41);
        assert_eq!(a.logits, b.logits);
        assert_eq!(a.argmax, b.argmax);
        // logits stay inside the i24 device representation
        for &l in &a.logits {
            assert!(
                (-(1 << 23)..(1 << 23)).contains(&l),
                "logit {l} escapes i24"
            );
        }
    }

    #[test]
    fn int_and_f32_forward_argmax_mostly_agree_on_synthetic_model() {
        // Not a strict gate (fidelity is measured on the real checkpoint);
        // this catches gross semantic porting errors.
        let ck = synthetic_checkpoint(3);
        let lowered = IntLoweredModel::lower(&ck).expect("lowers");
        let mut agree = 0usize;
        for byte in 0..=255u8 {
            let int_trace = lowered.forward(byte);
            let f32_logits = f32_forward(&ck, byte);
            let f32_argmax = f32_logits
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).expect("finite"))
                .map(|(i, _)| i as u8)
                .expect("nonempty");
            if int_trace.argmax == f32_argmax {
                agree += 1;
            }
        }
        assert!(
            agree >= 160,
            "int/f32 argmax agreement suspiciously low on synthetic model: {agree}/256"
        );
    }

    #[test]
    fn gelu_lut_endpoints_and_shape() {
        let ck = synthetic_checkpoint(1);
        let lowered = IntLoweredModel::lower(&ck).expect("lowers");
        // gelu(-8) ~ 0, gelu(8) ~ 8 -> q 127
        assert_eq!(lowered.gelu_lut[0], 128);
        assert_eq!(lowered.gelu_lut[254], 255);
        // p = 0 -> gelu(0) = 0
        assert_eq!(lowered.gelu_lut[127], 128);
        // minimum of gelu is about -0.17 -> q -3 -> 125
        let min = *lowered.gelu_lut.iter().min().expect("nonempty");
        assert_eq!(min, 125);
    }

    #[test]
    fn zero_point_bias_matches_negative_row_sum() {
        let layer = TernaryLayer::new(2, 4, vec![1, -1, 0, 1, -1, -1, -1, 0], vec![256, 256])
            .expect("valid");
        assert_eq!(layer.row_zero_point_bias(0), -128);
        assert_eq!(layer.row_zero_point_bias(1), 3 * 128);
    }

    #[test]
    fn down_epilogue_stays_in_u32_even_at_the_maximum_scale() {
        // Structural bound: |m|*2 + 127 with |m| <= 65535 * 16256 fits u32,
        // so lowering must accept even a maximal Q8.8 scale and the forward
        // pass must not panic on its debug bound assertions.
        let mut ck = synthetic_checkpoint(2);
        let down = &ck.blocks[0].down;
        let bad = TernaryLayer::new(
            down.rows(),
            down.cols(),
            down.weights.clone(),
            std::iter::once(65535u16)
                .chain(down.scales_raw.iter().copied().skip(1))
                .collect(),
        )
        .expect("valid layer");
        ck.blocks[0].down = bad;
        let lowered = IntLoweredModel::lower(&ck).expect("maximal scale lowers");
        let trace = lowered.forward(0x41);
        assert!(trace.stats.max_abs_scale_product * 2 + 127 < 1 << 32);
    }
}
