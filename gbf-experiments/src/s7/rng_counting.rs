//! S7-local RouterRng counting and recomputability helpers.
//!
//! This module pins the experiment-contract fixture for D14/Rep-S7-2 without
//! reaching into the production router implementation. The full producer
//! adoption remains owned by the S7 replay/end-to-end beads.

use std::fmt;

use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256, sha256};
use serde::Serialize;

const PCG_128_MULTIPLIER: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

/// Canonical text defining the S7 RouterRng fixture stream contract.
pub const S7_ROUTER_RNG_STREAM_DEFINITION_V1: &str = concat!(
    "gbf:s7:router_rng_stream_definition:v1;",
    "core=PCG XSL RR 128/64 MCG pcg64_fast;",
    "router=seed128('router',seed);",
    "dropout=seed128('router-dropout',seed) XOR step;",
    "jitter=seed128('router-jitter',seed) XOR step XOR (layer_id << 32);",
    "eval_export_router_draws=0"
);

/// Hash of the S7 RouterRng fixture stream contract.
#[must_use]
pub fn router_rng_stream_def_hash() -> Hash256 {
    sha256(S7_ROUTER_RNG_STREAM_DEFINITION_V1.as_bytes())
}

/// Execution mode used when auditing RouterRng draw counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterExecutionMode {
    /// Training may consume dropout and jitter substream draws.
    Train,
    /// Evaluation must consume zero RouterRng draws.
    Eval,
    /// Export must consume zero RouterRng draws.
    Export,
}

/// Recomputed stochastic router fixture sample for one `(seed, step, layer)`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterReplaySample {
    /// Run seed.
    pub seed: u64,
    /// Training step.
    pub step: u64,
    /// Layer-local router id.
    pub layer_id: u32,
    /// Number of experts sampled by the fixture.
    pub n_experts: u16,
    /// Dropout mask, where `true` means the expert was dropped.
    pub dropout_mask: Vec<bool>,
    /// Per-expert jitter samples.
    pub jitter_samples: Vec<f32>,
    /// Hard top-1 dispatch indicator after applying dropout and jitter.
    pub dispatch_indicator: Vec<u8>,
    /// Draws consumed by the dropout substream.
    pub dropout_draw_count: u64,
    /// Draws consumed by the jitter substream.
    pub jitter_draw_count: u64,
    /// Total RouterRng-family draws consumed by this training fixture.
    pub total_draw_count: u64,
}

impl RouterReplaySample {
    /// Canonical JSON bytes for hashing and replay comparison.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7RouterRngError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    /// Hash of the canonical replay sample bytes.
    pub fn sample_hash(&self) -> Result<Hash256, S7RouterRngError> {
        Ok(sha256(self.canonical_json_bytes()?))
    }
}

/// Deterministic draw-count report for a router execution mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDrawCountReport {
    /// Audited execution mode.
    pub mode: RouterExecutionMode,
    /// Number of RouterRng-family draws consumed.
    pub draw_count: u64,
}

/// Recompute the fixture dropout, jitter, and dispatch output from a context key.
///
/// The fixture uses independent per-step substreams, so callers can recompute a
/// sample from `(seed, step, layer_id)` without replaying earlier steps.
pub fn recompute_router_replay_sample(
    seed: u64,
    step: u64,
    layer_id: u32,
    n_experts: u16,
    dropout_rate: f32,
    jitter_stddev: f32,
) -> Result<RouterReplaySample, S7RouterRngError> {
    if n_experts == 0 {
        return Err(S7RouterRngError::EmptyExpertSet);
    }
    if !dropout_rate.is_finite() || !(0.0..1.0).contains(&dropout_rate) {
        return Err(S7RouterRngError::InvalidDropoutRate { dropout_rate });
    }
    if !jitter_stddev.is_finite() || jitter_stddev < 0.0 {
        return Err(S7RouterRngError::InvalidJitterStddev { jitter_stddev });
    }

    let mut dropout = CountingRouterSubRng::dropout(seed, step);
    let mut jitter = CountingRouterSubRng::jitter(seed, step, layer_id);

    let dropout_mask = (0..n_experts)
        .map(|_| dropout.draw_unit_f32() < dropout_rate)
        .collect::<Vec<_>>();
    let jitter_samples = (0..n_experts)
        .map(|expert| {
            let centered = (jitter.draw_unit_f32() * 2.0) - 1.0;
            centered.mul_add(jitter_stddev, base_logit(expert, n_experts))
        })
        .collect::<Vec<_>>();
    let dispatch_indicator = dispatch_indicator(&dropout_mask, &jitter_samples);
    let dropout_draw_count = dropout.draw_count();
    let jitter_draw_count = jitter.draw_count();

    Ok(RouterReplaySample {
        seed,
        step,
        layer_id,
        n_experts,
        dropout_mask,
        jitter_samples,
        dispatch_indicator,
        dropout_draw_count,
        jitter_draw_count,
        total_draw_count: dropout_draw_count + jitter_draw_count,
    })
}

