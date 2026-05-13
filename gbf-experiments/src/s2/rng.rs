//! S2 deterministic RNG stream extensions.

use std::collections::BTreeMap;

use gbf_foundation::{Hash256, sha256};

use crate::S2_LOG_TARGET;
use crate::s1::rng::{BatchRng, InitRng, Pcg64Mcg, S1Rng, ShuffleRng, seed128};

/// Canonical text defining the S2 RNG stream extension.
pub const S2_RNG_STREAM_DEFINITION_V1: &str = concat!(
    "gbf:s2:rng_stream_definition:v1;",
    "inherits=gbf:s1:rng_stream_definition:v1;",
    "streams=init,batch,shuffle,threshold_init,linearstate_smoke/linearstate_input_v1,linearstate_smoke/linearstate_params_v1;",
    "threshold_init=PCG XSL RR 128/64 MCG pcg64_fast seeded by seed128('threshold_init',seed);",
    "threshold_init_v1_draws=0"
);

/// Hash of the canonical S2 RNG stream definition.
#[must_use]
pub fn rng_stream_def_hash() -> Hash256 {
    sha256(S2_RNG_STREAM_DEFINITION_V1.as_bytes())
}

/// S2 threshold-initialization RNG stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThresholdInitRng {
    rng: Pcg64Mcg,
    seed: u64,
    draw_count: u64,
}

impl ThresholdInitRng {
    /// Construct the threshold-init stream from the run seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let seed128 = log_rng_stream_init("threshold_init", seed);
        Self {
            rng: Pcg64Mcg::new(seed128),
            seed,
            draw_count: 0,
        }
    }

    /// Run seed used to construct this stream.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Access the underlying PCG state for deterministic assertions.
    #[must_use]
    pub fn state(&self) -> u128 {
        self.rng.state()
    }

    /// Number of u64 draws consumed from this stream.
    #[must_use]
    pub const fn draw_count(&self) -> u64 {
        self.draw_count
    }
}

/// S2 run RNG streams initialized at train-run entry.
#[derive(Clone, Debug)]
pub struct S2RngStreams {
    /// Inherited initialization stream.
    pub init: InitRng,
    /// Inherited batch sampling stream.
    pub batch: BatchRng,
    /// Inherited shuffle stream.
    pub shuffle: ShuffleRng,
    /// S2 threshold initialization stream.
    pub threshold_init: ThresholdInitRng,
}

impl S2RngStreams {
    /// Initialize every S2 stream and emit one `rng_stream_init` event per stream.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        log_rng_stream_init("init", seed);
        log_rng_stream_init("batch", seed);
        log_rng_stream_init("shuffle", seed);
        Self {
            init: InitRng::new(seed),
            batch: BatchRng::new(seed),
            shuffle: ShuffleRng::new(seed),
            threshold_init: ThresholdInitRng::new(seed),
        }
    }
}

impl S1Rng for ThresholdInitRng {
    fn next_u64(&mut self) -> u64 {
        self.draw_count += 1;
        self.rng.next_u64()
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let draw = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&draw[..chunk.len()]);
        }
    }
}

/// Result of auditing seed-domain disjointness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RngDisjointnessAudit {
    /// Audited domains in deterministic order.
    pub domains: Vec<&'static str>,
    /// Number of seed128 collisions observed.
    pub collision_count: usize,
}

/// Domains that participate in the S2 RNG disjointness audit.
#[must_use]
pub const fn audited_domains() -> [&'static str; 6] {
    [
        "init",
        "batch",
        "shuffle",
        "threshold_init",
        "linearstate_smoke/linearstate_input_v1",
        "linearstate_smoke/linearstate_params_v1",
    ]
}

/// Audit seed128 disjointness for all S2 domains at a seed.
#[must_use]
pub fn audit_disjointness(seed: u64) -> RngDisjointnessAudit {
    let mut seen = BTreeMap::<u128, &'static str>::new();
    let mut collision_count = 0usize;
    let domains = audited_domains();
    for domain in domains {
        let derived = seed128(domain, seed);
        if seen.insert(derived, domain).is_some() {
            collision_count += 1;
        }
    }

    if collision_count == 0 {
        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "rng_disjointness_audit_pass",
            domains = ?domains,
            collision_count = 0_u64,
            "s2 rng disjointness audit pass"
        );
    } else {
        tracing::warn!(
            target: S2_LOG_TARGET,
            event_name = "rng_disjointness_audit_collision",
            domains = ?domains,
            collision_count,
            "s2 rng disjointness audit collision"
        );
    }

    RngDisjointnessAudit {
        domains: domains.to_vec(),
        collision_count,
    }
}

fn seed128_hex(seed: u128) -> String {
    format!("{seed:032x}")
}

fn log_rng_stream_init(domain: &'static str, seed: u64) -> u128 {
    let seed128 = seed128(domain, seed);
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "rng_stream_init",
        domain,
        seed,
        seed128 = %seed128_hex(seed128),
        "s2 rng stream init"
    );
    seed128
}
