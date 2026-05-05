//! `realism_report.v1` schema and semantic validator for F-B1.

use std::collections::BTreeSet;
use std::fmt;

use gbf_abi::compute_shape::SquareDim;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const REALISM_REPORT_SCHEMA: &str = "realism_report.v1";
pub const ZERO_SELF_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const BRINGUP_YIELD_QUANTUM: &str = "KLaneRow";
const BRINGUP_SCHEDULER_PROFILE: &str = "Bringup";
const BRINGUP_SOFT_DEADLINE_MARGIN_M_CYCLES: u32 = 128;
const BRINGUP_MULTIPLY_KERNEL: &str = "fixed_8_step_shift_add_i8_i32";
const BRINGUP_GATED_FRAME_START: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealismReportV1 {
    pub schema: String,
    pub headline_n: u16,
    pub run_order: Vec<u16>,
    pub toolchain_identity: ToolchainIdentity,
    pub reproducibility: Reproducibility,
    pub workload: Workload,
    pub runtime_knobs: RuntimeKnobs,
    pub runs: Vec<RealismRun>,
}

impl RealismReportV1 {
    pub fn validate_checked(&self) -> Result<(), RealismReportError> {
        if self.schema != REALISM_REPORT_SCHEMA {
            return Err(RealismReportError::BadSchema {
                found: self.schema.clone(),
            });
        }
        if self.headline_n != 128 {
            return Err(RealismReportError::BadHeadline {
                found: self.headline_n,
            });
        }
        if self.run_order != [32, 64, 96, 128] {
            return Err(RealismReportError::BadRunOrder {
                found: self.run_order.clone(),
            });
        }
        if self.workload.sizes != self.run_order {
            return Err(RealismReportError::WorkloadRunOrderMismatch);
        }
        validate_reproducibility(&self.reproducibility)?;
        validate_workload(&self.workload)?;
        validate_runtime_knobs(&self.runtime_knobs)?;
        if self.runs.len() != self.run_order.len() {
            return Err(RealismReportError::MissingRun {
                expected: self.run_order.clone(),
            });
        }
        let mut seen = BTreeSet::new();
        for (idx, run) in self.runs.iter().enumerate() {
            let expected_n = self.run_order[idx];
            if run.n != expected_n {
                return Err(RealismReportError::OutOfOrderRun {
                    index: idx,
                    expected: expected_n,
                    found: run.n,
                });
            }
            if !seen.insert(run.n) {
                return Err(RealismReportError::DuplicateRun { n: run.n });
            }
            validate_run(run)?;
        }
        let expected_hash = self_hash(self)?;
        if self.reproducibility.report_self_hash != expected_hash {
            return Err(RealismReportError::SelfHashMismatch {
                expected: expected_hash,
                found: self.reproducibility.report_self_hash.clone(),
            });
        }
        Ok(())
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, RealismReportError> {
        self.reproducibility.report_self_hash = ZERO_SELF_HASH.to_owned();
        self.reproducibility.report_self_hash = self_hash(&self)?;
        Ok(self)
    }

    pub fn to_canonical_json(&self) -> Result<String, RealismReportError> {
        serde_json::to_string(self).map_err(|error| RealismReportError::Json {
            reason: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainIdentity {
    pub rustc: String,
    pub host_triple: String,
    pub source_date_epoch: u64,
    pub target_profile_hash: String,
    pub emulator_adapter_hash: String,
    pub gameroy_core_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reproducibility {
    pub git_sha: String,
    pub git_dirty: bool,
    pub report_self_hash: String,
    pub regenerated_from_pinned_inputs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workload {
    pub kind: String,
    pub sizes: Vec<u16>,
    pub tile: TileShape,
    pub operand_fixture: String,
    pub operand_layout: OperandLayoutReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileShape {
    pub m: u16,
    pub n: u16,
    pub k: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperandLayoutReport {
    pub a: BankOffset,
    pub b: BankOffset,
    pub multiply_kernel: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankOffset {
    pub bank: u16,
    pub offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeKnobs {
    pub yield_quantum: String,
    pub soft_deadline_margin_mcycles: u32,
    pub scheduler_profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealismRun {
    pub n: u16,
    pub build_identity: BuildIdentity,
    pub structural_counts: StructuralCounts,
    pub compute_costs: ComputeCosts,
    pub memory_bandwidth: MemoryBandwidth,
    pub rom_layout: RomLayout,
    pub banking: BankingMetrics,
    pub scheduling: SchedulingMetrics,
    pub conformance: Conformance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub build_hash: String,
    pub runtime_nucleus_hash: String,
    pub compute_bringup_request_hash: String,
    pub asm_ir_hash: String,
    pub rom_sha256: String,
    pub operand_fixture_hash: String,
    pub multiply_kernel_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralCounts {
    pub products: u64,
    pub output_tiles: u32,
    pub k_tiles_per_output_tile: u32,
    pub operand_panel_copies: u32,
    pub operand_panel_bytes_copied: u32,
    pub full_output_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeCosts {
    pub cycles_per_product_accumulate: Distribution,
    pub cycles_per_output_tile: Distribution,
    pub cycles_per_full_matmul: u64,
    pub max_unyielded_compute_mcycles: u32,
    pub wall_clock_seconds_at_gb_clock_f64: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Distribution {
    pub sample_count: u32,
    pub min: u64,
    pub mean: f64,
    pub max: u64,
    pub p99: u64,
}

impl Distribution {
    pub fn validate_checked(self, field: &'static str) -> Result<(), RealismReportError> {
        if self.sample_count == 0 {
            return Err(RealismReportError::ZeroSampleCount { field });
        }
        if !self.mean.is_finite() {
            return Err(RealismReportError::NonFiniteFloat { field });
        }
        if self.min > self.max || self.p99 < self.min || self.p99 > self.max {
            return Err(RealismReportError::BadDistribution { field });
        }
        let mean_floor = self.mean.floor();
        let mean_ceil = self.mean.ceil();
        if mean_floor < self.min as f64 || mean_ceil > self.max as f64 {
            return Err(RealismReportError::BadDistribution { field });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryBandwidth {
    pub rom_bank0_table_bytes_read: u64,
    pub romx_operand_bytes_read: u64,
    pub effective_bank0_table_read_bandwidth_bytes_per_sec_f64: f64,
    pub effective_romx_operand_read_bandwidth_bytes_per_sec_f64: f64,
    pub wram_peak_bytes: u32,
    pub sram_peak_bytes: u32,
    pub hram_peak_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomLayout {
    pub bank0_used_bytes: u32,
    pub bank0_free_bytes: u32,
    pub romx_operand_bank_size_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankingMetrics {
    pub logical_bank_switches_per_output_tile: u32,
    pub logical_bank_switches_per_full_matmul: u32,
    pub mbc_rom_bank_register_write_count: u32,
    pub bank_lease_acquire_count: u32,
    pub bank_lease_release_count: u32,
    pub bank_lease_balance: i32,
    pub max_active_bank_leases: u16,
    pub yield_while_bank_lease_active_count: u32,
    pub harness_pause_while_bank_lease_active_count: u32,
    pub max_bank_lease_hold_mcycles: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulingMetrics {
    pub frames_to_completion: u32,
    pub gated_frame_start: u32,
    pub gated_frame_end_exclusive: u32,
    pub gated_frame_count: u32,
    pub vblank_count: u32,
    pub widget_update_count: u32,
    pub scheduler_service_count: u32,
    pub frame_service_misses: u32,
    pub max_no_progress_frames: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst_case_interrupt_latency_mcycles: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles_remaining_at_widget_tick: Option<Distribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles_remaining_at_scheduler_service: Option<Distribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conformance {
    pub byte_exact_match: bool,
    pub reference: String,
    pub reference_source_hash: String,
    pub fixture_hash: String,
    pub expected_output_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealismReportError {
    BadSchema {
        found: String,
    },
    BadHeadline {
        found: u16,
    },
    BadRunOrder {
        found: Vec<u16>,
    },
    WorkloadRunOrderMismatch,
    MissingRun {
        expected: Vec<u16>,
    },
    DuplicateRun {
        n: u16,
    },
    OutOfOrderRun {
        index: usize,
        expected: u16,
        found: u16,
    },
    StructuralCount {
        n: u16,
        field: &'static str,
        expected: u64,
        found: u64,
    },
    BankLeaseBalance {
        n: u16,
        found: i32,
    },
    ByteExactFalse {
        n: u16,
    },
    BadReproducibility {
        field: &'static str,
        reason: String,
    },
    BadRuntimeKnob {
        field: &'static str,
        expected: String,
        found: String,
    },
    FrameGate {
        n: u16,
        field: &'static str,
        expected: String,
        found: String,
    },
    ZeroSampleCount {
        field: &'static str,
    },
    BadDistribution {
        field: &'static str,
    },
    NonFiniteFloat {
        field: &'static str,
    },
    SelfHashMismatch {
        expected: String,
        found: String,
    },
    Json {
        reason: String,
    },
}

impl fmt::Display for RealismReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSchema { found } => write!(f, "unexpected schema {found}"),
            Self::BadHeadline { found } => write!(f, "headline_n must be 128, found {found}"),
            Self::BadRunOrder { found } => write!(f, "bad run_order {found:?}"),
            Self::WorkloadRunOrderMismatch => f.write_str("workload sizes differ from run_order"),
            Self::MissingRun { expected } => write!(f, "missing runs for {expected:?}"),
            Self::DuplicateRun { n } => write!(f, "duplicate run for N={n}"),
            Self::OutOfOrderRun {
                index,
                expected,
                found,
            } => write!(
                f,
                "run {index} is N={found}, expected ascending run_order N={expected}"
            ),
            Self::StructuralCount {
                n,
                field,
                expected,
                found,
            } => write!(
                f,
                "N={n} structural count {field} is {found}, expected {expected}"
            ),
            Self::BankLeaseBalance { n, found } => {
                write!(f, "N={n} bank lease balance is {found}, expected 0")
            }
            Self::ByteExactFalse { n } => write!(f, "N={n} byte_exact_match is false"),
            Self::BadReproducibility { field, reason } => {
                write!(f, "reproducibility field {field} is invalid: {reason}")
            }
            Self::BadRuntimeKnob {
                field,
                expected,
                found,
            } => write!(f, "runtime knob {field} is {found}, expected {expected}"),
            Self::FrameGate {
                n,
                field,
                expected,
                found,
            } => write!(
                f,
                "N={n} frame gate {field} is {found}, expected {expected}"
            ),
            Self::ZeroSampleCount { field } => write!(f, "{field} has zero samples"),
            Self::BadDistribution { field } => write!(f, "{field} has invalid distribution bounds"),
            Self::NonFiniteFloat { field } => write!(f, "{field} contains non-finite float"),
            Self::SelfHashMismatch { expected, found } => {
                write!(f, "report_self_hash {found} does not match {expected}")
            }
            Self::Json { reason } => write!(f, "json error: {reason}"),
        }
    }
}

impl std::error::Error for RealismReportError {}

fn validate_reproducibility(repro: &Reproducibility) -> Result<(), RealismReportError> {
    if repro.git_sha.trim().is_empty() || repro.git_sha == "workspace-unpinned" {
        return Err(RealismReportError::BadReproducibility {
            field: "git_sha",
            reason: "must identify pinned packet inputs".to_owned(),
        });
    }
    if repro.git_dirty {
        return Err(RealismReportError::BadReproducibility {
            field: "git_dirty",
            reason: "checked reports must be generated from clean pinned inputs".to_owned(),
        });
    }
    if !repro.regenerated_from_pinned_inputs {
        return Err(RealismReportError::BadReproducibility {
            field: "regenerated_from_pinned_inputs",
            reason: "must be true for checked F-B1 packet reports".to_owned(),
        });
    }
    Ok(())
}

fn validate_runtime_knobs(knobs: &RuntimeKnobs) -> Result<(), RealismReportError> {
    if knobs.yield_quantum != BRINGUP_YIELD_QUANTUM {
        return Err(RealismReportError::BadRuntimeKnob {
            field: "yield_quantum",
            expected: BRINGUP_YIELD_QUANTUM.to_owned(),
            found: knobs.yield_quantum.clone(),
        });
    }
    if knobs.scheduler_profile != BRINGUP_SCHEDULER_PROFILE {
        return Err(RealismReportError::BadRuntimeKnob {
            field: "scheduler_profile",
            expected: BRINGUP_SCHEDULER_PROFILE.to_owned(),
            found: knobs.scheduler_profile.clone(),
        });
    }
    if knobs.soft_deadline_margin_mcycles != BRINGUP_SOFT_DEADLINE_MARGIN_M_CYCLES {
        return Err(RealismReportError::BadRuntimeKnob {
            field: "soft_deadline_margin_mcycles",
            expected: BRINGUP_SOFT_DEADLINE_MARGIN_M_CYCLES.to_string(),
            found: knobs.soft_deadline_margin_mcycles.to_string(),
        });
    }
    Ok(())
}

fn validate_workload(workload: &Workload) -> Result<(), RealismReportError> {
    if workload.operand_layout.multiply_kernel != BRINGUP_MULTIPLY_KERNEL {
        return Err(RealismReportError::BadRuntimeKnob {
            field: "multiply_kernel",
            expected: BRINGUP_MULTIPLY_KERNEL.to_owned(),
            found: workload.operand_layout.multiply_kernel.clone(),
        });
    }
    Ok(())
}

fn validate_run(run: &RealismRun) -> Result<(), RealismReportError> {
    let dim = SquareDim::new(run.n).expect("report sizes are F-B1-valid");
    let n = u64::from(run.n);
    check_count(run, "products", n * n * n, run.structural_counts.products)?;
    let tiles = u64::from(dim.tiles_per_axis()) * u64::from(dim.tiles_per_axis());
    check_count(
        run,
        "output_tiles",
        tiles,
        u64::from(run.structural_counts.output_tiles),
    )?;
    check_count(
        run,
        "k_tiles_per_output_tile",
        u64::from(dim.tiles_per_axis()),
        u64::from(run.structural_counts.k_tiles_per_output_tile),
    )?;
    let panel_copies = tiles * u64::from(dim.tiles_per_axis()) * 2;
    check_count(
        run,
        "operand_panel_copies",
        panel_copies,
        u64::from(run.structural_counts.operand_panel_copies),
    )?;
    check_count(
        run,
        "operand_panel_bytes_copied",
        panel_copies * 256,
        u64::from(run.structural_counts.operand_panel_bytes_copied),
    )?;
    check_count(
        run,
        "full_output_bytes",
        n * n * 4,
        u64::from(run.structural_counts.full_output_bytes),
    )?;
    if run.banking.bank_lease_balance != 0 {
        return Err(RealismReportError::BankLeaseBalance {
            n: run.n,
            found: run.banking.bank_lease_balance,
        });
    }
    if !run.conformance.byte_exact_match {
        return Err(RealismReportError::ByteExactFalse { n: run.n });
    }
    check_count(
        run,
        "rom_bank0_table_bytes_read",
        0,
        run.memory_bandwidth.rom_bank0_table_bytes_read,
    )?;
    validate_frame_gate(run)?;
    run.compute_costs
        .cycles_per_product_accumulate
        .validate_checked("cycles_per_product_accumulate")?;
    run.compute_costs
        .cycles_per_output_tile
        .validate_checked("cycles_per_output_tile")?;
    if let Some(distribution) = run.scheduling.cycles_remaining_at_widget_tick {
        distribution.validate_checked("cycles_remaining_at_widget_tick")?;
    }
    if let Some(distribution) = run.scheduling.cycles_remaining_at_scheduler_service {
        distribution.validate_checked("cycles_remaining_at_scheduler_service")?;
    }
    Ok(())
}

fn validate_frame_gate(run: &RealismRun) -> Result<(), RealismReportError> {
    let expected_gated_frame_end = run
        .scheduling
        .frames_to_completion
        .saturating_sub(1)
        .max(BRINGUP_GATED_FRAME_START);
    frame_gate_eq_u32(
        run,
        "gated_frame_start",
        BRINGUP_GATED_FRAME_START,
        run.scheduling.gated_frame_start,
    )?;
    frame_gate_eq_u32(
        run,
        "gated_frame_end_exclusive",
        expected_gated_frame_end,
        run.scheduling.gated_frame_end_exclusive,
    )?;
    frame_gate_eq_u32(
        run,
        "gated_frame_count",
        run.scheduling
            .gated_frame_end_exclusive
            .saturating_sub(run.scheduling.gated_frame_start),
        run.scheduling.gated_frame_count,
    )?;
    frame_gate_eq_u32(
        run,
        "vblank_count",
        run.scheduling.gated_frame_count.saturating_add(1),
        run.scheduling.vblank_count,
    )?;
    let deadline =
        gbf_hw::timing::FRAME_M_CYCLES.saturating_sub(BRINGUP_SOFT_DEADLINE_MARGIN_M_CYCLES);
    frame_gate_eq_u32(
        run,
        "frame_service_misses",
        0,
        run.scheduling.frame_service_misses,
    )?;
    frame_gate_eq_u32(
        run,
        "widget_update_count",
        run.scheduling.gated_frame_count,
        run.scheduling.widget_update_count,
    )?;
    frame_gate_eq_u32(
        run,
        "scheduler_service_count",
        run.scheduling.gated_frame_count,
        run.scheduling.scheduler_service_count,
    )?;
    frame_gate_le_u32(
        run,
        "max_no_progress_frames",
        1,
        run.scheduling.max_no_progress_frames,
    )?;
    frame_gate_le_u32(
        run,
        "max_unyielded_compute_mcycles",
        deadline,
        run.compute_costs.max_unyielded_compute_mcycles,
    )?;
    frame_gate_le_u32(
        run,
        "max_bank_lease_hold_mcycles",
        deadline,
        run.banking.max_bank_lease_hold_mcycles,
    )?;
    frame_gate_eq_u32(
        run,
        "yield_while_bank_lease_active_count",
        0,
        run.banking.yield_while_bank_lease_active_count,
    )?;
    frame_gate_eq_u32(
        run,
        "harness_pause_while_bank_lease_active_count",
        0,
        run.banking.harness_pause_while_bank_lease_active_count,
    )?;
    Ok(())
}

fn frame_gate_eq_u32(
    run: &RealismRun,
    field: &'static str,
    expected: u32,
    found: u32,
) -> Result<(), RealismReportError> {
    if expected == found {
        Ok(())
    } else {
        Err(RealismReportError::FrameGate {
            n: run.n,
            field,
            expected: expected.to_string(),
            found: found.to_string(),
        })
    }
}

fn frame_gate_le_u32(
    run: &RealismRun,
    field: &'static str,
    max: u32,
    found: u32,
) -> Result<(), RealismReportError> {
    if found <= max {
        Ok(())
    } else {
        Err(RealismReportError::FrameGate {
            n: run.n,
            field,
            expected: format!("<= {max}"),
            found: found.to_string(),
        })
    }
}

fn check_count(
    run: &RealismRun,
    field: &'static str,
    expected: u64,
    found: u64,
) -> Result<(), RealismReportError> {
    if expected == found {
        Ok(())
    } else {
        Err(RealismReportError::StructuralCount {
            n: run.n,
            field,
            expected,
            found,
        })
    }
}

fn self_hash(report: &RealismReportV1) -> Result<String, RealismReportError> {
    let mut normalized = report.clone();
    normalized.reproducibility.report_self_hash = ZERO_SELF_HASH.to_owned();
    let bytes = serde_json::to_vec(&normalized).map_err(|error| RealismReportError::Json {
        reason: error.to_string(),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"gbf-report/f-b1/realism_report.v1/self_hash");
    hasher.update(bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fixture_report() -> RealismReportV1 {
        let runs = [32, 64, 96, 128]
            .into_iter()
            .map(run_fixture)
            .collect::<Vec<_>>();
        RealismReportV1 {
            schema: REALISM_REPORT_SCHEMA.to_owned(),
            headline_n: 128,
            run_order: vec![32, 64, 96, 128],
            toolchain_identity: ToolchainIdentity {
                rustc: "rustc 1.92.0".to_owned(),
                host_triple: "synthetic-host".to_owned(),
                source_date_epoch: 0,
                target_profile_hash: fake_hash(1),
                emulator_adapter_hash: fake_hash(2),
                gameroy_core_hash: fake_hash(3),
            },
            reproducibility: Reproducibility {
                git_sha: "0000000".to_owned(),
                git_dirty: false,
                report_self_hash: ZERO_SELF_HASH.to_owned(),
                regenerated_from_pinned_inputs: true,
            },
            workload: Workload {
                kind: "MatmulI8Bringup".to_owned(),
                sizes: vec![32, 64, 96, 128],
                tile: TileShape {
                    m: 16,
                    n: 16,
                    k: 16,
                },
                operand_fixture: "DeterministicAffineV1".to_owned(),
                operand_layout: OperandLayoutReport {
                    a: BankOffset { bank: 1, offset: 0 },
                    b: BankOffset { bank: 2, offset: 0 },
                    multiply_kernel: "fixed_8_step_shift_add_i8_i32".to_owned(),
                },
            },
            runtime_knobs: RuntimeKnobs {
                yield_quantum: "KLaneRow".to_owned(),
                soft_deadline_margin_mcycles: 128,
                scheduler_profile: "Bringup".to_owned(),
            },
            runs,
        }
        .with_computed_self_hash()
        .expect("hash")
    }

    fn run_fixture(n: u16) -> RealismRun {
        let dim = SquareDim::new(n).expect("valid");
        let tiles = u32::from(dim.tiles_per_axis()) * u32::from(dim.tiles_per_axis());
        let k_tiles = u32::from(dim.tiles_per_axis());
        let panel_copies = tiles * k_tiles * 2;
        RealismRun {
            n,
            build_identity: BuildIdentity {
                build_hash: fake_hash(4),
                runtime_nucleus_hash: fake_hash(5),
                compute_bringup_request_hash: fake_hash(n as u8),
                asm_ir_hash: fake_hash(6),
                rom_sha256: fake_hash(7),
                operand_fixture_hash: fake_hash(8),
                multiply_kernel_hash: fake_hash(9),
            },
            structural_counts: StructuralCounts {
                products: u64::from(n) * u64::from(n) * u64::from(n),
                output_tiles: tiles,
                k_tiles_per_output_tile: k_tiles,
                operand_panel_copies: panel_copies,
                operand_panel_bytes_copied: panel_copies * 256,
                full_output_bytes: u32::try_from(dim.output_bytes_i32()).expect("fits"),
            },
            compute_costs: ComputeCosts {
                cycles_per_product_accumulate: dist(4),
                cycles_per_output_tile: dist(1024),
                cycles_per_full_matmul: u64::from(n) * u64::from(n) * u64::from(n) * 4,
                max_unyielded_compute_mcycles: 320,
                wall_clock_seconds_at_gb_clock_f64: 1.0,
            },
            memory_bandwidth: MemoryBandwidth {
                rom_bank0_table_bytes_read: 0,
                romx_operand_bytes_read: u64::from(panel_copies) * 256,
                effective_bank0_table_read_bandwidth_bytes_per_sec_f64: 0.0,
                effective_romx_operand_read_bandwidth_bytes_per_sec_f64: 1.0,
                wram_peak_bytes: 1536,
                sram_peak_bytes: 0,
                hram_peak_bytes: 8,
            },
            rom_layout: RomLayout {
                bank0_used_bytes: 4096,
                bank0_free_bytes: 12_288,
                romx_operand_bank_size_bytes: 16_384,
            },
            banking: BankingMetrics {
                logical_bank_switches_per_output_tile: k_tiles * 2,
                logical_bank_switches_per_full_matmul: panel_copies,
                mbc_rom_bank_register_write_count: panel_copies,
                bank_lease_acquire_count: panel_copies,
                bank_lease_release_count: panel_copies,
                bank_lease_balance: 0,
                max_active_bank_leases: 1,
                yield_while_bank_lease_active_count: 0,
                harness_pause_while_bank_lease_active_count: 0,
                max_bank_lease_hold_mcycles: 128,
            },
            scheduling: SchedulingMetrics {
                frames_to_completion: tiles + 3,
                gated_frame_start: BRINGUP_GATED_FRAME_START,
                gated_frame_end_exclusive: tiles + BRINGUP_GATED_FRAME_START,
                gated_frame_count: tiles,
                vblank_count: tiles + 1,
                widget_update_count: tiles,
                scheduler_service_count: tiles,
                frame_service_misses: 0,
                max_no_progress_frames: 1,
                worst_case_interrupt_latency_mcycles: Some(40),
                cycles_remaining_at_widget_tick: Some(dist(17_000)),
                cycles_remaining_at_scheduler_service: Some(dist(16_900)),
            },
            conformance: Conformance {
                byte_exact_match: true,
                reference: "gbf-verify::matmul_reference_i8".to_owned(),
                reference_source_hash: fake_hash(10),
                fixture_hash: fake_hash(11),
                expected_output_hash: fake_hash(12),
            },
        }
    }

    fn dist(mean: u64) -> Distribution {
        Distribution {
            sample_count: 1,
            min: mean,
            mean: mean as f64,
            max: mean,
            p99: mean,
        }
    }

    fn fake_hash(byte: u8) -> String {
        format!("sha256:{}", hex_byte(byte).repeat(32))
    }

    fn hex_byte(byte: u8) -> String {
        format!("{byte:02x}")
    }

    #[test]
    fn realism_report_v1_accepts_checked_fixture() {
        fixture_report().validate_checked().expect("valid");
    }

    #[test]
    fn realism_report_v1_schema_accepts_fixture() {
        realism_report_v1_accepts_checked_fixture();
    }

    #[test]
    fn realism_report_v1_rejects_missing_required_fields() {
        let json = fixture_report().to_canonical_json().expect("json");
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("value");
        value.as_object_mut().expect("object").remove("runs");
        assert!(serde_json::from_value::<RealismReportV1>(value).is_err());
    }

    #[test]
    fn realism_report_v1_rejects_structural_count_mismatch() {
        let mut report = fixture_report();
        report.runs[0].structural_counts.products += 1;
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::StructuralCount {
                field: "products",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_rejects_zero_sample_count_in_checked_report() {
        let mut report = fixture_report();
        report.runs[0]
            .compute_costs
            .cycles_per_product_accumulate
            .sample_count = 0;
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::ZeroSampleCount {
                field: "cycles_per_product_accumulate"
            })
        ));
    }

    #[test]
    fn realism_report_v1_rejects_unpinned_runtime_knob() {
        let mut report = fixture_report();
        report.runtime_knobs.yield_quantum = "KLaneFullTile".to_owned();
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::BadRuntimeKnob {
                field: "yield_quantum",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_rejects_wrong_multiply_kernel() {
        let mut report = fixture_report();
        report.workload.operand_layout.multiply_kernel = "signed_repeated_add_i8_i32".to_owned();
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::BadRuntimeKnob {
                field: "multiply_kernel",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_rejects_unpinned_reproducibility() {
        let mut report = fixture_report();
        report.reproducibility.git_dirty = true;
        report.reproducibility.regenerated_from_pinned_inputs = false;
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::BadReproducibility {
                field: "git_dirty",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_checked_artifact_validates() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("docs/review/f-b1/artifacts/realism_report.v1.json");
        let text = std::fs::read_to_string(&path).expect("checked F-B1 report exists");
        let report: RealismReportV1 = serde_json::from_str(&text).expect("report decodes");
        report.validate_checked().expect("checked report validates");
    }

    #[test]
    fn realism_report_v1_rejects_bank0_table_reads_for_fixed_kernel() {
        let mut report = fixture_report();
        report.runs[0].memory_bandwidth.rom_bank0_table_bytes_read = 1024;
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::StructuralCount {
                field: "rom_bank0_table_bytes_read",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_rejects_self_owned_gated_frame_count() {
        let mut report = fixture_report();
        report.runs[0].scheduling.gated_frame_count = 0;
        report.runs[0].scheduling.widget_update_count = 0;
        report.runs[0].scheduling.scheduler_service_count = 0;
        report = report.with_computed_self_hash().expect("hash");
        assert!(matches!(
            report.validate_checked(),
            Err(RealismReportError::FrameGate {
                field: "gated_frame_count",
                ..
            })
        ));
    }

    #[test]
    fn realism_report_v1_self_hash_round_trip() {
        let report = fixture_report();
        let json = report.to_canonical_json().expect("json");
        let decoded: RealismReportV1 = serde_json::from_str(&json).expect("decode");
        decoded.validate_checked().expect("hash validates");
    }

    #[test]
    fn explicit_json_shape_pins_downstream_fields() {
        let value = serde_json::to_value(fixture_report()).expect("value");
        assert_eq!(value["schema"], serde_json::json!("realism_report.v1"));
        assert_eq!(value["runs"][3]["n"], serde_json::json!(128));
        assert_eq!(
            value["runs"][3]["structural_counts"]["products"],
            serde_json::json!(2_097_152_u64)
        );
        assert_eq!(
            value["workload"]["operand_layout"]["a"],
            serde_json::json!({"bank": 1, "offset": 0})
        );
        assert_eq!(
            value["workload"]["operand_layout"]["multiply_kernel"],
            serde_json::json!("fixed_8_step_shift_add_i8_i32")
        );
        assert_eq!(
            value["runs"][3]["build_identity"]["multiply_kernel_hash"],
            serde_json::json!(fake_hash(9))
        );
    }
}
