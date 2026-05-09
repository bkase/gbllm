//! Deterministic random-number generation primitives for S1.

use std::fmt;

use gbf_foundation::{Hash256, sha256};

const PCG_128_MULTIPLIER: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

/// Canonical text defining the S1 RNG stream contract.
///
/// This text is hashed into checkpoint metadata so ablation comparisons can
/// prove both builds used the same stream definitions, not merely the same
/// train config.
pub const RNG_STREAM_DEFINITION_V1: &str = concat!(
    "gbf:s1:rng_stream_definition:v1;",
    "core=PCG XSL RR 128/64 MCG pcg64_fast;",
    "multiplier=0x2360ed051fc65da44385df649fccf645;",
    "state=seed128(domain,seed)|1;",
    "seed128=sha256('gbf:s1:{domain}:{seed}')[0..16] little-endian;",
    "streams=init,batch,shuffle;",
    "uniform_u64_inclusive=rejection-sampled u64 over inclusive integer interval"
);

/// Hash of the canonical S1 RNG stream definition.
#[must_use]
pub fn rng_stream_def_hash() -> Hash256 {
    sha256(RNG_STREAM_DEFINITION_V1.as_bytes())
}

/// Minimal source of deterministic 64-bit draws used by S1 samplers.
pub trait S1Rng {
    /// Return the next deterministic 64-bit draw.
    fn next_u64(&mut self) -> u64;

    /// Fill bytes from little-endian `next_u64` draws.
    fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let draw = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&draw[..chunk.len()]);
        }
    }
}

/// PCG XSL RR 128/64 MCG (`pcg64_fast`) RNG pinned for S1.
#[derive(Clone, Eq, PartialEq)]
pub struct Pcg64Mcg {
    state: u128,
}

impl Pcg64Mcg {
    /// Construct from a 128-bit seed, matching PCG MCG seeding by forcing an odd state.
    ///
    /// MCG streams cannot use an even state. The `| 1` mapping is part of the
    /// S1 reproducibility contract: raw seed state `0` is stored as state `1`.
    pub fn new(seed: u128) -> Self {
        Self { state: seed | 1 }
    }

    /// Current internal state. Exposed for deterministic tests and fixtures.
    pub fn state(&self) -> u128 {
        self.state
    }
}

impl fmt::Debug for Pcg64Mcg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pcg64Mcg {}")
    }
}

impl S1Rng for Pcg64Mcg {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(PCG_128_MULTIPLIER);
        output_xsl_rr(self.state)
    }
}

macro_rules! s1_stream {
    ($name:ident, $domain:literal) => {
        #[doc = concat!("S1 `", stringify!($name), "` stream over a private PCG state.")]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            rng: Pcg64Mcg,
        }

        impl $name {
            /// Construct the stream from the run seed.
            pub fn new(seed: u64) -> Self {
                Self {
                    rng: Pcg64Mcg::new(seed128($domain, seed)),
                }
            }

            /// Access the underlying PCG state for deterministic assertions.
            pub fn state(&self) -> u128 {
                self.rng.state()
            }
        }

        impl S1Rng for $name {
            fn next_u64(&mut self) -> u64 {
                self.rng.next_u64()
            }

            fn fill_bytes(&mut self, out: &mut [u8]) {
                self.rng.fill_bytes(out);
            }
        }
    };
}

s1_stream!(InitRng, "init");
s1_stream!(BatchRng, "batch");
s1_stream!(ShuffleRng, "shuffle");

/// Derive the RFC-pinned 128-bit seed for an S1 domain and run seed.
pub fn seed128(domain: &str, seed: u64) -> u128 {
    let preimage = format!("gbf:s1:{domain}:{seed}");
    let digest = sha256(preimage.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

/// Draw uniformly from the inclusive interval `[lo, hi]` using rejection sampling.
///
/// This keeps all arithmetic in integer space. Values in the partial top bucket
/// of the `u64` sample space are discarded before reducing into the target span.
pub fn uniform_u64_inclusive(rng: &mut impl S1Rng, lo: u64, hi: u64) -> u64 {
    assert!(lo <= hi, "uniform_u64_inclusive requires lo <= hi");

    let range_size = u128::from(hi) - u128::from(lo) + 1;
    let sample_space = 1_u128 << u64::BITS;
    let acceptance_bound = (sample_space / range_size) * range_size;

    loop {
        let candidate = u128::from(rng.next_u64());
        if candidate < acceptance_bound {
            let bucket = candidate / range_size;
            let offset = candidate - bucket * range_size;
            return lo + u64::try_from(offset).expect("accepted offset must fit u64");
        }
    }
}

#[inline(always)]
fn output_xsl_rr(state: u128) -> u64 {
    let rot = (state >> 122) as u32;
    let xsl = ((state >> 64) as u64) ^ (state as u64);
    xsl.rotate_right(rot)
}
