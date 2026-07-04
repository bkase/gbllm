//! Ternary matvec kernel bake-off: measured cycles/MAC in gameroy (bd-rzq5n).
//!
//! Builds the three `gbf-kernel` bake-off kernels over identical deterministic
//! fixtures, executes each ROM in the emulator, verifies every output word
//! against the exact host reference, and reports measured M-cycles plus
//! projected cycles/token for the registered model size profiles.
//!
//! All rates are integer milli-units (`_x1000`) so the report is bit-stable.

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TraceDropPolicy,
};
use gbf_kernel::asm_impl::{
    KernelBuildError, KernelRom, OUTPUT_BASE, build_v1_interpreted, build_v2_dispatch,
    build_v3_weights_as_code,
};
use gbf_kernel::ref_impl::{RefKernelError, expected_output_bytes_le};
use gbf_kernel::spec::{
    TernaryKernelError, TernaryMatvecShape, TernaryWeights, deterministic_activations,
};
use serde::Serialize;
use std::fmt;

/// DMG CPU frequency in M-cycles per second (4.194304 MHz / 4).
pub const DMG_M_CYCLES_PER_SECOND: u64 = 1_048_576;

/// Fixture seeds; fixed so runs are reproducible byte-for-byte.
const WEIGHT_SEED: u64 = 0xBA5E_0FF0;
const ACTIVATION_SEED: u64 = 0xAC71;

/// Bake-off fixture shape: fan-in 64, 32 rows (2048 MACs per run).
const FAN_IN: u16 = 64;
const ROWS: u16 = 32;

/// Zero fractions measured, in permille.
const ZERO_PERMILLE_SWEEP: [u16; 4] = [0, 400, 600, 900];

/// Headline sparsity used for token projections.
const PROJECTION_ZERO_PERMILLE: u16 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KernelVariant {
    /// Generic loop interpreting `Ternary2` packed bytes.
    V1Interpreted,
    /// Threaded per-byte pattern dispatch (base-81 handler table).
    V2Dispatch,
    /// Straight-line weights-as-code with zero skipping.
    V3WeightsAsCode,
}

