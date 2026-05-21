//! Stage 12 symbolic `AsmIR` bundle construction.
//!
//! F-A1 owns the concrete assembly IR, layout, relaxation, and encoder types.
//! This module is the F-B15 front-end boundary: it packages compiler-produced
//! sections plus runtime nucleus sections into a deterministic, hashable
//! symbolic bundle and rejects backend-forbidden raw MBC writes before any
//! placement choice exists.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_asm::isa::Instr;
use gbf_asm::section::{PreLayoutOp, Section, SectionId};
use gbf_foundation::{DomainHash, Hash256, canonical_json_bytes_omitting_fields};
use serde::{Deserialize, Serialize};

pub const ASMIR_CODEGEN_VERSION: &str = "f-b15-asmir-codegen-v1";
const ASMIR_BUNDLE_SCHEMA_ID: &str = "gbf.codegen.f_b15.asmir_bundle";
const ASMIR_BUNDLE_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsmIRSourceKind {
    SchedOp,
    KernelBody,
    ContinuationHeader,
    EpochTrampoline,
    ExpertEntryStub,
    TensorPayload,
    LutPayload,
    RuntimeNucleus,
    CartridgeHeader,
    BuildIdentityBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AsmIRProvenanceEntry {
    pub section_id: u32,
    pub item_index: u32,
    pub source_kind: AsmIRSourceKind,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmIRBundle {
    pub codegen_version: String,
    pub codegen_sections: Vec<Section>,
    pub nucleus_sections: Vec<Section>,
    pub provenance: Vec<AsmIRProvenanceEntry>,
    pub asmir_bundle_hash: Hash256,
}

impl AsmIRBundle {
    #[must_use]
    pub fn all_sections(&self) -> Vec<Section> {
        let mut sections =
            Vec::with_capacity(self.nucleus_sections.len() + self.codegen_sections.len());
        sections.extend(self.nucleus_sections.iter().cloned());
        sections.extend(self.codegen_sections.iter().cloned());
        sections
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmIRCodegenInput {
    pub codegen_sections: Vec<Section>,
    pub nucleus_sections: Vec<Section>,
    pub provenance: Vec<AsmIRProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsmIRCodegenError {
    DuplicateSectionId {
        section_id: SectionId,
    },
    DuplicateSectionName {
        name: String,
    },
    RawMbcWrite {
        section_id: SectionId,
        seq_index: u32,
        addr: u16,
    },
    LeaseImbalance {
        section_id: SectionId,
        lease_id: u32,
    },
    CanonicalHash(String),
}

impl fmt::Display for AsmIRCodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSectionId { section_id } => {
                write!(f, "duplicate AsmIR section id {}", section_id.get())
            }
            Self::DuplicateSectionName { name } => write!(f, "duplicate AsmIR section name {name}"),
            Self::RawMbcWrite {
                section_id,
                seq_index,
                addr,
            } => write!(
                f,
                "section {} item {seq_index} writes raw MBC register ${addr:04X}",
                section_id.get()
            ),
            Self::LeaseImbalance {
                section_id,
                lease_id,
            } => write!(
                f,
                "section {} has unbalanced or reentrant bank lease {lease_id}",
                section_id.get()
            ),
            Self::CanonicalHash(message) => write!(f, "AsmIR bundle hash failed: {message}"),
        }
    }
}

impl std::error::Error for AsmIRCodegenError {}

pub fn build_asmir_bundle(input: AsmIRCodegenInput) -> Result<AsmIRBundle, AsmIRCodegenError> {
    validate_sections(&input.nucleus_sections)?;
    validate_sections(&input.codegen_sections)?;
    validate_disjoint_sections(&input.nucleus_sections, &input.codegen_sections)?;

    let mut bundle = AsmIRBundle {
        codegen_version: ASMIR_CODEGEN_VERSION.to_owned(),
        codegen_sections: canonical_sections(input.codegen_sections),
        nucleus_sections: canonical_sections(input.nucleus_sections),
        provenance: canonical_provenance(input.provenance),
        asmir_bundle_hash: Hash256::ZERO,
    };
    bundle.asmir_bundle_hash = asmir_bundle_hash(&bundle)
        .map_err(|error| AsmIRCodegenError::CanonicalHash(error.to_string()))?;
    Ok(bundle)
}

pub fn asmir_bundle_hash(
    bundle: &AsmIRBundle,
) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(bundle, &["asmir_bundle_hash"])?;
    DomainHash::new(
        "gbf-codegen",
        "AsmIRBundle",
        ASMIR_BUNDLE_SCHEMA_ID,
        ASMIR_BUNDLE_SCHEMA_VERSION,
    )
    .hash_canonical_bytes(&canonical)
}

fn validate_sections(sections: &[Section]) -> Result<(), AsmIRCodegenError> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for section in sections {
        if !ids.insert(section.id()) {
            return Err(AsmIRCodegenError::DuplicateSectionId {
                section_id: section.id(),
            });
        }
        if !names.insert(section.name().as_str().to_owned()) {
            return Err(AsmIRCodegenError::DuplicateSectionName {
                name: section.name().as_str().to_owned(),
            });
        }
        reject_raw_mbc_writes(section)?;
        reject_lease_imbalance(section)?;
    }
    Ok(())
}

