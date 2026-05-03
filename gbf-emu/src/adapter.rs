//! Headless `gameroy-core` adapter.

use std::io::Cursor;

use gameroy::gameboy::GameBoy;
use gameroy::gameboy::cartridge::Cartridge;
use gameroy::gameboy::cpu::{CpuState, ImeState};
use gameroy::interpreter::Interpreter;
use gbf_foundation::Hash256;
use gbf_hw::{joypad::Button, memory};
use sha2::{Digest, Sha256};

use crate::determinism::{CartridgeRtcMode, DeterminismPolicy, PowerOnRamPolicy};
use crate::harness::{
    HarnessChannel, HarnessCommand, HarnessMemory, HarnessResult, HarnessSlot, sram_offset,
};
use crate::primitives::{
    BootModeLineage, ClockCycles, CpuIdleState, CycleBudget, DMG_FRAME_CLOCK_CYCLES, EmuError,
    EmuVersionTag, Flags, Framebuffer, ImeSnapshot, JoypadFrame, MCycles, Regs, RunOutcome,
    Snapshot, SnapshotLineage, StepOutcome,
};
use crate::trace_ring::{
    NormalizedTraceEvent, TraceCursor, TraceDropPolicy, TraceMapper, TraceOrigin,
};
use crate::trap::{
    BreakpointId, EmuReadOnlyMemory, MemoryAccess, MemoryAccessKind, TrapAction, TrapDispatcher,
    TrapKind,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BootMode {
    #[default]
    PostBootDmg,
    BootRom(BootRomImage),
}

impl BootMode {
    #[must_use]
    pub fn lineage(&self) -> BootModeLineage {
        match self {
            Self::PostBootDmg => BootModeLineage::PostBootDmg,
            Self::BootRom(image) => BootModeLineage::BootRom {
                sha256: image.sha256,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootRomImage {
    pub bytes: Box<[u8; 0x100]>,
    pub sha256: Hash256,
}

impl BootRomImage {
    #[must_use]
    pub fn new(bytes: [u8; 0x100]) -> Self {
        Self {
            sha256: hash256(&bytes),
            bytes: Box::new(bytes),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EmulatorConfig {
    pub policy: DeterminismPolicy,
    pub boot_mode: BootMode,
    pub trace_capacity: usize,
    pub trace_drop_policy: TraceDropPolicy,
    pub audit_host_pokes: bool,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            policy: DeterminismPolicy::default(),
            boot_mode: BootMode::default(),
            trace_capacity: 4096,
            trace_drop_policy: TraceDropPolicy::DropOldest,
            audit_host_pokes: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EmulatorBuilder {
    config: EmulatorConfig,
}

impl EmulatorBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn policy(mut self, policy: DeterminismPolicy) -> Self {
        self.config.policy = policy;
        self
    }

    #[must_use]
    pub fn boot_mode(mut self, boot_mode: BootMode) -> Self {
        self.config.boot_mode = boot_mode;
        self
    }

    #[must_use]
    pub fn trace_capacity(mut self, capacity: usize) -> Self {
        self.config.trace_capacity = capacity;
        self
    }

    #[must_use]
    pub fn trace_drop_policy(mut self, drop_policy: TraceDropPolicy) -> Self {
        self.config.trace_drop_policy = drop_policy;
        self
    }

    #[must_use]
    pub fn audit_host_pokes(mut self, enabled: bool) -> Self {
        self.config.audit_host_pokes = enabled;
        self
    }

    pub fn load_rom(self, bytes: &[u8]) -> Result<Emulator, EmuError> {
        Emulator::load_rom(bytes, self.config)
    }
}

pub struct Emulator {
    inner: GameBoy,
    policy: DeterminismPolicy,
    boot_mode: BootMode,
    rom_sha256: Hash256,
    traps: TrapDispatcher,
    trace: TraceCursor,
    harness: Option<HarnessChannel>,
    audit_host_pokes: bool,
}

impl Emulator {
    #[must_use]
    pub fn builder() -> EmulatorBuilder {
        EmulatorBuilder::new()
    }

    pub fn load_rom(bytes: &[u8], config: EmulatorConfig) -> Result<Self, EmuError> {
        let rom_sha256 = hash256(bytes);
        let cartridge =
            Cartridge::new(bytes.to_vec()).map_err(|(reason, _)| EmuError::RomLoad { reason })?;
        let trace_mapper = trace_mapper_for_cartridge(&cartridge);

        if cartridge_has_rtc(&cartridge) {
            return Err(EmuError::Determinism {
                reason: match config.policy.cartridge_rtc() {
                    CartridgeRtcMode::Fixed => {
                        "cartridge RTC control unavailable for fixed RTC policy".to_owned()
                    }
                    CartridgeRtcMode::RealTime => {
                        "cartridge RTC passthrough unavailable in gameroy-core headless adapter"
                            .to_owned()
                    }
                },
            });
        }
        if matches!(
            config.policy.power_on_ram(),
            PowerOnRamPolicy::FixedFill { .. }
        ) && matches!(config.boot_mode, BootMode::PostBootDmg)
        {
            return Err(EmuError::Determinism {
                reason: "FixedFill power-on RAM policy requires BootMode::BootRom; PostBootDmg already contains initialized boot state".to_owned(),
            });
        }

        let boot_rom = match &config.boot_mode {
            BootMode::PostBootDmg => None,
            BootMode::BootRom(image) => Some(*image.bytes),
        };
        let mut inner = GameBoy::new(boot_rom, cartridge);
        apply_power_on_ram(&mut inner, config.policy.power_on_ram());

        Ok(Self {
            inner,
            policy: config.policy,
            boot_mode: config.boot_mode,
            rom_sha256,
            traps: TrapDispatcher::new(),
            trace: TraceCursor::with_mapper(
                config.trace_capacity,
                config.trace_drop_policy,
                trace_mapper,
            ),
            harness: None,
            audit_host_pokes: config.audit_host_pokes,
        })
    }

    pub fn step(&mut self) -> Result<StepOutcome, EmuError> {
        let start = self.clock_count();
        let regs = self.regs();
        let pc_hit = {
            let view = GameBoyReadOnly {
                gb: &self.inner,
                sram_bank: self.trace.current_sram_bank(),
            };
            self.traps.dispatch_pc(regs, start, &view)?
        };
        for hit in pc_hit {
            match hit.action {
                TrapAction::HaltAndReport => {
                    return Ok(StepOutcome::TrapHit {
                        trap_id: hit.id,
                        kind: hit.kind,
                        cycles: ClockCycles(0),
                    });
                }
                TrapAction::Continue => {
                    self.trace.record_trap_hit(hit.id, hit.kind, hit.cycle)?;
                }
            }
        }

        self.inner.io_trace.borrow_mut().clear();

        let instr_pc = self.inner.cpu.pc;
        Interpreter(&mut self.inner).interpret_op();
        let end = self.clock_count();
        let cycles = ClockCycles(end.0.saturating_sub(start.0));
        let accesses = self.drain_io_trace(start, end, instr_pc);

        let regs_after = self.regs();
        self.record_accesses(&accesses, TraceOrigin::GuestCpu)?;
        let mem_hits = {
            let view = GameBoyReadOnly {
                gb: &self.inner,
                sram_bank: self.trace.current_sram_bank(),
            };
            self.traps.dispatch_memory(regs_after, &accesses, &view)?
        };
        for hit in mem_hits {
            match hit.action {
                TrapAction::Continue => {
                    self.trace.record_trap_hit(hit.id, hit.kind, hit.cycle)?;
                }
                TrapAction::HaltAndReport => {
                    return Ok(StepOutcome::TrapHit {
                        trap_id: hit.id,
                        kind: hit.kind,
                        cycles,
                    });
                }
            }
        }

        match self.inner.cpu.state {
            CpuState::Running => Ok(StepOutcome::Stepped { cycles }),
            CpuState::Halt => Ok(StepOutcome::Idle {
                state: CpuIdleState::Halt,
                cycles,
            }),
            CpuState::Stopped => Ok(StepOutcome::Idle {
                state: CpuIdleState::Stop,
                cycles,
            }),
        }
    }

    pub fn run_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError> {
        let requested = budget.as_clock_cycles();
        let start = self.clock_count();
        let target = ClockCycles(start.0.saturating_add(requested.0));

        while self.clock_count() < target {
            match self.step()? {
                StepOutcome::Stepped { .. } => {}
                StepOutcome::TrapHit { trap_id, kind, .. } => {
                    return Ok(RunOutcome::TrapHit {
                        trap_id,
                        kind,
                        observed: self.clock_count(),
                    });
                }
                StepOutcome::Idle { state, .. } => {
                    return Ok(RunOutcome::Idle {
                        state,
                        observed: self.clock_count(),
                    });
                }
            }
        }

        Ok(RunOutcome::BudgetElapsed {
            observed: self.clock_count(),
            requested,
        })
    }

    /// Run one DMG frame through the full instrumented runner.
    pub fn run_frame(&mut self) -> Result<RunOutcome, EmuError> {
        self.run_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
    }

    /// Run a budget with low-overhead PC trap checks.
    ///
    /// This path is intended for long-running compute tests. It honors PC traps and idle states,
    /// but suppresses guest memory trace ingestion. If memory traps are installed, it returns
    /// `EmuError::FastRunBlockedByMemoryTraps` so the caller can decide whether to remove the traps
    /// or run the fully instrumented path instead.
    pub fn run_fast_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError> {
        let requested = budget.as_clock_cycles();
        let start = self.clock_count();
        let target = ClockCycles(start.0.saturating_add(requested.0));
        self.run_fast_until_target(target, requested, None)
    }

    /// Run one DMG frame using the fast runner.
    pub fn run_fast_frame(&mut self) -> Result<RunOutcome, EmuError> {
        self.run_fast_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
    }

    pub fn run_until_pc(&mut self, pc: u16, budget: CycleBudget) -> Result<RunOutcome, EmuError> {
        let requested = budget.as_clock_cycles();
        let start = self.clock_count();
        let target = ClockCycles(start.0.saturating_add(requested.0));

        loop {
            if self.regs().pc == pc {
                return Ok(RunOutcome::TrapHit {
                    trap_id: BreakpointId::RUN_UNTIL_PC,
                    kind: TrapKind::Pc { addr: pc },
                    observed: self.clock_count(),
                });
            }
            if self.clock_count() >= target {
                return Ok(RunOutcome::BudgetElapsed {
                    observed: self.clock_count(),
                    requested,
                });
            }

            match self.step()? {
                StepOutcome::Stepped { .. } => {}
                StepOutcome::TrapHit { trap_id, kind, .. } => {
                    return Ok(RunOutcome::TrapHit {
                        trap_id,
                        kind,
                        observed: self.clock_count(),
                    });
                }
                StepOutcome::Idle { state, .. } => {
                    return Ok(RunOutcome::Idle {
                        state,
                        observed: self.clock_count(),
                    });
                }
            }
        }
    }

    /// Run until `pc` is reached using the fast runner.
    pub fn run_fast_until_pc(
        &mut self,
        pc: u16,
        budget: CycleBudget,
    ) -> Result<RunOutcome, EmuError> {
        let requested = budget.as_clock_cycles();
        let start = self.clock_count();
        let target = ClockCycles(start.0.saturating_add(requested.0));
        self.run_fast_until_target(target, requested, Some(pc))
    }

    pub fn regs(&self) -> Regs {
        regs_from_gameboy(&self.inner)
    }

    pub fn set_regs(&mut self, regs: Regs) -> Result<(), EmuError> {
        self.inner.cpu.a = regs.a;
        self.inner.cpu.f = gameroy::gameboy::cpu::Flags(regs.f.bits());
        self.inner.cpu.b = regs.b;
        self.inner.cpu.c = regs.c;
        self.inner.cpu.d = regs.d;
        self.inner.cpu.e = regs.e;
        self.inner.cpu.h = regs.h;
        self.inner.cpu.l = regs.l;
        self.inner.cpu.sp = regs.sp;
        self.inner.cpu.pc = regs.pc;
        self.inner.cpu.ime = ime_to_gameroy(regs.ime);
        Ok(())
    }

    pub fn bus_read(&mut self, addr: u16) -> Result<u8, EmuError> {
        let value = self.inner.read(addr);
        self.inner.clock_count = self.inner.clock_count.saturating_add(4);
        Ok(value)
    }

    pub fn bus_write(&mut self, addr: u16, value: u8) -> Result<(), EmuError> {
        self.inner.write(addr, value);
        self.inner.clock_count = self.inner.clock_count.saturating_add(4);
        self.trace.record_access(
            MemoryAccess {
                addr,
                value,
                kind: MemoryAccessKind::Write,
                cycle: self.clock_count(),
            },
            TraceOrigin::HostBus,
        )
    }

    pub fn peek(&self, addr: u16) -> Result<u8, EmuError> {
        raw_peek(&self.inner, addr, self.trace.current_sram_bank())
    }

    pub fn poke(&mut self, addr: u16, value: u8) -> Result<(), EmuError> {
        raw_poke(&mut self.inner, addr, value, self.trace.current_sram_bank())?;
        if self.audit_host_pokes {
            self.record_host_poke(addr, value)?;
        }
        Ok(())
    }

    pub fn peek_range(&self, start: u16, len: usize) -> Result<Vec<u8>, EmuError> {
        <Self as EmuReadOnlyMemory>::peek_range(self, start, len)
    }

    pub fn framebuffer(&mut self) -> Framebuffer {
        self.inner.update_all();
        Framebuffer::from_pixels(self.inner.ppu.borrow().screen.packed())
    }

    pub fn set_joypad(&mut self, frame: JoypadFrame) {
        self.inner.joypad = joypad_to_gameroy(frame);
    }

    pub fn snapshot(&self) -> Result<Snapshot, EmuError> {
        let mut blob = Vec::new();
        self.inner
            .save_state(self.policy.save_state_timestamp(), &mut blob)
            .map_err(|error| EmuError::SnapshotSave {
                reason: error.to_string(),
            })?;
        Ok(Snapshot {
            blob,
            lineage: SnapshotLineage {
                rom_sha256: self.rom_sha256,
                boot: self.boot_mode.lineage(),
                policy_fingerprint: self.policy.fingerprint(),
                emu_version: self.version_tag(),
                cycle_count: self.clock_count(),
            },
            trace_bank: self.trace.bank_snapshot(),
        })
    }

    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), EmuError> {
        let expected_boot = self.boot_mode.lineage();
        let expected_policy = self.policy.fingerprint();
        let expected_version = self.version_tag();

        if snapshot.lineage.rom_sha256 != self.rom_sha256 {
            return Err(EmuError::SnapshotRomMismatch {
                expected: self.rom_sha256,
                observed: snapshot.lineage.rom_sha256,
            });
        }
        if snapshot.lineage.boot != expected_boot {
            return Err(EmuError::SnapshotBootMismatch {
                expected: expected_boot,
                observed: snapshot.lineage.boot,
            });
        }
        if snapshot.lineage.policy_fingerprint != expected_policy {
            return Err(EmuError::SnapshotPolicyMismatch {
                expected: expected_policy,
                observed: snapshot.lineage.policy_fingerprint,
            });
        }
        if snapshot.lineage.emu_version != expected_version {
            return Err(EmuError::SnapshotEmuVersionMismatch {
                expected: Box::new(expected_version),
                observed: Box::new(snapshot.lineage.emu_version),
            });
        }

        self.inner
            .load_state(&mut Cursor::new(&snapshot.blob))
            .map_err(|error| EmuError::SnapshotLoad {
                reason: format!("{error:?}"),
            })?;
        self.trace.set_bank_snapshot(snapshot.trace_bank);
        Ok(())
    }

    pub fn traps(&mut self) -> &mut TrapDispatcher {
        &mut self.traps
    }

    #[must_use]
    pub const fn traps_ref(&self) -> &TrapDispatcher {
        &self.traps
    }

    pub fn drain_trace(&mut self) -> Vec<NormalizedTraceEvent> {
        self.trace.drain()
    }

    pub fn record_typed_trace_event(
        &mut self,
        event: gbf_abi::trace::TraceEvent,
    ) -> Result<(), EmuError> {
        self.trace.record_typed(event)
    }

    pub fn attach_harness(&mut self, slot: HarnessSlot) {
        self.harness = Some(HarnessChannel::new(slot));
    }

    pub fn poll_harness(&mut self) -> Result<Option<HarnessCommand>, EmuError> {
        let Some(mut channel) = self.harness.take() else {
            return Ok(None);
        };
        let result = channel.read_command_from(self);
        self.harness = Some(channel);
        result
    }

    pub fn write_harness_result(&mut self, result: HarnessResult) -> Result<(), EmuError> {
        let Some(mut channel) = self.harness.take() else {
            return Err(EmuError::HarnessSramAccessUnavailable {
                reason: "no harness channel attached".to_owned(),
            });
        };
        let write = channel.write_result_to(self, result);
        self.harness = Some(channel);
        let writes = write?;
        if self.audit_host_pokes {
            for (addr, value) in writes {
                self.record_host_poke(addr, value)?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn rom_sha256(&self) -> Hash256 {
        self.rom_sha256
    }

    #[must_use]
    pub const fn clock_count(&self) -> ClockCycles {
        ClockCycles(self.inner.clock_count)
    }

    #[must_use]
    pub const fn m_cycle_count_floor(&self) -> MCycles {
        self.clock_count().as_m_cycles_floor()
    }

    #[must_use]
    pub const fn policy(&self) -> &DeterminismPolicy {
        &self.policy
    }

    #[must_use]
    pub fn boot_mode(&self) -> BootMode {
        self.boot_mode.clone()
    }

    #[must_use]
    pub fn version_tag(&self) -> EmuVersionTag {
        EmuVersionTag::current()
    }

    fn drain_io_trace(
        &mut self,
        start: ClockCycles,
        end: ClockCycles,
        instr_pc: u16,
    ) -> Vec<MemoryAccess> {
        self.inner
            .io_trace
            .borrow_mut()
            .drain(..)
            .enumerate()
            .map(|(index, (packed, addr, value))| {
                let cycle = reconstruct_io_trace_cycle(start, end, packed, index);
                let kind = if packed & GameBoy::IO_WRITE == GameBoy::IO_WRITE {
                    MemoryAccessKind::Write
                } else if is_instruction_fetch(instr_pc, addr) {
                    MemoryAccessKind::InstrFetch
                } else {
                    MemoryAccessKind::DataRead
                };
                MemoryAccess {
                    addr,
                    value,
                    kind,
                    cycle,
                }
            })
            .collect()
    }

    fn record_accesses(
        &mut self,
        accesses: &[MemoryAccess],
        origin: TraceOrigin,
    ) -> Result<(), EmuError> {
        for access in accesses {
            self.trace.record_access(*access, origin)?;
        }
        Ok(())
    }

    fn record_host_poke(&mut self, addr: u16, value: u8) -> Result<(), EmuError> {
        self.trace.record_access(
            MemoryAccess {
                addr,
                value,
                kind: MemoryAccessKind::Write,
                cycle: self.clock_count(),
            },
            TraceOrigin::HostPoke,
        )
    }

    fn run_fast_until_target(
        &mut self,
        target: ClockCycles,
        requested: ClockCycles,
        stop_pc: Option<u16>,
    ) -> Result<RunOutcome, EmuError> {
        let memory_trap_count = self.traps.memory_trap_count();
        if memory_trap_count != 0 {
            return Err(EmuError::FastRunBlockedByMemoryTraps { memory_trap_count });
        }

        let check_pc_traps = self.traps.has_pc_traps();
        self.inner.io_trace.borrow_mut().clear();

        loop {
            if let Some(pc) = stop_pc
                && self.inner.cpu.pc == pc
            {
                self.inner.io_trace.borrow_mut().clear();
                return Ok(RunOutcome::TrapHit {
                    trap_id: BreakpointId::RUN_UNTIL_PC,
                    kind: TrapKind::Pc { addr: pc },
                    observed: self.clock_count(),
                });
            }

            if self.clock_count() >= target {
                self.inner.io_trace.borrow_mut().clear();
                return Ok(RunOutcome::BudgetElapsed {
                    observed: self.clock_count(),
                    requested,
                });
            }

            if check_pc_traps {
                let start = self.clock_count();
                let regs = self.regs();
                let pc_hit = {
                    let view = GameBoyReadOnly {
                        gb: &self.inner,
                        sram_bank: self.trace.current_sram_bank(),
                    };
                    self.traps.dispatch_pc(regs, start, &view)?
                };
                for hit in pc_hit {
                    match hit.action {
                        TrapAction::HaltAndReport => {
                            self.inner.io_trace.borrow_mut().clear();
                            return Ok(RunOutcome::TrapHit {
                                trap_id: hit.id,
                                kind: hit.kind,
                                observed: self.clock_count(),
                            });
                        }
                        TrapAction::Continue => {
                            self.trace.record_trap_hit(hit.id, hit.kind, hit.cycle)?;
                        }
                    }
                }
            }

            Interpreter(&mut self.inner).interpret_op();
            self.inner.io_trace.borrow_mut().clear();

            match self.inner.cpu.state {
                CpuState::Running => {}
                CpuState::Halt => {
                    return Ok(RunOutcome::Idle {
                        state: CpuIdleState::Halt,
                        observed: self.clock_count(),
                    });
                }
                CpuState::Stopped => {
                    return Ok(RunOutcome::Idle {
                        state: CpuIdleState::Stop,
                        observed: self.clock_count(),
                    });
                }
            }
        }
    }
}

impl EmuReadOnlyMemory for Emulator {
    fn peek(&self, addr: u16) -> Result<u8, EmuError> {
        Emulator::peek(self, addr)
    }
}

impl HarnessMemory for Emulator {
    fn read_sram_bank(&self, bank: u8, addr: u16) -> Result<u8, EmuError> {
        if self.inner.cartridge.ram.is_empty() {
            return Err(EmuError::HarnessSramAccessUnavailable {
                reason: "cartridge has no SRAM".to_owned(),
            });
        }
        let offset = sram_offset(bank, addr, 1, self.inner.cartridge.ram.len())?;
        Ok(self.inner.cartridge.ram[offset])
    }

    fn write_sram_bank(&mut self, bank: u8, addr: u16, value: u8) -> Result<(), EmuError> {
        if self.inner.cartridge.ram.is_empty() {
            return Err(EmuError::HarnessSramAccessUnavailable {
                reason: "cartridge has no SRAM".to_owned(),
            });
        }
        let offset = sram_offset(bank, addr, 1, self.inner.cartridge.ram.len())?;
        self.inner.cartridge.ram[offset] = value;
        Ok(())
    }

    fn read_sram_bank_range(&self, bank: u8, addr: u16, len: usize) -> Result<Vec<u8>, EmuError> {
        if self.inner.cartridge.ram.is_empty() {
            return Err(EmuError::HarnessSramAccessUnavailable {
                reason: "cartridge has no SRAM".to_owned(),
            });
        }
        let offset = sram_offset(bank, addr, len, self.inner.cartridge.ram.len())?;
        Ok(self.inner.cartridge.ram[offset..offset + len].to_vec())
    }
}

struct GameBoyReadOnly<'a> {
    gb: &'a GameBoy,
    sram_bank: u8,
}

impl EmuReadOnlyMemory for GameBoyReadOnly<'_> {
    fn peek(&self, addr: u16) -> Result<u8, EmuError> {
        raw_peek(self.gb, addr, self.sram_bank)
    }
}

fn regs_from_gameboy(gb: &GameBoy) -> Regs {
    Regs {
        a: gb.cpu.a,
        f: Flags::new(gb.cpu.f.0),
        b: gb.cpu.b,
        c: gb.cpu.c,
        d: gb.cpu.d,
        e: gb.cpu.e,
        h: gb.cpu.h,
        l: gb.cpu.l,
        sp: gb.cpu.sp,
        pc: gb.cpu.pc,
        ime: ime_from_gameroy(gb.cpu.ime),
    }
}

fn ime_from_gameroy(ime: ImeState) -> ImeSnapshot {
    match ime {
        ImeState::Disabled => ImeSnapshot::Disabled,
        ImeState::Enabled => ImeSnapshot::Enabled,
        ImeState::ToBeEnable => ImeSnapshot::ToBeEnable,
    }
}

fn ime_to_gameroy(ime: ImeSnapshot) -> ImeState {
    match ime {
        ImeSnapshot::Disabled => ImeState::Disabled,
        ImeSnapshot::Enabled => ImeState::Enabled,
        ImeSnapshot::ToBeEnable => ImeState::ToBeEnable,
    }
}

fn raw_peek(gb: &GameBoy, addr: u16, sram_bank: u8) -> Result<u8, EmuError> {
    let addr = mirror_echo(addr);
    match memory::classify(addr) {
        memory::MemoryRegion::RomBank0 => {
            if gb.boot_rom_active && addr < 0x0100 {
                let boot = gb.boot_rom.ok_or(EmuError::MemoryAccess {
                    addr,
                    reason: "boot ROM is active but bytes are unavailable".to_owned(),
                })?;
                return Ok(boot[addr as usize]);
            }
            read_rom_bank(gb, gb.cartridge.lower_bank, addr)
        }
        memory::MemoryRegion::RomSwitchable => read_rom_bank(gb, gb.cartridge.upper_bank, addr),
        memory::MemoryRegion::Vram => Ok(gb.ppu.borrow().vram[(addr - memory::VRAM_BASE) as usize]),
        memory::MemoryRegion::Sram => {
            if gb.cartridge.ram.is_empty() {
                return Err(EmuError::MemoryAccess {
                    addr,
                    reason: "cartridge has no SRAM".to_owned(),
                });
            }
            let offset = sram_offset(sram_bank, addr, 1, gb.cartridge.ram.len())?;
            Ok(gb.cartridge.ram[offset])
        }
        memory::MemoryRegion::Wram0 | memory::MemoryRegion::WramX => {
            Ok(gb.wram[(addr - memory::WRAM_BASE) as usize])
        }
        memory::MemoryRegion::EchoRam => unreachable!("echo RAM is mirrored before classify"),
        memory::MemoryRegion::Oam => Ok(gb.ppu.borrow().oam[(addr - memory::OAM_BASE) as usize]),
        memory::MemoryRegion::Hram => Ok(gb.hram[(addr - memory::HRAM_BASE) as usize]),
        memory::MemoryRegion::InterruptEnable => Ok(gb.interrupt_enabled),
        memory::MemoryRegion::Io | memory::MemoryRegion::Unmapped => {
            Err(EmuError::DebugMemoryUnsupported { addr })
        }
    }
}

fn raw_poke(gb: &mut GameBoy, addr: u16, value: u8, sram_bank: u8) -> Result<(), EmuError> {
    let addr = mirror_echo(addr);
    match memory::classify(addr) {
        memory::MemoryRegion::Vram => {
            gb.ppu.borrow_mut().vram[(addr - memory::VRAM_BASE) as usize] = value;
        }
        memory::MemoryRegion::Sram => {
            if gb.cartridge.ram.is_empty() {
                return Err(EmuError::MemoryAccess {
                    addr,
                    reason: "cartridge has no SRAM".to_owned(),
                });
            }
            let offset = sram_offset(sram_bank, addr, 1, gb.cartridge.ram.len())?;
            gb.cartridge.ram[offset] = value;
        }
        memory::MemoryRegion::Wram0 | memory::MemoryRegion::WramX => {
            gb.wram[(addr - memory::WRAM_BASE) as usize] = value;
        }
        memory::MemoryRegion::EchoRam => unreachable!("echo RAM is mirrored before classify"),
        memory::MemoryRegion::Oam => {
            gb.ppu.borrow_mut().oam[(addr - memory::OAM_BASE) as usize] = value;
        }
        memory::MemoryRegion::Hram => {
            gb.hram[(addr - memory::HRAM_BASE) as usize] = value;
        }
        memory::MemoryRegion::InterruptEnable => {
            gb.interrupt_enabled = value;
            gb.update_next_interrupt();
        }
        memory::MemoryRegion::RomBank0
        | memory::MemoryRegion::RomSwitchable
        | memory::MemoryRegion::Io
        | memory::MemoryRegion::Unmapped => {
            return Err(EmuError::DebugMemoryUnsupported { addr });
        }
    }
    Ok(())
}

fn read_rom_bank(gb: &GameBoy, bank: u16, addr: u16) -> Result<u8, EmuError> {
    let bank_offset = bank as usize * 0x4000;
    let addr_offset = if addr <= memory::ROM_BANK0_END {
        addr as usize
    } else {
        (addr - memory::ROM_SWITCHABLE_BASE) as usize
    };
    gb.cartridge
        .rom
        .get(bank_offset + addr_offset)
        .copied()
        .ok_or_else(|| EmuError::MemoryAccess {
            addr,
            reason: format!("ROM bank {bank} is outside cartridge bytes"),
        })
}

fn mirror_echo(addr: u16) -> u16 {
    if (memory::ECHO_RAM_BASE..=memory::ECHO_RAM_END).contains(&addr) {
        addr - 0x2000
    } else {
        addr
    }
}

fn joypad_to_gameroy(frame: JoypadFrame) -> u8 {
    let mut pressed = 0_u8;
    for (button, bit) in [
        (Button::Right, 0),
        (Button::Left, 1),
        (Button::Up, 2),
        (Button::Down, 3),
        (Button::A, 4),
        (Button::B, 5),
        (Button::Select, 6),
        (Button::Start, 7),
    ] {
        if frame.is_pressed(button) {
            pressed |= 1 << bit;
        }
    }
    !pressed
}

fn cartridge_has_rtc(cartridge: &Cartridge) -> bool {
    matches!(cartridge.header.cartridge_type, 0x0F | 0x10 | 0xFE)
}

fn trace_mapper_for_cartridge(cartridge: &Cartridge) -> TraceMapper {
    if matches!(cartridge.header.cartridge_type, 0x19..=0x1E) {
        TraceMapper::Mbc5
    } else {
        TraceMapper::Fixed
    }
}

fn is_instruction_fetch(instr_pc: u16, addr: u16) -> bool {
    let distance = addr.wrapping_sub(instr_pc);
    distance <= 3
}

fn reconstruct_io_trace_cycle(
    start: ClockCycles,
    end: ClockCycles,
    packed: u8,
    index: usize,
) -> ClockCycles {
    const PACKED_CYCLE_MODULUS: u64 = 512;

    let packed_cycle = ((packed & !GameBoy::IO_WRITE) as u64) << 1;
    let base = start.0 & !(PACKED_CYCLE_MODULUS - 1);
    let mut candidate = base | packed_cycle;
    while candidate < start.0 {
        candidate = candidate.saturating_add(PACKED_CYCLE_MODULUS);
    }
    if candidate <= end.0 {
        return ClockCycles(candidate);
    }

    let fallback = start.0.saturating_add(index as u64 * 4).min(end.0);
    ClockCycles(fallback)
}

fn apply_power_on_ram(inner: &mut GameBoy, policy: PowerOnRamPolicy) {
    match policy {
        PowerOnRamPolicy::GameroyDefault => {}
        PowerOnRamPolicy::FixedFill {
            wram,
            hram,
            cartridge_ram,
        } => {
            inner.wram.fill(wram);
            inner.hram.fill(hram);
            inner.cartridge.ram.fill(cartridge_ram);
        }
    }
}

fn hash256(bytes: &[u8]) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_rom_round_trip() {
        let rom = test_rom(&[0x00, 0x76], 0x00, 0x00);
        let emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");

        assert_eq!(emu.rom_sha256(), hash256(&rom));
    }

    #[test]
    fn step_advances_pc_and_clock() {
        let rom = test_rom(&[0x00, 0x76], 0x00, 0x00);
        let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
        let start = emu.regs();

        assert!(matches!(
            emu.step().expect("step succeeds"),
            StepOutcome::Stepped {
                cycles: ClockCycles(4)
            }
        ));
        assert_eq!(emu.regs().pc, start.pc + 1);
        assert!(emu.clock_count() > ClockCycles(0));
    }

    #[test]
    fn run_until_pc_returns_immediately_when_already_there() {
        let rom = test_rom(&[0x00], 0x00, 0x00);
        let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
        let pc = emu.regs().pc;

        assert_eq!(
            emu.run_until_pc(pc, CycleBudget::Clock(ClockCycles(16)))
                .expect("run succeeds"),
            RunOutcome::TrapHit {
                trap_id: BreakpointId::RUN_UNTIL_PC,
                kind: TrapKind::Pc { addr: pc },
                observed: emu.clock_count()
            }
        );
    }

    #[test]
    fn peek_does_not_advance_clock_or_emit_trace_events() {
        let rom = test_rom(&[0x00], 0x00, 0x00);
        let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
        let before = emu.clock_count();

        assert_eq!(emu.peek(0x0100), Ok(0x00));

        assert_eq!(emu.clock_count(), before);
        assert!(emu.drain_trace().is_empty());
    }

    #[test]
    fn peek_unsupported_for_io_region() {
        let rom = test_rom(&[0x00], 0x00, 0x00);
        let emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");

        assert_eq!(
            emu.peek(gbf_hw::joypad::JOYP_REGISTER),
            Err(EmuError::DebugMemoryUnsupported {
                addr: gbf_hw::joypad::JOYP_REGISTER
            })
        );
    }

    #[test]
    fn snapshot_round_trip_restores_state() {
        let rom = test_rom(&[0x00, 0x00, 0x76], 0x00, 0x00);
        let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
        emu.step().unwrap();
        let snapshot_regs = emu.regs();
        let snapshot = emu.snapshot().expect("snapshot saves");
        emu.step().unwrap();

        emu.restore(&snapshot).expect("snapshot restores");

        assert_eq!(emu.regs(), snapshot_regs);
        assert_eq!(emu.clock_count(), snapshot.lineage.cycle_count);
    }

    #[test]
    fn fixed_fill_rejected_for_post_boot_mode() {
        let rom = test_rom(&[0x00], 0x00, 0x00);
        let config = EmulatorConfig {
            policy: DeterminismPolicy::builder()
                .with_power_on_ram(PowerOnRamPolicy::FixedFill {
                    wram: 1,
                    hram: 2,
                    cartridge_ram: 3,
                })
                .build(),
            ..EmulatorConfig::default()
        };

        assert!(matches!(
            Emulator::load_rom(&rom, config),
            Err(EmuError::Determinism { reason }) if reason.contains("PostBootDmg")
        ));
    }

    #[test]
    fn bus_write_origin_host_bus() {
        let rom = test_rom(&[0x00], 0x00, 0x00);
        let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");

        emu.bus_write(0xC000, 0xAA).unwrap();

        assert!(matches!(
            emu.drain_trace().as_slice(),
            [NormalizedTraceEvent::MemoryWrite {
                addr: 0xC000,
                value: 0xAA,
                origin: TraceOrigin::HostBus,
                ..
            }]
        ));
    }

    fn test_rom(program: &[u8], cartridge_type: u8, ram_size: u8) -> Vec<u8> {
        const LOGO: [u8; 48] = [
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ];
        let mut rom = vec![0x00; 0x8000];
        rom[0x0100..0x0100 + program.len()].copy_from_slice(program);
        rom[0x0104..=0x0133].copy_from_slice(&LOGO);
        rom[0x0134..0x013B].copy_from_slice(b"GBFEMU\0");
        rom[0x0147] = cartridge_type;
        rom[0x0148] = 0x00;
        rom[0x0149] = ram_size;
        rom[0x014D] = rom[0x0134..=0x014C]
            .iter()
            .fold(0_u8, |acc, byte| acc.wrapping_add(!byte));
        rom
    }
}
