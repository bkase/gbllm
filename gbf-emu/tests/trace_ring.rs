mod common;

use gbf_abi::trace::{TraceEvent, TraceProbeId};
use gbf_abi::{CompactCheckpointId, SliceId};
use gbf_emu::{
    BankSwitchSource, ClockCycles, Emulator, EmulatorConfig, NormalizedTraceEvent, TraceDropPolicy,
    TraceOrigin,
};

#[test]
fn bus_write_normalizes_host_bus_memory_write() {
    let mut emu = Emulator::load_rom(&common::rom(&[0x00], 0x00, 0x00), EmulatorConfig::default())
        .expect("ROM loads");

    emu.bus_write(0xC000, 0x5A).expect("bus write");

    assert!(matches!(
        emu.drain_trace().as_slice(),
        [NormalizedTraceEvent::MemoryWrite {
            addr: 0xC000,
            value: 0x5A,
            origin: TraceOrigin::HostBus,
            ..
        }]
    ));
}

#[test]
fn guest_io_trace_drains_per_instruction_and_preserves_duplicate_writes() {
    let rom = common::rom(
        &[0x3E, 0x55, 0xEA, 0x00, 0xC0, 0xEA, 0x00, 0xC0, 0x76],
        0x00,
        0x00,
    );
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");

    emu.run_for(gbf_emu::CycleBudget::Clock(ClockCycles(128)))
        .expect("run succeeds");
    let trace = emu.drain_trace();
    let writes = trace
        .iter()
        .filter(|event| {
            matches!(
                event,
                NormalizedTraceEvent::MemoryWrite {
                    addr: 0xC000,
                    value: 0x55,
                    origin: TraceOrigin::GuestCpu,
                    ..
                }
            )
        })
        .count();

    assert_eq!(writes, 2);
}

#[test]
fn guest_io_trace_normalizes_io_and_mbc5_bank_switches() {
    let mut rom = common::rom(
        &[
            0x3E, 0x20, 0xE0, 0x00, 0x3E, 0x02, 0xEA, 0x00, 0x20, 0x3E, 0x01, 0xEA, 0x00, 0x30,
            0x3E, 0x03, 0xEA, 0x00, 0x40, 0x76,
        ],
        0x1A,
        0x03,
    );
    rom.resize(0x10000, 0);
    rom[0x0148] = 0x01;
    rom[0x014D] = rom[0x0134..=0x014C]
        .iter()
        .fold(0_u8, |acc, byte| acc.wrapping_add(!byte));
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("MBC5 ROM loads");

    emu.run_for(gbf_emu::CycleBudget::Clock(ClockCycles(256)))
        .expect("run succeeds");
    let trace = emu.drain_trace();

    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::IoWrite {
                reg: 0xFF00,
                value: 0x20,
                ..
            }
        )
    }));
    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::RomBankSwitch {
                from: 1,
                to: 2,
                source: BankSwitchSource::Bank1Write { value: 2 },
                ..
            }
        )
    }));
    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::RomBankSwitch {
                from: 2,
                to: 258,
                source: BankSwitchSource::Bank2Write { value: 1 },
                ..
            }
        )
    }));
    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::SramBankSwitch { from: 0, to: 3, .. }
        )
    }));
}

#[test]
fn non_mbc5_cartridge_does_not_emit_mbc5_bank_switch_events() {
    let mut emu = Emulator::load_rom(&common::rom(&[0x00], 0x00, 0x00), EmulatorConfig::default())
        .expect("ROM loads");

    emu.bus_write(0x2000, 0x02).expect("host bus write");

    assert!(!emu.drain_trace().iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::RomBankSwitch { .. }
                | NormalizedTraceEvent::SramBankSwitch { .. }
        )
    }));
}

#[test]
fn host_poke_audit_and_typed_passthrough_are_recorded() {
    let mut config = EmulatorConfig::default();
    config.audit_host_pokes = true;
    let mut emu = Emulator::load_rom(&common::rom(&[0x00], 0x00, 0x00), config).expect("ROM loads");

    emu.poke(0xC000, 0x44).expect("poke succeeds");
    let typed = TraceEvent {
        seq: 7,
        timestamp_m_cycles: 8,
        slice: SliceId(9),
        probe: TraceProbeId(10),
        checkpoint: CompactCheckpointId(11),
        data: [12; 16],
    };
    emu.record_typed_trace_event(typed)
        .expect("typed event records");

    let trace = emu.drain_trace();
    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::MemoryWrite {
                addr: 0xC000,
                value: 0x44,
                origin: TraceOrigin::HostPoke,
                ..
            }
        )
    }));
    assert!(
        trace
            .iter()
            .any(|event| matches!(event, NormalizedTraceEvent::Typed(event) if *event == typed))
    );
    serde_json::to_string(&trace).expect("normalized trace serializes");
}

#[test]
fn halt_and_error_drop_policy_is_enforced_through_adapter() {
    let mut config = EmulatorConfig::default();
    config.trace_capacity = 0;
    config.trace_drop_policy = TraceDropPolicy::HaltAndError;
    let mut emu = Emulator::load_rom(&common::rom(&[0x00], 0x00, 0x00), config).expect("ROM loads");

    assert_eq!(
        emu.bus_write(0xC000, 0x01),
        Err(gbf_emu::EmuError::TraceCapacityExceeded { capacity: 0 })
    );
}

#[test]
fn restore_resets_trace_bank_shadow() {
    let mut rom = common::rom(&[0x00], 0x1A, 0x03);
    rom.resize(0x10000, 0);
    rom[0x0148] = 0x01;
    rom[0x014D] = rom[0x0134..=0x014C]
        .iter()
        .fold(0_u8, |acc, byte| acc.wrapping_add(!byte));
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("MBC5 ROM loads");

    emu.bus_write(0x2000, 0x02).expect("switch to bank 2");
    let snapshot = emu.snapshot().expect("snapshot saves");
    emu.bus_write(0x2000, 0x03).expect("switch to bank 3");
    emu.restore(&snapshot).expect("snapshot restores");
    emu.drain_trace();
    emu.bus_write(0xC000, 0x99).expect("host bus write");

    assert!(emu.drain_trace().iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::MemoryWrite {
                addr: 0xC000,
                bank,
                ..
            } if bank.rom == 2
        )
    }));
}