/// Count RouterRng-family draws for the fixture execution mode.
pub fn router_draw_count_for_mode(
    mode: RouterExecutionMode,
) -> Result<RouterDrawCountReport, S7RouterRngError> {
    let draw_count = match mode {
        RouterExecutionMode::Train => {
            recompute_router_replay_sample(0, 1, 0, 4, 0.25, 0.03125)?.total_draw_count
        }
        RouterExecutionMode::Eval | RouterExecutionMode::Export => 0,
    };
    Ok(RouterDrawCountReport { mode, draw_count })
}

/// Errors raised by S7 RouterRng fixture helpers.
#[derive(Debug)]
pub enum S7RouterRngError {
    /// The fixture was asked to route across zero experts.
    EmptyExpertSet,
    /// Dropout rate was outside `[0, 1)`.
    InvalidDropoutRate {
        /// Observed dropout rate.
        dropout_rate: f32,
    },
    /// Jitter standard deviation was negative or non-finite.
    InvalidJitterStddev {
        /// Observed jitter standard deviation.
        jitter_stddev: f32,
    },
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for S7RouterRngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyExpertSet => {
                f.write_str("S7 RouterRng fixture requires at least one expert")
            }
            Self::InvalidDropoutRate { dropout_rate } => {
                write!(
                    f,
                    "dropout_rate must be finite and in [0, 1), got {dropout_rate}"
                )
            }
            Self::InvalidJitterStddev { jitter_stddev } => {
                write!(
                    f,
                    "jitter_stddev must be finite and non-negative, got {jitter_stddev}"
                )
            }
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S7RouterRngError {}

impl From<CanonicalJsonError> for S7RouterRngError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountingRouterSubRng {
    rng: Pcg64Mcg,
    draw_count: u64,
}

impl CountingRouterSubRng {
    fn dropout(seed: u64, step: u64) -> Self {
        Self::new(seed128("router-dropout", seed) ^ u128::from(step))
    }

    fn jitter(seed: u64, step: u64, layer_id: u32) -> Self {
        Self::new(seed128("router-jitter", seed) ^ u128::from(step) ^ (u128::from(layer_id) << 32))
    }

    fn new(seed: u128) -> Self {
        Self {
            rng: Pcg64Mcg::new(seed),
            draw_count: 0,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.draw_count += 1;
        self.rng.next_u64()
    }

    fn draw_unit_f32(&mut self) -> f32 {
        const MANTISSA_BITS: u32 = 24;
        let mantissa = self.next_u64() >> (u64::BITS - MANTISSA_BITS);
        (mantissa as f32) / ((1_u32 << MANTISSA_BITS) as f32)
    }

    const fn draw_count(&self) -> u64 {
        self.draw_count
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pcg64Mcg {
    state: u128,
}

impl Pcg64Mcg {
    const fn new(seed: u128) -> Self {
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(PCG_128_MULTIPLIER);
        output_xsl_rr(self.state)
    }
}

fn seed128(domain: &str, seed: u64) -> u128 {
    let preimage = format!("gbf:s7:{domain}:{seed}");
    let digest = sha256(preimage.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

fn output_xsl_rr(state: u128) -> u64 {
    let rot = (state >> 122) as u32;
    let xsl = ((state >> 64) as u64) ^ (state as u64);
    xsl.rotate_right(rot)
}

fn base_logit(expert: u16, n_experts: u16) -> f32 {
    f32::from(n_experts - expert) * 0.125
}

fn dispatch_indicator(dropout_mask: &[bool], jittered_logits: &[f32]) -> Vec<u8> {
    let best = jittered_logits
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, _)| !dropout_mask[*index])
        .fold(None::<(usize, f32)>, |best, (index, value)| match best {
            Some((best_index, best_value)) if best_value >= value => Some((best_index, best_value)),
            _ => Some((index, value)),
        })
        .map(|(index, _)| index)
        .unwrap_or(0);

    let mut indicator = vec![0; jittered_logits.len()];
    indicator[best] = 1;
    indicator
}
