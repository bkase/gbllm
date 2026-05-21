//! Stage 12 whole-program reachability validation.
//!
//! This narrow F-B15 implementation has two typed surfaces:
//! pre-placement symbolic section class seeds, and post-placement validation.
//! Full F-B15 will run the edge walker before placement; v1 makes the bridge
//! explicit by letting placement consume symbolic class rows and then
//! revalidating the placed/legalized ROM. Report-facing data deliberately uses
//! row lists instead of JSON maps keyed by typed IDs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use gbf_asm::effect::{MachineEffect, classify_effect};
use gbf_asm::encoder;
use gbf_asm::isa::Instr;
use gbf_asm::layout::{AddressSpace, BankIndex, PlacedSection};
use gbf_asm::section::{ExecutionContext, SectionId};
use gbf_asm::symbols::{SymbolAddress, SymbolName};
use gbf_foundation::{
    DomainHash, Hash256, canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use serde::{Deserialize, Serialize};

use crate::lower_asm::{AsmIRBundle, is_mbc_register_addr};
use crate::place::{PlacedRom, is_fixed_resident_bank, placed_section};

pub const REACHABILITY_WALKER_VERSION: &str = "f-b15-reachability-v1";
const REACHABILITY_REPORT_SCHEMA_ID: &str = "gbf.codegen.f_b15.reachability_report";
const REACHABILITY_REPORT_SCHEMA_VERSION: &str = "1.0.0";
const REACHABILITY_CERT_SCHEMA_ID: &str = "gbf.codegen.f_b15.reachability_cert";
const REACHABILITY_CERT_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityClass {
    IsrReachable,
    YieldResumeReachable,
    FaultPathReachable,
    HarnessEntryReachable,
    BankLeaseProtected,
    NormalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityRootKind {
    InterruptVector,
    ModeEntry,
    ContinuationEntry,
    FaultEntry,
    PanicEntry,
    HarnessEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityRoot {
    pub symbol: SymbolName,
    pub root_kind: ReachabilityRootKind,
    pub classes: Vec<ReachabilityClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReachabilityValidationInput {
    pub roots: Vec<ReachabilityRoot>,
    pub continuation_targets: Vec<SymbolName>,
    pub f_b13_expected_classes: Vec<ExpectedSectionClasses>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedSectionClasses {
    pub section_id: u32,
    pub classes: Vec<ReachabilityClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrePlacementReachability {
    pub section_classes: Vec<PrePlacementSectionClass>,
    pub findings: Vec<ReachabilityFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrePlacementSectionClass {
    pub section_id: u32,
    pub symbol: String,
    pub classes: Vec<ReachabilityClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityReport {
    pub walker_version: String,
    pub section_classes: Vec<ReachabilitySectionClass>,
    pub class_per_byte: Vec<ClassPerByteRow>,
    pub edges: Vec<ReachabilityEdge>,
    pub findings: Vec<ReachabilityFinding>,
    pub dead_code: Vec<DeadCodeSection>,
    pub f_b13_disagreements: Vec<ClassDisagreement>,
    pub reachability_report_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityCertificate {
    pub walker_version: String,
    pub roots: Vec<ReachabilityRoot>,
    pub class_summary: Vec<ReachabilityClassSummary>,
    pub class_per_byte: Vec<ClassPerByteRow>,
    pub findings: Vec<ReachabilityFinding>,
    pub validator_witness_hash: Hash256,
    pub cert_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilitySectionClass {
    pub section_id: u32,
    pub bank: String,
    pub address_space: String,
    pub classes: Vec<ReachabilityClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityClassSummary {
    pub class: ReachabilityClass,
    pub section_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassPerByteRow {
    pub section_id: u32,
    pub byte_start: u16,
    pub byte_end_exclusive: u16,
    pub classes: Vec<ReachabilityClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReachabilityEdge {
    pub from_section_id: u32,
    pub to_section_id: u32,
    pub edge_kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Call,
    JumpRelative,
    JumpAbsolute,
    /// Thunk-to-callee transfer for a legalized far call. The caller-to-thunk
    /// leg is emitted from the rewritten call instruction as `Call`.
    FarCallViaThunk,
    RstVector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
pub enum ReachabilityRule {
    R1IsrResident,
    R2PrivilegedMbcWriteProtected,
    R3PrivilegeEffects,
    R4NoPrivilegedSwitchableDependency,
    R5NoLeaseReentrancy,
    R6ContinuationReachable,
    R7FaultPathResident,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Holds,
    Violated,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityFinding {
    pub rule: ReachabilityRule,
    pub status: FindingStatus,
    pub code: Option<ReachabilityDiagnosticCode>,
    pub evidence: Vec<String>,
    pub witnesses: Vec<ReachabilityWitness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
pub enum ReachabilityDiagnosticCode {
    IsrBankDependency,
    PrivilegedMbcWrite,
    PrivilegeViolation,
    PrivilegedSwitchableDependency,
    LeaseReentrancy,
    ClassDisagreement,
    ContinuationUnreachable,
    DeadCode,
    FaultPathNonresidentData,
    DeferredNarrowV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityWitness {
    pub section_id: u32,
    pub symbol: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadCodeSection {
    pub section_id: u32,
    pub bank: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassDisagreement {
    pub section_id: u32,
    pub expected: Vec<ReachabilityClass>,
    pub computed: Vec<ReachabilityClass>,
}

#[derive(Debug)]
pub enum ReachabilityValidationError {
    MissingRootSymbol(SymbolName),
    MissingContinuationSymbol(SymbolName),
    MissingPlacement { section_id: SectionId },
    Encode(encoder::EncodeError),
    CanonicalHash(String),
}

impl fmt::Display for ReachabilityValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRootSymbol(symbol) => write!(f, "reachability root {symbol} is missing"),
            Self::MissingContinuationSymbol(symbol) => {
                write!(f, "continuation target {symbol} is missing")
            }
            Self::MissingPlacement { section_id } => {
                write!(f, "section {} has no placement", section_id.get())
            }
            Self::Encode(error) => write!(f, "{error}"),
            Self::CanonicalHash(message) => write!(f, "reachability hash failed: {message}"),
        }
    }
}

impl std::error::Error for ReachabilityValidationError {}

impl From<encoder::EncodeError> for ReachabilityValidationError {
    fn from(value: encoder::EncodeError) -> Self {
        Self::Encode(value)
    }
}

pub fn preplacement_reachability_from_asmir(
    bundle: &AsmIRBundle,
    input: &ReachabilityValidationInput,
) -> PrePlacementReachability {
    let mut classes_by_section: BTreeMap<u32, BTreeSet<ReachabilityClass>> = BTreeMap::new();
    let mut symbol_to_section = BTreeMap::new();
    for section in bundle
        .nucleus_sections
        .iter()
        .chain(bundle.codegen_sections.iter())
    {
        classes_by_section.entry(section.id().get()).or_default();
        symbol_to_section.insert(section.name().as_str().to_owned(), section.id().get());
        if section
            .pre_layout_ops()
            .iter()
            .any(|op| matches!(op.data, gbf_asm::section::PreLayoutOp::BankLease(_)))
        {
            classes_by_section
                .entry(section.id().get())
                .or_default()
                .insert(ReachabilityClass::BankLeaseProtected);
        }
    }

    for root in &input.roots {
        if let Some(section_id) = symbol_to_section.get(root.symbol.as_str()).copied() {
            classes_by_section
                .entry(section_id)
                .or_default()
                .extend(root.classes.iter().copied());
        }
    }

    let mut section_classes = Vec::new();
    for section in bundle
        .nucleus_sections
        .iter()
        .chain(bundle.codegen_sections.iter())
    {
        section_classes.push(PrePlacementSectionClass {
            section_id: section.id().get(),
            symbol: section.name().as_str().to_owned(),
            classes: classes_by_section
                .remove(&section.id().get())
                .unwrap_or_default()
                .into_iter()
                .collect(),
        });
    }
    section_classes.sort();
    PrePlacementReachability {
        section_classes,
        findings: vec![ReachabilityFinding {
            rule: ReachabilityRule::R5NoLeaseReentrancy,
            status: FindingStatus::Deferred,
            code: Some(ReachabilityDiagnosticCode::DeferredNarrowV1),
            evidence: vec![
                "narrow_v1: build_asmir_bundle validates per-section lease balance before placement"
                    .to_owned(),
                "deferred: whole-program/cross-section runtime BankGuard reentrancy proof".to_owned(),
            ],
            witnesses: Vec::new(),
        }],
    }
}

pub fn validate_reachability(
    placed: &PlacedRom,
    mut input: ReachabilityValidationInput,
) -> Result<(ReachabilityReport, ReachabilityCertificate), ReachabilityValidationError> {
    if input.roots.is_empty() {
        input.roots = inferred_roots(placed);
    }

    let edges = collect_edges(placed)?;
    let class_by_section = propagate_classes(placed, &input.roots, &edges)?;
    let disagreements = class_disagreements(&input, &class_by_section);
    let mut findings = evaluate_rules(placed, &input, &class_by_section, &edges)?;
    if !disagreements.is_empty() {
        findings.push(ReachabilityFinding {
            rule: ReachabilityRule::R1IsrResident,
            status: FindingStatus::Violated,
            code: Some(ReachabilityDiagnosticCode::ClassDisagreement),
            evidence: vec!["F-B15 computed class set overrides F-B13 annotation".to_owned()],
            witnesses: disagreements
                .iter()
                .map(|item| ReachabilityWitness {
                    section_id: item.section_id,
                    symbol: None,
                    detail: "F-B15 computed class set disagrees with F-B13 annotation".to_owned(),
                })
                .collect(),
        });
    }

    let class_per_byte = class_per_byte_rows(placed, &class_by_section)?;
    let mut report = ReachabilityReport {
        walker_version: REACHABILITY_WALKER_VERSION.to_owned(),
        section_classes: section_class_rows(placed, &class_by_section)?,
        class_per_byte: class_per_byte.clone(),
        edges,
        dead_code: dead_code_rows(placed, &class_by_section),
        f_b13_disagreements: disagreements,
        findings,
        reachability_report_hash: Hash256::ZERO,
    };
    report.reachability_report_hash = reachability_report_hash(&report)
        .map_err(|error| ReachabilityValidationError::CanonicalHash(error.to_string()))?;
    let validator_witness_hash = reachability_witness_hash(&report)
        .map_err(|error| ReachabilityValidationError::CanonicalHash(error.to_string()))?;
    let mut cert = ReachabilityCertificate {
        walker_version: REACHABILITY_WALKER_VERSION.to_owned(),
        roots: input.roots,
        class_summary: class_summary(&class_by_section),
        class_per_byte,
        findings: report.findings.clone(),
        validator_witness_hash,
        cert_self_hash: Hash256::ZERO,
    };
    cert.cert_self_hash = reachability_certificate_hash(&cert)
        .map_err(|error| ReachabilityValidationError::CanonicalHash(error.to_string()))?;
    Ok((report, cert))
}

pub fn reachability_report_hash(
    report: &ReachabilityReport,
) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(report, &["reachability_report_hash"])?;
    DomainHash::new(
        "gbf-codegen",
        "ReachabilityReport",
        REACHABILITY_REPORT_SCHEMA_ID,
        REACHABILITY_REPORT_SCHEMA_VERSION,
    )
    .hash_canonical_bytes(&canonical)
}

pub fn reachability_certificate_hash(
    cert: &ReachabilityCertificate,
) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "ReachabilityCertificate",
            REACHABILITY_CERT_SCHEMA_ID,
            REACHABILITY_CERT_SCHEMA_VERSION,
        ),
        cert,
        "cert_self_hash",
        &[],
    )
}

fn reachability_witness_hash(
    report: &ReachabilityReport,
) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "ReachabilityWitness",
        REACHABILITY_CERT_SCHEMA_ID,
        REACHABILITY_CERT_SCHEMA_VERSION,
    )
    .hash(&(
        &report.section_classes,
        &report.class_per_byte,
        &report.edges,
        &report.findings,
        &report.dead_code,
        &report.f_b13_disagreements,
    ))
}

fn inferred_roots(placed: &PlacedRom) -> Vec<ReachabilityRoot> {
    let mut roots = Vec::new();
    for section in &placed.legalized_sections {
        let (root_kind, classes) = match section.privilege.execution_context {
            ExecutionContext::InterruptHandler => (
                ReachabilityRootKind::InterruptVector,
                vec![ReachabilityClass::IsrReachable],
            ),
            ExecutionContext::PanicOnly => (
                ReachabilityRootKind::FaultEntry,
                vec![ReachabilityClass::FaultPathReachable],
            ),
            ExecutionContext::Normal | ExecutionContext::VideoCommitOnly => continue,
        };
        roots.push(ReachabilityRoot {
            symbol: section.name.clone(),
            root_kind,
            classes,
        });
    }
    if roots.is_empty()
        && let Some(section) = placed.legalized_sections.first()
    {
        roots.push(ReachabilityRoot {
            symbol: section.name.clone(),
            root_kind: ReachabilityRootKind::HarnessEntry,
            classes: vec![
                ReachabilityClass::HarnessEntryReachable,
                ReachabilityClass::NormalOnly,
            ],
        });
    }
    roots
}

fn collect_edges(placed: &PlacedRom) -> Result<Vec<ReachabilityEdge>, ReachabilityValidationError> {
    let mut edges = BTreeSet::new();
    for section in &placed.legalized_sections {
        let Some(source_placed) = placed_section(placed, section.id) else {
            return Err(ReachabilityValidationError::MissingPlacement {
                section_id: section.id,
            });
        };
        let encoded = match source_placed.space {
            AddressSpace::Rom0 | AddressSpace::RomX => {
                encoder::encode_section(section, source_placed)?
            }
            AddressSpace::Wram
            | AddressSpace::Hram
            | AddressSpace::Sram
            | AddressSpace::Vram
            | AddressSpace::Oam => continue,
        };
        let span_by_seq: BTreeMap<_, _> = encoded
            .item_spans
            .iter()
            .map(|span| ((span.seq_index, span.sub_index), *span))
            .collect();
        for instr in &section.instrs {
            let here = span_by_seq
                .get(&(instr.seq_index, instr.sub_index))
                .map(|span| source_placed.cpu_start.wrapping_add(span.offset))
                .unwrap_or(source_placed.cpu_start);
            if let Some((target_addr, kind)) = instr_target(instr.data, here)
                && let Some(target) = find_section_for_target(placed, source_placed, target_addr)
            {
                edges.insert(ReachabilityEdge {
                    from_section_id: section.id.get(),
                    to_section_id: target.id.get(),
                    edge_kind: kind,
                });
            }
        }
    }
    for thunk in &placed.thunk_pool {
        let Some(thunk_addr) = placed.symbol_table.resolve(&thunk.thunk_symbol) else {
            continue;
        };
        let Some(target_addr) = placed.symbol_table.resolve(&thunk.target) else {
            continue;
        };
        edges.insert(ReachabilityEdge {
            from_section_id: thunk_addr.section.get(),
            to_section_id: target_addr.section.get(),
            edge_kind: EdgeKind::FarCallViaThunk,
        });
    }
    Ok(edges.into_iter().collect())
}

fn instr_target(instr: Instr, here: u16) -> Option<(u16, EdgeKind)> {
    match instr {
        Instr::Call { addr, .. } => Some((addr, EdgeKind::Call)),
        Instr::JpAbs { addr, .. } => Some((addr, EdgeKind::JumpAbsolute)),
        Instr::JrRel { off, .. } => {
            let target = i32::from(here) + 2 + i32::from(off);
            u16::try_from(target)
                .ok()
                .map(|addr| (addr, EdgeKind::JumpRelative))
        }
        Instr::Rst { vector } => Some((u16::from(vector.addr()), EdgeKind::RstVector)),
        Instr::JpHl => None,
        _ => None,
    }
}

fn find_section_for_target<'a>(
    placed: &'a PlacedRom,
    source: &PlacedSection,
    target_addr: u16,
) -> Option<&'a PlacedSection> {
    placed.layout.sections.iter().find(|candidate| {
        let same_visible_bank = match (source.space, source.bank, candidate.space, candidate.bank) {
            (_, _, AddressSpace::Rom0, BankIndex::Rom(0)) => true,
            (AddressSpace::RomX, bank, AddressSpace::RomX, target_bank) => bank == target_bank,
            (AddressSpace::Wram, _, AddressSpace::Wram, _) => true,
            (AddressSpace::Hram, _, AddressSpace::Hram, _) => true,
            _ => false,
        };
        same_visible_bank
            && u32::from(target_addr) >= u32::from(candidate.cpu_start)
            && u32::from(target_addr) < candidate.cpu_end_exclusive()
    })
}

fn propagate_classes(
    placed: &PlacedRom,
    roots: &[ReachabilityRoot],
    edges: &[ReachabilityEdge],
) -> Result<BTreeMap<u32, BTreeSet<ReachabilityClass>>, ReachabilityValidationError> {
    let mut classes: BTreeMap<u32, BTreeSet<ReachabilityClass>> = BTreeMap::new();
    let mut queue_seeds = BTreeSet::new();
    for row in &placed.preplacement_reachability_classes {
        let section_classes = classes.entry(row.section_id).or_default();
        for class in &row.classes {
            section_classes.insert(*class);
        }
        if !row.classes.is_empty() {
            queue_seeds.insert(row.section_id);
        }
    }
    for root in roots {
        let Some(address) = placed.symbol_table.resolve(&root.symbol) else {
            return Err(ReachabilityValidationError::MissingRootSymbol(
                root.symbol.clone(),
            ));
        };
        let section_id = address.section.get();
        for class in &root.classes {
            classes.entry(section_id).or_default().insert(*class);
        }
        if !root.classes.is_empty() {
            queue_seeds.insert(section_id);
        }
    }
    let mut queue: VecDeque<_> = queue_seeds.into_iter().collect();

    let mut outgoing: BTreeMap<u32, Vec<&ReachabilityEdge>> = BTreeMap::new();
    for edge in edges {
        outgoing.entry(edge.from_section_id).or_default().push(edge);
    }
    while let Some(section_id) = queue.pop_front() {
        let current = classes.get(&section_id).cloned().unwrap_or_default();
        for edge in outgoing.get(&section_id).into_iter().flatten() {
            let target = classes.entry(edge.to_section_id).or_default();
            let before = target.len();
            target.extend(current.iter().copied());
            if target.len() != before {
                queue.push_back(edge.to_section_id);
            }
        }
    }
    for section in &placed.legalized_sections {
        classes.entry(section.id.get()).or_default();
    }
    Ok(classes)
}

fn evaluate_rules(
    placed: &PlacedRom,
    input: &ReachabilityValidationInput,
    class_by_section: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
    _edges: &[ReachabilityEdge],
) -> Result<Vec<ReachabilityFinding>, ReachabilityValidationError> {
    let mut findings = Vec::new();
    push_rule(
        &mut findings,
        ReachabilityRule::R1IsrResident,
        ReachabilityDiagnosticCode::IsrBankDependency,
        resident_witnesses(
            placed,
            class_by_section,
            ReachabilityClass::IsrReachable,
            "ISR-reachable section is not fixed-resident",
        )?,
    );
    push_rule(
        &mut findings,
        ReachabilityRule::R2PrivilegedMbcWriteProtected,
        ReachabilityDiagnosticCode::PrivilegedMbcWrite,
        mbc_write_witnesses(placed, class_by_section),
    );
    push_rule(
        &mut findings,
        ReachabilityRule::R3PrivilegeEffects,
        ReachabilityDiagnosticCode::PrivilegeViolation,
        privilege_witnesses(placed),
    );
    push_rule(
        &mut findings,
        ReachabilityRule::R4NoPrivilegedSwitchableDependency,
        ReachabilityDiagnosticCode::PrivilegedSwitchableDependency,
        switchable_privileged_witnesses(placed, class_by_section)?,
    );
    findings.push(ReachabilityFinding {
        rule: ReachabilityRule::R5NoLeaseReentrancy,
        status: FindingStatus::Deferred,
        code: Some(ReachabilityDiagnosticCode::LeaseReentrancy),
        evidence: vec![
            "narrow_v1: symbolic AsmIR construction rejects per-section lease imbalance before lowering".to_owned(),
            "deferred: cross-section BankGuard reentrancy proof after runtime integration".to_owned(),
        ],
        witnesses: Vec::new(),
    });
    push_rule(
        &mut findings,
        ReachabilityRule::R6ContinuationReachable,
        ReachabilityDiagnosticCode::ContinuationUnreachable,
        continuation_witnesses(placed, input, class_by_section)?,
    );
    push_rule(
        &mut findings,
        ReachabilityRule::R7FaultPathResident,
        ReachabilityDiagnosticCode::FaultPathNonresidentData,
        resident_witnesses(
            placed,
            class_by_section,
            ReachabilityClass::FaultPathReachable,
            "fault-path-reachable section is not fixed-resident",
        )?,
    );
    Ok(findings)
}

fn push_rule(
    findings: &mut Vec<ReachabilityFinding>,
    rule: ReachabilityRule,
    code: ReachabilityDiagnosticCode,
    witnesses: Vec<ReachabilityWitness>,
) {
    findings.push(ReachabilityFinding {
        rule,
        status: if witnesses.is_empty() {
            FindingStatus::Holds
        } else {
            FindingStatus::Violated
        },
        code: (!witnesses.is_empty()).then_some(code),
        evidence: Vec::new(),
        witnesses,
    });
}

fn resident_witnesses(
    placed: &PlacedRom,
    class_by_section: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
    class: ReachabilityClass,
    detail: &str,
) -> Result<Vec<ReachabilityWitness>, ReachabilityValidationError> {
    let mut witnesses = Vec::new();
    for section in &placed.legalized_sections {
        let classes = class_by_section
            .get(&section.id.get())
            .cloned()
            .unwrap_or_default();
        if classes.contains(&class) {
            let Some(placement) = placed_section(placed, section.id) else {
                return Err(ReachabilityValidationError::MissingPlacement {
                    section_id: section.id,
                });
            };
            if !is_fixed_resident_bank(placement.bank) {
                witnesses.push(ReachabilityWitness {
                    section_id: section.id.get(),
                    symbol: Some(section.name.as_str().to_owned()),
                    detail: detail.to_owned(),
                });
            }
        }
    }
    Ok(witnesses)
}

fn mbc_write_witnesses(
    placed: &PlacedRom,
    class_by_section: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Vec<ReachabilityWitness> {
    let privileged_classes = [
        ReachabilityClass::IsrReachable,
        ReachabilityClass::YieldResumeReachable,
        ReachabilityClass::FaultPathReachable,
    ];
    let mut witnesses = Vec::new();
    for section in &placed.legalized_sections {
        let classes = class_by_section
            .get(&section.id.get())
            .cloned()
            .unwrap_or_default();
        if !privileged_classes
            .iter()
            .any(|class| classes.contains(class))
            || classes.contains(&ReachabilityClass::BankLeaseProtected)
        {
            continue;
        }
        for instr in &section.instrs {
            let effect = classify_effect(&instr.data);
            if let Instr::LdDirectFromA { addr } = instr.data
                && is_mbc_register_addr(addr.get())
            {
                witnesses.push(ReachabilityWitness {
                    section_id: section.id.get(),
                    symbol: Some(section.name.as_str().to_owned()),
                    detail: format!("raw MBC write at item {}", instr.seq_index),
                });
            }
            if matches!(
                effect,
                MachineEffect::StoreToDynamic { .. } | MachineEffect::ReadModifyWriteDynamic { .. }
            ) {
                witnesses.push(ReachabilityWitness {
                    section_id: section.id.get(),
                    symbol: Some(section.name.as_str().to_owned()),
                    detail: format!(
                        "dynamic-address write/RMW at item {} cannot prove it avoids MBC registers in narrow-v1",
                        instr.seq_index
                    ),
                });
            }
        }
    }
    witnesses
}

fn privilege_witnesses(placed: &PlacedRom) -> Vec<ReachabilityWitness> {
    let mut witnesses = Vec::new();
    for section in &placed.legalized_sections {
        for instr in &section.instrs {
            let effect = classify_effect(&instr.data);
            if !section.privilege.permits_effect(effect) {
                witnesses.push(ReachabilityWitness {
                    section_id: section.id.get(),
                    symbol: Some(section.name.as_str().to_owned()),
                    detail: format!(
                        "item {} effect {effect:?} exceeds section privilege",
                        instr.seq_index
                    ),
                });
            }
        }
    }
    witnesses
}

fn switchable_privileged_witnesses(
    placed: &PlacedRom,
    class_by_section: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Result<Vec<ReachabilityWitness>, ReachabilityValidationError> {
    let mut witnesses = Vec::new();
    for section in &placed.legalized_sections {
        let classes = class_by_section
            .get(&section.id.get())
            .cloned()
            .unwrap_or_default();
        if !(classes.contains(&ReachabilityClass::IsrReachable)
            || classes.contains(&ReachabilityClass::YieldResumeReachable))
        {
            continue;
        }
        let Some(placement) = placed_section(placed, section.id) else {
            return Err(ReachabilityValidationError::MissingPlacement {
                section_id: section.id,
            });
        };
        if matches!(placement.space, AddressSpace::RomX) {
            witnesses.push(ReachabilityWitness {
                section_id: section.id.get(),
                symbol: Some(section.name.as_str().to_owned()),
                detail: "privileged path depends on switchable ROM".to_owned(),
            });
        }
    }
    Ok(witnesses)
}

fn continuation_witnesses(
    placed: &PlacedRom,
    input: &ReachabilityValidationInput,
    class_by_section: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Result<Vec<ReachabilityWitness>, ReachabilityValidationError> {
    let mut witnesses = Vec::new();
    for symbol in &input.continuation_targets {
        let Some(SymbolAddress { section, .. }) = placed.symbol_table.resolve(symbol) else {
            return Err(ReachabilityValidationError::MissingContinuationSymbol(
                symbol.clone(),
            ));
        };
        let classes = class_by_section
            .get(&section.get())
            .cloned()
            .unwrap_or_default();
        if !classes.contains(&ReachabilityClass::YieldResumeReachable) {
            witnesses.push(ReachabilityWitness {
                section_id: section.get(),
                symbol: Some(symbol.as_str().to_owned()),
                detail: "continuation target is not yield-resume reachable".to_owned(),
            });
        }
    }
    Ok(witnesses)
}

fn section_class_rows(
    placed: &PlacedRom,
    classes: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Result<Vec<ReachabilitySectionClass>, ReachabilityValidationError> {
    let mut rows = Vec::new();
    for section in &placed.legalized_sections {
        let Some(placement) = placed_section(placed, section.id) else {
            return Err(ReachabilityValidationError::MissingPlacement {
                section_id: section.id,
            });
        };
        rows.push(ReachabilitySectionClass {
            section_id: section.id.get(),
            bank: placement.bank.to_string(),
            address_space: format!("{:?}", placement.space).to_ascii_lowercase(),
            classes: classes
                .get(&section.id.get())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
        });
    }
    rows.sort_by_key(|row| row.section_id);
    Ok(rows)
}

fn class_per_byte_rows(
    placed: &PlacedRom,
    classes: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Result<Vec<ClassPerByteRow>, ReachabilityValidationError> {
    let mut rows = Vec::new();
    for section in &placed.legalized_sections {
        let Some(placement) = placed_section(placed, section.id) else {
            return Err(ReachabilityValidationError::MissingPlacement {
                section_id: section.id,
            });
        };
        if placement.final_size == 0 {
            continue;
        }
        rows.push(ClassPerByteRow {
            section_id: section.id.get(),
            byte_start: 0,
            byte_end_exclusive: placement.final_size,
            classes: classes
                .get(&section.id.get())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
        });
    }
    rows.sort_by_key(|row| (row.section_id, row.byte_start));
    Ok(rows)
}

fn dead_code_rows(
    placed: &PlacedRom,
    classes: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Vec<DeadCodeSection> {
    let mut rows = Vec::new();
    for section in &placed.legalized_sections {
        if classes
            .get(&section.id.get())
            .map(BTreeSet::is_empty)
            .unwrap_or(true)
            && let Some(placement) = placed_section(placed, section.id)
        {
            rows.push(DeadCodeSection {
                section_id: section.id.get(),
                bank: placement.bank.to_string(),
            });
        }
    }
    rows.sort_by_key(|row| row.section_id);
    rows
}

fn class_summary(
    classes: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Vec<ReachabilityClassSummary> {
    let all = [
        ReachabilityClass::IsrReachable,
        ReachabilityClass::YieldResumeReachable,
        ReachabilityClass::FaultPathReachable,
        ReachabilityClass::HarnessEntryReachable,
        ReachabilityClass::BankLeaseProtected,
        ReachabilityClass::NormalOnly,
    ];
    all.into_iter()
        .map(|class| ReachabilityClassSummary {
            class,
            section_count: classes.values().filter(|set| set.contains(&class)).count(),
        })
        .collect()
}

fn class_disagreements(
    input: &ReachabilityValidationInput,
    classes: &BTreeMap<u32, BTreeSet<ReachabilityClass>>,
) -> Vec<ClassDisagreement> {
    let mut rows = Vec::new();
    for expected in &input.f_b13_expected_classes {
        let expected_set: BTreeSet<_> = expected.classes.iter().copied().collect();
        let computed = classes
            .get(&expected.section_id)
            .cloned()
            .unwrap_or_default();
        if expected_set != computed {
            rows.push(ClassDisagreement {
                section_id: expected.section_id,
                expected: expected_set.into_iter().collect(),
                computed: computed.into_iter().collect(),
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use gbf_asm::builder::Builder;
    use gbf_asm::isa::{DirectAddr, Instr};
    use gbf_asm::section::{Section, SectionId, SectionPrivilege, SectionRole, SymbolicBranch};
    use gbf_asm::symbols::SymbolName;
    use gbf_policy::PlacementProfile;

    use super::*;
    use crate::lower_asm::{AsmIRCodegenInput, build_asmir_bundle};
    use crate::place::{place_asmir_bundle, place_asmir_bundle_with_reachability};

    fn named(name: &'static str) -> SymbolName {
        SymbolName::new(name).expect("symbol")
    }

    fn section(id: u32, role: SectionRole, name: &'static str, instr: Instr) -> Section {
        let name = named(name);
        let mut builder = Builder::new_with_id(SectionId::new(id), role, name.clone());
        builder.label(name);
        builder.emit(instr);
        builder.finish()
    }

    fn placed(sections: Vec<Section>) -> PlacedRom {
        let bundle = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: sections,
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        place_asmir_bundle(&bundle, PlacementProfile::Budgeted).expect("place")
    }

    #[test]
    fn continuation_reachability_holds_through_call_edge() {
        let entry = named("runtime.test.entry");
        let cont = named("runtime.test.cont");
        let mut entry_builder =
            Builder::new_with_id(SectionId::new(1), SectionRole::Bank0Nucleus, entry.clone());
        entry_builder.label(entry.clone());
        entry_builder.branch(SymbolicBranch::call(cont.clone(), None));
        let cont_section = section(
            2,
            SectionRole::Bank0Nucleus,
            "runtime.test.cont",
            Instr::Ret { cond: None },
        );
        let placed = placed(vec![entry_builder.finish(), cont_section]);
        let (report, _) = validate_reachability(
            &placed,
            ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: entry,
                    root_kind: ReachabilityRootKind::ModeEntry,
                    classes: vec![ReachabilityClass::YieldResumeReachable],
                }],
                continuation_targets: vec![cont],
                f_b13_expected_classes: vec![],
            },
        )
        .expect("reachability");

        assert!(
            report
                .findings
                .iter()
                .find(|finding| finding.rule == ReachabilityRule::R6ContinuationReachable)
                .is_some_and(|finding| finding.status == FindingStatus::Holds)
        );
    }

    #[test]
    fn isr_reachable_switchable_section_violates_residency() {
        let placed = placed(vec![section(
            9,
            SectionRole::ExpertBank,
            "expert.0.9",
            Instr::Ret { cond: None },
        )]);
        let (report, _) = validate_reachability(
            &placed,
            ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: named("expert.0.9"),
                    root_kind: ReachabilityRootKind::InterruptVector,
                    classes: vec![ReachabilityClass::IsrReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        )
        .expect("reachability");

        assert!(
            report
                .findings
                .iter()
                .find(|finding| finding.rule == ReachabilityRule::R1IsrResident)
                .is_some_and(|finding| finding.status == FindingStatus::Violated)
        );
    }

    #[test]
    fn fault_path_switchable_section_violates_residency() {
        let placed = placed(vec![section(
            8,
            SectionRole::ExpertBank,
            "expert.0.8",
            Instr::Ret { cond: None },
        )]);
        let (report, _) = validate_reachability(
            &placed,
            ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: named("expert.0.8"),
                    root_kind: ReachabilityRootKind::FaultEntry,
                    classes: vec![ReachabilityClass::FaultPathReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        )
        .expect("reachability");

        assert!(
            report
                .findings
                .iter()
                .find(|finding| finding.rule == ReachabilityRule::R7FaultPathResident)
                .is_some_and(|finding| finding.status == FindingStatus::Violated)
        );
    }

    #[test]
    fn certificate_and_report_are_deterministic_json_shapes() {
        let placed = placed(vec![section(
            1,
            SectionRole::Bank0Nucleus,
            "runtime.test.entry",
            Instr::Nop,
        )]);
        let (first_report, first_cert) =
            validate_reachability(&placed, ReachabilityValidationInput::default()).expect("first");
        let (second_report, second_cert) =
            validate_reachability(&placed, ReachabilityValidationInput::default()).expect("second");

        assert_eq!(
            first_report.reachability_report_hash,
            second_report.reachability_report_hash
        );
        assert_eq!(first_cert.cert_self_hash, second_cert.cert_self_hash);
        let value = serde_json::to_value(first_report).expect("json");
        assert!(value["section_classes"].is_array());
        assert!(value["class_per_byte"].is_array());
        assert_eq!(
            first_cert.class_per_byte.len(),
            second_cert.class_per_byte.len()
        );
        assert!(first_cert.findings.iter().any(|finding| finding.rule
            == ReachabilityRule::R5NoLeaseReentrancy
            && finding.status == FindingStatus::Deferred));
    }

    #[test]
    fn preplacement_seed_flows_banklease_class_into_placed_validation() {
        let bundle = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![section(
                1,
                SectionRole::Bank0Nucleus,
                "runtime.test.entry",
                Instr::Nop,
            )],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        let seed = PrePlacementReachability {
            section_classes: vec![PrePlacementSectionClass {
                section_id: 1,
                symbol: "runtime.test.entry".to_owned(),
                classes: vec![
                    ReachabilityClass::HarnessEntryReachable,
                    ReachabilityClass::BankLeaseProtected,
                ],
            }],
            findings: Vec::new(),
        };
        let placed =
            place_asmir_bundle_with_reachability(&bundle, PlacementProfile::Budgeted, &seed)
                .expect("place");
        let (report, cert) =
            validate_reachability(&placed, ReachabilityValidationInput::default()).expect("report");

        assert!(
            report.section_classes[0]
                .classes
                .contains(&ReachabilityClass::BankLeaseProtected)
        );
        assert!(
            cert.class_per_byte[0]
                .classes
                .contains(&ReachabilityClass::BankLeaseProtected)
        );
    }

    #[test]
    fn preplacement_root_overlap_still_walks_outgoing_call_edge() {
        let entry = named("runtime.test.entry");
        let callee = named("runtime.test.callee");
        let mut entry_builder =
            Builder::new_with_id(SectionId::new(1), SectionRole::Bank0Nucleus, entry.clone());
        entry_builder.label(entry.clone());
        entry_builder.branch(SymbolicBranch::call(callee.clone(), None));
        let callee_section = section(
            2,
            SectionRole::Bank0Nucleus,
            "runtime.test.callee",
            Instr::Ret { cond: None },
        );
        let bundle = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![entry_builder.finish(), callee_section],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        let seed = PrePlacementReachability {
            section_classes: vec![PrePlacementSectionClass {
                section_id: 1,
                symbol: "runtime.test.entry".to_owned(),
                classes: vec![ReachabilityClass::HarnessEntryReachable],
            }],
            findings: Vec::new(),
        };
        let placed =
            place_asmir_bundle_with_reachability(&bundle, PlacementProfile::Budgeted, &seed)
                .expect("place");
        let (report, _) = validate_reachability(
            &placed,
            ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: entry,
                    root_kind: ReachabilityRootKind::HarnessEntry,
                    classes: vec![ReachabilityClass::HarnessEntryReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        )
        .expect("report");

        assert!(report.edges.iter().any(|edge| {
            edge.from_section_id == 1 && edge.to_section_id == 2 && edge.edge_kind == EdgeKind::Call
        }));
        assert!(
            report
                .section_classes
                .iter()
                .find(|row| row.section_id == 2)
                .is_some_and(|row| row
                    .classes
                    .contains(&ReachabilityClass::HarnessEntryReachable))
        );
        assert!(!report.dead_code.iter().any(|row| row.section_id == 2));
    }

    #[test]
    fn preplacement_seed_derives_banklease_class_from_symbolic_asmir() {
        let name = named("runtime.test.entry");
        let mut builder =
            Builder::new_with_id(SectionId::new(1), SectionRole::Bank0Nucleus, name.clone());
        builder.label(name.clone());
        builder.bank_lease(
            gbf_asm::section::BankLeaseSpec::new(
                gbf_asm::section::LeaseId::new(1),
                gbf_asm::section::LeaseGeneration(0),
                gbf_asm::section::MbcBankClass::Rom,
                1,
                gbf_asm::section::LeaseLifetime::Slice,
            )
            .expect("lease"),
        );
        builder.bank_release_to(
            gbf_asm::section::LeaseId::new(1),
            gbf_asm::section::BankReleaseDisposition::RomBank1,
        );
        let bundle = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![builder.finish()],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        let seed = preplacement_reachability_from_asmir(
            &bundle,
            &ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: name,
                    root_kind: ReachabilityRootKind::HarnessEntry,
                    classes: vec![ReachabilityClass::HarnessEntryReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        );

        assert!(
            seed.section_classes[0]
                .classes
                .contains(&ReachabilityClass::BankLeaseProtected)
        );
    }

    #[test]
    fn privilege_effect_violation_is_reported_for_manual_illegal_section() {
        let mut placed = placed(vec![section(
            1,
            SectionRole::Bank0Nucleus,
            "runtime.test.entry",
            Instr::Nop,
        )]);
        placed.legalized_sections[0].instrs[0].data = Instr::Di;
        placed.legalized_sections[0].privilege = SectionPrivilege::normal();

        let (report, _) =
            validate_reachability(&placed, ReachabilityValidationInput::default()).expect("report");
        assert!(
            report
                .findings
                .iter()
                .find(|finding| finding.rule == ReachabilityRule::R3PrivilegeEffects)
                .is_some_and(|finding| finding.status == FindingStatus::Violated)
        );
    }

    #[test]
    fn privileged_raw_mbc_write_without_bank_lease_class_is_reported() {
        let mut placed = placed(vec![section(
            1,
            SectionRole::Bank0Nucleus,
            "runtime.test.entry",
            Instr::Nop,
        )]);
        placed.legalized_sections[0].instrs[0].data = Instr::LdDirectFromA {
            addr: DirectAddr::new(0x2000).expect("MBC register is direct"),
        };
        placed.legalized_sections[0].privilege = SectionPrivilege::privileged();
        placed.layout.sections[0].final_size = 3;
        placed.layout.sections[0].estimated_size = 3;

        let (report, _) = validate_reachability(
            &placed,
            ReachabilityValidationInput {
                roots: vec![ReachabilityRoot {
                    symbol: named("runtime.test.entry"),
                    root_kind: ReachabilityRootKind::InterruptVector,
                    classes: vec![ReachabilityClass::IsrReachable],
                }],
                ..ReachabilityValidationInput::default()
            },
        )
        .expect("report");

        assert!(
            report
                .findings
                .iter()
                .find(|finding| finding.rule == ReachabilityRule::R2PrivilegedMbcWriteProtected)
                .is_some_and(|finding| finding.status == FindingStatus::Violated)
        );
    }
}
