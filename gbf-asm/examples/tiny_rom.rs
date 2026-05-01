use std::error::Error;
use std::fs;
use std::path::PathBuf;

use gbf_asm::builder::Builder;
use gbf_asm::encoder::{EncodedSection, encode_section};
use gbf_asm::isa::{HighDirectOffset, Instr, Reg8};
use gbf_asm::layout::{PlacedSection, PlacementProfile, layout_into_banks};
use gbf_asm::listing::{ListingOptions, emit_program_listing};
use gbf_asm::lowering::{StubPreLayoutOpLowering, lower_pre_layout_ops};
use gbf_asm::relax::relax_and_legalize;
use gbf_asm::rom::{CartridgeHeader, assemble_rom};
use gbf_asm::section::{BranchKind, SectionId, SectionRole, SymbolicBranch};
use gbf_asm::symbols::{SymOptions, SymbolName, SymbolTable, write_sym};

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/review/f-a1"));
    fs::create_dir_all(&out_dir)?;

    let section = tiny_boot_section();
    let lowered = lower_pre_layout_ops(
        vec![section],
        &StubPreLayoutOpLowering::default(),
        &SymbolTable::new(),
    )?;
    let layout = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &[])?;
    let relaxed = relax_and_legalize(&lowered, &layout)?;

    let mut encoded_sections = Vec::new();
    let mut rom_pairs = Vec::new();
    for section in &relaxed.sections {
        let placed = relaxed
            .layout
            .sections
            .iter()
            .find(|candidate| candidate.id == section.id)
            .expect("relaxed section has placement")
            .clone();
        let encoded = encode_section(section, &placed)?;
        encoded_sections.push(encoded.clone());
        rom_pairs.push((encoded, placed));
    }

    let header = CartridgeHeader::new("GBFASM")?;
    let rom = assemble_rom(&rom_pairs, &relaxed.layout, &header)?;
    let listing = emit_program_listing(
        &relaxed.sections,
        &encoded_sections,
        &relaxed.layout,
        &relaxed.symbols,
        &ListingOptions {
            show_cycle_costs: true,
            ..ListingOptions::default()
        },
    )?;
    let sym = write_sym(
        &relaxed.layout,
        &relaxed.symbols,
        &SymOptions {
            include_externals_as_comments: false,
            dot_safe_separator: true,
            ..SymOptions::default()
        },
    )?;

    fs::write(out_dir.join("tiny_rom.gb"), rom)?;
    fs::write(out_dir.join("tiny_rom.lst"), listing)?;
    fs::write(out_dir.join("tiny_rom.sym"), sym)?;
    Ok(())
}

fn tiny_boot_section() -> gbf_asm::section::Section {
    let entry = SymbolName::runtime("tiny", "entry").expect("entry symbol");
    let loop_label = SymbolName::runtime("tiny", "loop").expect("loop symbol");
    let mut builder =
        Builder::new_with_id(SectionId::new(1), SectionRole::Bank0Nucleus, entry.clone());
    builder.label(entry);
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0x42,
    });
    builder.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(0x80),
    });
    builder.label(loop_label.clone());
    builder.branch(SymbolicBranch::new(BranchKind::Jump, None, loop_label));
    builder.finish()
}

#[allow(dead_code)]
fn _keep_types_referenced(_: &[EncodedSection], _: &[PlacedSection]) {}
