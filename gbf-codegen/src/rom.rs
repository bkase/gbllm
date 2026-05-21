//! Stage 12 byte encoding: `PlacedRom` to `.gb`, `.sym`, `.lst`.
//!
//! This module intentionally contains no placement choices. It consumes the
//! already-legalized `PlacedRom`, emits byte artifacts through F-A1, and hashes
//! the final ROM bytes.

use std::fmt;

use gbf_asm::encoder::{self, EncodedSection};
use gbf_asm::layout;
use gbf_asm::listing::{self, ListingOptions};
use gbf_asm::rom::{self, CartridgeHeader};
use gbf_asm::symbols::{self, SymOptions};
use gbf_foundation::{Hash256, sha256};
use serde::{Deserialize, Serialize};

use crate::place::{PlacedRom, placed_section};
use crate::stage_cache::{
    CodegenStageCacheError, Stage12BackendCacheKeyMaterial, StoreBackedStageCacheKeys,
    StoreBackedStageCellKind, StoreBackedStageExpectedHashes, StoreBackedStageRunOutput,
    StoreBackedStageRunResult, run_store_backed_stage_with_cache, stage12_backend_store_key,
};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const ENCODER_VERSION: &str = "f-b15-encoded-rom-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedRom {
    pub gb_bytes: Vec<u8>,
    pub sym_lines: Vec<String>,
    pub lst_text: String,
    pub encoded_sections: Vec<EncodedSection>,
    pub identity: EncodedRomIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedRomIdentity {
    /// Stage-12 narrow-v1 records the caller-supplied build hash but does not
    /// patch a BuildIdentityBlock byte range in the ROM image. The explicit
    /// status prevents this field from implying the F-A3/F-F1 byte patch has
    /// landed.
    pub build_hash: Hash256,
    pub build_hash_patch: BuildHashPatchStatus,
    pub encoded_rom_self_hash: Hash256,
    pub encoder_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildHashPatchStatus {
    NotPatchedNarrowV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage12BackendProduct {
    pub encoded_rom: EncodedRom,
}

#[derive(Debug, Clone)]
pub struct Stage12BackendInputs<'a> {
    pub placed_rom: &'a PlacedRom,
    pub cartridge_header: &'a CartridgeHeader,
    pub build_hash: Hash256,
    pub schedule_pack_self_hash: Hash256,
    pub resource_state_cert_self_hash: Hash256,
    pub schedule_cost_report_self_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub cartridge_header_hash: Hash256,
    pub backend_policy_projection_hash: Hash256,
    /// Narrow-v1 Stage 12 has byte/listing products but no unified
    /// backend report envelope yet. Callers pass the report/package hash
    /// they want the StageCache cell and cache_status entry to bind.
    pub report_self_hash: Hash256,
}

impl Stage12BackendInputs<'_> {
    #[must_use]
    pub fn cache_key_material(&self) -> Stage12BackendCacheKeyMaterial {
        Stage12BackendCacheKeyMaterial::new(
            self.schedule_pack_self_hash,
            self.resource_state_cert_self_hash,
            self.schedule_cost_report_self_hash,
            self.runtime_nucleus_hash,
            self.cartridge_header_hash,
            self.backend_policy_projection_hash,
        )
    }
}

#[derive(Debug)]
pub enum EncodedRomError {
    MissingPlacement { section_id: u32 },
    Encode(encoder::EncodeError),
    Rom(rom::RomAssemblyError),
    Sym(symbols::SymError),
    Listing(listing::ListingError),
    Layout(layout::LayoutError),
    NonDeterministic,
}

impl fmt::Display for EncodedRomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPlacement { section_id } => {
                write!(f, "section {section_id} has no placement")
            }
            Self::Encode(error) => write!(f, "{error}"),
            Self::Rom(error) => write!(f, "{error}"),
            Self::Sym(error) => write!(f, "{error}"),
            Self::Listing(error) => write!(f, "{error}"),
            Self::Layout(error) => write!(f, "{error}"),
            Self::NonDeterministic => {
                f.write_str("encoding the same PlacedRom produced different bytes")
            }
        }
    }
}

impl std::error::Error for EncodedRomError {}

impl From<encoder::EncodeError> for EncodedRomError {
    fn from(value: encoder::EncodeError) -> Self {
        Self::Encode(value)
    }
}

impl From<rom::RomAssemblyError> for EncodedRomError {
    fn from(value: rom::RomAssemblyError) -> Self {
        Self::Rom(value)
    }
}

impl From<symbols::SymError> for EncodedRomError {
    fn from(value: symbols::SymError) -> Self {
        Self::Sym(value)
    }
}

impl From<listing::ListingError> for EncodedRomError {
    fn from(value: listing::ListingError) -> Self {
        Self::Listing(value)
    }
}

impl From<layout::LayoutError> for EncodedRomError {
    fn from(value: layout::LayoutError) -> Self {
        Self::Layout(value)
    }
}

pub fn encode_placed_rom(
    placed: &PlacedRom,
    header: &CartridgeHeader,
    build_hash: Hash256,
) -> Result<EncodedRom, EncodedRomError> {
    let first = encode_once(placed, header, build_hash)?;
    let second = encode_once(placed, header, build_hash)?;
    if first.gb_bytes != second.gb_bytes
        || first.sym_lines != second.sym_lines
        || first.lst_text != second.lst_text
    {
        return Err(EncodedRomError::NonDeterministic);
    }
    Ok(first)
}

