#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use gbf_abi::FaultCode;
use gbf_asm::builder::Builder;
use gbf_asm::encoder::encode_section;
use gbf_asm::isa::{DirectAddr, HighDirectOffset, Instr, Reg8};
use gbf_asm::layout::{BankIndex, PinnedPlacement, PlacementProfile, layout_into_banks};
use gbf_asm::lowering::{PreLayoutOpLowering, StubPreLayoutOpLowering, lower_pre_layout_ops};
use gbf_asm::relax::relax_and_legalize;
use gbf_asm::rom::{CartridgeHeader, RamSize, RomSize, assemble_rom};
use gbf_asm::section::{
    Section, SectionId, SectionPrivilege, SectionRole, SymbolicBranch, YieldKind,
};
use gbf_asm::symbols::{SymOptions, SymbolName, SymbolTable, write_sym};
use gbf_debug::{ExecArgs, InitArgs, InspectArgs, ScriptConfig, run_exec, run_init, run_inspect};
use gbf_runtime::banking::{
    BankingPreLayoutLowering, LeaseLifetime, ReturnRomBank, ReturnSramState, ReturnState,
    ValidatedBankLeaseSpec, lease_rom_switchable, lease_sram, mbc_write_provenance_audit,
    release_bank,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const MANIFEST_PATH: &str = "docs/review/t-a8.8b/conformance-manifest.json";
const REVIEW_DIR: &str = "target/review/t-a8.8b";
const ROM_SWITCH_SECTION_ID: SectionId = SectionId::new(0xA88B);
const SRAM_WINDOW_SECTION_ID: SectionId = SectionId::new(0xA88C);
const PANIC_SMOKE_SECTION_ID: SectionId = SectionId::new(0xA88D);
const ROM_BANK3_SENTINEL_SECTION_ID: SectionId = SectionId::new(0xA88E);
const ROM_BANK256_SENTINEL_SECTION_ID: SectionId = SectionId::new(0xA88F);
const YIELD_SAFE_POINT_SECTION_ID: SectionId = SectionId::new(0xA890);
const ROM_BANK3_SENTINEL_ADDR: u16 = 0xC370;
const ROM_BANK256_SENTINEL_ADDR: u16 = 0xC371;
const YIELD_CLEARED_SENTINEL_ADDR: u16 = 0xC372;
const YIELD_SKIPPED_SENTINEL_ADDR: u16 = 0xC373;

#[test]
fn f_a1_tiny_rom_runs_through_scripted_debugger() {
    let manifest = manifest();
    let case = manifest.case("f-a1-tiny-rom");
    let root = workspace_root();
    run_case(
        case,
        FixtureBytes {
            rom: fs::read(root.join(&case.rom_path)).expect("tiny ROM fixture"),
            sym: fs::read_to_string(root.join(&case.sym_path)).expect("tiny ROM symbols"),
        },
    );
}

#[test]
fn f_a4_banklease_rom_switches_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a4-banklease-rom-switch"),
        banklease_rom_switch_fixture(),
    );
}

#[test]
fn f_a4_banklease_sram_window_runs_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a4-banklease-sram-window"),
        banklease_sram_window_fixture(),
    );
}

#[test]
fn f_a5_runtime_boots_to_scheduler_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a5-runtime-boot-scheduler"),
        runtime_demo_fixture(),
    );
}

#[test]
fn f_a5_runtime_timer_irq_sets_yield_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a5-runtime-irq-timer"),
        runtime_demo_fixture(),
    );
}

#[test]
fn f_a5_runtime_safe_point_clears_yield_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a5-runtime-yield-safe-point"),
        runtime_yield_safe_point_fixture(),
    );
}

#[test]
fn f_a5_runtime_panic_path_renders_fault_under_scripted_debugger() {
    let manifest = manifest();
    run_case(
        manifest.case("f-a5-runtime-panic-smoke"),
        runtime_panic_fixture(),
    );
}

