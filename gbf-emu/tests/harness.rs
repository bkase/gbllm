mod common;

use gbf_abi::harness::{
    HarnessCommandBlock, HarnessOp, HarnessResultBlock, HarnessResultKind, doorbell,
};
use gbf_emu::{
    Emulator, EmulatorConfig, HarnessResult, HarnessSlot, NormalizedTraceEvent, TraceOrigin,
};

#[test]
fn harness_poll_and_result_use_sram_direct_plumbing() {
    let mut config = EmulatorConfig::default();
    config.audit_host_pokes = true;
    let mut emu =
        Emulator::load_rom(&common::rom(&[0x00], 0x1A, 0x02), config).expect("MBC5+RAM ROM loads");
    emu.attach_harness(HarnessSlot {
        sram_bank: 0,
        command_addr: 0xA000,
        result_addr: 0xA040,
        doorbell_addr: 0xA080,
    });

    let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [9; 32]);
    command.raise_doorbell();
    for (offset, value) in command.to_bytes().iter().copied().enumerate() {
        emu.poke(0xA000 + offset as u16, value)
            .expect("write command");
    }
    emu.poke(0xA080, doorbell::DOORBELL_RAISED)
        .expect("write doorbell");
    emu.drain_trace();

    let parsed = emu
        .poll_harness()
        .expect("poll succeeds")
        .expect("command is present");
    assert_eq!(parsed.block, command);
    let before = emu.clock_count();
    emu.write_harness_result(HarnessResult {
        block: HarnessResultBlock::new(1, HarnessResultKind::Ok, [1; 32]),
    })
    .expect("result writes");

    assert_eq!(emu.clock_count(), before);
    assert_eq!(
        emu.peek(0xA040 + doorbell::RESULT_READY_OFFSET as u16),
        Ok(doorbell::DOORBELL_RAISED)
    );

    let trace = emu.drain_trace();
    assert!(trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::MemoryWrite {
                addr: 0xA040,
                origin: TraceOrigin::HostPoke,
                ..
            }
        )
    }));
    assert!(!trace.iter().any(|event| {
        matches!(
            event,
            NormalizedTraceEvent::MemoryWrite {
                origin: TraceOrigin::GuestCpu,
                ..
            }
        )
    }));
}
