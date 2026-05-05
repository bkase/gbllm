//! F-B1 bringup workload runner and report aggregation.

use std::collections::BTreeMap;

use gbf_abi::compute_shape::SquareDim;
use gbf_abi::harness::{HarnessCommandBlock, HarnessOp};
use gbf_codegen::f_b1::{
    ACCUMULATOR_TILE_BYTES, BringupRun, BringupStreamingRom, ComputeBringupRequest,
    EMITTED_MULTIPLY_KERNEL_ID, FB1_L0_OUTPUT_BASE, FrameEvent, FrameEventEnvelope, OperandKind,
    TileDump, YieldQuantum, build_l3_streaming_rom, materialize_operand_fixture, reassemble_tiles,
    run_bringup_model, tile_schedule, validate_structural_counts,
};
use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator,
    NormalizedTraceEvent, Predicate, RunOutcome, StepOutcome, TraceDropPolicy, TrapAction,
    TrapKind,
};
use gbf_foundation::Hash256;
use gbf_report::realism::{
    BankOffset, BankingMetrics, BuildIdentity, ComputeCosts, Conformance, Distribution,
    MemoryBandwidth, OperandLayoutReport, RealismReportError, RealismReportV1, RealismRun,
    Reproducibility, RomLayout, RuntimeKnobs, SchedulingMetrics, StructuralCounts, TileShape,
    ToolchainIdentity, Workload, ZERO_SELF_HASH,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REVIEW_PACKET_PINNED_INPUTS_ID: &str = "f-b1-review-packet-pinned-inputs-v1";
const FRAME_M_CYCLES: u64 = gbf_hw::timing::FRAME_M_CYCLES as u64;
const GATED_FRAME_START: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatmulI8BringupWorkload {
    pub run_order: Vec<u16>,
    pub yield_quantum: YieldQuantum,
}

impl Default for MatmulI8BringupWorkload {
    fn default() -> Self {
        Self {
            run_order: vec![32, 64, 96, 128],
            yield_quantum: YieldQuantum::KLaneRow,
        }
    }
}

impl MatmulI8BringupWorkload {
    pub fn run(&self) -> Result<Vec<BringupRun>, RealismReportError> {
        self.run_emulated()
    }

    pub fn run_reference_model(&self) -> Result<Vec<BringupRun>, RealismReportError> {
        self.run_order
            .iter()
            .copied()
            .map(|n| {
                let request = ComputeBringupRequest {
                    matrix_dim: SquareDim::new(n).expect("F-B1 sweep sizes are valid"),
                    yield_quantum: self.yield_quantum,
                    ..ComputeBringupRequest::headline_n128()
                };
                run_bringup_model(&request).map_err(|error| RealismReportError::Json {
                    reason: error.to_string(),
                })
            })
            .collect()
    }

    pub fn run_emulated(&self) -> Result<Vec<BringupRun>, RealismReportError> {
        self.run_order
            .iter()
            .copied()
            .map(|n| {
                let request = ComputeBringupRequest {
                    matrix_dim: SquareDim::new(n).expect("F-B1 sweep sizes are valid"),
                    yield_quantum: self.yield_quantum,
                    ..ComputeBringupRequest::headline_n128()
                };
                run_emulated_request(&request)
            })
            .collect()
    }

    pub fn report(&self) -> Result<RealismReportV1, RealismReportError> {
        let runs = self.run()?;
        self.report_from_runs(&runs)
    }

    pub fn report_from_runs(
        &self,
        runs: &[BringupRun],
    ) -> Result<RealismReportV1, RealismReportError> {
        let reference_runs = self.run_reference_model()?;
        if reference_runs.len() != runs.len() {
            return Err(RealismReportError::Json {
                reason: format!(
                    "reference run count {} != emulated run count {}",
                    reference_runs.len(),
                    runs.len()
                ),
            });
        }
        let byte_exact_matches: Vec<bool> = runs
            .iter()
            .zip(reference_runs.iter())
            .map(|(run, reference)| {
                run.dim == reference.dim && run.output_i32 == reference.output_i32
            })
            .collect();
        self.report_from_runs_with_conformance(runs, &byte_exact_matches)
    }

    pub fn report_from_runs_with_conformance(
        &self,
        runs: &[BringupRun],
        byte_exact_matches: &[bool],
    ) -> Result<RealismReportV1, RealismReportError> {
        if byte_exact_matches.len() != runs.len() {
            return Err(RealismReportError::Json {
                reason: format!(
                    "byte-exact match count {} != run count {}",
                    byte_exact_matches.len(),
                    runs.len()
                ),
            });
        }
        for run in runs {
            validate_structural_counts(&run.metrics, run.dim).map_err(|reason| {
                RealismReportError::Json {
                    reason: format!("structural count gate failed: {reason}"),
                }
            })?;
        }
        let reproducibility = reproducibility_from_compile_env();
        let report = RealismReportV1 {
            schema: gbf_report::realism::REALISM_REPORT_SCHEMA.to_owned(),
            headline_n: 128,
            run_order: self.run_order.clone(),
            toolchain_identity: ToolchainIdentity {
                rustc: option_env!("RUSTC_VERSION")
                    .unwrap_or("rustc pinned-by-toolchain")
                    .to_owned(),
                host_triple: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
                source_date_epoch: 0,
                target_profile_hash: hash_string(gbf_hw::target::BRINGUP_TARGET_PROFILE_ID),
                emulator_adapter_hash: hash_string("gbf-emu::f-b1-bringup-adapter"),
                gameroy_core_hash: hash_string(
                    "gameroy-core:a5acdc921c0561ed93a077622b598df0e068583c",
                ),
            },
            reproducibility,
            workload: Workload {
                kind: "MatmulI8Bringup".to_owned(),
                sizes: self.run_order.clone(),
                tile: TileShape {
                    m: 16,
                    n: 16,
                    k: 16,
                },
                operand_fixture: "DeterministicAffineV1".to_owned(),
                operand_layout: OperandLayoutReport {
                    a: BankOffset { bank: 1, offset: 0 },
                    b: BankOffset { bank: 2, offset: 0 },
                    multiply_kernel: EMITTED_MULTIPLY_KERNEL_ID.to_owned(),
                },
            },
            runtime_knobs: RuntimeKnobs {
                yield_quantum: format!("{:?}", self.yield_quantum),
                soft_deadline_margin_mcycles: gbf_runtime::scheduler::SchedulerPolicy::bring_up()
                    .soft_deadline_margin,
                scheduler_profile: "Bringup".to_owned(),
            },
            runs: runs
                .iter()
                .zip(byte_exact_matches.iter().copied())
                .map(|(run, byte_exact_match)| realism_run_from_bringup(run, byte_exact_match))
                .collect(),
        }
        .with_computed_self_hash()?;
        report.validate_checked()?;
        Ok(report)
    }
}

fn realism_run_from_bringup(run: &BringupRun, byte_exact_match: bool) -> RealismRun {
    let n = run.dim.n();
    let metrics = &run.metrics;
    let request_hash = ComputeBringupRequest {
        matrix_dim: run.dim,
        ..ComputeBringupRequest::headline_n128()
    }
    .hash()
    .expect("valid request hash");
    let output_hash = hash_bytes(&run.output_bytes_le());
    let a = materialize_operand_fixture(
        gbf_codegen::f_b1::OperandFixtureSpec::DeterministicAffineV1,
        OperandKind::A,
        run.dim,
    );
    let b = materialize_operand_fixture(
        gbf_codegen::f_b1::OperandFixtureSpec::DeterministicAffineV1,
        OperandKind::B,
        run.dim,
    );
    let mut fixture_bytes = a;
    fixture_bytes.extend_from_slice(&b);
    RealismRun {
        n,
        build_identity: BuildIdentity {
            build_hash: hash_string("f-b1-build"),
            runtime_nucleus_hash: format!(
                "sha256:{}",
                gbf_runtime::compute_runtime_nucleus_hash_for_test()
            ),
            compute_bringup_request_hash: format!("sha256:{request_hash}"),
            asm_ir_hash: hash_string("f-b1-asm-ir"),
            rom_sha256: run
                .rom_sha256
                .clone()
                .unwrap_or_else(|| hash_string(&format!("f-b1-reference-model-n{n}"))),
            operand_fixture_hash: hash_bytes(&fixture_bytes),
            multiply_kernel_hash: hash_string(EMITTED_MULTIPLY_KERNEL_ID),
        },
        structural_counts: StructuralCounts {
            products: metrics.products,
            output_tiles: metrics.output_tiles,
            k_tiles_per_output_tile: metrics.k_tiles_per_output_tile,
            operand_panel_copies: metrics.operand_panel_copies,
            operand_panel_bytes_copied: metrics.operand_panel_bytes_copied,
            full_output_bytes: metrics.full_output_bytes,
        },
        compute_costs: ComputeCosts {
            cycles_per_product_accumulate: dist(
                metrics.cycles_per_product_sample_count,
                metrics
                    .cycles_per_full_matmul_mcycles
                    .div_ceil(metrics.products),
            ),
            cycles_per_output_tile: dist(
                metrics.output_tiles,
                metrics
                    .cycles_per_full_matmul_mcycles
                    .div_ceil(u64::from(metrics.output_tiles)),
            ),
            cycles_per_full_matmul: metrics.cycles_per_full_matmul_mcycles,
            max_unyielded_compute_mcycles: metrics.max_unyielded_compute_mcycles,
            wall_clock_seconds_at_gb_clock_f64: metrics.cycles_per_full_matmul_mcycles as f64
                / f64::from(gbf_hw::timing::NORMAL_M_CYCLES_PER_SECOND),
        },
        memory_bandwidth: MemoryBandwidth {
            rom_bank0_table_bytes_read: 0,
            romx_operand_bytes_read: u64::from(metrics.operand_panel_bytes_copied),
            effective_bank0_table_read_bandwidth_bytes_per_sec_f64: 0.0,
            effective_romx_operand_read_bandwidth_bytes_per_sec_f64: bytes_per_second(
                u64::from(metrics.operand_panel_bytes_copied),
                metrics.cycles_per_full_matmul_mcycles,
            ),
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
            logical_bank_switches_per_output_tile: metrics.k_tiles_per_output_tile * 2,
            logical_bank_switches_per_full_matmul: metrics.operand_panel_copies,
            mbc_rom_bank_register_write_count: metrics.operand_panel_copies,
            bank_lease_acquire_count: metrics.bank_lease_acquire_count,
            bank_lease_release_count: metrics.bank_lease_release_count,
            bank_lease_balance: metrics.bank_lease_balance,
            max_active_bank_leases: metrics.max_active_bank_leases,
            yield_while_bank_lease_active_count: metrics.yield_while_bank_lease_active_count,
            harness_pause_while_bank_lease_active_count: metrics
                .harness_pause_while_bank_lease_active_count,
            max_bank_lease_hold_mcycles: metrics.max_bank_lease_hold_mcycles,
        },
        scheduling: scheduling_metrics_from_bringup(run),
        conformance: Conformance {
            byte_exact_match,
            reference: "gbf-verify::matmul_reference_i8".to_owned(),
            reference_source_hash: hash_string("gbf-verify::matmul_reference_i8"),
            fixture_hash: hash_bytes(&fixture_bytes),
            expected_output_hash: output_hash,
        },
    }
}

fn scheduling_metrics_from_bringup(run: &BringupRun) -> SchedulingMetrics {
    let metrics = &run.metrics;
    let frames_to_completion = u32::try_from(
        metrics
            .cycles_per_full_matmul_mcycles
            .div_ceil(FRAME_M_CYCLES)
            .saturating_add(1),
    )
    .unwrap_or(u32::MAX);
    let gated_frame_start = GATED_FRAME_START;
    let gated_frame_end_exclusive = frames_to_completion
        .saturating_sub(1)
        .max(gated_frame_start);
    let gated_frame_count = gated_frame_end_exclusive.saturating_sub(gated_frame_start);
    let margins = service_margins(
        &run.frame_events,
        gated_frame_start,
        gated_frame_end_exclusive,
    );
    let margin_dist = dist_from_samples(&margins);

    SchedulingMetrics {
        frames_to_completion,
        gated_frame_start,
        gated_frame_end_exclusive,
        gated_frame_count,
        vblank_count: count_vblank_events(
            &run.frame_events,
            gated_frame_start,
            gated_frame_end_exclusive,
        ),
        widget_update_count: metrics.widget_update_count,
        scheduler_service_count: metrics.scheduler_service_count,
        frame_service_misses: metrics.frame_service_misses,
        max_no_progress_frames: metrics.max_no_progress_frames,
        worst_case_interrupt_latency_mcycles: Some(u32::from(
            gbf_runtime::scheduler::SchedulerPolicy::bring_up()
                .max_interrupt_entry_latency_m_cycles,
        )),
        cycles_remaining_at_widget_tick: margin_dist,
        cycles_remaining_at_scheduler_service: margin_dist,
    }
}

fn service_margins(
    events: &[FrameEventEnvelope],
    gated_frame_start: u32,
    gated_frame_end_exclusive: u32,
) -> Vec<u64> {
    let mut vblanks = BTreeMap::new();
    let mut services = BTreeMap::new();
    for envelope in events {
        match envelope.event {
            FrameEvent::VBlankFired {
                frame,
                mcycle_since_boot,
            } => {
                vblanks.insert(frame, mcycle_since_boot);
            }
            FrameEvent::WidgetTickDispatched {
                frame,
                mcycle_since_boot,
            } => {
                services.insert(frame, mcycle_since_boot);
            }
            _ => {}
        }
    }

    (gated_frame_start..gated_frame_end_exclusive)
        .filter_map(|frame| {
            let service_mcycle = services.get(&frame).copied()?;
            let next_vblank = vblanks.get(&frame.saturating_add(1)).copied()?;
            Some(next_vblank.saturating_sub(service_mcycle))
        })
        .collect()
}

fn count_vblank_events(
    events: &[FrameEventEnvelope],
    gated_frame_start: u32,
    gated_frame_end_exclusive: u32,
) -> u32 {
    let count = events
        .iter()
        .filter(|envelope| {
            matches!(
                envelope.event,
                FrameEvent::VBlankFired { frame, .. }
                    if frame >= gated_frame_start && frame <= gated_frame_end_exclusive
            )
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn run_emulated_request(request: &ComputeBringupRequest) -> Result<BringupRun, RealismReportError> {
    request
        .validate()
        .map_err(|error| RealismReportError::Json {
            reason: error.to_string(),
        })?;
    let dim = request.matrix_dim;
    let mut output_tiles =
        Vec::with_capacity(usize::from(dim.tiles_per_axis() * dim.tiles_per_axis()));
    let streaming_rom =
        build_l3_streaming_rom(request).map_err(|error| RealismReportError::Json {
            reason: error.to_string(),
        })?;
    let rom_sha256 = hash_bytes(&streaming_rom.rom);
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_capacity(8_192)
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&streaming_rom.rom)
        .map_err(emu_error)?;
    emu.traps().add_pc(
        streaming_rom.compute_yield_safe_point_pc,
        Predicate::Always,
        TrapAction::Continue,
    );
    emu.traps().add_pc(
        streaming_rom.copy_yield_safe_point_pc,
        Predicate::Always,
        TrapAction::Continue,
    );
    emu.traps().add_pc(
        streaming_rom.vblank_handler_pc,
        Predicate::Always,
        TrapAction::Continue,
    );
    let tile_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(2_000));
    let compute_start_mcycles = emu.m_cycle_count_floor().0;
    let mut last_checkpoint_mcycles = compute_start_mcycles;
    let mut max_checkpoint_gap_mcycles = 0_u64;
    let mut max_copy_checkpoint_gap_mcycles = 0_u64;
    let mut frame_tracker = FrameServiceTracker::new(compute_start_mcycles);

    for coord in tile_schedule(dim) {
        match emu
            .run_fast_until_pc(streaming_rom.tile_safe_point_pc, tile_budget)
            .map_err(emu_error)?
        {
            RunOutcome::TrapHit { .. } => {}
            other => {
                return Err(RealismReportError::Json {
                    reason: format!("streaming ROM did not reach tile safe point: {other:?}"),
                });
            }
        }
        for checkpoint in drain_yield_checkpoints(&mut emu, &streaming_rom)? {
            frame_tracker.observe_checkpoint(checkpoint);
            if checkpoint.kind == YieldCheckpointKind::VBlank {
                continue;
            }
            let gap = checkpoint.mcycle.saturating_sub(last_checkpoint_mcycles);
            max_checkpoint_gap_mcycles = max_checkpoint_gap_mcycles.max(gap);
            if checkpoint.kind == YieldCheckpointKind::Copy {
                max_copy_checkpoint_gap_mcycles = max_copy_checkpoint_gap_mcycles.max(gap);
            }
            last_checkpoint_mcycles = checkpoint.mcycle;
        }
        let now = emu.m_cycle_count_floor().0;
        let tile_gap = now.saturating_sub(last_checkpoint_mcycles);
        max_checkpoint_gap_mcycles = max_checkpoint_gap_mcycles.max(tile_gap);
        frame_tracker.observe_checkpoint(YieldCheckpoint {
            mcycle: now,
            kind: YieldCheckpointKind::TileSafePoint,
            completed_quantum_products: 0,
        });
        last_checkpoint_mcycles = now;
        let bytes = dump_arena_tile(&emu, u32::from(coord.tile_index).saturating_add(1))?;
        output_tiles.push(TileDump {
            tile_index: coord.tile_index,
            mt: coord.mt,
            nt: coord.nt,
            source_wram_addr: FB1_L0_OUTPUT_BASE,
            bytes,
        });
        match emu.step().map_err(emu_error)? {
            StepOutcome::Stepped { .. } => {}
            other => {
                return Err(RealismReportError::Json {
                    reason: format!("streaming ROM safe-point step did not advance: {other:?}"),
                });
            }
        }
    }

    match emu
        .run_fast_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
        .map_err(emu_error)?
    {
        RunOutcome::Idle { .. } | RunOutcome::BudgetElapsed { .. } => {}
        other => {
            return Err(RealismReportError::Json {
                reason: format!("streaming ROM did not halt after final tile: {other:?}"),
            });
        }
    }
    for checkpoint in drain_yield_checkpoints(&mut emu, &streaming_rom)? {
        frame_tracker.observe_checkpoint(checkpoint);
    }
    let end_mcycles = emu.m_cycle_count_floor().0;
    let compute_mcycles = end_mcycles.saturating_sub(compute_start_mcycles);
    let frame_summary = frame_tracker.finish(end_mcycles);

    let mut metrics = gbf_codegen::f_b1::BringupRunMetrics::structural(dim);
    metrics.cycles_per_full_matmul_mcycles = compute_mcycles;
    metrics.cycles_per_product_sample_count =
        u32::try_from(metrics.products).expect("F-B1 product count fits u32");
    metrics.max_unyielded_compute_mcycles =
        u32::try_from(max_checkpoint_gap_mcycles).unwrap_or(u32::MAX);
    metrics.max_bank_lease_hold_mcycles =
        u32::try_from(max_copy_checkpoint_gap_mcycles).unwrap_or(u32::MAX);
    metrics.widget_update_count = frame_summary.widget_update_count;
    metrics.scheduler_service_count = frame_summary.scheduler_service_count;
    metrics.frame_service_misses = frame_summary.frame_service_misses;
    metrics.max_no_progress_frames = frame_summary.max_no_progress_frames;

    let output_i32 = reassemble_tiles(dim, &output_tiles);
    Ok(BringupRun {
        dim,
        rom_sha256: Some(rom_sha256),
        output_i32,
        output_tiles,
        metrics,
        frame_events: frame_summary.events,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YieldCheckpointKind {
    Compute,
    Copy,
    VBlank,
    TileSafePoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct YieldCheckpoint {
    mcycle: u64,
    kind: YieldCheckpointKind,
    completed_quantum_products: u16,
}

fn drain_yield_checkpoints(
    emu: &mut Emulator,
    streaming_rom: &BringupStreamingRom,
) -> Result<Vec<YieldCheckpoint>, RealismReportError> {
    let mut checkpoints = Vec::new();
    for event in emu.drain_trace() {
        let NormalizedTraceEvent::TrapHit {
            kind: TrapKind::Pc { addr },
            cycle,
            ..
        } = event
        else {
            continue;
        };
        let Some(kind) = checkpoint_kind_for_pc(addr, streaming_rom) else {
            continue;
        };
        checkpoints.push(YieldCheckpoint {
            mcycle: cycle.as_m_cycles_floor().0,
            kind,
            completed_quantum_products: match kind {
                YieldCheckpointKind::Compute => YieldQuantum::KLaneRow.products_per_quantum(),
                YieldCheckpointKind::Copy
                | YieldCheckpointKind::VBlank
                | YieldCheckpointKind::TileSafePoint => 0,
            },
        });
    }
    Ok(checkpoints)
}

fn checkpoint_kind_for_pc(
    addr: u16,
    streaming_rom: &BringupStreamingRom,
) -> Option<YieldCheckpointKind> {
    if addr == streaming_rom.compute_yield_safe_point_pc {
        Some(YieldCheckpointKind::Compute)
    } else if addr == streaming_rom.copy_yield_safe_point_pc {
        Some(YieldCheckpointKind::Copy)
    } else if addr == streaming_rom.vblank_handler_pc {
        Some(YieldCheckpointKind::VBlank)
    } else {
        None
    }
}

#[derive(Debug)]
struct FrameServiceTracker {
    base_mcycle: u64,
    latest_vblank_frame: u32,
    last_serviced_frame: u32,
    progress_epoch: u32,
    records: Vec<FrameServiceRecord>,
}

impl FrameServiceTracker {
    fn new(base_mcycle: u64) -> Self {
        Self {
            base_mcycle,
            latest_vblank_frame: 0,
            last_serviced_frame: 0,
            progress_epoch: 0,
            records: Vec::new(),
        }
    }

    fn observe_checkpoint(&mut self, checkpoint: YieldCheckpoint) {
        match checkpoint.kind {
            YieldCheckpointKind::VBlank => self.push_vblank(checkpoint.mcycle),
            YieldCheckpointKind::Compute => {
                self.progress_epoch = self.progress_epoch.saturating_add(1);
                self.service_latest_frame(checkpoint);
            }
            YieldCheckpointKind::Copy | YieldCheckpointKind::TileSafePoint => {
                self.service_latest_frame(checkpoint);
            }
        }
    }

    fn finish(self, end_mcycle: u64) -> FrameServiceSummary {
        let compute_mcycles = end_mcycle.saturating_sub(self.base_mcycle);
        let frames_to_completion =
            u32::try_from(compute_mcycles.div_ceil(FRAME_M_CYCLES).saturating_add(1))
                .unwrap_or(u32::MAX);
        let gated_frame_start = GATED_FRAME_START;
        let gated_frame_end_exclusive = frames_to_completion
            .saturating_sub(1)
            .max(gated_frame_start);

        let mut by_frame = BTreeMap::new();
        for record in &self.records {
            by_frame.insert(record.frame, record);
        }

        let mut serviced_records = Vec::new();
        let mut frame_service_misses = 0_u32;
        let mut max_no_progress_frames = 0_u32;
        let mut no_progress_streak = 0_u32;
        let mut last_serviced_progress_epoch = 0_u32;
        for frame in gated_frame_start..gated_frame_end_exclusive {
            match by_frame.get(&frame).copied() {
                Some(record)
                    if frame_service_is_on_time(record, by_frame.get(&(frame + 1)).copied()) =>
                {
                    if record.progress_epoch > last_serviced_progress_epoch {
                        last_serviced_progress_epoch = record.progress_epoch;
                        no_progress_streak = 0;
                    } else {
                        no_progress_streak = no_progress_streak.saturating_add(1);
                    }
                    max_no_progress_frames = max_no_progress_frames.max(no_progress_streak);
                    serviced_records.push(*record);
                }
                _ => {
                    frame_service_misses = frame_service_misses.saturating_add(1);
                    no_progress_streak = no_progress_streak.saturating_add(1);
                    max_no_progress_frames = max_no_progress_frames.max(no_progress_streak);
                }
            }
        }

        let events = build_frame_events(
            self.base_mcycle,
            gated_frame_start,
            gated_frame_end_exclusive,
            &self.records,
            &serviced_records,
        );
        let widget_update_count = u32::try_from(serviced_records.len()).unwrap_or(u32::MAX);
        FrameServiceSummary {
            events,
            widget_update_count,
            scheduler_service_count: widget_update_count,
            frame_service_misses,
            max_no_progress_frames,
        }
    }

    fn push_vblank(&mut self, mcycle: u64) {
        self.latest_vblank_frame = self.latest_vblank_frame.saturating_add(1);
        self.records.push(FrameServiceRecord {
            frame: self.latest_vblank_frame,
            vblank_mcycle: Some(mcycle),
            service_mcycle: None,
            progress_epoch: self.progress_epoch,
            completed_quantum_products: 0,
        });
    }

    fn service_latest_frame(&mut self, checkpoint: YieldCheckpoint) {
        if self.latest_vblank_frame == 0 || self.latest_vblank_frame == self.last_serviced_frame {
            return;
        }
        if let Some(record) = self
            .records
            .iter_mut()
            .rev()
            .find(|record| record.frame == self.latest_vblank_frame)
        {
            record.service_mcycle = Some(checkpoint.mcycle);
            record.progress_epoch = self.progress_epoch;
            record.completed_quantum_products = checkpoint.completed_quantum_products;
            self.last_serviced_frame = self.latest_vblank_frame;
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FrameServiceRecord {
    frame: u32,
    vblank_mcycle: Option<u64>,
    service_mcycle: Option<u64>,
    progress_epoch: u32,
    completed_quantum_products: u16,
}

fn frame_service_is_on_time(
    record: &FrameServiceRecord,
    next_record: Option<&FrameServiceRecord>,
) -> bool {
    let (Some(vblank_mcycle), Some(service_mcycle), Some(next_vblank_mcycle)) = (
        record.vblank_mcycle,
        record.service_mcycle,
        next_record.and_then(|next| next.vblank_mcycle),
    ) else {
        return false;
    };
    vblank_mcycle < service_mcycle && service_mcycle < next_vblank_mcycle
}

#[derive(Debug)]
struct FrameServiceSummary {
    events: Vec<FrameEventEnvelope>,
    widget_update_count: u32,
    scheduler_service_count: u32,
    frame_service_misses: u32,
    max_no_progress_frames: u32,
}

#[derive(Debug)]
struct RawFrameEvent {
    mcycle: u64,
    order: u8,
    event: FrameEvent,
}

fn build_frame_events(
    base_mcycle: u64,
    gated_frame_start: u32,
    gated_frame_end_exclusive: u32,
    all_records: &[FrameServiceRecord],
    serviced_records: &[FrameServiceRecord],
) -> Vec<FrameEventEnvelope> {
    let mut raw = Vec::with_capacity(all_records.len() + serviced_records.len() * 4);
    for record in all_records {
        if (gated_frame_start..=gated_frame_end_exclusive).contains(&record.frame)
            && let Some(vblank_mcycle) = record.vblank_mcycle
        {
            raw.push(RawFrameEvent {
                mcycle: vblank_mcycle,
                order: 0,
                event: FrameEvent::VBlankFired {
                    frame: record.frame,
                    mcycle_since_boot: vblank_mcycle,
                },
            });
        }
    }
    let mut last_progress_epoch_seen = 0_u32;
    for record in serviced_records {
        let Some(service_mcycle) = record.service_mcycle else {
            continue;
        };
        let progressed = record.progress_epoch > last_progress_epoch_seen;
        if progressed {
            last_progress_epoch_seen = record.progress_epoch;
        }
        raw.push(RawFrameEvent {
            mcycle: service_mcycle,
            order: 1,
            event: FrameEvent::WidgetTickDispatched {
                frame: record.frame,
                mcycle_since_boot: service_mcycle,
            },
        });
        raw.push(RawFrameEvent {
            mcycle: service_mcycle,
            order: 2,
            event: FrameEvent::SchedulerServicedFrame {
                frame: record.frame,
                mcycle_since_boot: service_mcycle,
            },
        });
        raw.push(RawFrameEvent {
            mcycle: service_mcycle,
            order: 3,
            event: FrameEvent::YieldReturnedToScheduler {
                frame: record.frame,
                mcycle_since_boot: service_mcycle,
                remaining_frame_mcycles_i32: remaining_frame_mcycles(
                    base_mcycle,
                    record.frame,
                    service_mcycle,
                ),
                completed_quantum_products: record.completed_quantum_products,
                compute_progress_epoch: record.progress_epoch,
            },
        });
        if progressed {
            raw.push(RawFrameEvent {
                mcycle: service_mcycle,
                order: 4,
                event: FrameEvent::ComputeProgressEpochAdvanced {
                    frame: record.frame,
                    mcycle_since_boot: service_mcycle,
                    compute_progress_epoch: record.progress_epoch,
                },
            });
        }
    }
    raw.sort_by_key(|event| (event.mcycle, event.order));
    raw.into_iter()
        .enumerate()
        .map(|(seq, raw)| FrameEventEnvelope {
            seq: u64::try_from(seq).expect("frame event seq fits u64"),
            event: raw.event,
        })
        .collect()
}

fn frame_start(base_mcycle: u64, frame: u32) -> u64 {
    base_mcycle.saturating_add(u64::from(frame) * FRAME_M_CYCLES)
}

fn remaining_frame_mcycles(base_mcycle: u64, frame: u32, service_mcycle: u64) -> i32 {
    let next = frame_start(base_mcycle, frame.saturating_add(1));
    i32::try_from(next as i128 - service_mcycle as i128).unwrap_or(if service_mcycle <= next {
        i32::MAX
    } else {
        i32::MIN
    })
}

fn reproducibility_from_compile_env() -> Reproducibility {
    let pinned_git_sha = option_env!("GIT_SHA").filter(|value| !value.is_empty());
    let git_sha = pinned_git_sha
        .unwrap_or(REVIEW_PACKET_PINNED_INPUTS_ID)
        .to_owned();
    let git_dirty = option_env!("GIT_DIRTY").is_some_and(compile_env_bool);
    Reproducibility {
        git_sha,
        git_dirty,
        report_self_hash: ZERO_SELF_HASH.to_owned(),
        regenerated_from_pinned_inputs: !git_dirty,
    }
}

fn compile_env_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "dirty")
}

fn dump_arena_tile(emu: &Emulator, seq: u32) -> Result<Vec<u8>, RealismReportError> {
    let command = dump_arena_command(seq, FB1_L0_OUTPUT_BASE, ACCUMULATOR_TILE_BYTES);
    if command.decode_op().map_err(emu_error)? != HarnessOp::DumpArena {
        return Err(RealismReportError::Json {
            reason: "F-A3 DumpArena command decoded as a different op".to_owned(),
        });
    }
    let (addr, len) = decode_dump_arena_args(command.args)?;
    emu.peek_range(addr, usize::from(len)).map_err(emu_error)
}

fn dump_arena_command(seq: u32, addr: u16, len: u16) -> HarnessCommandBlock {
    let mut args = [0_u8; 32];
    args[0..2].copy_from_slice(&addr.to_le_bytes());
    args[2..4].copy_from_slice(&len.to_le_bytes());
    let mut command = HarnessCommandBlock::new(seq, HarnessOp::DumpArena, args);
    command.raise_doorbell();
    command
}

fn decode_dump_arena_args(args: [u8; 32]) -> Result<(u16, u16), RealismReportError> {
    let addr = u16::from_le_bytes([args[0], args[1]]);
    let len = u16::from_le_bytes([args[2], args[3]]);
    if addr != FB1_L0_OUTPUT_BASE || len != ACCUMULATOR_TILE_BYTES {
        return Err(RealismReportError::Json {
            reason: format!(
                "unsupported F-B1 DumpArena request addr={addr:#06x} len={len}; expected addr={:#06x} len={}",
                FB1_L0_OUTPUT_BASE, ACCUMULATOR_TILE_BYTES
            ),
        });
    }
    Ok((addr, len))
}

fn bytes_per_second(bytes: u64, mcycles: u64) -> f64 {
    if mcycles == 0 {
        return 0.0;
    }
    bytes as f64 * f64::from(gbf_hw::timing::NORMAL_M_CYCLES_PER_SECOND) / mcycles as f64
}

fn emu_error(error: impl std::fmt::Display) -> RealismReportError {
    RealismReportError::Json {
        reason: error.to_string(),
    }
}

fn dist(sample_count: u32, value: u64) -> Distribution {
    Distribution {
        sample_count,
        min: value,
        mean: value as f64,
        max: value,
        p99: value,
    }
}

fn dist_from_samples(samples: &[u64]) -> Option<Distribution> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let sum = sorted.iter().copied().sum::<u64>();
    let p99_index = ((sorted.len() * 99).div_ceil(100)).saturating_sub(1);
    Some(Distribution {
        sample_count: u32::try_from(sorted.len()).unwrap_or(u32::MAX),
        min: sorted[0],
        mean: sum as f64 / sorted.len() as f64,
        max: sorted[sorted.len() - 1],
        p99: sorted[p99_index],
    })
}

fn hash_string(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hash_raw(bytes))
}

fn hash_raw(bytes: &[u8]) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Hash256::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_b1_structural_counts_formula_n128() {
        let metrics =
            gbf_codegen::f_b1::BringupRunMetrics::structural(SquareDim::new(128).expect("valid"));
        assert_eq!(metrics.products, 2_097_152);
        assert_eq!(metrics.output_tiles, 64);
        assert_eq!(metrics.k_tiles_per_output_tile, 8);
        assert_eq!(metrics.operand_panel_copies, 1024);
        assert_eq!(metrics.operand_panel_bytes_copied, 262_144);
    }

    #[test]
    #[ignore = "heavy F-B1 report generation is owned by scripts/review/f-b1/regen.sh"]
    fn f_b1_report_aggregation_known_fixture() {
        let report = MatmulI8BringupWorkload::default().report().expect("report");
        assert_eq!(report.runs.len(), 4);
        assert_eq!(report.runs[3].n, 128);
        assert_eq!(report.runs[3].structural_counts.products, 2_097_152);
    }

    #[test]
    fn f_b1_tile_reassembly_round_trip() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            ..ComputeBringupRequest::headline_n128()
        };
        let run = run_bringup_model(&request).expect("run");
        let reassembled = gbf_codegen::f_b1::reassemble_tiles(run.dim, &run.output_tiles);
        assert_eq!(reassembled, run.output_i32);
    }

    #[test]
    fn f_b1_dump_arena_command_pins_existing_harness_op() {
        let command = dump_arena_command(7, FB1_L0_OUTPUT_BASE, ACCUMULATOR_TILE_BYTES);
        assert_eq!(
            command.decode_op().expect("op decodes"),
            HarnessOp::DumpArena
        );
        assert_eq!(
            decode_dump_arena_args(command.args).expect("args decode"),
            (FB1_L0_OUTPUT_BASE, ACCUMULATOR_TILE_BYTES)
        );
    }

    #[test]
    fn f_b1_emulated_partial_run_matches_reference_n32() {
        let workload = MatmulI8BringupWorkload {
            run_order: vec![32],
            yield_quantum: YieldQuantum::KLaneRow,
        };
        let run = workload
            .run_emulated()
            .expect("emulated run")
            .pop()
            .expect("n32 run");
        let reference = workload
            .run_reference_model()
            .expect("reference run")
            .pop()
            .expect("n32 reference");
        assert_eq!(run.output_i32, reference.output_i32);
        assert_eq!(
            gbf_codegen::f_b1::reassemble_tiles(run.dim, &run.output_tiles),
            run.output_i32
        );
    }

    #[test]
    fn f_b1_l4_emulated_partial_run_services_frames_n32() {
        let workload = MatmulI8BringupWorkload {
            run_order: vec![32],
            yield_quantum: YieldQuantum::KLaneRow,
        };
        let run = workload
            .run_emulated()
            .expect("emulated run")
            .pop()
            .expect("n32 run");
        let deadline = gbf_hw::timing::FRAME_M_CYCLES
            - gbf_runtime::scheduler::SchedulerPolicy::bring_up().soft_deadline_margin;
        assert_eq!(run.metrics.frame_service_misses, 0);
        assert_eq!(
            run.metrics.widget_update_count,
            run.metrics.scheduler_service_count
        );
        assert!(run.metrics.widget_update_count > 0);
        assert!(run.metrics.max_no_progress_frames <= 1);
        assert!(run.metrics.max_unyielded_compute_mcycles <= deadline);
        assert!(run.metrics.max_bank_lease_hold_mcycles <= deadline);
    }
}