fn run_case(case: &FixtureCase, fixture: FixtureBytes) {
    let (rom_path, sym_path) = materialize_fixture(case, &fixture);
    assert_fixture_hashes(case, &fixture);

    let first = run_once(case, &rom_path, &sym_path, 0);
    let second = run_once(case, &rom_path, &sym_path, 1);
    assert_eq!(
        first.result, second.result,
        "{} observations differed across deterministic reruns; first session: {}, second session: {}",
        case.id, first.session_path, second.session_path
    );
    assert_eq!(
        first.digest, second.digest,
        "{} observation digest differed across deterministic reruns; first session: {}, second session: {}",
        case.id, first.session_path, second.session_path
    );
    assert_eq!(
        first.digest, case.observation_sha256,
        "{} observation fingerprint drifted; update {MANIFEST_PATH} only after reviewing result and trace changes; session: {}",
        case.id, first.session_path
    );

    let expected = case
        .expected
        .as_object()
        .expect("manifest expected field is an object");
    for (key, expected_value) in expected {
        assert_eq!(
            first.result.get(key),
            Some(expected_value),
            "{} unexpected result field {key}; session: {}",
            case.id,
            first.session_path
        );
    }
}

fn assert_fixture_hashes(case: &FixtureCase, fixture: &FixtureBytes) {
    assert_eq!(
        sha256_hex(&fixture.rom),
        case.rom_sha256,
        "{} ROM hash drifted; update {MANIFEST_PATH} only after reviewing the fixture diff",
        case.id
    );
    assert_eq!(
        sha256_hex(normalized_sym_for_hash(&fixture.sym).as_bytes()),
        case.sym_sha256,
        "{} .sym hash drifted; update {MANIFEST_PATH} only after reviewing the symbol diff",
        case.id
    );
}

fn materialize_fixture(case: &FixtureCase, fixture: &FixtureBytes) -> (PathBuf, PathBuf) {
    let root = workspace_root();
    let rom_path = root.join(&case.rom_path);
    let sym_path = root.join(&case.sym_path);
    if case.rom_path.starts_with("target") {
        fs::create_dir_all(rom_path.parent().expect("target ROM has parent"))
            .expect("create generated review dir");
        fs::write(&rom_path, &fixture.rom).expect("write generated ROM fixture");
        fs::write(&sym_path, &fixture.sym).expect("write generated symbol fixture");
    } else {
        assert_eq!(
            fs::read(&rom_path).expect("manifest ROM readable"),
            fixture.rom,
            "{} manifest ROM path does not match fixture bytes",
            case.id
        );
        assert_eq!(
            fs::read_to_string(&sym_path).expect("manifest .sym readable"),
            fixture.sym,
            "{} manifest .sym path does not match fixture symbols",
            case.id
        );
    }
    (rom_path, sym_path)
}