pub fn run_stage12_backend_with_cache(
    cache: &StoreStageCache<'_>,
    input: &Stage12BackendInputs<'_>,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<Stage12BackendProduct>, CodegenStageCacheError> {
    let material = input.cache_key_material();
    let success_key = stage12_backend_store_key(&material, StoreBackedStageCellKind::Success);
    let failure_key = stage12_backend_store_key(&material, StoreBackedStageCellKind::FailureMemo);
    let keys = StoreBackedStageCacheKeys::new("12", success_key, failure_key);
    let input_identity_hash = sha256(
        gbf_store::stage_cache::compose_key(&keys.success_key)
            .to_string()
            .as_bytes(),
    );
    run_store_backed_stage_with_cache(cache, &keys, input_identity_hash, expected_hashes, || {
        let encoded = encode_placed_rom(input.placed_rom, input.cartridge_header, input.build_hash)
            .map_err(|error| CodegenStageCacheError::StageEmit {
                stage_id: "12",
                message: error.to_string(),
            })?;
        let product_self_hash = encoded.identity.encoded_rom_self_hash;
        Ok(StoreBackedStageRunResult::Success {
            product: Stage12BackendProduct {
                encoded_rom: encoded,
            },
            product_self_hash,
            report_self_hash: input.report_self_hash,
        })
    })
}

fn encode_once(
    placed: &PlacedRom,
    header: &CartridgeHeader,
    build_hash: Hash256,
) -> Result<EncodedRom, EncodedRomError> {
    let mut encoded_sections = Vec::new();
    let mut rom_pairs = Vec::new();
    for section in &placed.legalized_sections {
        let placed_section =
            placed_section(placed, section.id).ok_or(EncodedRomError::MissingPlacement {
                section_id: section.id.get(),
            })?;
        if placed_section.rom_file_offset()?.is_none() {
            continue;
        }
        let encoded = encoder::encode_section(section, placed_section)?;
        encoded_sections.push(encoded.clone());
        rom_pairs.push((encoded, placed_section.clone()));
    }

    let gb_bytes = rom::assemble_rom(&rom_pairs, &placed.layout, header)?;
    let sym = symbols::write_sym(
        &placed.layout,
        &placed.symbol_table,
        &SymOptions {
            include_externals_as_comments: false,
            dot_safe_separator: true,
            ..SymOptions::default()
        },
    )?;
    let lst_text = listing::emit_program_listing(
        &placed.legalized_sections,
        &encoded_sections,
        &placed.layout,
        &placed.symbol_table,
        &ListingOptions {
            show_cycle_costs: true,
            ..ListingOptions::default()
        },
    )?;
    let encoded_rom_self_hash = sha256(&gb_bytes);
    Ok(EncodedRom {
        gb_bytes,
        sym_lines: sym.lines().map(str::to_owned).collect(),
        lst_text,
        encoded_sections,
        identity: EncodedRomIdentity {
            build_hash,
            build_hash_patch: BuildHashPatchStatus::NotPatchedNarrowV1,
            encoded_rom_self_hash,
            encoder_version: ENCODER_VERSION.to_owned(),
        },
    })
}

#[cfg(test)]
mod tests {
    use gbf_asm::builder::Builder;
    use gbf_asm::isa::Instr;
    use gbf_asm::rom::RomSize;
    use gbf_asm::section::{Section, SectionId, SectionRole};
    use gbf_asm::symbols::SymbolName;
    use gbf_foundation::Hash256;
    use gbf_policy::PlacementProfile;

    use super::*;
    use crate::lower_asm::{AsmIRCodegenInput, build_asmir_bundle};
    use crate::place::place_asmir_bundle;

    fn section(id: u32, role: SectionRole, name: &'static str, instr: Instr) -> Section {
        let name = SymbolName::new(name).expect("symbol");
        let mut builder = Builder::new_with_id(SectionId::new(id), role, name.clone());
        builder.label(name);
        builder.emit(instr);
        builder.finish()
    }

    fn placed() -> PlacedRom {
        let bundle = build_asmir_bundle(AsmIRCodegenInput {
            codegen_sections: vec![
                section(
                    1,
                    SectionRole::Bank0Nucleus,
                    "runtime.test.entry",
                    Instr::Nop,
                ),
                section(
                    2,
                    SectionRole::ExpertBank,
                    "expert.0.0",
                    Instr::Ret { cond: None },
                ),
            ],
            nucleus_sections: vec![],
            provenance: vec![],
        })
        .expect("bundle");
        place_asmir_bundle(&bundle, PlacementProfile::Budgeted).expect("place")
    }

    #[test]
    fn encoded_rom_regenerates_byte_identically() {
        let placed = placed();
        let header = CartridgeHeader {
            rom_size: RomSize::Kib64,
            ..CartridgeHeader::new("GBFTEST").expect("header")
        };
        let first =
            encode_placed_rom(&placed, &header, Hash256::from_bytes([0x11; 32])).expect("first");
        let second =
            encode_placed_rom(&placed, &header, Hash256::from_bytes([0x11; 32])).expect("second");

        assert_eq!(first.gb_bytes, second.gb_bytes);
        assert_eq!(
            first.identity.encoded_rom_self_hash,
            sha256(&first.gb_bytes)
        );
        assert_eq!(
            first.identity.build_hash_patch,
            BuildHashPatchStatus::NotPatchedNarrowV1
        );
        assert!(
            first
                .sym_lines
                .iter()
                .any(|line| line.contains("runtime_dtest_dentry"))
        );
        assert!(first.lst_text.contains("runtime.test.entry"));
    }
}
