//! S4 deterministic RNG stream declaration.

use serde::{Deserialize, Serialize};

use gbf_foundation::sha256;

/// D9 initialization stream domain. It is reserved but consumes zero draws.
pub const S4_INIT_RNG_DOMAIN: &str = "s4-init-init";
/// D9 batch sampling stream domain for Gutenberg continuation.
pub const S4_BATCH_RNG_DOMAIN: &str = "s4-init-batch";
/// D9 shuffle stream domain. S4 v1 reserves it but consumes zero draws.
pub const S4_SHUFFLE_RNG_DOMAIN: &str = "s4-init-shuffle";

/// Canonical text reserving the S4 RNG stream definition namespace.
pub const S4_RNG_STREAM_DEFINITION_V1: &str = concat!(
    "gbf:s4:rng_stream_definition:v1;",
    "core=PCG XSL RR 128/64 MCG pcg64_fast;",
    "seed128=sha256('gbf:s1:{domain}:{seed}')[0..16] little-endian;",
    "state=seed128(domain,seed)|1;",
    "streams=s4-init-init,s4-init-batch,s4-init-shuffle;",
    "init_rng_d9_draws_before_first_step=0;",
    "shuffle_rng_d9_draws_total=0"
);

/// Serializable S4 RNG stream initialization descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4RngStreamState {
    /// Domain string passed to the inherited `seed128` derivation.
    pub domain: String,
    /// 128-bit seed rendered as fixed-width lowercase hex.
    pub seed128_hex: String,
    /// Initial PCG MCG state after forcing the state odd.
    pub initial_state_hex: String,
    /// Number of draws consumed from this stream at initialization.
    pub draw_count: u64,
}

impl S4RngStreamState {
    /// Construct one S4 stream descriptor from a D9 domain and run seed.
    #[must_use]
    pub fn new(domain: &'static str, seed: u64) -> Self {
        let seed128 = s4_seed128(domain, seed);
        Self {
            domain: domain.to_owned(),
            seed128_hex: format!("{seed128:032x}"),
            initial_state_hex: format!("{:032x}", seed128 | 1),
            draw_count: 0,
        }
    }
}

/// S4 run RNG stream bundle initialized at continuation entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4RngStreams {
    /// Run seed used to construct the future streams.
    pub seed: u64,
    /// InitRng descriptor; D9 warm-start consumes zero draws before step 1.
    pub init: S4RngStreamState,
    /// BatchRng descriptor over the Gutenberg train corpus.
    pub batch: S4RngStreamState,
    /// ShuffleRng descriptor; S4 v1 consumes zero draws total.
    pub shuffle: S4RngStreamState,
}

impl S4RngStreams {
    /// Construct every S4 stream descriptor from the run seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            init: S4RngStreamState::new(S4_INIT_RNG_DOMAIN, seed),
            batch: S4RngStreamState::new(S4_BATCH_RNG_DOMAIN, seed),
            shuffle: S4RngStreamState::new(S4_SHUFFLE_RNG_DOMAIN, seed),
        }
    }
}

/// Derive the inherited 128-bit seed for an S4 stream domain and run seed.
#[must_use]
pub fn s4_seed128(domain: &str, seed: u64) -> u128 {
    let preimage = format!("gbf:s1:{domain}:{seed}");
    let digest = sha256(preimage.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

/// S4 stream domains registered by F-S4.02.
#[must_use]
pub const fn s4_stream_domains() -> [&'static str; 3] {
    [
        S4_INIT_RNG_DOMAIN,
        S4_BATCH_RNG_DOMAIN,
        S4_SHUFFLE_RNG_DOMAIN,
    ]
}
