//! S3 deterministic RNG stream declaration.

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
))]
pub use crate::s2::rng::S2RngStreams as S3RngStreams;

/// S3 stream domains inherited from F-S2 without extension.
#[must_use]
pub const fn s3_stream_domains() -> [&'static str; 4] {
    ["init", "batch", "shuffle", "threshold_init"]
}

/// Runtime audit that S3 has not declared a new RNG stream domain.
pub fn assert_no_new_rng_domains() {
    const EXPECTED: [&str; 4] = ["init", "batch", "shuffle", "threshold_init"];
    assert_eq!(s3_stream_domains(), EXPECTED);
}
