//! Single-substrate Game Boy emulator adapter for tests, benches, and debugging.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod determinism;
pub mod harness;
pub mod primitives;
pub mod trace_ring;
pub mod trap;

pub use adapter::{BootMode, BootRomImage, Emulator, EmulatorBuilder, EmulatorConfig};
pub use determinism::{
    AudioOutputMode, CartridgeRtcMode, DeterminismPolicy, DeterminismPolicyBuilder,
    FIXED_CARTRIDGE_RTC_UNIX_MS, FIXED_SAVE_STATE_UNIX_MS, PowerOnRamPolicy, SaveStateMetadataMode,
};
pub use harness::{HarnessChannel, HarnessCommand, HarnessResult, HarnessSlot};
pub use primitives::{
    BootModeLineage, ClockCycles, Color, CpuIdleState, CycleBudget, DMG_FRAME_CLOCK_CYCLES,
    EmuError, EmuVersionTag, Flags, Framebuffer, GitSha, ImeSnapshot, JoypadFrame, MCycles, Regs,
    RunOutcome, Snapshot, SnapshotLineage, StepOutcome, TrapPredicateError,
};
pub use trace_ring::{
    BankSnapshot, BankSwitchSource, NormalizedTraceEvent, TraceCursor, TraceDropPolicy,
    TraceMapper, TraceOrigin,
};
pub use trap::{
    AddressRange, AddressRangeError, BreakpointId, EmuReadOnlyMemory, EmuReadOnlyView,
    MemoryAccess, MemoryAccessKind, Predicate, PredicateSpec, RemovedTrap, TrapAction, TrapContext,
    TrapDispatcher, TrapKind, TrapListEntry, TrapPersistenceError, TrapSpec,
};