fn run_once(case: &FixtureCase, rom_path: &Path, sym_path: &Path, index: u8) -> CaseRun {
    let root = workspace_root();
    let script_path = root.join(&case.script);
    let run_dir = root
        .join(REVIEW_DIR)
        .join("runs")
        .join(&case.id)
        .join(format!("run{index}"));
    fs::create_dir_all(&run_dir).expect("create run artifact dir");
    let s0 = run_dir.join("s0.gbsess");
    let s1 = run_dir.join("s1.gbsess");

    let init = run_init(InitArgs {
        rom_path: rom_path.to_path_buf(),
        sym_path: Some(sym_path.to_path_buf()),
        out_path: s0.clone(),
        trace_capacity: case.trace_capacity,
        replace_existing_out: true,
    })
    .unwrap_or_else(|error| {
        panic!(
            "{} init failed: {error}; reproduce after fixing init with ROM {} and SYM {}",
            case.id,
            rom_path.display(),
            sym_path.display()
        )
    });
    assert!(init.symbol_count > 0, "{} hydrated symbols", case.id);

    let exec = run_exec(ExecArgs {
        in_path: s0.clone(),
        script_path: script_path.clone(),
        out_path: s1.clone(),
        config: ScriptConfig::default(),
        emit_metrics: false,
        write_partial_on_timeout: true,
        replace_existing_out: true,
    })
    .unwrap_or_else(|error| {
        panic!(
            "{} script failed: {error}; reproduce with `cargo run -p gbf-debug -- exec --in {} --script {} --out {}`",
            case.id,
            s0.display(),
            script_path.display(),
            s1.display()
        )
    });
    let inspect = run_inspect(InspectArgs {
        in_path: s1.clone(),
    })
    .expect("inspect completed session");
    assert_eq!(
        inspect.trace_ring_summary.dropped,
        0,
        "{} dropped trace events; session: {}",
        case.id,
        s1.display()
    );
    let digest = sha256_hex(
        &serde_json::to_vec(&json!({
            "case": case.id,
            "rom_sha256": case.rom_sha256,
            "sym_sha256": case.sym_sha256,
            "result": exec.result,
            "trace_ring": inspect.trace_ring_summary,
        }))
        .expect("digest JSON serializes"),
    );
    CaseRun {
        result: exec.result,
        digest,
        session_path: s1.display().to_string(),
    }
}

fn banklease_rom_switch_fixture() -> FixtureBytes {
    let entry = SymbolName::runtime("conformance", "rom_switch_entry").expect("entry symbol");
    let done = SymbolName::runtime("conformance", "rom_switch_done").expect("done symbol");
    let mut builder = Builder::new_with_id(
        ROM_SWITCH_SECTION_ID,
        SectionRole::Bank0Nucleus,
        entry.clone(),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(entry);
    for bank in [3, 256] {
        let guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(bank, LeaseLifetime::Slice)
                .expect("valid ROM lease"),
        )
        .expect("lease emits");
        builder.emit(Instr::LdAFromDirect {
            addr: DirectAddr::new(0x4000).expect("ROMX sentinel address is direct"),
        });
        builder.emit(Instr::LdDirectFromA {
            addr: DirectAddr::new(if bank == 3 {
                ROM_BANK3_SENTINEL_ADDR
            } else {
                ROM_BANK256_SENTINEL_ADDR
            })
            .expect("WRAM sentinel address is direct"),
        });
        release_bank(&mut builder, guard, ReturnState::Rom(ReturnRomBank::Bank1))
            .expect("release emits");
    }
    builder.label(done.clone());
    builder.branch(SymbolicBranch::jump(done, None));

    let lowerer = BankingPreLayoutLowering::default();
    let lowered = lower_sections(
        vec![
            builder.finish(),
            romx_sentinel_section(ROM_BANK3_SENTINEL_SECTION_ID, "rom_bank3_sentinel", 0xA3),
            romx_sentinel_section(
                ROM_BANK256_SENTINEL_SECTION_ID,
                "rom_bank256_sentinel",
                0xC0,
            ),
        ],
        &lowerer,
    );
    mbc_write_provenance_audit(&lowered, &Default::default(), &lowerer)
        .expect("MBC writes are trusted banking emits");
    assemble_lowered_fixture(
        "GBFA8BROM",
        lowered,
        &[
            PinnedPlacement {
                section_id: ROM_BANK3_SENTINEL_SECTION_ID,
                bank: BankIndex::Rom(3),
                cpu_start: 0x4000,
            },
            PinnedPlacement {
                section_id: ROM_BANK256_SENTINEL_SECTION_ID,
                bank: BankIndex::Rom(256),
                cpu_start: 0x4000,
            },
        ],
        RomSize::Mib8,
        RamSize::None,
    )
}

