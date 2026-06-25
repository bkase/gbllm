//! H10 one-token emulator comparison helpers.

use gbf_artifact::S7Topology;
use gbf_foundation::Hash256;

use crate::s7::schema::{EmulatorOneTokenReport, EmulatorOneTokenReportError};

/// Artifact-oracle route-tracer output for the fixed H10 prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArtifactOracleOneTokenTrace {
    /// Artifact-oracle logits hash for the fixed prompt.
    pub logits_sha: Hash256,
    /// Bank switches per token computed from consecutive argmax routes.
    pub bank_switches_per_token: f32,
}

/// Emulator observation for the same fixed H10 prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmulatorOneTokenObservation {
    /// Emulator logits hash for the fixed prompt.
    pub logits_sha: Hash256,
    /// Bank switches per token observed by the emulator.
    pub bank_switches_per_token: f32,
}

/// Inputs for comparing a one-token emulator observation with the artifact
/// oracle route tracer.
#[derive(Debug, Clone, PartialEq)]
pub struct EmulatorOneTokenComparison {
    /// Experiment seed.
    pub seed: u64,
    /// S7 topology under test.
    pub topology: S7Topology,
    /// Encoded ROM hash used by the emulator.
    pub encoded_rom_sha: Hash256,
    /// Fixed prompt hash.
    pub prompt_sha: Hash256,
    /// Artifact-oracle route-tracer output for the same prompt.
    pub artifact_oracle_trace: ArtifactOracleOneTokenTrace,
    /// Emulator observation for the same prompt.
    pub emulator_observation: EmulatorOneTokenObservation,
    /// Pairwise max absolute logit difference.
    pub pairwise_max_abs_diff: f64,
    /// S5 pinned output tolerance.
    pub s5_tolerance: f64,
    /// Number of deployable blocks for bank-switch bounds.
    pub n_blocks: u32,
}

/// Build an `s7_emulator_one_token.v1` report by comparing emulator output
/// against the artifact-oracle route tracer, not the training log.
pub fn compare_with_artifact_oracle_trace(
    comparison: EmulatorOneTokenComparison,
) -> Result<EmulatorOneTokenReport, EmulatorOneTokenReportError> {
    EmulatorOneTokenReport::from_artifact_oracle_trace(
        comparison.seed,
        comparison.topology,
        comparison.encoded_rom_sha,
        comparison.prompt_sha,
        comparison.artifact_oracle_trace.logits_sha,
        comparison.emulator_observation.logits_sha,
        comparison.pairwise_max_abs_diff,
        comparison.s5_tolerance,
        comparison.emulator_observation.bank_switches_per_token,
        comparison.artifact_oracle_trace.bank_switches_per_token,
        comparison.n_blocks,
    )
}