impl KernelVariant {
    pub const ALL: [Self; 3] = [Self::V1Interpreted, Self::V2Dispatch, Self::V3WeightsAsCode];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::V1Interpreted => "v1_interpreted",
            Self::V2Dispatch => "v2_dispatch",
            Self::V3WeightsAsCode => "v3_weights_as_code",
        }
    }

    fn build(
        self,
        weights: &TernaryWeights,
        activations: &[u8],
    ) -> Result<KernelRom, KernelBuildError> {
        match self {
            Self::V1Interpreted => build_v1_interpreted(weights, activations),
            Self::V2Dispatch => build_v2_dispatch(weights, activations),
            Self::V3WeightsAsCode => build_v3_weights_as_code(weights, activations),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KernelBakeoffRun {
    pub variant: KernelVariant,
    pub zero_permille: u16,
    pub mac_count: u32,
    pub nonzero_count: u32,
    pub measured_m_cycles: u64,
    /// Measured M-cycles per MAC in milli-cycles (all MACs, zeros included).
    pub m_cycles_per_mac_x1000: u64,
    pub program_bytes: usize,
    pub data_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenProjection {
    pub profile: &'static str,
    pub d_model: u16,
    pub d_ff: u16,
    pub n_blocks: u8,
    pub macs_per_token: u64,
    pub per_variant: Vec<VariantProjection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantProjection {
    pub variant: KernelVariant,
    pub m_cycles_per_token: u64,
    /// Tokens per second at 100% CPU, in milli-tokens.
    pub tokens_per_second_x1000: u64,
    /// Tokens per second with 30% reserved for UI/runtime, in milli-tokens.
    pub tokens_per_second_ui_reserve_x1000: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KernelBakeoffReport {
    pub schema: &'static str,
    pub fan_in: u16,
    pub rows: u16,
    pub weight_seed: u64,
    pub activation_seed: u64,
    pub projection_zero_permille: u16,
    pub runs: Vec<KernelBakeoffRun>,
    pub projections: Vec<TokenProjection>,
    pub caveats: Vec<&'static str>,
}

/// Model shapes projected against. The first four mirror
/// `gbf_policy::model_profile::ModelSizeProfile` registry constants; the
/// `QualityDense` rows probe the 2026-07-04 quality-first UX budget
/// (revised same day to <= ~10 s/char, i.e. >= 0.1 tok/s at 70% CPU).
const PROJECTION_PROFILES: [(&str, u16, u16, u8); 7] = [
    ("Toy1", 32, 64, 2),
    ("MoeTiny", 64, 128, 4),
    ("UpperBankCandidate-96", 96, 192, 4),
    ("UpperBankCandidate-128", 128, 192, 4),
    ("QualityDense-144x288x6", 144, 288, 6),
    ("QualityDense-160x320x6", 160, 320, 6),
    ("QualityDense-192x384x7", 192, 384, 7),
];

/// Estimated matvec MACs per generated token: per block one `d_model^2`
/// state-mix plus a two-matrix expert FFN (`2 * d_model * d_ff`), plus the
/// tied 80-token head. Norms, router, scales, and decode are excluded, so
/// projections are a floor on real cost.
#[must_use]
pub fn estimated_macs_per_token(d_model: u16, d_ff: u16, n_blocks: u8) -> u64 {
    let d_model = u64::from(d_model);
    let d_ff = u64::from(d_ff);
    let per_block = d_model * d_model + 2 * d_model * d_ff;
    u64::from(n_blocks) * per_block + d_model * 80
}

/// Build, execute, verify, and time every (variant, sparsity) combination.
pub fn run_kernel_bakeoff() -> Result<KernelBakeoffReport, KernelBakeoffError> {
    let shape = TernaryMatvecShape::new(FAN_IN, ROWS)?;
    let activations = deterministic_activations(shape.fan_in(), ACTIVATION_SEED);

    let mut runs = Vec::new();
    for zero_permille in ZERO_PERMILLE_SWEEP {
        let weights = TernaryWeights::deterministic(shape, WEIGHT_SEED, zero_permille)?;
        let expected = expected_output_bytes_le(&weights, &activations)?;
        for variant in KernelVariant::ALL {
            let rom = variant.build(&weights, &activations)?;
            let measured_m_cycles = execute_and_verify(variant, &rom, &expected)?;
            let mac_count = shape.mac_count();
            runs.push(KernelBakeoffRun {
                variant,
                zero_permille,
                mac_count,
                nonzero_count: weights.nonzero_count(),
                measured_m_cycles,
                m_cycles_per_mac_x1000: measured_m_cycles * 1000 / u64::from(mac_count),
                program_bytes: rom.program_bytes,
                data_bytes: rom.data_bytes,
            });
        }
    }

    let projections = PROJECTION_PROFILES
        .iter()
        .map(|&(profile, d_model, d_ff, n_blocks)| {
            let macs_per_token = estimated_macs_per_token(d_model, d_ff, n_blocks);
            let per_variant = KernelVariant::ALL
                .iter()
                .map(|&variant| {
                    let per_mac_x1000 = runs
                        .iter()
                        .find(|run| {
                            run.variant == variant && run.zero_permille == PROJECTION_ZERO_PERMILLE
                        })
                        .expect("sweep covers the projection sparsity")
                        .m_cycles_per_mac_x1000;
                    let m_cycles_per_token = macs_per_token * per_mac_x1000 / 1000;
                    VariantProjection {
                        variant,
                        m_cycles_per_token,
                        tokens_per_second_x1000: DMG_M_CYCLES_PER_SECOND * 1000
                            / m_cycles_per_token.max(1),
                        tokens_per_second_ui_reserve_x1000: DMG_M_CYCLES_PER_SECOND * 700
                            / m_cycles_per_token.max(1),
                    }
                })
                .collect();
            TokenProjection {
                profile,
                d_model,
                d_ff,
                n_blocks,
                macs_per_token,
                per_variant,
            }
        })
        .collect();

    Ok(KernelBakeoffReport {
        schema: "kernel_bakeoff.v1",
        fan_in: FAN_IN,
        rows: ROWS,
        weight_seed: WEIGHT_SEED,
        activation_seed: ACTIVATION_SEED,
        projection_zero_permille: PROJECTION_ZERO_PERMILLE,
        runs,
        projections,
        caveats: vec![
            "Single-bank fixture: no ROM bank switching or SRAM paging in the measured region.",
            "Kernels run with interrupts disabled and SP repurposed (V2/V3); production kernels pay yield/safe-point overhead on top.",
            "Projections cover matvec MACs only; norms, router, per-row scales, and decode are excluded.",
            "MACs-per-token formula assumes one d_model^2 state mix plus a 2*d_model*d_ff expert per block plus a tied 80-token head.",
        ],
    })
}

/// Run one kernel ROM to its end label and byte-compare outputs.
fn execute_and_verify(
    variant: KernelVariant,
    rom: &KernelRom,
    expected: &[u8],
) -> Result<u64, KernelBakeoffError> {
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(|error| KernelBakeoffError::Emulator {
            variant,
            reason: error.to_string(),
        })?;

    let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(1_000));
    let run_to = |emu: &mut Emulator, pc: u16, phase: &str| -> Result<(), KernelBakeoffError> {
        match emu.run_fast_until_pc(pc, budget) {
            Ok(RunOutcome::TrapHit { .. }) => Ok(()),
            Ok(other) => Err(KernelBakeoffError::Emulator {
                variant,
                reason: format!("did not reach {phase} at {pc:#06x}: {other:?}"),
            }),
            Err(error) => Err(KernelBakeoffError::Emulator {
                variant,
                reason: error.to_string(),
            }),
        }
    };
    run_to(&mut emu, rom.kernel_start_pc, "kernel start")?;
    let start = emu.m_cycle_count_floor().0;
    run_to(&mut emu, rom.kernel_end_pc, "kernel end")?;
    let end = emu.m_cycle_count_floor().0;

    let actual = emu
        .peek_range(OUTPUT_BASE, expected.len())
        .map_err(|error| KernelBakeoffError::Emulator {
            variant,
            reason: error.to_string(),
        })?;
    if actual != expected {
        return Err(KernelBakeoffError::OutputMismatch {
            variant,
            expected: expected.to_vec(),
            actual,
        });
    }
    Ok(end.saturating_sub(start))
}

#[derive(Debug)]
pub enum KernelBakeoffError {
    Kernel(TernaryKernelError),
    Reference(RefKernelError),
    Build(KernelBuildError),
    Emulator {
        variant: KernelVariant,
        reason: String,
    },
    OutputMismatch {
        variant: KernelVariant,
        expected: Vec<u8>,
        actual: Vec<u8>,
    },
}

impl fmt::Display for KernelBakeoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(error) => write!(f, "{error}"),
            Self::Reference(error) => write!(f, "{error}"),
            Self::Build(error) => write!(f, "{error}"),
            Self::Emulator { variant, reason } => {
                write!(f, "{}: emulator failure: {reason}", variant.label())
            }
            Self::OutputMismatch { variant, .. } => {
                write!(f, "{}: outputs disagree with reference", variant.label())
            }
        }
    }
}

impl std::error::Error for KernelBakeoffError {}

impl From<TernaryKernelError> for KernelBakeoffError {
    fn from(error: TernaryKernelError) -> Self {
        Self::Kernel(error)
    }
}

impl From<RefKernelError> for KernelBakeoffError {
    fn from(error: RefKernelError) -> Self {
        Self::Reference(error)
    }
}

impl From<KernelBuildError> for KernelBakeoffError {
    fn from(error: KernelBuildError) -> Self {
        Self::Build(error)
    }
}

/// Render the report as a Markdown summary table pair.
#[must_use]
pub fn report_to_markdown(report: &KernelBakeoffReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Kernel bake-off ({})", report.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Fixture: {} fan-in x {} rows = {} MACs; weight seed {:#x}, activation seed {:#x}.",
        report.fan_in,
        report.rows,
        u32::from(report.fan_in) * u32::from(report.rows),
        report.weight_seed,
        report.activation_seed
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| variant | zeros (permille) | M-cycles | M-cycles/MAC | program bytes | data bytes |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for run in &report.runs {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {}.{:03} | {} | {} |",
            run.variant.label(),
            run.zero_permille,
            run.measured_m_cycles,
            run.m_cycles_per_mac_x1000 / 1000,
            run.m_cycles_per_mac_x1000 % 1000,
            run.program_bytes,
            run.data_bytes
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Projections at {} permille zeros (matvec floor, no norms/router/decode):",
        report.projection_zero_permille
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| profile | MACs/token | variant | M-cycles/token | tok/s (100%) | tok/s (70%) |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for projection in &report.projections {
        for entry in &projection.per_variant {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {}.{:03} | {}.{:03} |",
                projection.profile,
                projection.macs_per_token,
                entry.variant.label(),
                entry.m_cycles_per_token,
                entry.tokens_per_second_x1000 / 1000,
                entry.tokens_per_second_x1000 % 1000,
                entry.tokens_per_second_ui_reserve_x1000 / 1000,
                entry.tokens_per_second_ui_reserve_x1000 % 1000,
            );
        }
    }
    let _ = writeln!(out);
    for caveat in &report.caveats {
        let _ = writeln!(out, "- {caveat}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macs_per_token_matches_hand_math_for_moe_tiny() {
        // 4 * (64*64 + 2*64*128) + 64*80 = 4 * 20480 + 5120 = 87040.
        assert_eq!(estimated_macs_per_token(64, 128, 4), 87_040);
    }

    #[test]
    fn bakeoff_smoke_v3_conforms_and_reports_cycles() {
        // Full sweep runs in the integration test/bin; keep the lib smoke to
        // one variant + one sparsity for hook latency.
        let shape = TernaryMatvecShape::new(FAN_IN, ROWS).expect("valid shape");
        let weights =
            TernaryWeights::deterministic(shape, WEIGHT_SEED, 400).expect("valid weights");
        let activations = deterministic_activations(shape.fan_in(), ACTIVATION_SEED);
        let expected = expected_output_bytes_le(&weights, &activations).expect("fits i16");
        let rom = KernelVariant::V3WeightsAsCode
            .build(&weights, &activations)
            .expect("builds");
        let cycles = execute_and_verify(KernelVariant::V3WeightsAsCode, &rom, &expected)
            .expect("kernel output matches reference");
        assert!(cycles > 0);
    }
}