fn banklease_sram_window_fixture() -> FixtureBytes {
    let entry = SymbolName::runtime("conformance", "sram_window_entry").expect("entry symbol");
    let done = SymbolName::runtime("conformance", "sram_window_done").expect("done symbol");
    let mut builder = Builder::new_with_id(
        SRAM_WINDOW_SECTION_ID,
        SectionRole::Bank0Nucleus,
        entry.clone(),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(entry);
    let guard = lease_sram(
        &mut builder,
        ValidatedBankLeaseSpec::for_sram(2, LeaseLifetime::Slice).expect("valid SRAM lease"),
    )
    .expect("SRAM lease emits");
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0x5A,
    });
    builder.emit(Instr::LdDirectFromA {
        addr: DirectAddr::new(0xA000).expect("SRAM address is direct"),
    });
    release_bank(
        &mut builder,
        guard,
        ReturnState::Sram(ReturnSramState::Disable),
    )
    .expect("SRAM release emits");
    builder.label(done.clone());
    builder.branch(SymbolicBranch::jump(done, None));

    let lowerer = BankingPreLayoutLowering::default();
    let lowered = lower_sections(vec![builder.finish()], &lowerer);
    mbc_write_provenance_audit(&lowered, &Default::default(), &lowerer)
        .expect("MBC writes are trusted banking emits");
    assemble_lowered_fixture("GBFA8BSRAM", lowered, &[], RomSize::Kib64, RamSize::Kib64)
}

fn runtime_demo_fixture() -> FixtureBytes {
    FixtureBytes {
        rom: gbf_runtime::demo_bank0_rom_image().expect("F-A5 demo ROM emits"),
        sym: gbf_runtime::demo_bank0_sym_file().expect("F-A5 demo symbols emit"),
    }
}

fn runtime_panic_fixture() -> FixtureBytes {
    let entry = SymbolName::runtime("conformance", "panic_entry").expect("entry symbol");
    let mut builder = Builder::new_with_id(
        PANIC_SMOKE_SECTION_ID,
        SectionRole::Bank0Nucleus,
        entry.clone(),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(entry);
    gbf_runtime::panic::emit_panic(&mut builder, FaultCode::UiCommitQueueFull);
    assemble_lowered_fixture(
        "GBFA8BPANIC",
        lower_sections(vec![builder.finish()], &StubPreLayoutOpLowering::default()),
        &[],
        RomSize::Kib64,
        RamSize::None,
    )
}

fn runtime_yield_safe_point_fixture() -> FixtureBytes {
    let entry = SymbolName::runtime("conformance", "yield_safe_point_entry").expect("entry symbol");
    let yielded =
        SymbolName::runtime("conformance", "yield_safe_point_observed").expect("yield symbol");
    let continue_label =
        SymbolName::runtime("conformance", "yield_safe_point_continue").expect("continue symbol");
    let done = SymbolName::runtime("conformance", "yield_safe_point_done").expect("done symbol");
    let mut builder = Builder::new_with_id(
        YIELD_SAFE_POINT_SECTION_ID,
        SectionRole::Bank0Nucleus,
        entry.clone(),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(entry);
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    builder.emit(Instr::LdDirectFromA {
        addr: DirectAddr::new(YIELD_CLEARED_SENTINEL_ADDR).expect("WRAM sentinel address"),
    });
    builder.emit(Instr::LdDirectFromA {
        addr: DirectAddr::new(YIELD_SKIPPED_SENTINEL_ADDR).expect("WRAM sentinel address"),
    });
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 1,
    });
    builder.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(gbf_runtime::scheduler::HRAM_LDH_YIELD_REQUESTED),
    });
    gbf_runtime::scheduler::emit_yield_check(
        &mut builder,
        YieldKind::Cooperative,
        continue_label.clone(),
    );
    builder.label(yielded);
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0x59,
    });
    builder.emit(Instr::LdDirectFromA {
        addr: DirectAddr::new(YIELD_CLEARED_SENTINEL_ADDR).expect("WRAM sentinel address"),
    });
    builder.branch(SymbolicBranch::jump(done.clone(), None));
    builder.label(continue_label);
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0x4E,
    });
    builder.emit(Instr::LdDirectFromA {
        addr: DirectAddr::new(YIELD_SKIPPED_SENTINEL_ADDR).expect("WRAM sentinel address"),
    });
    builder.branch(SymbolicBranch::jump(done.clone(), None));
    builder.label(done.clone());
    builder.branch(SymbolicBranch::jump(done, None));

    assemble_lowered_fixture(
        "GBFA8BYIELD",
        lower_sections(vec![builder.finish()], &StubPreLayoutOpLowering::default()),
        &[],
        RomSize::Kib64,
        RamSize::None,
    )
}