fn validate_disjoint_sections(
    left: &[Section],
    right: &[Section],
) -> Result<(), AsmIRCodegenError> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for section in left.iter().chain(right) {
        if !ids.insert(section.id()) {
            return Err(AsmIRCodegenError::DuplicateSectionId {
                section_id: section.id(),
            });
        }
        if !names.insert(section.name().as_str().to_owned()) {
            return Err(AsmIRCodegenError::DuplicateSectionName {
                name: section.name().as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn reject_raw_mbc_writes(section: &Section) -> Result<(), AsmIRCodegenError> {
    for instr in section.instrs() {
        if let Instr::LdDirectFromA { addr } = instr.data {
            let addr = addr.get();
            if is_mbc_register_addr(addr) {
                return Err(AsmIRCodegenError::RawMbcWrite {
                    section_id: section.id(),
                    seq_index: instr.seq_index,
                    addr,
                });
            }
        }
    }
    Ok(())
}

fn reject_lease_imbalance(section: &Section) -> Result<(), AsmIRCodegenError> {
    let mut active: BTreeMap<u32, u32> = BTreeMap::new();
    for op in section.pre_layout_ops() {
        match &op.data {
            PreLayoutOp::BankLease(spec) => {
                let lease_id = spec.lease_id().get();
                if active.insert(lease_id, op.seq_index).is_some() {
                    return Err(AsmIRCodegenError::LeaseImbalance {
                        section_id: section.id(),
                        lease_id,
                    });
                }
            }
            PreLayoutOp::BankRelease { lease_id, .. } => {
                if active.remove(&lease_id.get()).is_none() {
                    return Err(AsmIRCodegenError::LeaseImbalance {
                        section_id: section.id(),
                        lease_id: lease_id.get(),
                    });
                }
            }
            PreLayoutOp::Yield { .. }
            | PreLayoutOp::TraceProbe { .. }
            | PreLayoutOp::AssertBank { .. } => {}
        }
    }
    if let Some(lease_id) = active.keys().next().copied() {
        return Err(AsmIRCodegenError::LeaseImbalance {
            section_id: section.id(),
            lease_id,
        });
    }
    Ok(())
}

pub(crate) const fn is_mbc_register_addr(addr: u16) -> bool {
    matches!(addr, 0x0000..=0x7FFF)
}

pub(crate) fn canonical_sections(mut sections: Vec<Section>) -> Vec<Section> {
    sections.sort_by(|a, b| {
        (a.role().canonical_name(), a.name().as_str(), a.id().get()).cmp(&(
            b.role().canonical_name(),
            b.name().as_str(),
            b.id().get(),
        ))
    });
    sections
}

fn canonical_provenance(mut provenance: Vec<AsmIRProvenanceEntry>) -> Vec<AsmIRProvenanceEntry> {
    provenance.sort();
    provenance
}

#[cfg(test)]
mod tests {
    use gbf_asm::builder::Builder;
    use gbf_asm::isa::{DirectAddr, Instr};
    use gbf_asm::section::{SectionId, SectionPrivilege, SectionRole};
    use gbf_asm::symbols::SymbolName;

    use super::*;

    fn section(id: u32, name: &'static str) -> Section {
        let name = SymbolName::new(name).expect("symbol");
        let mut builder =
            Builder::new_with_id(SectionId::new(id), SectionRole::Bank0Nucleus, name.clone());
        builder.label(name);
        builder.emit(Instr::Nop);
        builder.finish()
    }

    #[test]
    fn asmir_bundle_hash_is_byte_identical_for_input_order() {
        let a = section(2, "runtime.test.b");
        let b = section(1, "runtime.test.a");
        let first = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![a.clone(), b.clone()],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        let second = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![b, a],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");

        assert_eq!(first.asmir_bundle_hash, second.asmir_bundle_hash);
        assert_eq!(first.codegen_sections[0].id(), SectionId::new(1));
    }

    #[test]
    fn codegen_rejects_raw_mbc_write() {
        let name = SymbolName::new("runtime.test.raw_mbc").expect("symbol");
        let mut builder =
            Builder::new_with_id(SectionId::new(7), SectionRole::Bank0Nucleus, name.clone());
        builder.set_section_privilege(SectionPrivilege::privileged());
        builder.label(name);
        builder.emit(Instr::LdDirectFromA {
            addr: DirectAddr::new(0x2000).expect("MBC register is direct"),
        });
        let err = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![builder.finish()],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect_err("raw MBC writes are forbidden");

        assert!(matches!(
            err,
            AsmIRCodegenError::RawMbcWrite { addr: 0x2000, .. }
        ));
    }
}
