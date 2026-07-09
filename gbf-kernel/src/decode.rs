//! Integer top-k / temperature sampling decode (host reference semantics).
//!
//! This module is the exp-LUT + integer sampling design that the planv0
//! 2026-07-04 amendment section 3 decode pin requires before
//! `DecodeMode::TopKTemperature` may enter a `DecodeCapabilitySet`
//! (F-G3, bd-a4du): greedy argmax collapses the stateful checkpoint into
//! repetition loops, and this sampler is the integer-only, LR35902-feasible
//! replacement with the same oracle/ROM agreement obligations as the
//! forward pass. The ROM implementation lives in
//! [`crate::asm_impl_state::build_state_multi_token_sampling_rom`] and must
//! reproduce [`sample_topk`] **byte-exactly** given the same RNG seed.
//!
//! # Pinned integer sampling semantics (`sampling_decode.v1`)
//!
//! Inputs: a slice of integer logits (i24 range values held in `i32`; the
//! stateful head produces 80 of them), a [`SamplerConfig`] `(k,
//! scale_q16)`, and a [`XorShift16`] RNG state. One token is sampled per
//! call, advancing the RNG exactly once.
//!
//! 1. **Top-k selection by k partial scans.** Pass 0 is the plain argmax
//!    (strictly-greater update scanning ascending ids, so the lowest index
//!    wins ties — identical to the deployed argmax rule). Each later pass
//!    re-scans, skipping already-selected ids, with the same
//!    first-unused-then-strictly-greater rule. Candidates are therefore
//!    ordered by descending logit, ties broken toward lower ids.
//! 2. **Exp LUT domain.** For each candidate, `d = logit_max - logit`
//!    (a non-negative integer difference in raw logit units; the i24 logit
//!    bound keeps `d < 2^24`). The LUT index is
//!    `u = min(255, (d * scale_q16 + 0x8000) >> 16)` — a Q16 fixed-point
//!    multiply with round-half-up, exactly the rounding shape of the
//!    deployed state-out epilogue. The weight is `w = EXP2_LUT[u]`, where
//!    `EXP2_LUT[u] = round_ties_even(255 * 2^(-u / 16))` (u8 entries;
//!    [`EXP2_LUT_ALPHA`] = 16 index units per halving; entries are 0 for
//!    `u >= 144`, so the usable dynamic range is ~9 halvings = ~6.2 nats
//!    below the max logit). `u = 0` always maps to 255, so the max logit
//!    always carries nonzero weight and the total is never zero.
//! 3. **Temperature.** Temperature is folded into the single build-time
//!    constant `scale_q16 = round(65536 * 16 * logit_step / (T * ln 2))`
//!    where `logit_step` is the real value of one integer logit unit
//!    (`IntStateLoweredModel::logit_dequant_step`). This makes the LUT
//!    weights approximate `exp((logit - max) * logit_step / T)`, i.e. real
//!    softmax-with-temperature restricted to the top k. A generic Q0.16
//!    multiplier was chosen over a pow2 shift so temperature is
//!    continuously tunable; the multiply is one `mul16` + one `mul16x8` on
//!    device (the same cost shape as every existing epilogue).
//! 4. **Draw.** `r = rng.next()` (uniform over 1..=65535, one draw per
//!    token), `total = sum(w)` (`<= 8 * 255 = 2040`, fits u16), and
//!    `threshold = (r * total) >> 16` (truncating; a value in
//!    `0..total`). The scaled-multiply draw replaces `r % total` so no
//!    16-bit division is needed on device; the resulting per-candidate
//!    probability deviates from `w/total` by at most `total/65536`
//!    (documented, deterministic).
//! 5. **Cumulative walk.** Scan candidates in selection order accumulating
//!    `cum += w`; pick the first candidate with `cum > threshold`
//!    (strictly greater). Because `threshold < total`, the walk always
//!    terminates, and zero-weight candidates can never be picked.
//!
//! With `k = 1` the sampler reduces exactly to argmax for every RNG value
//! (`total = 255`, `threshold <= 254 < 255`).
//!
//! # RNG: `RngSpec::XorShift16`
//!
//! planv0 pins `RngSpec::XorShift16`. The pinned constants are the shift
//! triple **(7, 9, 8)**:
//!
//! ```text
//! x ^= x << 7;  x ^= x >> 9;  x ^= x << 8;   (16-bit, wrapping)
//! ```
//!
//! This is John Metcalf's classic Z80 16-bit xorshift; it has full period
//! 65535 over the nonzero u16 states (verified by a unit test here) and
//! never yields 0 from a nonzero state. Seed 0 is canonicalized to 1 by
//! [`XorShift16::new`]; the ROM applies the identical rule to the
//! host-poked seed bytes. Each token consumes exactly one RNG step and the
//! value used for the draw is the state *after* stepping.
//!
//! Every rounding above is pinned: round-half-even for the (build-time
//! f64) LUT and `scale_q16` construction, round-half-up for the `u` index
//! multiply, truncation for the threshold multiply. The runtime path is
//! pure integer arithmetic.

