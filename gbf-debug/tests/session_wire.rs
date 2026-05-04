#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;

use gbf_debug::{
    InitArgs, SCHEMA_VERSION, Session, SessionLoadError, SessionSymbolTable, SymbolResolutionError,
    WatchpointKind, run_init,
};
use gbf_foundation::Hash256;

#[test]
fn symbols_sorted_and_ambiguous() {
    let hydrated =
        SessionSymbolTable::from_sym_text("01:4000 same\n00:0100 same\n").expect("hydrate");
    assert_eq!(hydrated.table.entries[0].bank, Some(0));
    assert!(matches!(
        hydrated.table.resolve("same"),
        Err(SymbolResolutionError::AmbiguousName { .. })
    ));
    assert_eq!(hydrated.table.resolve_in_bank("same", 1), Some(0x4000));
}

#[test]
fn watchpoint_kind_parse_contract() {
    assert_eq!(WatchpointKind::parse("read"), Some(WatchpointKind::Read));
    assert_eq!(WatchpointKind::parse("write"), Some(WatchpointKind::Write));
    assert_eq!(WatchpointKind::parse("rw"), Some(WatchpointKind::ReadWrite));
    assert_eq!(WatchpointKind::parse("other"), None);
}

#[test]
fn session_wire_rejects_bad_container_inputs() {
    let bytes = valid_session_bytes();

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        Session::load_bytes(&bad_magic),
        Err(SessionLoadError::BadMagic { .. })
    ));

    let mut bad_flags = bytes.clone();
    bad_flags[4] = 1;
    assert!(matches!(
        Session::load_bytes(&bad_flags),
        Err(SessionLoadError::BadFlags { observed: 1 })
    ));

    assert!(matches!(
        Session::load_bytes(&bytes[..4]),
        Err(SessionLoadError::Truncated { .. })
    ));

    let mut bad_zstd = bytes.clone();
    bad_zstd.truncate(10);
    assert!(matches!(
        Session::load_bytes(&bad_zstd),
        Err(SessionLoadError::ZstdDecode(_))
    ));

    let mut bad_json = b"GBSE".to_vec();
    bad_json.extend_from_slice(&0_u32.to_le_bytes());
    bad_json.extend(zstd::encode_all(&b"not json"[..], 3).expect("compress"));
    assert!(matches!(
        Session::load_bytes(&bad_json),
        Err(SessionLoadError::JsonDecode(_))
    ));
}

#[test]
fn session_wire_rejects_schema_and_lineage_mismatches() {
    let bytes = valid_session_bytes();
    let mut session = Session::load_bytes(&bytes).expect("valid session");

    session.schema_version = SCHEMA_VERSION + 1;
    assert!(matches!(
        Session::load_bytes(&session.to_bytes().expect("serialize")),
        Err(SessionLoadError::SchemaMismatch { .. })
    ));

    let mut session = Session::load_bytes(&bytes).expect("valid session");
    session.rom_sha256[0] ^= 0xff;
    assert!(matches!(
        Session::load_bytes(&session.to_bytes().expect("serialize")),
        Err(SessionLoadError::RomHashMismatch { .. })
    ));

    let mut session = Session::load_bytes(&bytes).expect("valid session");
    let mut snapshot_hash = session.rom_sha256;
    snapshot_hash[0] ^= 0xff;
    session.emulator_snapshot.0.lineage.rom_sha256 = Hash256::from_bytes(snapshot_hash);
    assert!(matches!(
        Session::load_bytes(&session.to_bytes().expect("serialize")),
        Err(SessionLoadError::SnapshotRomMismatch { .. })
    ));
}

fn valid_session_bytes() -> Vec<u8> {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("s0.gbsess");
    let root = workspace_root();
    run_init(InitArgs {
        rom_path: root.join("gbf-emu/tests/fixtures/tiny_rom.gb"),
        sym_path: Some(root.join("docs/review/f-a1/artifacts/tiny_rom.sym")),
        out_path: out.clone(),
        trace_capacity: 16,
        replace_existing_out: false,
    })
    .expect("init");
    fs::read(out).expect("read session")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