fn romx_sentinel_section(id: SectionId, symbol: &'static str, value: u8) -> Section {
    let name = SymbolName::runtime("conformance", symbol).expect("sentinel symbol");
    let mut builder = Builder::new_with_id(id, SectionRole::CommonData, name.clone());
    builder.label(name);
    builder.db_bytes([value]);
    builder.finish()
}

fn lower_sections(
    sections: Vec<Section>,
    lowerer: &dyn PreLayoutOpLowering,
) -> Vec<gbf_asm::section::LoweredSection> {
    lower_pre_layout_ops(sections, lowerer, &SymbolTable::new()).expect("pre-layout ops lower")
}

fn assemble_lowered_fixture(
    title: &str,
    lowered: Vec<gbf_asm::section::LoweredSection>,
    pinned: &[PinnedPlacement],
    rom_size: RomSize,
    ram_size: RamSize,
) -> FixtureBytes {
    let layout = layout_into_banks(&lowered, PlacementProfile::PackedExperts, pinned)
        .expect("fixture lays out");
    let linked = relax_and_legalize(&lowered, &layout).expect("fixture relaxes");

    let mut rom_pairs = Vec::new();
    for section in &linked.sections {
        let placed = linked
            .layout
            .sections
            .iter()
            .find(|candidate| candidate.id == section.id)
            .expect("linked section has placement")
            .clone();
        rom_pairs.push((
            encode_section(section, &placed).expect("section encodes"),
            placed,
        ));
    }

    let mut header = CartridgeHeader::new(title).expect("valid title");
    header.rom_size = rom_size;
    header.ram_size = ram_size;
    let rom = assemble_rom(&rom_pairs, &linked.layout, &header).expect("ROM assembles");
    let sym = write_sym(
        &linked.layout,
        &linked.symbols,
        &SymOptions {
            include_externals_as_comments: true,
            rom_only: true,
            dot_safe_separator: false,
        },
    )
    .expect("symbols emit");

    FixtureBytes { rom, sym }
}

fn manifest() -> ConformanceManifest {
    let root = workspace_root();
    let text = fs::read_to_string(root.join(MANIFEST_PATH)).expect("manifest readable");
    serde_json::from_str(&text).expect("manifest parses")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn normalized_sym_for_hash(sym: &str) -> String {
    sym.replace("\r\n", "\n")
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug)]
struct FixtureBytes {
    rom: Vec<u8>,
    sym: String,
}

#[derive(Debug)]
struct CaseRun {
    result: serde_json::Value,
    digest: String,
    session_path: String,
}

#[derive(Debug, Deserialize)]
struct ConformanceManifest {
    fixtures: Vec<FixtureCase>,
}

impl ConformanceManifest {
    fn case(&self, id: &str) -> &FixtureCase {
        self.fixtures
            .iter()
            .find(|case| case.id == id)
            .unwrap_or_else(|| panic!("manifest missing fixture {id}"))
    }
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    id: String,
    rom_path: PathBuf,
    sym_path: PathBuf,
    rom_sha256: String,
    sym_sha256: String,
    observation_sha256: String,
    script: PathBuf,
    trace_capacity: u32,
    expected: serde_json::Value,
}