use std::fmt;

/// LUT index units per weight halving: `EXP2_LUT[u] ~ 255 * 2^(-u/16)`.
pub const EXP2_LUT_ALPHA: u32 = 16;

/// Maximum top-k the ROM sampler supports (candidate id/weight tables are
/// 8 bytes each in WRAM; `8 * 255` also keeps the weight total far inside
/// u16).
pub const MAX_TOP_K: u8 = 8;

/// The pinned XorShift16 shift triple `(left, right, left)`.
pub const XORSHIFT16_SHIFTS: (u32, u32, u32) = (7, 9, 8);

/// Build the pinned exp2 LUT: `lut[u] = round_ties_even(255 * 2^(-u/16))`.
#[must_use]
pub fn build_exp2_lut() -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (u, entry) in lut.iter_mut().enumerate() {
        let v = 255.0f64 * (-(u as f64) / f64::from(EXP2_LUT_ALPHA)).exp2();
        let r = crate::model_ref::rte_i64(v);
        *entry = u8::try_from(r).expect("255 * 2^(-u/16) rounds into 0..=255");
    }
    lut
}

/// planv0 `RngSpec::XorShift16` with the pinned (7, 9, 8) shift triple.
/// Full period 65535 over nonzero states; never yields 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XorShift16 {
    state: u16,
}

impl XorShift16 {
    /// Seed the RNG. Seed 0 (the only degenerate state) is canonicalized
    /// to 1; the ROM applies the identical rule to the poked seed bytes.
    #[must_use]
    pub fn new(seed: u16) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Advance one step and return the new state (uniform over 1..=65535).
    pub fn next_u16(&mut self) -> u16 {
        let mut x = self.state;
        x ^= x << XORSHIFT16_SHIFTS.0;
        x ^= x >> XORSHIFT16_SHIFTS.1;
        x ^= x << XORSHIFT16_SHIFTS.2;
        self.state = x;
        x
    }

    /// Current state (what the ROM keeps in WRAM between tokens).
    #[must_use]
    pub fn state(&self) -> u16 {
        self.state
    }
}

/// Sampler configuration failure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplerConfigError {
    /// `k` outside `1..=MAX_TOP_K`.
    BadK { k: u8 },
    /// `scale_q16` must be nonzero (zero would flatten every distribution
    /// to uniform-over-top-k regardless of logits).
    ZeroScale,
    /// The requested temperature maps outside the representable u16 Q0.16
    /// scale (temperature too small / step too large).
    ScaleOverflow { scale: f64 },
    /// Temperature/step must be finite and positive.
    BadTemperature,
}

impl fmt::Display for SamplerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadK { k } => write!(f, "top-k {k} outside 1..={MAX_TOP_K}"),
            Self::ZeroScale => write!(f, "scale_q16 must be nonzero"),
            Self::ScaleOverflow { scale } => {
                write!(
                    f,
                    "scale_q16 {scale} does not fit u16 (temperature too small)"
                )
            }
            Self::BadTemperature => write!(f, "temperature and logit step must be finite and > 0"),
        }
    }
}

impl std::error::Error for SamplerConfigError {}

/// Pinned integer sampler parameters: top-k and the Q0.16 LUT-index scale
/// (temperature folded in at build time; see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerConfig {
    k: u8,
    scale_q16: u16,
}

