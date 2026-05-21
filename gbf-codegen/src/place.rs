//! Stage 12 placement: symbolic `AsmIR` to legalized, placed ROM sections.
//!
//! This is the F-B15 owner boundary around the F-A1 layout/relax machinery. It
//! keeps the report-facing plan/map schemas as string-key-free row lists so
//! canonical JSON consumers do not inherit Rust map-key encodings.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_asm::layout::{
    self, BankIndex, LayoutPlan, PinnedPlacement, PlacedSection,
    PlacementProfile as AsmPlacementProfile,
};
use gbf_asm::lowering::{self, StubPreLayoutOpLowering};
use gbf_asm::relax::{self, RelaxedProgram, ResolvedThunkRequest};
use gbf_asm::section::{LegalizedSection, LoweredSection, SectionId, SectionRole};
use gbf_asm::symbols::SymbolTable;
use gbf_foundation::{DomainHash, Hash256, self_hash_omitting_fields};
use gbf_policy::PlacementProfile;
use serde::{Deserialize, Serialize};

use crate::lower_asm::{AsmIRBundle, canonical_sections};
use crate::reachability::{
    PrePlacementReachability, PrePlacementSectionClass, ReachabilityClass, ReachabilityReport,
};

pub const PLACED_ROM_LAYOUT_VERSION: &str = "f-b15-placed-rom-v1";
const PLACED_ROM_SCHEMA_ID: &str = "gbf.codegen.f_b15.placed_rom";
const PLACED_ROM_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedRom {
    pub layout_version: String,
    pub placement_profile: PlacementProfile,
    pub lowered_sections: Vec<LoweredSection>,
    pub legalized_sections: Vec<LegalizedSection>,
    pub layout: LayoutPlan,
    pub symbol_table: SymbolTable,
    pub thunk_pool: Vec<ResolvedThunkRequest>,
    pub layout_iterations: u8,
    pub bank_assignments: Vec<BankAssignment>,
    pub preplacement_reachability_classes: Vec<PrePlacementSectionClass>,
    pub placed_rom_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BankAssignment {
    pub section_id: u32,
    pub role: String,
    pub bank: String,
    pub cpu_start: u16,
    pub final_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedRomPlanReport {
    pub placement_profile: PlacementProfile,
    pub layout_iterations: u8,
    pub bank_assignments: Vec<BankAssignment>,
    pub thunks: Vec<ThunkReportEntry>,
    pub global_constraints: Vec<PlacedRomConstraintStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThunkReportEntry {
    pub thunk_symbol: String,
    pub target: String,
    pub callee_bank: String,
    pub target_cpu_addr: u16,
    pub lease_chain: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedRomConstraintStatus {
    pub rule: String,
    pub status: ConstraintStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintStatus {
    Holds,
    Violated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMapReport {
    pub sections: Vec<AddressMapSection>,
    pub symbols: Vec<AddressMapSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMapSection {
    pub section_id: u32,
    pub role: String,
    pub bank: String,
    pub address_space: String,
    pub cpu_start: u16,
    pub cpu_end_exclusive: u32,
    pub rom_file_offset: Option<usize>,
    pub final_size: u16,
    pub reachability_classes: Vec<ReachabilityClass>,
    pub provenance_summary: AddressMapProvenanceSummary,
    pub cycles_estimate: Option<u32>,
    pub bank_switches_estimate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMapProvenanceSummary {
    pub item_count: usize,
    pub stages: Vec<String>,
    pub source_ops: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressMapSymbol {
    pub name: String,
    pub section_id: u32,
    pub offset: u32,
}

#[derive(Debug)]
pub enum PlacedRomError {
    Lowering(lowering::LoweringError),
    Layout(layout::LayoutError),
    Relax(relax::RelaxError),
    MissingPlacement { section_id: SectionId },
    CanonicalHash(String),
}

impl fmt::Display for PlacedRomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lowering(error) => write!(f, "{error}"),
            Self::Layout(error) => write!(f, "{error}"),
            Self::Relax(error) => write!(f, "{error}"),
            Self::MissingPlacement { section_id } => {
                write!(f, "section {} has no placement", section_id.get())
            }
            Self::CanonicalHash(message) => write!(f, "placed ROM hash failed: {message}"),
        }
    }
}

impl std::error::Error for PlacedRomError {}

impl From<lowering::LoweringError> for PlacedRomError {
    fn from(value: lowering::LoweringError) -> Self {
        Self::Lowering(value)
    }
}

impl From<layout::LayoutError> for PlacedRomError {
    fn from(value: layout::LayoutError) -> Self {
        Self::Layout(value)
    }
}

impl From<relax::RelaxError> for PlacedRomError {
    fn from(value: relax::RelaxError) -> Self {
        Self::Relax(value)
    }
}

pub fn place_asmir_bundle(
    bundle: &AsmIRBundle,
    placement_profile: PlacementProfile,
) -> Result<PlacedRom, PlacedRomError> {
    place_asmir_bundle_with_pins_and_reachability(
        bundle,
        placement_profile,
        &[],
        &PrePlacementReachability::default(),
    )
}

pub fn place_asmir_bundle_with_pins(
    bundle: &AsmIRBundle,
    placement_profile: PlacementProfile,
    pinned: &[PinnedPlacement],
) -> Result<PlacedRom, PlacedRomError> {
    place_asmir_bundle_with_pins_and_reachability(
        bundle,
        placement_profile,
        pinned,
        &PrePlacementReachability::default(),
    )
}

pub fn place_asmir_bundle_with_reachability(
    bundle: &AsmIRBundle,
    placement_profile: PlacementProfile,
    reachability: &PrePlacementReachability,
) -> Result<PlacedRom, PlacedRomError> {
    place_asmir_bundle_with_pins_and_reachability(bundle, placement_profile, &[], reachability)
}

pub fn place_asmir_bundle_with_pins_and_reachability(
    bundle: &AsmIRBundle,
    placement_profile: PlacementProfile,
    pinned: &[PinnedPlacement],
    reachability: &PrePlacementReachability,
) -> Result<PlacedRom, PlacedRomError> {
    let sections = canonical_sections(bundle.all_sections());
    let lowered = lowering::lower_pre_layout_ops(
        sections,
        &StubPreLayoutOpLowering::default(),
        &SymbolTable::new(),
    )?;
    let layout = layout::layout_into_banks(&lowered, asm_profile(placement_profile), pinned)?;
    let relaxed = relax::relax_and_legalize(&lowered, &layout)?;
    build_placed_rom(placement_profile, lowered, relaxed, reachability)
}

fn build_placed_rom(
    placement_profile: PlacementProfile,
    lowered_sections: Vec<LoweredSection>,
    relaxed: RelaxedProgram,
    reachability: &PrePlacementReachability,
) -> Result<PlacedRom, PlacedRomError> {
    let bank_assignments = bank_assignments(&relaxed.layout, &relaxed.sections);
    let mut placed = PlacedRom {
        layout_version: PLACED_ROM_LAYOUT_VERSION.to_owned(),
        placement_profile,
        lowered_sections,
        legalized_sections: relaxed.sections,
        layout: relaxed.layout,
        symbol_table: relaxed.symbols,
        thunk_pool: relaxed.thunk_requests,
        layout_iterations: relaxed.iterations,
        bank_assignments,
        preplacement_reachability_classes: reachability.section_classes.clone(),
        placed_rom_self_hash: Hash256::ZERO,
    };
    placed.placed_rom_self_hash = placed_rom_hash(&placed)
        .map_err(|error| PlacedRomError::CanonicalHash(error.to_string()))?;
    Ok(placed)
}

pub fn placed_rom_hash(placed: &PlacedRom) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "PlacedRom",
            PLACED_ROM_SCHEMA_ID,
            PLACED_ROM_SCHEMA_VERSION,
        ),
        placed,
        "placed_rom_self_hash",
        &[],
    )
}

#[must_use]
pub fn placed_rom_plan_report(placed: &PlacedRom) -> PlacedRomPlanReport {
    PlacedRomPlanReport {
        placement_profile: placed.placement_profile,
        layout_iterations: placed.layout_iterations,
        bank_assignments: placed.bank_assignments.clone(),
        thunks: placed
            .thunk_pool
            .iter()
            .map(|thunk| ThunkReportEntry {
                thunk_symbol: thunk.thunk_symbol.as_str().to_owned(),
                target: thunk.target.as_str().to_owned(),
                callee_bank: thunk.callee_bank.to_string(),
                target_cpu_addr: thunk.target_cpu_addr,
                lease_chain: thunk.lease_chain.iter().map(|lease| lease.get()).collect(),
            })
            .collect(),
        global_constraints: vec![
            PlacedRomConstraintStatus {
                rule: "section_boundary".to_owned(),
                status: ConstraintStatus::Holds,
            },
            PlacedRomConstraintStatus {
                rule: "placement_determinism".to_owned(),
                status: ConstraintStatus::Holds,
            },
            PlacedRomConstraintStatus {
                rule: "continuation_validity_deferred_to_reachability".to_owned(),
                status: ConstraintStatus::Holds,
            },
        ],
    }
}

pub fn address_map_report(placed: &PlacedRom) -> Result<AddressMapReport, PlacedRomError> {
    address_map_report_with_reachability(placed, None)
}

pub fn address_map_report_with_reachability(
    placed: &PlacedRom,
    reachability: Option<&ReachabilityReport>,
) -> Result<AddressMapReport, PlacedRomError> {
    let reachability_by_section = reachability_classes_by_section(placed, reachability);
    let mut sections = Vec::with_capacity(placed.layout.sections.len());
    for section in &placed.layout.sections {
        let legalized = placed
            .legalized_sections
            .iter()
            .find(|candidate| candidate.id == section.id);
        let role = legalized
            .map(|section| section.role)
            .unwrap_or(SectionRole::HeaderCartridge);
        sections.push(AddressMapSection {
            section_id: section.id.get(),
            role: role.canonical_name().to_owned(),
            bank: section.bank.to_string(),
            address_space: format!("{:?}", section.space).to_ascii_lowercase(),
            cpu_start: section.cpu_start,
            cpu_end_exclusive: section.cpu_end_exclusive(),
            rom_file_offset: section.rom_file_offset().map_err(PlacedRomError::Layout)?,
            final_size: section.final_size,
            reachability_classes: reachability_by_section
                .get(&section.id.get())
                .cloned()
                .unwrap_or_default(),
            provenance_summary: legalized
                .map(provenance_summary)
                .unwrap_or_else(AddressMapProvenanceSummary::empty),
            cycles_estimate: legalized.map(cycles_estimate),
            // F-B14 ScheduleCostReport annotations are not wired into the
            // Stage-12 narrow-v1 bridge yet. Keep the field present and
            // explicit rather than silently emitting a zero that would look
            // like a computed bank-switch budget.
            bank_switches_estimate: None,
        });
    }
    sections.sort_by_key(|section| {
        (
            section.rom_file_offset.unwrap_or(usize::MAX),
            section.bank.clone(),
            section.cpu_start,
            section.section_id,
        )
    });

    let mut symbols: Vec<_> = placed
        .symbol_table
        .iter()
        .map(|(name, addr)| AddressMapSymbol {
            name: name.as_str().to_owned(),
            section_id: addr.section.get(),
            offset: addr.offset,
        })
        .collect();
    symbols.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(AddressMapReport { sections, symbols })
}

impl AddressMapProvenanceSummary {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            item_count: 0,
            stages: Vec::new(),
            source_ops: Vec::new(),
        }
    }
}

fn reachability_classes_by_section(
    placed: &PlacedRom,
    reachability: Option<&ReachabilityReport>,
) -> BTreeMap<u32, Vec<ReachabilityClass>> {
    let mut out = BTreeMap::new();
    for row in &placed.preplacement_reachability_classes {
        out.insert(row.section_id, row.classes.clone());
    }
    if let Some(reachability) = reachability {
        for row in &reachability.section_classes {
            out.insert(row.section_id, row.classes.clone());
        }
    }
    out
}

fn provenance_summary(section: &LegalizedSection) -> AddressMapProvenanceSummary {
    let mut stages = BTreeSet::new();
    let mut source_ops = BTreeSet::new();
    let mut item_count = 0;
    for provenance in section
        .labels
        .iter()
        .map(|item| &item.provenance)
        .chain(section.instrs.iter().map(|item| &item.provenance))
        .chain(section.data_blocks.iter().map(|item| &item.provenance))
        .chain(section.alignments.iter().map(|item| &item.provenance))
    {
        item_count += 1;
        stages.insert(provenance.stage.canonical_name().to_owned());
        if let Some(source_op) = &provenance.source_op {
            source_ops.insert(source_op.to_string());
        }
    }
    AddressMapProvenanceSummary {
        item_count,
        stages: stages.into_iter().collect(),
        source_ops: source_ops.into_iter().collect(),
    }
}

fn cycles_estimate(section: &LegalizedSection) -> u32 {
    section
        .instrs
        .iter()
        .map(|instr| u32::from(instr.data.cycle_cost().worst_case()))
        .sum()
}

pub(crate) fn placed_section(placed: &PlacedRom, section_id: SectionId) -> Option<&PlacedSection> {
    placed.layout.placement_for(section_id)
}

fn bank_assignments(layout: &LayoutPlan, sections: &[LegalizedSection]) -> Vec<BankAssignment> {
    let mut assignments: Vec<_> = layout
        .sections
        .iter()
        .map(|section| BankAssignment {
            section_id: section.id.get(),
            role: sections
                .iter()
                .find(|candidate| candidate.id == section.id)
                .map(|section| section.role.canonical_name())
                .unwrap_or("banking_thunk")
                .to_owned(),
            bank: section.bank.to_string(),
            cpu_start: section.cpu_start,
            final_size: section.final_size,
        })
        .collect();
    assignments.sort_by_key(|assignment| {
        (
            bank_sort_key(&assignment.bank),
            assignment.cpu_start,
            assignment.section_id,
        )
    });
    assignments
}

fn bank_sort_key(bank: &str) -> (u8, u16) {
    if let Some(rest) = bank.strip_prefix("rom") {
        return (0, rest.parse().unwrap_or(u16::MAX));
    }
    if let Some(rest) = bank.strip_prefix("sram") {
        return (1, rest.parse().unwrap_or(u16::MAX));
    }
    (2, u16::MAX)
}

const fn asm_profile(profile: PlacementProfile) -> AsmPlacementProfile {
    match profile {
        PlacementProfile::StrictOnePerBank => AsmPlacementProfile::StrictOneExpertPerBank,
        PlacementProfile::Budgeted => AsmPlacementProfile::Budgeted {
            reserve_bytes_per_bank: 0,
        },
        PlacementProfile::PackedExperts => AsmPlacementProfile::PackedExperts,
    }
}

#[must_use]
pub(crate) const fn is_fixed_resident_bank(bank: BankIndex) -> bool {
    matches!(bank, BankIndex::Rom(0) | BankIndex::Wram | BankIndex::Hram)
}

#[cfg(test)]
mod tests {
    use gbf_asm::builder::Builder;
    use gbf_asm::isa::Instr;
    use gbf_asm::section::{Section, SectionId, SectionRole};
    use gbf_asm::symbols::SymbolName;

    use super::*;
    use crate::lower_asm::{AsmIRCodegenInput, build_asmir_bundle};
    use crate::reachability::{
        ReachabilityClass, ReachabilityRoot, ReachabilityRootKind, ReachabilityValidationInput,
        preplacement_reachability_from_asmir,
    };

    fn section(id: u32, role: SectionRole, name: &'static str, instr: Instr) -> Section {
        let name = SymbolName::new(name).expect("symbol");
        let mut builder = Builder::new_with_id(SectionId::new(id), role, name.clone());
        builder.label(name);
        builder.emit(instr);
        builder.finish()
    }

    fn bundle(sections: Vec<Section>) -> AsmIRBundle {
        build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: sections,
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle")
    }

    #[test]
    fn placement_is_deterministic_for_input_order() {
        let a = section(
            2,
            SectionRole::ExpertBank,
            "expert.0.2",
            Instr::Ret { cond: None },
        );
        let b = section(
            1,
            SectionRole::Bank0Nucleus,
            "runtime.test.entry",
            Instr::Nop,
        );
        let first = place_asmir_bundle(
            &bundle(vec![a.clone(), b.clone()]),
            PlacementProfile::Budgeted,
        )
        .expect("first placement");
        let second = place_asmir_bundle(&bundle(vec![b, a]), PlacementProfile::Budgeted)
            .expect("second placement");

        assert_eq!(first.placed_rom_self_hash, second.placed_rom_self_hash);
        assert_eq!(first.layout.sections, second.layout.sections);
    }

    #[test]
    fn map_report_uses_string_banks_and_row_lists() {
        let placed = place_asmir_bundle(
            &bundle(vec![section(
                1,
                SectionRole::Bank0Nucleus,
                "runtime.test.entry",
                Instr::Nop,
            )]),
            PlacementProfile::Budgeted,
        )
        .expect("place");
        let value = serde_json::to_value(address_map_report(&placed).expect("map")).expect("json");

        assert!(value["sections"].is_array());
        assert_eq!(value["sections"][0]["bank"], serde_json::json!("rom0"));
        assert!(value["sections"][0]["reachability_classes"].is_array());
        assert_eq!(
            value["sections"][0]["bank_switches_estimate"],
            serde_json::Value::Null
        );
        assert!(value["sections"][0]["cycles_estimate"].is_number());
        assert!(value["sections"][0]["provenance_summary"].is_object());
        assert!(value.as_object().expect("object").get("rom0").is_none());
    }

    #[test]
    fn placement_map_can_consume_preplacement_reachability_classes() {
        let entry = SymbolName::new("runtime.test.entry").expect("symbol");
        let bundle = bundle(vec![section(
            1,
            SectionRole::Bank0Nucleus,
            "runtime.test.entry",
            Instr::Nop,
        )]);
        let seed = preplacement_reachability_from_asmir(
            &bundle,
            &ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: entry,
                    root_kind: ReachabilityRootKind::HarnessEntry,
                    classes: vec![ReachabilityClass::HarnessEntryReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        );
        let placed =
            place_asmir_bundle_with_reachability(&bundle, PlacementProfile::Budgeted, &seed)
                .expect("place");
        let map = address_map_report(&placed).expect("map");

        assert_eq!(
            map.sections[0].reachability_classes,
            vec![ReachabilityClass::HarnessEntryReachable]
        );
    }
}