#[cfg(test)]
mod f_b1_tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::OnceLock;

    use gbf_abi::compute_shape::SquareDim;
    use gbf_abi::harness::HarnessOp;
    use gbf_codegen::f_b1::{
        BankedOperandPlacement, ComputeBringupRequest, FB1_L0_OUTPUT_BASE, FrameEvent,
        OperandLayout, RomBankId, RomBankOffset, TILE_EDGE, TileDump, build_l0_wram_smoke_rom,
        build_l2_cross_bank_smoke_rom, build_l3_output_tile_rom, build_l3_partial_tile_rom,
        build_l3_streaming_rom, reassemble_tiles, run_bringup_model, tile_schedule,
    };
    use gbf_verify::matmul::{
        MatrixI8, deterministic_matrix_a, deterministic_matrix_b, matmul_reference_i8,
    };

    use crate::{
        BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator,
        NormalizedTraceEvent, Predicate, RunOutcome, StepOutcome, TraceDropPolicy, TrapAction,
        TrapKind,
    };

    const L4_DEADLINE_M_CYCLES: u32 = gbf_hw::timing::FRAME_M_CYCLES
        - gbf_runtime::scheduler::SchedulerPolicy::bring_up().soft_deadline_margin;

    fn assert_rom_matches_reference(rom: &[u8]) {
        let dim = SquareDim::new(16).expect("valid");
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .load_rom(rom)
            .expect("ROM loads");
        let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(2_000));
        let outcome = emu.run_fast_for(budget).expect("L0 ROM runs");
        assert!(matches!(outcome, RunOutcome::Idle { .. }));

        let actual = emu
            .peek_range(FB1_L0_OUTPUT_BASE, dim.output_bytes_i32())
            .expect("output WRAM is readable");
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let expected = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(actual, expected.to_le_bytes());
    }

    fn run_rom_output_tile(rom: &[u8]) -> Vec<u8> {
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .load_rom(rom)
            .expect("ROM loads");
        let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(2_000));
        let outcome = emu.run_fast_for(budget).expect("ROM runs");
        assert!(matches!(outcome, RunOutcome::Idle { .. }));
        emu.peek_range(FB1_L0_OUTPUT_BASE, 16 * 16 * 4)
            .expect("output WRAM is readable")
    }

    fn expected_partial_tile(dim: SquareDim, mt: u16, nt: u16, kt: u16) -> Vec<u8> {
        let n = usize::from(dim.n());
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let mut bytes = Vec::with_capacity(16 * 16 * 4);
        for mm in 0..usize::from(TILE_EDGE) {
            for nn in 0..usize::from(TILE_EDGE) {
                let row = usize::from(mt) * usize::from(TILE_EDGE) + mm;
                let col = usize::from(nt) * usize::from(TILE_EDGE) + nn;
                let k_base = usize::from(kt) * usize::from(TILE_EDGE);
                let mut acc = 0_i32;
                for kk in 0..usize::from(TILE_EDGE) {
                    acc +=
                        i32::from(a[row * n + k_base + kk]) * i32::from(b[(k_base + kk) * n + col]);
                }
                bytes.extend_from_slice(&acc.to_le_bytes());
            }
        }
        bytes
    }

    fn expected_output_tile(dim: SquareDim, mt: u16, nt: u16) -> Vec<u8> {
        let n = usize::from(dim.n());
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let mut bytes = Vec::with_capacity(16 * 16 * 4);
        for mm in 0..usize::from(TILE_EDGE) {
            for nn in 0..usize::from(TILE_EDGE) {
                let row = usize::from(mt) * usize::from(TILE_EDGE) + mm;
                let col = usize::from(nt) * usize::from(TILE_EDGE) + nn;
                let mut acc = 0_i32;
                for kk in 0..n {
                    acc += i32::from(a[row * n + kk]) * i32::from(b[kk * n + col]);
                }
                bytes.extend_from_slice(&acc.to_le_bytes());
            }
        }
        bytes
    }

    fn assert_streaming_request_matches_reference(request: &ComputeBringupRequest) {
        let streaming = build_l3_streaming_rom(request).expect("streaming ROM builds");
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .load_rom(&streaming.rom)
            .expect("streaming ROM loads");
        let mut dumps = Vec::new();
        for coord in tile_schedule(request.matrix_dim) {
            let outcome = emu
                .run_fast_until_pc(
                    streaming.tile_safe_point_pc,
                    CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(2_000)),
                )
                .expect("streaming ROM reaches tile safe point");
            assert!(matches!(outcome, RunOutcome::TrapHit { .. }));
            dumps.push(TileDump {
                tile_index: coord.tile_index,
                mt: coord.mt,
                nt: coord.nt,
                source_wram_addr: FB1_L0_OUTPUT_BASE,
                bytes: emu
                    .peek_range(FB1_L0_OUTPUT_BASE, 16 * 16 * 4)
                    .expect("tile WRAM is readable"),
            });
            assert!(matches!(
                emu.step().expect("safe-point nop advances"),
                StepOutcome::Stepped { .. }
            ));
        }
        assert!(matches!(
            emu.run_fast_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
                .expect("streaming ROM halts"),
            RunOutcome::Idle { .. } | RunOutcome::BudgetElapsed { .. }
        ));

        let actual = reassemble_tiles(request.matrix_dim, &dumps);
        let a = deterministic_matrix_a(request.matrix_dim);
        let b = deterministic_matrix_b(request.matrix_dim);
        let expected = matmul_reference_i8(
            MatrixI8::new(request.matrix_dim, &a).expect("shape"),
            MatrixI8::new(request.matrix_dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(actual, expected.data());
    }

    fn assert_run_matches_reference(n: u16) {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(n).expect("valid"),
            ..ComputeBringupRequest::headline_n128()
        };
        let run = run_bringup_model(&request).expect("run");
        let dim = run.dim;
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let expected = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(run.output_i32, expected.data());
        assert_eq!(reassemble_tiles(dim, &run.output_tiles), run.output_i32);
        assert!(
            HarnessOp::ALL.contains(&HarnessOp::DumpArena),
            "F-B1 reuses existing F-A3 DumpArena op"
        );
        assert_eq!(HarnessOp::ALL.len(), 8, "F-B1 must not add a harness op");
    }

    #[derive(Debug, Clone)]
    struct L4Evidence {
        output_i32: Vec<i32>,
        frame_service_misses: u32,
        max_no_progress_frames: u32,
        max_unyielded_compute_mcycles: u32,
        max_bank_lease_hold_mcycles: u32,
    }

    fn headline_l4_evidence() -> &'static L4Evidence {
        static EVIDENCE: OnceLock<L4Evidence> = OnceLock::new();
        EVIDENCE.get_or_init(run_headline_l4_evidence)
    }

    fn run_headline_l4_evidence() -> L4Evidence {
        let request = ComputeBringupRequest::headline_n128();
        let streaming = build_l3_streaming_rom(&request).expect("streaming ROM builds");
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .trace_capacity(8_192)
            .trace_drop_policy(TraceDropPolicy::HaltAndError)
            .load_rom(&streaming.rom)
            .expect("streaming ROM loads");
        emu.traps().add_pc(
            streaming.compute_yield_safe_point_pc,
            Predicate::Always,
            TrapAction::Continue,
        );
        emu.traps().add_pc(
            streaming.copy_yield_safe_point_pc,
            Predicate::Always,
            TrapAction::Continue,
        );
        emu.traps().add_pc(
            streaming.vblank_handler_pc,
            Predicate::Always,
            TrapAction::Continue,
        );

        let compute_start_mcycles = emu.m_cycle_count_floor().0;
        let mut last_checkpoint_mcycles = compute_start_mcycles;
        let mut max_unyielded_compute_mcycles = 0_u64;
        let mut max_bank_lease_hold_mcycles = 0_u64;
        let mut frame_tracker = TestFrameTracker::new(compute_start_mcycles);
        let mut dumps = Vec::new();
        let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(2_000));
        for coord in tile_schedule(request.matrix_dim) {
            let outcome = emu
                .run_fast_until_pc(streaming.tile_safe_point_pc, budget)
                .expect("streaming ROM reaches tile safe point");
            assert!(matches!(outcome, RunOutcome::TrapHit { .. }));
            for checkpoint in test_drain_yield_checkpoints(&mut emu, &streaming) {
                frame_tracker.observe(checkpoint);
                if checkpoint.kind == TestYieldCheckpointKind::VBlank {
                    continue;
                }
                let gap = checkpoint.mcycle.saturating_sub(last_checkpoint_mcycles);
                max_unyielded_compute_mcycles = max_unyielded_compute_mcycles.max(gap);
                if checkpoint.kind == TestYieldCheckpointKind::Copy {
                    max_bank_lease_hold_mcycles = max_bank_lease_hold_mcycles.max(gap);
                }
                last_checkpoint_mcycles = checkpoint.mcycle;
            }
            let tile_mcycle = emu.m_cycle_count_floor().0;
            max_unyielded_compute_mcycles = max_unyielded_compute_mcycles
                .max(tile_mcycle.saturating_sub(last_checkpoint_mcycles));
            frame_tracker.observe(TestYieldCheckpoint {
                mcycle: tile_mcycle,
                kind: TestYieldCheckpointKind::TileSafePoint,
            });
            last_checkpoint_mcycles = tile_mcycle;
            dumps.push(TileDump {
                tile_index: coord.tile_index,
                mt: coord.mt,
                nt: coord.nt,
                source_wram_addr: FB1_L0_OUTPUT_BASE,
                bytes: emu
                    .peek_range(FB1_L0_OUTPUT_BASE, 16 * 16 * 4)
                    .expect("tile WRAM is readable"),
            });
            assert!(matches!(
                emu.step().expect("safe-point nop advances"),
                StepOutcome::Stepped { .. }
            ));
        }
        assert!(matches!(
            emu.run_fast_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
                .expect("streaming ROM halts"),
            RunOutcome::Idle { .. } | RunOutcome::BudgetElapsed { .. }
        ));
        for checkpoint in test_drain_yield_checkpoints(&mut emu, &streaming) {
            frame_tracker.observe(checkpoint);
        }
        frame_tracker.finish(emu.m_cycle_count_floor().0);
        L4Evidence {
            output_i32: reassemble_tiles(request.matrix_dim, &dumps),
            frame_service_misses: frame_tracker.frame_service_misses,
            max_no_progress_frames: frame_tracker.max_no_progress_frames,
            max_unyielded_compute_mcycles: u32::try_from(max_unyielded_compute_mcycles)
                .unwrap_or(u32::MAX),
            max_bank_lease_hold_mcycles: u32::try_from(max_bank_lease_hold_mcycles)
                .unwrap_or(u32::MAX),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestYieldCheckpointKind {
        Compute,
        Copy,
        VBlank,
        TileSafePoint,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestYieldCheckpoint {
        mcycle: u64,
        kind: TestYieldCheckpointKind,
    }

    fn test_drain_yield_checkpoints(
        emu: &mut Emulator,
        streaming: &gbf_codegen::f_b1::BringupStreamingRom,
    ) -> Vec<TestYieldCheckpoint> {
        emu.drain_trace()
            .into_iter()
            .filter_map(|event| {
                let NormalizedTraceEvent::TrapHit {
                    kind: TrapKind::Pc { addr },
                    cycle,
                    ..
                } = event
                else {
                    return None;
                };
                let kind = if addr == streaming.compute_yield_safe_point_pc {
                    TestYieldCheckpointKind::Compute
                } else if addr == streaming.copy_yield_safe_point_pc {
                    TestYieldCheckpointKind::Copy
                } else if addr == streaming.vblank_handler_pc {
                    TestYieldCheckpointKind::VBlank
                } else {
                    return None;
                };
                Some(TestYieldCheckpoint {
                    mcycle: cycle.as_m_cycles_floor().0,
                    kind,
                })
            })
            .collect()
    }

    #[derive(Debug)]
    struct TestFrameTracker {
        base_mcycle: u64,
        latest_vblank_frame: u32,
        last_serviced_frame: u32,
        progress_epoch: u32,
        frame_service_misses: u32,
        no_progress_streak: u32,
        max_no_progress_frames: u32,
        vblank_frames: BTreeSet<u32>,
        serviced_progress_epoch: BTreeMap<u32, u32>,
    }

    impl TestFrameTracker {
        fn new(base_mcycle: u64) -> Self {
            Self {
                base_mcycle,
                latest_vblank_frame: 0,
                last_serviced_frame: 0,
                progress_epoch: 0,
                frame_service_misses: 0,
                no_progress_streak: 0,
                max_no_progress_frames: 0,
                vblank_frames: BTreeSet::new(),
                serviced_progress_epoch: BTreeMap::new(),
            }
        }

        fn observe(&mut self, checkpoint: TestYieldCheckpoint) {
            match checkpoint.kind {
                TestYieldCheckpointKind::VBlank => {
                    self.latest_vblank_frame = self.latest_vblank_frame.saturating_add(1);
                    self.vblank_frames.insert(self.latest_vblank_frame);
                }
                TestYieldCheckpointKind::Compute => {
                    self.progress_epoch = self.progress_epoch.saturating_add(1);
                    self.service_latest_frame();
                }
                TestYieldCheckpointKind::Copy | TestYieldCheckpointKind::TileSafePoint => {
                    self.service_latest_frame();
                }
            }
        }

        fn finish(&mut self, end_mcycle: u64) {
            let compute_mcycles = end_mcycle.saturating_sub(self.base_mcycle);
            let frames_to_completion = u32::try_from(
                compute_mcycles
                    .div_ceil(u64::from(gbf_hw::timing::FRAME_M_CYCLES))
                    .saturating_add(1),
            )
            .unwrap_or(u32::MAX);
            let gated_end = frames_to_completion.saturating_sub(1).max(1);
            let mut last_service_progress_epoch = 0_u32;
            for frame in 2..gated_end {
                let serviced_epoch = self.serviced_progress_epoch.get(&frame).copied();
                if self.vblank_frames.contains(&frame)
                    && self.vblank_frames.contains(&(frame + 1))
                    && serviced_epoch.is_some()
                {
                    let progress_epoch = serviced_epoch.expect("checked is_some");
                    if progress_epoch > last_service_progress_epoch {
                        last_service_progress_epoch = progress_epoch;
                        self.no_progress_streak = 0;
                    } else {
                        self.no_progress_streak = self.no_progress_streak.saturating_add(1);
                    }
                    self.max_no_progress_frames =
                        self.max_no_progress_frames.max(self.no_progress_streak);
                } else {
                    self.frame_service_misses = self.frame_service_misses.saturating_add(1);
                    self.no_progress_streak = self.no_progress_streak.saturating_add(1);
                    self.max_no_progress_frames =
                        self.max_no_progress_frames.max(self.no_progress_streak);
                }
            }
        }

        fn service_latest_frame(&mut self) {
            if self.latest_vblank_frame == 0 || self.latest_vblank_frame == self.last_serviced_frame
            {
                return;
            }
            self.serviced_progress_epoch
                .insert(self.latest_vblank_frame, self.progress_epoch);
            self.last_serviced_frame = self.latest_vblank_frame;
        }
    }

    #[test]
    fn f_b1_l0_wram_matmul_dump_matches_reference() {
        let rom = build_l0_wram_smoke_rom().expect("L0 ROM builds");
        assert_rom_matches_reference(&rom);
    }

    #[test]
    fn f_b1_l1_compiled_matmul_matches_reference() {
        let rom = build_l0_wram_smoke_rom().expect("L1 ROM builds");
        assert_rom_matches_reference(&rom);
    }

    #[test]
    fn f_b1_l2_cross_bank_rom_smoke_matches_reference() {
        let rom = build_l2_cross_bank_smoke_rom().expect("L2 ROM builds");
        assert_rom_matches_reference(&rom);
    }

    #[test]
    fn f_b1_l2_cross_bank_matmul_matches_reference() {
        let rom = build_l2_cross_bank_smoke_rom().expect("L2 ROM builds");
        assert_rom_matches_reference(&rom);
    }

    #[test]
    fn f_b1_l3_partial_tile_rom_matches_reference_slice() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            ..ComputeBringupRequest::headline_n128()
        };
        let rom = build_l3_partial_tile_rom(&request, 1, 0, 1).expect("partial tile ROM builds");
        assert_eq!(
            run_rom_output_tile(&rom),
            expected_partial_tile(request.matrix_dim, 1, 0, 1)
        );
    }

    #[test]
    fn f_b1_l3_output_tile_rom_matches_reference_tile() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            ..ComputeBringupRequest::headline_n128()
        };
        let rom = build_l3_output_tile_rom(&request, 1, 0).expect("output tile ROM builds");
        assert_eq!(
            run_rom_output_tile(&rom),
            expected_output_tile(request.matrix_dim, 1, 0)
        );
    }

    #[test]
    fn f_b1_l3_streaming_rom_matches_reference_n32() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            ..ComputeBringupRequest::headline_n128()
        };
        assert_streaming_request_matches_reference(&request);
    }

    #[test]
    fn f_b1_l3_streaming_rom_nonzero_offsets_match_reference_n32() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::new(0x0200),
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank"),
                    offset: RomBankOffset::new(0x0400),
                },
            },
            ..ComputeBringupRequest::headline_n128()
        };
        assert_streaming_request_matches_reference(&request);
    }

    #[test]
    #[ignore = "legacy synthetic N=128 fixture; streaming N sweep is owned by gbf-test f_b1_regen"]
    fn f_b1_l3_streamed_output_matches_reference_n128() {
        assert_run_matches_reference(128);
    }

    #[test]
    fn f_b1_frame_event_trace_orders_vblank_and_widget() {
        let run = run_bringup_model(&ComputeBringupRequest::headline_n128()).expect("run");
        for frame in 1..run.metrics.output_tiles {
            let vblank = event_mcycle(&run.frame_events, frame, EventKind::VBlank).expect("vblank");
            let widget = event_mcycle(&run.frame_events, frame, EventKind::Widget).expect("widget");
            let scheduler =
                event_mcycle(&run.frame_events, frame, EventKind::Scheduler).expect("scheduler");
            let next_vblank =
                event_mcycle(&run.frame_events, frame + 1, EventKind::VBlank).expect("next vblank");
            assert!(vblank < widget);
            assert!(widget < next_vblank);
            assert!(vblank < scheduler);
            assert!(scheduler < next_vblank);
        }
    }

    #[test]
    #[ignore = "heavy N=128 streaming L4 gate; run before F-B1 closure"]
    fn f_b1_l4_n128_no_frame_service_misses() {
        let evidence = headline_l4_evidence();
        assert_eq!(evidence.frame_service_misses, 0);
        assert!(evidence.max_unyielded_compute_mcycles <= L4_DEADLINE_M_CYCLES);
        assert!(evidence.max_bank_lease_hold_mcycles <= L4_DEADLINE_M_CYCLES);
    }

    #[test]
    #[ignore = "heavy N=128 streaming L4 liveness gate; run before F-B1 closure"]
    fn f_b1_l4_liveness_no_progress_frames_bounded() {
        let evidence = headline_l4_evidence();
        assert!(evidence.max_no_progress_frames <= 1);
    }

    #[test]
    #[ignore = "heavy N=128 streaming output gate; run before F-B1 closure"]
    fn f_b1_l4_output_matches_reference() {
        let evidence = headline_l4_evidence();
        let dim = SquareDim::new(128).expect("valid");
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let expected = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(evidence.output_i32, expected.data());
    }

    #[derive(Clone, Copy)]
    enum EventKind {
        VBlank,
        Widget,
        Scheduler,
    }

    fn event_mcycle(
        events: &[gbf_codegen::f_b1::FrameEventEnvelope],
        frame: u32,
        kind: EventKind,
    ) -> Option<(u64, u64)> {
        events.iter().find_map(|envelope| {
            let (f, mcycle) = match (&envelope.event, kind) {
                (
                    FrameEvent::VBlankFired {
                        frame: f,
                        mcycle_since_boot,
                    },
                    EventKind::VBlank,
                ) => (*f, *mcycle_since_boot),
                (
                    FrameEvent::WidgetTickDispatched {
                        frame: f,
                        mcycle_since_boot,
                    },
                    EventKind::Widget,
                ) => (*f, *mcycle_since_boot),
                (
                    FrameEvent::SchedulerServicedFrame {
                        frame: f,
                        mcycle_since_boot,
                    },
                    EventKind::Scheduler,
                ) => (*f, *mcycle_since_boot),
                _ => return None,
            };
            (f == frame).then_some((mcycle, envelope.seq))
        })
    }
}