impl SamplerConfig {
    pub fn new(k: u8, scale_q16: u16) -> Result<Self, SamplerConfigError> {
        if k == 0 || k > MAX_TOP_K {
            return Err(SamplerConfigError::BadK { k });
        }
        if scale_q16 == 0 {
            return Err(SamplerConfigError::ZeroScale);
        }
        Ok(Self { k, scale_q16 })
    }

    /// Derive the integer scale from a real temperature:
    /// `scale_q16 = round_ties_even(65536 * ALPHA * logit_step / (T ln 2))`.
    /// `logit_step` is the real value of one integer logit unit
    /// (`IntStateLoweredModel::logit_dequant_step`).
    pub fn from_temperature(
        k: u8,
        logit_step: f64,
        temperature: f64,
    ) -> Result<Self, SamplerConfigError> {
        if !(temperature.is_finite() && temperature > 0.0 && logit_step.is_finite())
            || logit_step <= 0.0
        {
            return Err(SamplerConfigError::BadTemperature);
        }
        let scale = 65536.0 * f64::from(EXP2_LUT_ALPHA) * logit_step
            / (temperature * std::f64::consts::LN_2);
        let raw = crate::model_ref::rte_i64(scale);
        let scale_q16 =
            u16::try_from(raw).map_err(|_| SamplerConfigError::ScaleOverflow { scale })?;
        if scale_q16 == 0 {
            return Err(SamplerConfigError::ZeroScale);
        }
        Self::new(k, scale_q16)
    }

    #[must_use]
    pub fn k(&self) -> u8 {
        self.k
    }

    #[must_use]
    pub fn scale_q16(&self) -> u16 {
        self.scale_q16
    }

    /// The real temperature this config realizes for a given logit step
    /// (inverse of [`Self::from_temperature`], for reporting).
    #[must_use]
    pub fn effective_temperature(&self, logit_step: f64) -> f64 {
        65536.0 * f64::from(EXP2_LUT_ALPHA) * logit_step
            / (f64::from(self.scale_q16) * std::f64::consts::LN_2)
    }
}

/// One selected top-k candidate (in selection order: descending logit,
/// ties toward lower ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleCandidate {
    pub id: usize,
    pub logit: i32,
    /// LUT index `u` (0..=255).
    pub lut_index: u8,
    /// LUT weight (0..=255).
    pub weight: u8,
}

/// Full trace of one sampling step: everything the ROM computes, for tests
/// and gate debugging.
#[derive(Debug, Clone)]
pub struct SampleTrace {
    pub candidates: Vec<SampleCandidate>,
    /// `sum(weight)`, `255..=2040`.
    pub total: u16,
    /// The RNG value consumed by this draw (state after stepping).
    pub r: u16,
    /// `(r * total) >> 16`, in `0..total`.
    pub threshold: u16,
    /// The sampled id.
    pub picked: usize,
}

/// Sample one token id from integer logits with the pinned semantics.
/// Advances `rng` exactly once. Works over any logits slice with
/// `1 <= len` and every `|logit| < 2^23` (the i24 device bound; checked by
/// `debug_assert`). If the slice is shorter than `k`, all entries become
/// candidates.
#[must_use]
pub fn sample_topk(logits: &[i32], cfg: &SamplerConfig, rng: &mut XorShift16) -> usize {
    sample_topk_trace(logits, cfg, rng).picked
}

