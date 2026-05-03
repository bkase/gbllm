mod common;

use gbf_emu::{DeterminismPolicy, Emulator, EmulatorConfig};

#[test]
fn snapshot_lineage_matches_loaded_rom_and_policy() {
    let config = EmulatorConfig {
        policy: DeterminismPolicy::default(),
        ..EmulatorConfig::default()
    };
    let emu = Emulator::load_rom(&common::rom(&[0x00], 0x00, 0x00), config).expect("ROM loads");
    let snapshot = emu.snapshot().expect("snapshot saves");

    assert_eq!(snapshot.lineage.rom_sha256, emu.rom_sha256());
    assert_eq!(
        snapshot.lineage.policy_fingerprint,
        emu.policy().fingerprint()
    );
    assert_eq!(
        snapshot.lineage.emu_version.gameroy_git_rev,
        emu.version_tag().gameroy_git_rev
    );
}
