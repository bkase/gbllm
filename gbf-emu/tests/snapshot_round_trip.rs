mod common;

use gbf_emu::{Emulator, EmulatorConfig};
use gbf_foundation::SemVer;

#[test]
fn snapshot_round_trip_restores_registers_and_clock() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    emu.step().expect("first step");
    let regs = emu.regs();
    let clock = emu.clock_count();
    let snapshot = emu.snapshot().expect("snapshot saves");
    emu.step().expect("second step");

    emu.restore(&snapshot).expect("snapshot restores");

    assert_eq!(emu.regs(), regs);
    assert_eq!(emu.clock_count(), clock);
}

#[test]
fn snapshot_rejects_full_emu_version_mismatch() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let mut snapshot = emu.snapshot().expect("snapshot saves");
    snapshot.lineage.emu_version.gbf_emu_version = SemVer::new(99, 0, 0);

    assert!(matches!(
        emu.restore(&snapshot),
        Err(gbf_emu::EmuError::SnapshotEmuVersionMismatch { .. })
    ));
}