/// [`sample_topk`] returning the full integer trace.
#[must_use]
pub fn sample_topk_trace(logits: &[i32], cfg: &SamplerConfig, rng: &mut XorShift16) -> SampleTrace {
    assert!(!logits.is_empty(), "sampling needs at least one logit");
    debug_assert!(
        logits.iter().all(|&l| (-(1 << 23)..(1 << 23)).contains(&l)),
        "logits must be in the i24 device range"
    );
    let lut = build_exp2_lut();
    let k = usize::from(cfg.k).min(logits.len());
    let mut used = vec![false; logits.len()];
    let mut candidates = Vec::with_capacity(k);
    let mut total: u16 = 0;
    let mut logit_max: i64 = 0;
    for pass in 0..k {
        let mut best: Option<usize> = None;
        for (id, &l) in logits.iter().enumerate() {
            if used[id] {
                continue;
            }
            match best {
                None => best = Some(id),
                Some(b) if l > logits[b] => best = Some(id),
                Some(_) => {}
            }
        }
        let id = best.expect("k <= len leaves an unused id every pass");
        used[id] = true;
        if pass == 0 {
            logit_max = i64::from(logits[id]);
        }
        let d = u64::try_from(logit_max - i64::from(logits[id]))
            .expect("candidates are scanned in descending order");
        let u = ((d * u64::from(cfg.scale_q16) + 0x8000) >> 16).min(255) as usize;
        let weight = lut[u];
        total += u16::from(weight);
        candidates.push(SampleCandidate {
            id,
            logit: logits[id],
            lut_index: u as u8,
            weight,
        });
    }
    debug_assert!(total >= 255, "the max logit always contributes LUT[0]");

    let r = rng.next_u16();
    let threshold = ((u32::from(r) * u32::from(total)) >> 16) as u16;
    let mut cum: u16 = 0;
    let mut picked = candidates[0].id;
    for cand in &candidates {
        cum += u16::from(cand.weight);
        if cum > threshold {
            picked = cand.id;
            break;
        }
    }
    SampleTrace {
        candidates,
        total,
        r,
        threshold,
        picked,
    }
}

/// Sample one token id from an ALREADY-SELECTED candidate set (the paged
/// head's finalized running top-k heap), in selection order (logit descending,
/// id ascending on ties). Byte-identical to [`sample_topk`] run over the full
/// logit vector for the same `scale_q16`, `k`, and RNG seed: the two share the
/// exp-LUT weight, `total`, draw, and cumulative-walk arithmetic; only the
/// candidate SELECTION differs in provenance (paged heap vs full scan), and the
/// heap's selected set + order is proven equal to the scan's (see
/// [`RunningTopK`] and the parity tests).
///
/// `candidates` must be in selection order with `candidates[0]` the argmax
/// (its logit is `logit_max`, so `d = 0 -> u = 0 -> w = EXP2_LUT[0] = 255`).
/// Advances `rng` exactly once, exactly as [`sample_topk`] does.
#[must_use]
pub fn sample_topk_from_candidates(
    candidates: &[(i32, usize)],
    scale_q16: u16,
    rng: &mut XorShift16,
) -> usize {
    sample_topk_from_candidates_trace(candidates, scale_q16, rng).picked
}

/// [`sample_topk_from_candidates`] returning the full integer trace, so gates
/// can compare the device heap/weights/threshold against the golden.
#[must_use]
pub fn sample_topk_from_candidates_trace(
    candidates: &[(i32, usize)],
    scale_q16: u16,
    rng: &mut XorShift16,
) -> SampleTrace {
    assert!(
        !candidates.is_empty(),
        "sampling needs at least one candidate"
    );
    debug_assert!(
        candidates
            .iter()
            .all(|&(l, _)| (-(1 << 23)..(1 << 23)).contains(&l)),
        "logits must be in the i24 device range"
    );
    let lut = build_exp2_lut();
    let logit_max = i64::from(candidates[0].0);
    let mut selected = Vec::with_capacity(candidates.len());
    let mut total: u16 = 0;
    for &(logit, id) in candidates {
        let d = u64::try_from(logit_max - i64::from(logit))
            .expect("candidates are in descending order (candidates[0] is the max)");
        let u = ((d * u64::from(scale_q16) + 0x8000) >> 16).min(255) as usize;
        let weight = lut[u];
        total += u16::from(weight);
        selected.push(SampleCandidate {
            id,
            logit,
            lut_index: u as u8,
            weight,
        });
    }
    debug_assert!(total >= 255, "the max logit always contributes LUT[0]");

    let r = rng.next_u16();
    let threshold = ((u32::from(r) * u32::from(total)) >> 16) as u16;
    let mut cum: u16 = 0;
    let mut picked = selected[0].id;
    for cand in &selected {
        cum += u16::from(cand.weight);
        if cum > threshold {
            picked = cand.id;
            break;
        }
    }
    SampleTrace {
        candidates: selected,
        total,
        r,
        threshold,
        picked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp2_lut_pinned_shape() {
        let lut = build_exp2_lut();
        assert_eq!(lut[0], 255);
        assert_eq!(lut[16], 128, "one halving: 127.5 rounds ties-to-even");
        assert_eq!(lut[32], 64);
        assert_eq!(lut[48], 32);
        // Monotone non-increasing, zero tail from u = 144.
        for u in 1..256 {
            assert!(lut[u] <= lut[u - 1], "LUT must be non-increasing at {u}");
        }
        assert!(lut[143] > 0);
        assert_eq!(lut[144], 0);
        assert!(lut[255] == 0);
    }

    #[test]
    fn xorshift16_full_period_and_never_zero() {
        let mut rng = XorShift16::new(1);
        let mut n = 0u32;
        loop {
            let v = rng.next_u16();
            assert_ne!(v, 0, "xorshift16 must never yield 0");
            n += 1;
            if v == 1 {
                break;
            }
            assert!(n <= 65535, "period exceeded 65535");
        }
        assert_eq!(n, 65535, "(7,9,8) is full-period over nonzero u16");
    }

    #[test]
    fn xorshift16_zero_seed_canonicalizes_to_one() {
        assert_eq!(XorShift16::new(0), XorShift16::new(1));
        assert_eq!(XorShift16::new(0).state(), 1);
    }

    #[test]
    fn xorshift16_pinned_first_values() {
        // Golden values so any constant drift is loud (seed 1).
        let mut rng = XorShift16::new(1);
        let got: Vec<u16> = (0..4).map(|_| rng.next_u16()).collect();
        let mut x: u16 = 1;
        let expect: Vec<u16> = (0..4)
            .map(|_| {
                x ^= x << 7;
                x ^= x >> 9;
                x ^= x << 8;
                x
            })
            .collect();
        assert_eq!(got, expect);
        assert_eq!(got[0], {
            let mut x: u16 = 1;
            x ^= x << 7;
            x ^= x >> 9;
            x ^= x << 8;
            x
        });
    }

    #[test]
    fn config_validates_k_and_scale() {
        assert!(SamplerConfig::new(0, 100).is_err());
        assert!(SamplerConfig::new(9, 100).is_err());
        assert!(SamplerConfig::new(8, 0).is_err());
        let cfg = SamplerConfig::new(8, 2253).expect("valid");
        assert_eq!(cfg.k(), 8);
        assert_eq!(cfg.scale_q16(), 2253);
    }

    #[test]
    fn from_temperature_round_trips_and_rejects_degenerate() {
        let step = 1.5e-3;
        let cfg = SamplerConfig::from_temperature(8, step, 1.0).expect("valid");
        let t = cfg.effective_temperature(step);
        assert!((t - 1.0).abs() < 1e-3, "effective T {t} drifted");
        assert!(SamplerConfig::from_temperature(8, step, 0.0).is_err());
        assert!(SamplerConfig::from_temperature(8, step, -1.0).is_err());
        assert!(SamplerConfig::from_temperature(8, 0.0, 1.0).is_err());
        // Temperature so small the scale overflows u16.
        assert!(SamplerConfig::from_temperature(8, step, 1e-9).is_err());
    }

    #[test]
    fn k1_sampling_is_argmax_for_every_rng_value() {
        let logits = [-5, 900, 900, -20000, 344];
        let cfg = SamplerConfig::new(1, 3000).expect("valid");
        let mut rng = XorShift16::new(0x1234);
        for _ in 0..1000 {
            assert_eq!(sample_topk(&logits, &cfg, &mut rng), 1);
        }
    }

    #[test]
    fn selection_order_is_descending_with_low_index_ties() {
        let logits = [10, 500, 500, -3, 499];
        let cfg = SamplerConfig::new(4, 100).expect("valid");
        let mut rng = XorShift16::new(7);
        let trace = sample_topk_trace(&logits, &cfg, &mut rng);
        let ids: Vec<usize> = trace.candidates.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2, 4, 0]);
        assert_eq!(trace.candidates[0].lut_index, 0);
        assert_eq!(trace.candidates[0].weight, 255);
    }

    #[test]
    fn deterministic_given_seed_and_state_advances_once_per_call() {
        let logits = [100, 90, 80, 70, 60];
        let cfg = SamplerConfig::new(4, 2000).expect("valid");
        let mut a = XorShift16::new(0xBEEF);
        let mut b = XorShift16::new(0xBEEF);
        let seq_a: Vec<usize> = (0..64)
            .map(|_| sample_topk(&logits, &cfg, &mut a))
            .collect();
        let seq_b: Vec<usize> = (0..64)
            .map(|_| sample_topk(&logits, &cfg, &mut b))
            .collect();
        assert_eq!(seq_a, seq_b);
        // Exactly one RNG step per sample.
        let mut c = XorShift16::new(0xBEEF);
        for _ in 0..64 {
            c.next_u16();
        }
        assert_eq!(a.state(), c.state());
    }

    #[test]
    fn zero_weight_candidates_are_never_picked() {
        // Candidate 1 is ~2^20 below the max: with scale 65535 its LUT
        // index saturates at 255 -> weight 0.
        let logits = [1_000_000, -1_000_000];
        let cfg = SamplerConfig::new(2, 65535).expect("valid");
        let mut rng = XorShift16::new(42);
        for _ in 0..4096 {
            assert_eq!(sample_topk(&logits, &cfg, &mut rng), 0);
        }
    }

    /// Distribution sanity: over the full RNG period every u16 value
    /// appears exactly once, so empirical counts must match the pinned
    /// threshold arithmetic to within the documented `total/65536`
    /// deviation from the ideal `w/total`.
    #[test]
    fn empirical_frequencies_match_lut_weights_over_full_period() {
        let logits = [8000, 7000, 6000, 2000];
        let cfg = SamplerConfig::new(4, 1200).expect("valid");
        let mut rng = XorShift16::new(1);
        let mut counts = [0u32; 4];
        for _ in 0..65535 {
            counts[sample_topk(&logits, &cfg, &mut rng)] += 1;
        }
        assert_eq!(counts.iter().sum::<u32>(), 65535);
        let mut probe = XorShift16::new(1);
        let trace = sample_topk_trace(&logits, &cfg, &mut probe);
        let total = f64::from(trace.total);
        for (cand, &count) in trace.candidates.iter().zip(counts.iter()) {
            let expected = f64::from(cand.weight) / total * 65535.0;
            let tolerance = total / 65536.0 * 65535.0 + 1.0;
            assert!(
                (f64::from(count) - expected).abs() <= tolerance,
                "candidate {} count {count} vs expected {expected:.1} (tol {tolerance:.1})",
                cand.id
            );
        }
        // All four candidates must actually be reachable at this setting.
        assert!(counts.iter().all(|&c| c > 0), "counts {counts:?}");
    }

    #[test]
    fn sample_from_candidates_equals_full_scan() {
        // The candidate-set sampler must be byte-identical to the full-scan
        // sampler for the same seed/k when fed the full-scan's own selection.
        let mut lcg: u64 = 0x1234_5678_9abc_def1;
        let mut rand = move || {
            lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1);
            (lcg >> 33) as i32
        };
        for k in [1u8, 4, 8] {
            for trial in 0..64u16 {
                let n = 200usize;
                let logits: Vec<i32> = (0..n).map(|_| (rand() % 200_000) - 100_000).collect();
                let cfg = SamplerConfig::new(k, 1500).expect("valid cfg");
                let mut rng_full = XorShift16::new(0xABCD ^ trial);
                let full = sample_topk_trace(&logits, &cfg, &mut rng_full);
                let cands: Vec<(i32, usize)> =
                    full.candidates.iter().map(|c| (c.logit, c.id)).collect();
                let mut rng_cand = XorShift16::new(0xABCD ^ trial);
                let cand =
                    sample_topk_from_candidates_trace(&cands, cfg.scale_q16(), &mut rng_cand);
                assert_eq!(full.total, cand.total, "total k={k} trial={trial}");
                assert_eq!(full.threshold, cand.threshold);
                assert_eq!(full.picked, cand.picked);
                assert_eq!(rng_full.state(), rng_cand.state(), "one rng step each");
            }
        }
    }

    #[test]
    fn short_slices_clamp_k() {
        let logits = [5, 3];
        let cfg = SamplerConfig::new(8, 1000).expect("valid");
        let mut rng = XorShift16::new(9);
        let trace = sample_topk_trace(&logits, &cfg, &mut rng);
        assert_eq!(trace.candidates.len(), 2);
    }
}
