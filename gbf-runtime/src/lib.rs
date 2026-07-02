//! Bank0 and common-bank runtime builders for scheduling, banking, IO, tracing, and persistence.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use gbf_abi::{AbiVersion, BuildIdentityArgs, BuildIdentityBlock, RuntimeShellModule};
use gbf_asm::builder::Builder;
use gbf_asm::effect::MachineEffectKind;
use gbf_asm::section::{Section, SectionId};
use gbf_asm::symbols::SymbolName;
use gbf_foundation::{
    BudgetSlotId, CanonicalJson, CanonicalJsonError, CompileProfileId, Hash256, PackerVersion,
    TargetProfileId,
};
use gbf_policy::{
    BudgetSlotClass, PlacementProfile, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeChromeBudgetValidationError, RuntimeMemoryCapSection, RuntimeNucleusHash,
    WramPolicyError, WramReserved, bringup_profile, pinned_reference_shell,
};
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod banking;
pub mod boot;
pub mod harness;
pub mod interrupts;
pub mod joypad;
pub mod keyboard;
pub mod panic;
pub mod persistence;
pub mod scheduler;
pub mod text;
pub mod trace;
pub mod video_commit;

pub use gbf_abi::RuntimeShellModule as RuntimeModule;

pub const RUNTIME_NUCLEUS_HASH_DOMAIN: &[u8] = b"gbf-runtime/v1/bank0-nucleus";
pub const RUNTIME_NUCLEUS_BUDGET_CONTRACT_HASH_DOMAIN: &[u8] =
    b"gbf-runtime/v1/runtime-nucleus-budget-contract";
pub const RUNTIME_CHROME_BUDGET_FILE_NAME: &str = "chrome_budget.json";
pub const RUNTIME_PACKER_VERSION: PackerVersion = PackerVersion::new(1, 0, 0);
pub const FUTURE_PERSISTENCE_ROM_BYTES_BANK0: usize = 768;
pub const FUTURE_TRACE_ROM_BYTES_BANK0: usize = 256;
pub const FUTURE_HARNESS_ROM_BYTES_BANK0: usize = 384;
pub const FUTURE_RESERVATION_ROM_BYTES_BANK0: usize = FUTURE_PERSISTENCE_ROM_BYTES_BANK0
    + FUTURE_TRACE_ROM_BYTES_BANK0
    + FUTURE_HARNESS_ROM_BYTES_BANK0;
pub const BANK0_NUCLEUS_BUDGET_BYTES: usize =
    gbf_hw::memory::BANK0_SIZE_BYTES as usize - FUTURE_RESERVATION_ROM_BYTES_BANK0;
pub const COMMON_BANK_RESERVED_SLACK_BYTES: u16 = 512;
pub const EXPERT_BANK_RESERVED_SLACK_BYTES: u16 = 384;

pub const SECTION_ID_BOOT: SectionId = SectionId::new(0xA500);
pub const SECTION_ID_IRQ_VECTORS: SectionId = SectionId::new(0xA501);
pub const SECTION_ID_ISR_STUBS: SectionId = SectionId::new(0xA502);
pub const SECTION_ID_INTERRUPTS: SectionId = SectionId::new(0xA503);
pub const SECTION_ID_SCHEDULER: SectionId = SectionId::new(0xA504);
pub const SECTION_ID_JOYPAD: SectionId = SectionId::new(0xA505);
pub const SECTION_ID_TEXT: SectionId = SectionId::new(0xA506);
pub const SECTION_ID_KEYBOARD: SectionId = SectionId::new(0xA507);
pub const SECTION_ID_VIDEO_COMMIT: SectionId = SectionId::new(0xA508);
pub const SECTION_ID_PANIC: SectionId = SectionId::new(0xA509);
pub const SECTION_ID_BUILD_IDENTITY: SectionId = SectionId::new(0xA50A);
pub const SECTION_ID_TEXT_FONT: SectionId = SectionId::new(0xA50B);
pub const SECTION_ID_KEYBOARD_LAYOUT: SectionId = SectionId::new(0xA50C);

pub const BUILD_IDENTITY_BLOCK_ADDR: u16 = 0x01C0;
pub const TEXT_FONT_DATA_ADDR: u16 = 0x1400;
pub const KEYBOARD_LAYOUT_DATA_ADDR: u16 = 0x1C00;

/// Validated WRAM address owned by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WramAddr(u16);

impl WramAddr {
    pub const MIN: u16 = gbf_hw::memory::WRAM_BASE;
    pub const MAX_INCLUSIVE: u16 = gbf_hw::memory::WRAM_END;

    #[must_use]
    pub const fn new(addr: u16) -> Self {
        assert!(gbf_hw::memory::is_wram(addr), "address must be in WRAM");
        Self(addr)
    }

    pub const fn try_new(addr: u16) -> Result<Self, WramAddrError> {
        if gbf_hw::memory::is_wram(addr) {
            Ok(Self(addr))
        } else {
            Err(WramAddrError { addr })
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn add(self, offset: u16) -> Self {
        Self::new(self.0 + offset)
    }
}

impl Serialize for WramAddr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for WramAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let addr = u16::deserialize(deserializer)?;
        Self::try_new(addr).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WramAddrError {
    pub addr: u16,
}

impl fmt::Display for WramAddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "address ${:04X} is outside WRAM ${:04X}..=${:04X}",
            self.addr,
            WramAddr::MIN,
            WramAddr::MAX_INCLUSIVE
        )
    }
}

impl std::error::Error for WramAddrError {}

/// Runtime section bundled with its reference-shell annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNucleusSection {
    pub module: RuntimeShellModule,
    pub section: Section,
}

impl RuntimeNucleusSection {
    #[must_use]
    pub fn new(module: RuntimeShellModule, section: Section) -> Self {
        Self { module, section }
    }
}

impl gbf_abi::RuntimeShellAnnotated for RuntimeNucleusSection {
    fn runtime_shell_module(&self) -> RuntimeShellModule {
        self.module
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNucleusBuild {
    pub sections: Vec<RuntimeNucleusSection>,
    pub support_sections: Vec<RuntimeNucleusSection>,
    pub interrupt_safety_table: banking::InterruptSafetyTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSectionSize {
    pub module: RuntimeShellModule,
    pub section_id: SectionId,
    pub name: String,
    pub bytes: u32,
}

pub fn build_bank0_nucleus_sections() -> Vec<Section> {
    let build = build_bank0_nucleus();
    build
        .sections
        .into_iter()
        .chain(build.support_sections)
        .map(|entry| entry.section)
        .collect()
}

pub fn build_bank0_nucleus() -> RuntimeNucleusBuild {
    let sections = vec![
        RuntimeNucleusSection::new(RuntimeShellModule::Boot, boot::build_boot_section()),
        RuntimeNucleusSection::new(RuntimeShellModule::Boot, boot::build_irq_vectors_section()),
        RuntimeNucleusSection::new(
            RuntimeShellModule::Interrupts,
            boot::build_isr_stubs_section(),
        ),
        RuntimeNucleusSection::new(
            RuntimeShellModule::Interrupts,
            interrupts::build_handlers_section(),
        ),
        RuntimeNucleusSection::new(
            RuntimeShellModule::Scheduler,
            scheduler::build_scheduler_section(),
        ),
        RuntimeNucleusSection::new(RuntimeShellModule::Joypad, joypad::build_joypad_section()),
        RuntimeNucleusSection::new(RuntimeShellModule::Text, text::build_text_section()),
        RuntimeNucleusSection::new(
            RuntimeShellModule::Keyboard,
            keyboard::build_keyboard_section(),
        ),
        RuntimeNucleusSection::new(
            RuntimeShellModule::VideoCommit,
            video_commit::build_video_commit_section(),
        ),
        RuntimeNucleusSection::new(RuntimeShellModule::Panic, panic::build_panic_section()),
    ];
    let support_sections = vec![
        RuntimeNucleusSection::new(
            RuntimeShellModule::Boot,
            build_identity_placeholder_section(),
        ),
        RuntimeNucleusSection::new(RuntimeShellModule::Text, text::build_font_data_section()),
        RuntimeNucleusSection::new(
            RuntimeShellModule::Keyboard,
            keyboard::build_layout_data_section(),
        ),
    ];

    let mut interrupt_safety_table = banking::InterruptSafetyTable::default();
    for entry in sections.iter().chain(support_sections.iter()) {
        match entry.section.id() {
            SECTION_ID_ISR_STUBS | SECTION_ID_INTERRUPTS => {
                banking::mark_isr(&mut interrupt_safety_table, &entry.section)
                    .expect("ISR sections have one safety declaration");
            }
            SECTION_ID_PANIC => {
                banking::mark_isr_unreachable(&mut interrupt_safety_table, &entry.section)
                    .expect("panic section has one safety declaration");
            }
            _ => {
                banking::mark_isr_reachable(&mut interrupt_safety_table, &entry.section)
                    .expect("Bank0 sections have one safety declaration");
            }
        }
    }

    RuntimeNucleusBuild {
        sections,
        support_sections,
        interrupt_safety_table,
    }
}

pub fn build_identity_placeholder_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_BUILD_IDENTITY,
        gbf_asm::section::SectionRole::Bank0Data,
        SymbolName::runtime("boot", "build_identity_placeholder").expect("static symbol"),
    );
    let block = BuildIdentityBlock::new(BuildIdentityArgs {
        abi: AbiVersion {
            major: gbf_abi::CURRENT_ABI.major,
            minor: gbf_abi::CURRENT_ABI.minor,
            patch: gbf_abi::CURRENT_ABI.patch,
        },
        build_hash: [0; 32],
        artifact_core_hash: [0; 32],
        runtime_nucleus_hash: [0; 32],
        compile_request_hash: [0; 32],
        timestamp_unix: 0,
        continuation_tail_bytes: 0,
        semantic_schema_version: 1,
    });
    builder.db_bytes(block.to_bytes());
    builder.finish()
}

#[must_use]
pub fn runtime_nucleus_section_order() -> Vec<RuntimeShellModule> {
    build_bank0_nucleus()
        .sections
        .into_iter()
        .map(|entry| entry.module)
        .collect()
}

#[must_use]
pub fn runtime_nucleus_section_sizes() -> Vec<RuntimeSectionSize> {
    let build = build_bank0_nucleus();
    build
        .sections
        .into_iter()
        .chain(build.support_sections)
        .map(|entry| RuntimeSectionSize {
            module: entry.module,
            section_id: entry.section.id(),
            name: entry.section.name().to_string(),
            bytes: section_estimated_bytes(&entry.section),
        })
        .collect()
}

#[must_use]
pub fn section_estimated_bytes(section: &Section) -> u32 {
    match (section.fixed_item_bytes(), section.size_hint_bytes()) {
        (Some(fixed), Some(hint)) => fixed.max(hint),
        (Some(fixed), None) => fixed,
        (None, Some(hint)) => hint,
        (None, None) => panic!("runtime nucleus sections must declare a fixed size or size hint"),
    }
}

#[must_use]
pub fn compute_runtime_nucleus_hash(normalized_bank0: &[u8; 16 * 1024]) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(RUNTIME_NUCLEUS_HASH_DOMAIN);
    hasher.update([
        gbf_abi::CURRENT_ABI.major,
        gbf_abi::CURRENT_ABI.minor,
        gbf_abi::CURRENT_ABI.patch,
    ]);
    hasher.update(normalized_bank0);
    Hash256::from_bytes(hasher.finalize().into())
}

#[must_use]
pub fn compute_runtime_nucleus_hash_for_test() -> Hash256 {
    compute_runtime_nucleus_hash(&normalized_bank0_image_for_test())
}

/// Budget emission facts derived from the runtime shell build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeChromeBudgetEmission {
    pub budget: RuntimeChromeBudget,
    pub runtime_image_hash: Hash256,
    pub bank0_free_bytes_after_shell: u32,
    pub bank0_future_reservation_bytes: u16,
    pub common_bank_occupied_bytes: u32,
    pub runtime_wram_internal_bytes: u16,
    pub section_sizes: Vec<RuntimeSectionSize>,
}

/// Build the current runtime shell and derive its RuntimeChromeBudget.
pub fn runtime_chrome_budget_emission()
-> Result<RuntimeChromeBudgetEmission, RuntimeChromeBudgetEmissionError> {
    let artifacts = demo_bank0_artifacts().map_err(RuntimeChromeBudgetEmissionError::Rom)?;
    runtime_chrome_budget_emission_from_bank0_and_layout(&artifacts.bank0, &artifacts.layout)
}

/// Derive a RuntimeChromeBudget from a normalized Bank0 runtime image.
///
/// This convenience path uses the current demo shell layout for capacity
/// accounting. Use [`runtime_chrome_budget_emission_from_bank0_and_layout`] when
/// the caller already has the layout from the runtime shell build pass.
pub fn runtime_chrome_budget_emission_from_bank0(
    bank0: &[u8; gbf_hw::memory::BANK0_SIZE_BYTES as usize],
) -> Result<RuntimeChromeBudgetEmission, RuntimeChromeBudgetEmissionError> {
    let (_, _, layout) = normalized_bank0_image_and_symbols_for_test();
    runtime_chrome_budget_emission_from_bank0_and_layout(bank0, &layout)
}

/// Derive a RuntimeChromeBudget from a normalized Bank0 runtime image and layout.
///
/// The hash is byte-driven, while the Bank0 capacity is derived from the placed
/// sections and reserved ranges. That avoids treating literal `0xFF` bytes in
/// occupied code or cartridge header fields as usable model capacity.
pub fn runtime_chrome_budget_emission_from_bank0_and_layout(
    bank0: &[u8; gbf_hw::memory::BANK0_SIZE_BYTES as usize],
    layout: &gbf_asm::layout::LayoutPlan,
) -> Result<RuntimeChromeBudgetEmission, RuntimeChromeBudgetEmissionError> {
    let target = gbf_hw::target::dmg_mbc5_8mib_128kib();
    let profile = bringup_profile();
    let reference_shell_modules = RuntimeChromeBudget::pinned_reference_shell_modules();
    let runtime_image_hash = compute_runtime_nucleus_hash(bank0);
    let bank0_free_bytes_after_shell = bank0_free_bytes_after_shell(layout);
    let bank0_future_reservation_bytes = reference_shell_bank0_future_reservation_bytes();
    let common_bank_occupied_bytes = runtime_common_bank_occupied_bytes();
    let runtime_wram_internal_bytes = runtime_wram_internal_reserved_bytes();
    let wram_layout = profile.wram_layout;
    let wram_reserved_total = wram_layout
        .overlay_bytes
        .checked_add(runtime_wram_internal_bytes)
        .ok_or(RuntimeChromeBudgetEmissionError::WramTotalOverflow)?;
    let rom_slots = vec![
        // The byte count comes from the actual placed Bank0 shell layout.
        // F-A5 still links a minimal panic section, so those bytes reduce
        // capacity here even though the pinned policy module set reserves
        // the full future panic path as slack rather than declaring it.
        RomBudgetSlot::new(
            BudgetSlotId::new(0),
            BudgetSlotClass::Bank0Free,
            bank0_free_bytes_after_shell,
            bank0_future_reservation_bytes,
            BTreeSet::from([PlacementProfile::StrictOnePerBank]),
        )?,
        RomBudgetSlot::new(
            BudgetSlotId::new(1),
            BudgetSlotClass::CommonBank,
            gbf_hw::memory::SWITCHABLE_BANK_SIZE_BYTES.saturating_sub(common_bank_occupied_bytes),
            COMMON_BANK_RESERVED_SLACK_BYTES,
            BTreeSet::from([PlacementProfile::Budgeted]),
        )?,
        RomBudgetSlot::new(
            BudgetSlotId::new(2),
            BudgetSlotClass::ExpertBank,
            gbf_hw::memory::SWITCHABLE_BANK_SIZE_BYTES,
            EXPERT_BANK_RESERVED_SLACK_BYTES,
            BTreeSet::from([
                PlacementProfile::StrictOnePerBank,
                PlacementProfile::Budgeted,
            ]),
        )?,
    ];
    let memory_caps = RuntimeMemoryCapSection {
        wram_usable_bytes: gbf_hw::memory::WRAM_SIZE_BYTES,
        sram_usable_bytes: target.cartridge().ram_size().kib() * 1024,
        hram_usable_bytes: gbf_hw::memory::HRAM_SIZE_BYTES,
        source_target_profile_hash: target_profile_content_hash(&target)?,
    };
    let wram_reserved = WramReserved::new(
        wram_layout.overlay_bytes,
        wram_layout.hot_arena_bytes_min,
        wram_reserved_total,
    )?;
    let sram_reserved = runtime_sram_reserved_bytes();
    let runtime_nucleus_hash =
        RuntimeNucleusHash::real(compute_runtime_chrome_budget_nucleus_hash(
            runtime_image_hash,
            target.id(),
            &reference_shell_modules,
            &profile.id,
            &rom_slots,
            &memory_caps,
            &wram_reserved,
            sram_reserved,
        )?);

    let budget = RuntimeChromeBudget::new(
        target.id().clone(),
        profile.id.clone(),
        runtime_nucleus_hash,
        reference_shell_modules,
        rom_slots,
        memory_caps,
        wram_reserved,
        sram_reserved,
    )?;

    Ok(RuntimeChromeBudgetEmission {
        budget,
        runtime_image_hash,
        bank0_free_bytes_after_shell,
        bank0_future_reservation_bytes,
        common_bank_occupied_bytes,
        runtime_wram_internal_bytes,
        section_sizes: runtime_nucleus_section_sizes(),
    })
}

/// Write a checked RuntimeChromeBudget as `chrome_budget.json`.
pub fn write_runtime_chrome_budget_json(
    out_dir: impl AsRef<Path>,
    budget: &RuntimeChromeBudget,
) -> Result<PathBuf, RuntimeChromeBudgetEmissionError> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir)?;
    let path = out_dir.join(RUNTIME_CHROME_BUDGET_FILE_NAME);
    fs::write(&path, serde_json::to_vec_pretty(budget)?)?;
    Ok(path)
}

/// Build the current runtime shell and emit `chrome_budget.json`.
pub fn emit_runtime_chrome_budget_json(
    out_dir: impl AsRef<Path>,
) -> Result<RuntimeChromeBudgetEmission, RuntimeChromeBudgetEmissionError> {
    let emission = runtime_chrome_budget_emission()?;
    write_runtime_chrome_budget_json(out_dir, &emission.budget)?;
    Ok(emission)
}

#[allow(clippy::too_many_arguments)]
pub fn compute_runtime_chrome_budget_nucleus_hash(
    runtime_image_hash: Hash256,
    target: &TargetProfileId,
    reference_shell_modules: &BTreeSet<RuntimeShellModule>,
    compile_profile: &CompileProfileId,
    rom_slots: &[RomBudgetSlot],
    memory_caps: &RuntimeMemoryCapSection,
    wram_reserved: &WramReserved,
    sram_reserved: u32,
) -> Result<Hash256, CanonicalJsonError> {
    #[derive(Serialize)]
    struct HashInput<'a> {
        schema: &'static str,
        runtime_image_hash: Hash256,
        target: &'a TargetProfileId,
        reference_shell_modules: &'a BTreeSet<RuntimeShellModule>,
        compile_profile: &'a CompileProfileId,
        rom_slots: &'a [RomBudgetSlot],
        memory_caps: &'a RuntimeMemoryCapSection,
        wram_reserved: &'a WramReserved,
        sram_reserved: u32,
        runtime_packer_version: PackerVersion,
        gbf_runtime_crate_version: &'static str,
    }

    let canonical = CanonicalJson::to_vec(&HashInput {
        schema: "runtime_nucleus_budget_contract.v2",
        runtime_image_hash,
        target,
        reference_shell_modules,
        compile_profile,
        rom_slots,
        memory_caps,
        wram_reserved,
        sram_reserved,
        runtime_packer_version: RUNTIME_PACKER_VERSION,
        gbf_runtime_crate_version: env!("CARGO_PKG_VERSION"),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(RUNTIME_NUCLEUS_BUDGET_CONTRACT_HASH_DOMAIN);
    hasher.update(canonical);
    Ok(Hash256::from_bytes(hasher.finalize().into()))
}

#[must_use]
pub fn bank0_free_bytes_after_shell(layout: &gbf_asm::layout::LayoutPlan) -> u32 {
    let occupied = merged_occupied_bytes(
        bank0_occupied_intervals(layout),
        u32::from(gbf_asm::layout::ROM0_START),
        u32::from(gbf_asm::layout::ROM0_END_EXCLUSIVE),
    );
    u32::from(gbf_asm::layout::ROM0_END_EXCLUSIVE) - occupied
}

fn bank0_occupied_intervals(layout: &gbf_asm::layout::LayoutPlan) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    for reserved in &layout.reserved_ranges {
        if reserved.bank == gbf_asm::layout::BankIndex::Rom(0) {
            ranges.push((
                u32::from(reserved.start),
                u32::from(reserved.end_inclusive) + 1,
            ));
        }
    }
    for section in &layout.sections {
        if section.bank == gbf_asm::layout::BankIndex::Rom(0) {
            ranges.push((u32::from(section.cpu_start), section.cpu_end_exclusive()));
        }
    }
    ranges
}

fn merged_occupied_bytes(mut ranges: Vec<(u32, u32)>, start: u32, end: u32) -> u32 {
    ranges.sort_unstable();
    let mut total = 0_u32;
    let mut current: Option<(u32, u32)> = None;
    for (range_start, range_end) in ranges {
        let range_start = range_start.max(start);
        let range_end = range_end.min(end);
        if range_start >= range_end {
            continue;
        }

        current = match current {
            None => Some((range_start, range_end)),
            Some((active_start, active_end)) if range_start <= active_end => {
                Some((active_start, active_end.max(range_end)))
            }
            Some((active_start, active_end)) => {
                total += active_end - active_start;
                Some((range_start, range_end))
            }
        };
    }
    if let Some((active_start, active_end)) = current {
        total += active_end - active_start;
    }
    total
}

#[must_use]
pub fn reference_shell_bank0_future_reservation_bytes() -> u16 {
    pinned_reference_shell()
        .future_reservations
        .values()
        .map(|reservation| reservation.rom_bytes_per_bank0)
        .sum()
}

#[must_use]
pub const fn runtime_common_bank_occupied_bytes() -> u32 {
    // The current F-A5 reference shell links only Bank0 sections. When common
    // bank runtime kernels land, this becomes the measured common-bank total.
    0
}

#[must_use]
pub fn runtime_wram_internal_reserved_bytes() -> u16 {
    let ranges = [
        WramRange::inclusive(
            joypad::JOYPAD_CACHED_STATE_ADDR,
            joypad::JOYPAD_PREV_STATE_ADDR,
        ),
        WramRange::inclusive(
            video_commit::COMMIT_QUEUE_BASE_ADDR,
            video_commit::COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR,
        ),
        WramRange::inclusive(panic::WRAM_LAST_FAULT_ADDR, panic::WRAM_LAST_FAULT_HI_ADDR),
        WramRange::inclusive(
            keyboard::PROMPT_BUFFER_BASE_ADDR,
            keyboard::KEYBOARD_WORK_CELL_VALUE_ADDR,
        ),
    ];
    merged_wram_range_bytes(&ranges)
}

#[must_use]
pub const fn runtime_sram_reserved_bytes() -> u32 {
    // Persistence is intentionally outside the pinned minimal+UI shell, so the
    // real runtime shell currently owns no SRAM bytes.
    0
}

pub fn target_profile_content_hash(
    profile: &gbf_hw::target::TargetProfile,
) -> Result<Hash256, CanonicalJsonError> {
    let canonical = CanonicalJson::to_vec(profile)?;
    let mut hasher = Sha256::new();
    hasher.update(gbf_hw::target::TARGET_PROFILE_CONTENT_HASH_DOMAIN);
    hasher.update(canonical);
    Ok(Hash256::from_bytes(hasher.finalize().into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct WramRange {
    start: u16,
    end_exclusive: u16,
}

impl WramRange {
    #[must_use]
    const fn inclusive(start: WramAddr, end_inclusive: WramAddr) -> Self {
        Self {
            start: start.get(),
            end_exclusive: end_inclusive.get() + 1,
        }
    }
}

fn merged_wram_range_bytes(ranges: &[WramRange]) -> u16 {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();

    let mut total = 0_u16;
    let mut current: Option<WramRange> = None;
    for range in sorted {
        current = match current {
            None => Some(range),
            Some(mut active) if range.start <= active.end_exclusive => {
                active.end_exclusive = active.end_exclusive.max(range.end_exclusive);
                Some(active)
            }
            Some(active) => {
                total = total.saturating_add(active.end_exclusive - active.start);
                Some(range)
            }
        };
    }
    if let Some(active) = current {
        total = total.saturating_add(active.end_exclusive - active.start);
    }
    total
}

#[derive(Debug)]
pub enum RuntimeChromeBudgetEmissionError {
    Rom(gbf_asm::rom::RomAssemblyError),
    Budget(RuntimeChromeBudgetValidationError),
    Wram(WramPolicyError),
    WramTotalOverflow,
    CanonicalJson(CanonicalJsonError),
    Json(serde_json::Error),
    Io(std::io::Error),
}

impl fmt::Display for RuntimeChromeBudgetEmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rom(error) => write!(f, "runtime shell ROM assembly failed: {error}"),
            Self::Budget(error) => write!(f, "runtime chrome budget validation failed: {error}"),
            Self::Wram(error) => write!(f, "runtime WRAM reservation failed: {error}"),
            Self::WramTotalOverflow => f.write_str("runtime WRAM reserved total overflowed u16"),
            Self::CanonicalJson(error) => {
                write!(f, "runtime chrome budget canonical JSON failed: {error}")
            }
            Self::Json(error) => write!(f, "runtime chrome budget JSON failed: {error}"),
            Self::Io(error) => write!(f, "runtime chrome budget I/O failed: {error}"),
        }
    }
}

impl std::error::Error for RuntimeChromeBudgetEmissionError {}

impl From<RuntimeChromeBudgetValidationError> for RuntimeChromeBudgetEmissionError {
    fn from(error: RuntimeChromeBudgetValidationError) -> Self {
        Self::Budget(error)
    }
}

impl From<WramPolicyError> for RuntimeChromeBudgetEmissionError {
    fn from(error: WramPolicyError) -> Self {
        Self::Wram(error)
    }
}

impl From<CanonicalJsonError> for RuntimeChromeBudgetEmissionError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<serde_json::Error> for RuntimeChromeBudgetEmissionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<std::io::Error> for RuntimeChromeBudgetEmissionError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn demo_bank0_rom_image() -> Result<Vec<u8>, gbf_asm::rom::RomAssemblyError> {
    let (bank0, _, _) = normalized_bank0_image_and_symbols_for_test();
    build_demo_rom_from_bank0(&bank0)
}

/// `.sym` file for the Bank0 demo ROM, sharing layout with [`demo_bank0_rom_image`].
pub fn demo_bank0_sym_file() -> Result<String, gbf_asm::symbols::SymError> {
    let (_, symbols, layout) = normalized_bank0_image_and_symbols_for_test();
    gbf_asm::symbols::write_sym(&layout, &symbols, &demo_bank0_sym_options())
}

/// Bank0 demo outputs produced by a single Bank0 lower/layout/relax pass.
///
/// Use this when a caller (the F-A5 example, conformance fixtures) needs more
/// than one of `bank0`/`rom`/`sym` and would otherwise re-run the pipeline.
#[derive(Debug, Clone)]
pub struct DemoBank0Artifacts {
    pub bank0: [u8; 16 * 1024],
    pub rom: Vec<u8>,
    pub sym: String,
    pub layout: gbf_asm::layout::LayoutPlan,
}

pub fn demo_bank0_artifacts() -> Result<DemoBank0Artifacts, gbf_asm::rom::RomAssemblyError> {
    let (bank0, symbols, layout) = normalized_bank0_image_and_symbols_for_test();
    let rom = build_demo_rom_from_bank0(&bank0)?;
    let sym = gbf_asm::symbols::write_sym(&layout, &symbols, &demo_bank0_sym_options())
        .expect("Bank0 demo symbol table emits");
    Ok(DemoBank0Artifacts {
        bank0,
        rom,
        sym,
        layout,
    })
}

fn demo_bank0_sym_options() -> gbf_asm::symbols::SymOptions {
    gbf_asm::symbols::SymOptions {
        rom_only: true,
        ..Default::default()
    }
}

fn build_demo_rom_from_bank0(
    bank0: &[u8; 16 * 1024],
) -> Result<Vec<u8>, gbf_asm::rom::RomAssemblyError> {
    use gbf_asm::rom::global_checksum;

    let mut rom = header_stamped_demo_rom()?;
    let cartridge_header: [u8; 0x50] = rom[0x0100..0x0150]
        .try_into()
        .expect("0x50 byte cartridge header slice");
    rom[..bank0.len()].copy_from_slice(bank0);
    rom[0x0100..0x0150].copy_from_slice(&cartridge_header);
    let checksum = global_checksum(&rom);
    rom[0x014E] = (checksum >> 8) as u8;
    rom[0x014F] = checksum as u8;
    Ok(rom)
}

fn header_stamped_demo_rom() -> Result<Vec<u8>, gbf_asm::rom::RomAssemblyError> {
    use gbf_asm::encoder::EncodedSection;
    use gbf_asm::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};
    use gbf_asm::rom::{CartridgeHeader, assemble_rom};

    let encoded = EncodedSection {
        id: SECTION_ID_BOOT,
        bytes: vec![0x76],
        item_spans: Vec::new(),
    };
    let placed = PlacedSection {
        id: SECTION_ID_BOOT,
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start: gbf_asm::rom::ENTRY_POINT,
        final_size: 1,
        estimated_size: 1,
        alignment_padding: Default::default(),
    };
    let layout = LayoutPlan {
        sections: vec![placed.clone()],
        bank_count: 2,
        free_bytes_per_bank: Default::default(),
        reserved_ranges: Vec::new(),
    };
    assemble_rom(
        &[(encoded, placed)],
        &layout,
        &CartridgeHeader::new("GBFA5")?,
    )
}

pub const SECTION_JOYPAD_ADDR: u16 = 0x0600;
pub const SECTION_TEXT_ADDR: u16 = 0x0700;
pub const SECTION_KEYBOARD_ADDR: u16 = 0x0900;
pub const SECTION_VIDEO_COMMIT_ADDR: u16 = 0x0B00;
pub const SECTION_PANIC_ADDR: u16 = 0x1200;

#[must_use]
pub const fn runtime_fixed_addr(section_id: SectionId) -> Option<u16> {
    let id = section_id.get();
    if id == SECTION_ID_IRQ_VECTORS.get() {
        Some(gbf_hw::interrupts::INT_VECTOR_VBLANK)
    } else if id == SECTION_ID_BOOT.get() {
        Some(gbf_asm::rom::ENTRY_POINT)
    } else if id == SECTION_ID_BUILD_IDENTITY.get() {
        Some(BUILD_IDENTITY_BLOCK_ADDR)
    } else if id == SECTION_ID_ISR_STUBS.get() {
        Some(boot::ISR_STUBS_BASE_ADDR)
    } else if id == SECTION_ID_INTERRUPTS.get() {
        Some(interrupts::INTERRUPT_HANDLERS_BASE_ADDR)
    } else if id == SECTION_ID_SCHEDULER.get() {
        Some(boot::SCHEDULER_MAIN_LOOP_ADDR)
    } else if id == SECTION_ID_JOYPAD.get() {
        Some(SECTION_JOYPAD_ADDR)
    } else if id == SECTION_ID_TEXT.get() {
        Some(SECTION_TEXT_ADDR)
    } else if id == SECTION_ID_KEYBOARD.get() {
        Some(SECTION_KEYBOARD_ADDR)
    } else if id == SECTION_ID_VIDEO_COMMIT.get() {
        Some(SECTION_VIDEO_COMMIT_ADDR)
    } else if id == SECTION_ID_PANIC.get() {
        Some(SECTION_PANIC_ADDR)
    } else if id == SECTION_ID_TEXT_FONT.get() {
        Some(TEXT_FONT_DATA_ADDR)
    } else if id == SECTION_ID_KEYBOARD_LAYOUT.get() {
        Some(KEYBOARD_LAYOUT_DATA_ADDR)
    } else {
        None
    }
}

/// Deterministic Bank0 normalized image used by tests and the demo packet
/// until the backend owns final placement of the full runtime nucleus.
#[must_use]
pub fn normalized_bank0_image_for_test() -> [u8; 16 * 1024] {
    normalized_bank0_image_and_symbols_for_test().0
}

fn normalized_bank0_image_and_symbols_for_test() -> (
    [u8; 16 * 1024],
    gbf_asm::symbols::SymbolTable,
    gbf_asm::layout::LayoutPlan,
) {
    use std::collections::BTreeMap;

    use gbf_asm::encoder::encode_section;
    use gbf_asm::layout::{
        AddressSpace, BankIndex, PinnedPlacement, PlacedSection, PlacementProfile,
        layout_into_banks,
    };
    use gbf_asm::lowering::{StubPreLayoutOpLowering, lower_pre_layout_ops};
    use gbf_asm::relax::relax_and_legalize;
    use gbf_asm::symbols::SymbolTable;

    let mut image = [0xFF_u8; 16 * 1024];
    image[0x0100] = 0x00;
    image[0x0101] = 0xC3;
    image[0x0102..=0x0103].copy_from_slice(&gbf_asm::rom::ENTRY_POINT.to_le_bytes());
    image[0x0104..=0x0133].copy_from_slice(&gbf_hw::cartridge_header::NINTENDO_LOGO);

    let build = build_bank0_nucleus();

    let irq_vector = build
        .sections
        .iter()
        .find(|entry| entry.section.id() == SECTION_ID_IRQ_VECTORS)
        .expect("Bank0 build includes IRQ vectors")
        .section
        .clone();
    let non_vector_sections: Vec<Section> = build
        .sections
        .iter()
        .chain(build.support_sections.iter())
        .filter(|entry| entry.section.id() != SECTION_ID_IRQ_VECTORS)
        .map(|entry| entry.section.clone())
        .collect();

    let lowered = lower_pre_layout_ops(
        non_vector_sections,
        &StubPreLayoutOpLowering::default(),
        &SymbolTable::new(),
    )
    .expect("nucleus sections lower");
    let pins: Vec<PinnedPlacement> = lowered
        .iter()
        .filter_map(|section| {
            runtime_fixed_addr(section.id).map(|cpu_start| PinnedPlacement {
                section_id: section.id,
                bank: BankIndex::Rom(0),
                cpu_start,
            })
        })
        .collect();
    let layout = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &pins)
        .expect("fixed runtime layout fits");
    let linked = relax_and_legalize(&lowered, &layout).expect("runtime branches legalize");

    let irq_lowered = lower_pre_layout_ops(
        vec![irq_vector],
        &StubPreLayoutOpLowering::default(),
        &SymbolTable::new(),
    )
    .expect("IRQ vectors lower")
    .pop()
    .expect("one IRQ vector section");
    let irq_legalized = legalize_without_branches(irq_lowered);
    let irq_size = encoded_size(&irq_legalized);
    let irq_placed = PlacedSection {
        id: irq_legalized.id,
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start: runtime_fixed_addr(SECTION_ID_IRQ_VECTORS).expect("IRQ vector fixed address"),
        final_size: irq_size,
        estimated_size: irq_size,
        alignment_padding: BTreeMap::new(),
    };
    let irq_encoded = encode_section(&irq_legalized, &irq_placed).expect("IRQ vectors encode");
    append_image_bytes(
        &mut image,
        usize::from(irq_placed.cpu_start),
        &irq_encoded.bytes,
    );

    let linked_layout = linked.layout.clone();
    let linked_symbols = linked.symbols.clone();
    for section in linked.sections {
        let placed = linked_layout
            .placement_for(section.id)
            .expect("linked section has placement");
        let final_size = encoded_size(&section);
        assert_eq!(final_size, placed.final_size);
        let encoded = encode_section(&section, placed).expect("nucleus section encodes");
        append_image_bytes(&mut image, usize::from(placed.cpu_start), &encoded.bytes);
    }

    image[0x014E] = 0;
    image[0x014F] = 0;
    (image, linked_symbols, linked_layout)
}

fn append_image_bytes(image: &mut [u8; 16 * 1024], offset: usize, bytes: &[u8]) {
    let end = offset + bytes.len();
    image[offset..end].copy_from_slice(bytes);
}

fn legalize_without_branches(
    section: gbf_asm::section::LoweredSection,
) -> gbf_asm::section::LegalizedSection {
    assert!(
        section.legalization_ops.is_empty() && section.branches.is_empty(),
        "test image expects concrete runtime sections"
    );
    gbf_asm::section::LegalizedSection {
        id: section.id,
        role: section.role,
        name: section.name,
        interrupt_vector: section.interrupt_vector,
        privilege: section.privilege,
        align: section.align,
        size_hint_bytes: section.size_hint_bytes,
        next_seq_index: section.next_seq_index,
        labels: section.labels,
        instrs: section.instrs,
        data_blocks: section.data_blocks,
        alignments: section.alignments,
    }
}

fn encoded_size(section: &gbf_asm::section::LegalizedSection) -> u16 {
    let instrs: u32 = section
        .instrs
        .iter()
        .map(|item| u32::from(item.data.byte_len()))
        .sum();
    let data: u32 = section
        .data_blocks
        .iter()
        .map(|item| match &item.data {
            gbf_asm::section::DataBlock::Bytes(bytes) => bytes.len() as u32,
            gbf_asm::section::DataBlock::Words(words) => words.len() as u32 * 2,
        })
        .sum();
    u16::try_from(instrs + data).expect("test section fits u16")
}

#[must_use]
pub fn section_effect_kinds(section: &Section) -> Vec<MachineEffectKind> {
    section
        .iter_items()
        .into_iter()
        .filter_map(|item| item.machine_effect().map(|effect| effect.kind()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_abi::FaultCode;
    use gbf_asm::effect::PrivilegeClass;
    use gbf_asm::section::{ExecutionContext, InterruptDiscipline};
    use gbf_asm::symbols::SymbolName;
    use gbf_emu::{
        BootMode, ClockCycles, CycleBudget, DeterminismPolicy, Emulator, ImeSnapshot, JoypadFrame,
    };
    use gbf_hw::joypad::Button;

    const EMU_BOOT_BUDGET: CycleBudget =
        CycleBudget::Clock(gbf_emu::DMG_FRAME_CLOCK_CYCLES.saturating_mul(5));
    const EMU_STEP_BUDGET: ClockCycles = gbf_emu::DMG_FRAME_CLOCK_CYCLES.saturating_mul(3);
    const TEST_RETURN_PC: u16 = 0x3FF0;
    const TEST_STACK_SP: u16 = 0xDFFE;

    #[test]
    fn nucleus_section_order_pinned() {
        assert_eq!(
            runtime_nucleus_section_order(),
            vec![
                RuntimeShellModule::Boot,
                RuntimeShellModule::Boot,
                RuntimeShellModule::Interrupts,
                RuntimeShellModule::Interrupts,
                RuntimeShellModule::Scheduler,
                RuntimeShellModule::Joypad,
                RuntimeShellModule::Text,
                RuntimeShellModule::Keyboard,
                RuntimeShellModule::VideoCommit,
                RuntimeShellModule::Panic,
            ]
        );
    }

    #[test]
    fn nucleus_fits_bank0_budget() {
        let total: u32 = runtime_nucleus_section_sizes()
            .iter()
            .map(|section| section.bytes)
            .sum();
        assert!(
            total <= BANK0_NUCLEUS_BUDGET_BYTES as u32,
            "Bank 0 nucleus is {total} bytes, budget is {BANK0_NUCLEUS_BUDGET_BYTES}"
        );
    }

    #[test]
    fn runtime_nucleus_hash_deterministic() {
        assert_eq!(
            compute_runtime_nucleus_hash_for_test(),
            compute_runtime_nucleus_hash_for_test()
        );
    }

    #[test]
    fn runtime_nucleus_hash_normalization_zeroes_lineage_fields() {
        let image = normalized_bank0_image_for_test();
        assert_eq!(image[0x014E], 0);
        assert_eq!(image[0x014F], 0);
        let base = usize::from(BUILD_IDENTITY_BLOCK_ADDR);
        assert_eq!(&image[base + 8..base + 136], &[0; 128]);
    }

    #[test]
    fn runtime_nucleus_hash_excludes_compile_profile() {
        let image = normalized_bank0_image_for_test();
        assert_eq!(
            compute_runtime_nucleus_hash(&image),
            compute_runtime_nucleus_hash(&image),
        );
    }

    #[test]
    fn runtime_chrome_budget_json_deserializes_through_policy_schema() {
        let emission = runtime_chrome_budget_emission().expect("runtime budget emits");
        assert!(
            !emission
                .budget
                .runtime_nucleus_hash
                .is_synthetic_reference()
        );
        assert_eq!(
            emission.budget.reference_shell_modules,
            RuntimeChromeBudget::pinned_reference_shell_modules()
        );
        assert_eq!(
            target_profile_content_hash(&gbf_hw::target::dmg_mbc5_8mib_128kib())
                .expect("target hashes")
                .to_string(),
            "sha256:64a347991811c5db12b7bc17dc2802d617b461c610ccde6ef81a22a1c28947c7"
        );

        let encoded = serde_json::to_vec_pretty(&emission.budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_slice(&encoded).expect("policy budget schema deserializes");

        assert_eq!(decoded, emission.budget);
        assert_eq!(
            decoded.target,
            gbf_hw::target::dmg_mbc5_8mib_128kib().id().clone()
        );
        assert_eq!(decoded.profile, bringup_profile().id);
    }

    #[test]
    fn runtime_chrome_budget_bank0_capacity_uses_actual_shell_layout() {
        let artifacts = demo_bank0_artifacts().expect("demo artifacts build");
        let baseline = runtime_chrome_budget_emission_from_bank0_and_layout(
            &artifacts.bank0,
            &artifacts.layout,
        )
        .expect("baseline budget emits");
        let mut expanded_layout = artifacts.layout.clone();
        let expandable_section = expanded_layout
            .sections
            .iter_mut()
            .find(|section| {
                section.bank == gbf_asm::layout::BankIndex::Rom(0)
                    && bank0_address_is_free(&artifacts.layout, section.cpu_end_exclusive())
            })
            .expect("runtime layout has a Bank0 section followed by free capacity");
        expandable_section.final_size += 1;

        let changed = runtime_chrome_budget_emission_from_bank0_and_layout(
            &artifacts.bank0,
            &expanded_layout,
        )
        .expect("changed budget emits");

        assert_eq!(
            changed.bank0_free_bytes_after_shell,
            baseline.bank0_free_bytes_after_shell - 1
        );
        assert_eq!(
            bank0_slot(&changed.budget).usable_bytes,
            bank0_slot(&baseline.budget).usable_bytes - 1
        );
    }

    #[test]
    fn runtime_chrome_budget_hash_binds_bank0_image_bytes() {
        let artifacts = demo_bank0_artifacts().expect("demo artifacts build");
        let baseline = runtime_chrome_budget_emission_from_bank0_and_layout(
            &artifacts.bank0,
            &artifacts.layout,
        )
        .expect("baseline budget emits");
        let mut changed_bank0 = artifacts.bank0;
        let free_index = first_free_bank0_address(&artifacts.layout)
            .expect("normalized image has free Bank0 capacity") as usize;
        changed_bank0[free_index] = changed_bank0[free_index].wrapping_sub(1);

        let changed =
            runtime_chrome_budget_emission_from_bank0_and_layout(&changed_bank0, &artifacts.layout)
                .expect("changed budget emits");

        assert_eq!(
            changed.bank0_free_bytes_after_shell,
            baseline.bank0_free_bytes_after_shell
        );
        assert_ne!(
            changed.budget.runtime_nucleus_hash,
            baseline.budget.runtime_nucleus_hash
        );
    }

    #[test]
    fn runtime_chrome_budget_hash_binds_profile_module_set_and_budget_contract() {
        let image_hash = compute_runtime_nucleus_hash_for_test();
        let emission = runtime_chrome_budget_emission().expect("runtime budget emits");
        let budget = &emission.budget;
        let base = compute_runtime_chrome_budget_nucleus_hash(
            image_hash,
            &budget.target,
            &budget.reference_shell_modules,
            &budget.profile,
            &budget.rom_slots,
            &budget.memory_caps,
            &budget.wram_reserved,
            budget.sram_reserved,
        )
        .expect("budget hash computes");
        let profile_changed = compute_runtime_chrome_budget_nucleus_hash(
            image_hash,
            &budget.target,
            &budget.reference_shell_modules,
            &CompileProfileId::from("OtherProfile"),
            &budget.rom_slots,
            &budget.memory_caps,
            &budget.wram_reserved,
            budget.sram_reserved,
        )
        .expect("profile-variant budget hash computes");
        let mut fewer_modules = budget.reference_shell_modules.clone();
        fewer_modules.remove(&RuntimeShellModule::VideoCommit);
        let modules_changed = compute_runtime_chrome_budget_nucleus_hash(
            image_hash,
            &budget.target,
            &fewer_modules,
            &budget.profile,
            &budget.rom_slots,
            &budget.memory_caps,
            &budget.wram_reserved,
            budget.sram_reserved,
        )
        .expect("module-variant budget hash computes");
        let mut slack_changed_slots = budget.rom_slots.clone();
        slack_changed_slots[0].reserved_slack = slack_changed_slots[0]
            .reserved_slack
            .checked_add(1)
            .expect("fixture slack increments");
        let slot_slack_changed = compute_runtime_chrome_budget_nucleus_hash(
            image_hash,
            &budget.target,
            &budget.reference_shell_modules,
            &budget.profile,
            &slack_changed_slots,
            &budget.memory_caps,
            &budget.wram_reserved,
            budget.sram_reserved,
        )
        .expect("slot-slack-variant budget hash computes");

        assert_ne!(base, profile_changed);
        assert_ne!(base, modules_changed);
        assert_ne!(base, slot_slack_changed);
    }

    #[test]
    fn runtime_chrome_budget_file_emission_writes_chrome_budget_json() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let out_dir = std::env::temp_dir().join(format!(
            "gbf-runtime-bd-1g9-{}-{unique}",
            std::process::id()
        ));

        let emission = emit_runtime_chrome_budget_json(&out_dir).expect("budget file emits");
        let path = out_dir.join(RUNTIME_CHROME_BUDGET_FILE_NAME);
        let bytes = std::fs::read(&path).expect("chrome_budget.json readable");
        let decoded: RuntimeChromeBudget =
            serde_json::from_slice(&bytes).expect("chrome_budget.json policy-decodes");

        assert_eq!(decoded, emission.budget);
        std::fs::remove_dir_all(out_dir).expect("temp budget dir removed");
    }

    #[test]
    fn runtime_shell_module_annotations() {
        use gbf_abi::RuntimeShellAnnotated;

        let build = build_bank0_nucleus();
        assert_eq!(build.sections.len(), runtime_nucleus_section_order().len());
        assert!(
            build
                .sections
                .iter()
                .chain(build.support_sections.iter())
                .all(|entry| RuntimeShellModule::ALL.contains(&entry.runtime_shell_module()))
        );
    }

    #[test]
    fn wram_addr_deserialization_is_constructor_checked() {
        assert_eq!(
            serde_json::from_str::<WramAddr>("49152").unwrap().get(),
            0xC000
        );
        assert!(serde_json::from_str::<WramAddr>("32768").is_err());
    }

    fn bank0_slot(budget: &RuntimeChromeBudget) -> &RomBudgetSlot {
        budget
            .rom_slots
            .iter()
            .find(|slot| slot.class == BudgetSlotClass::Bank0Free)
            .expect("budget has Bank0Free slot")
    }

    fn bank0_address_is_free(layout: &gbf_asm::layout::LayoutPlan, address: u32) -> bool {
        if address >= u32::from(gbf_asm::layout::ROM0_END_EXCLUSIVE) {
            return false;
        }
        !bank0_occupied_intervals(layout)
            .into_iter()
            .any(|(start, end)| start <= address && address < end)
    }

    fn first_free_bank0_address(layout: &gbf_asm::layout::LayoutPlan) -> Option<u32> {
        (u32::from(gbf_asm::layout::ROM0_START)..u32::from(gbf_asm::layout::ROM0_END_EXCLUSIVE))
            .find(|address| bank0_address_is_free(layout, *address))
    }

    #[test]
    fn isr_residency_pure() {
        let build = build_bank0_nucleus();
        for entry in build.sections {
            let privilege = entry.section.privilege();
            match entry.section.id() {
                SECTION_ID_ISR_STUBS | SECTION_ID_INTERRUPTS => {
                    assert_eq!(
                        privilege.default_privilege,
                        PrivilegeClass::InterruptHandler
                    );
                    assert_eq!(
                        privilege.execution_context,
                        ExecutionContext::InterruptHandler
                    );
                    assert_eq!(
                        privilege.interrupt_discipline,
                        InterruptDiscipline::ImeDisabled
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn demo_bank0_rom_boots_to_scheduler_in_real_emu() {
        let mut emu = load_demo_emu();

        emu.run_until_pc(boot::SCHEDULER_MAIN_LOOP_ADDR, EMU_BOOT_BUDGET)
            .expect("runtime reaches scheduler");

        assert_eq!(emu.peek(banking::HRAM_ADDR_CURRENT_ROM_BANK_LO).unwrap(), 0);
        assert_eq!(emu.peek(banking::HRAM_ADDR_CURRENT_ROM_BANK_HI).unwrap(), 0);
        assert_eq!(emu.peek(banking::HRAM_ADDR_CURRENT_SRAM_BANK).unwrap(), 0);
        assert_eq!(emu.peek(banking::HRAM_ADDR_SRAM_ENABLED).unwrap(), 0);
        assert_eq!(emu.peek(scheduler::HRAM_ADDR_YIELD_REQUESTED).unwrap(), 0);
        assert_eq!(emu.peek(scheduler::HRAM_ADDR_FRAME_COUNT).unwrap(), 0);

        assert_eq!(
            emu.bus_read(gbf_hw::lcd::LCDC_REG).expect("LCDC readable"),
            boot::BootInitPolicy::bring_up().default_lcdc
        );
        assert_ne!(
            emu.bus_read(gbf_hw::lcd::STAT_REG).expect("STAT readable")
                & gbf_hw::lcd::STAT_INTERRUPT_HBLANK_ENABLE,
            0
        );
        assert_eq!(
            emu.peek(gbf_hw::interrupts::IE_REGISTER).unwrap()
                & boot::BootInitPolicy::bring_up().default_ie_mask,
            boot::BootInitPolicy::bring_up().default_ie_mask
        );

        let font = include_bytes!("../assets/font_8x8.bin");
        let glyph_a = usize::from(b'A') * 16;
        assert_eq!(
            emu.peek_range(gbf_hw::memory::VRAM_BASE + glyph_a as u16, 16)
                .expect("VRAM glyph bytes readable"),
            font[glyph_a..glyph_a + 16]
        );
        assert_eq!(
            emu.peek_range(
                video_commit::BOOTSTRAP_BG_MAP_ORIGIN,
                usize::from(video_commit::BOOTSTRAP_BG_MAP_BYTES)
            )
            .expect("BG map readable"),
            vec![0; usize::from(video_commit::BOOTSTRAP_BG_MAP_BYTES)]
        );
    }

    #[test]
    fn joypad_reader_decodes_buttons_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.poke(joypad::JOYPAD_CACHED_STATE_ADDR.get(), 0).unwrap();
        emu.set_joypad(JoypadFrame::default().with(Button::A).with(Button::Right));

        call_runtime_subroutine(&mut emu, symbol_addr("joypad", "read"), 0);

        let state = emu.peek(joypad::JOYPAD_CACHED_STATE_ADDR.get()).unwrap();
        assert_ne!(state & Button::A.state_mask(), 0);
        assert_ne!(state & Button::Right.state_mask(), 0);
        assert_eq!(state & Button::Left.state_mask(), 0);
        assert_eq!(emu.peek(joypad::JOYPAD_PREV_STATE_ADDR.get()).unwrap(), 0);
    }

    #[test]
    fn keyboard_step_accepts_selected_layout_cell_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.poke(joypad::JOYPAD_PREV_STATE_ADDR.get(), 0).unwrap();
        emu.poke(
            joypad::JOYPAD_CACHED_STATE_ADDR.get(),
            Button::A.state_mask(),
        )
        .unwrap();
        emu.poke(keyboard::KEYBOARD_CURSOR_ADDR.get(), 1).unwrap();
        emu.poke(keyboard::PROMPT_CURSOR_ADDR.get(), 21).unwrap();
        emu.poke(keyboard::PROMPT_SUBMITTED_FLAG_ADDR.get(), 0)
            .unwrap();
        emu.poke(video_commit::COMMIT_QUEUE_HEAD_ADDR.get(), 0)
            .unwrap();
        emu.poke(video_commit::COMMIT_QUEUE_TAIL_ADDR.get(), 0)
            .unwrap();

        call_runtime_subroutine(&mut emu, symbol_addr("keyboard", "step"), 0);

        assert_eq!(
            emu.peek(keyboard::PROMPT_BUFFER_BASE_ADDR.get() + 21)
                .unwrap(),
            b'b'
        );
        assert_eq!(emu.peek(keyboard::PROMPT_CURSOR_ADDR.get()).unwrap(), 22);
        assert_eq!(
            emu.peek(keyboard::PROMPT_SUBMITTED_FLAG_ADDR.get())
                .unwrap(),
            0
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_TAIL_ADDR.get())
                .unwrap(),
            1
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get())
                .unwrap(),
            video_commit::UiCommitOpKind::PutGlyphCell as u8
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 2)
                .unwrap(),
            1
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 3)
                .unwrap(),
            1
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 4)
                .unwrap(),
            b'b'
        );
    }

    #[test]
    fn keyboard_backspace_enqueues_blank_glyph_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.poke(joypad::JOYPAD_PREV_STATE_ADDR.get(), 0).unwrap();
        emu.poke(
            joypad::JOYPAD_CACHED_STATE_ADDR.get(),
            Button::A.state_mask(),
        )
        .unwrap();
        emu.poke(keyboard::KEYBOARD_CURSOR_ADDR.get(), 38).unwrap();
        emu.poke(keyboard::PROMPT_CURSOR_ADDR.get(), 21).unwrap();
        emu.poke(video_commit::COMMIT_QUEUE_HEAD_ADDR.get(), 0)
            .unwrap();
        emu.poke(video_commit::COMMIT_QUEUE_TAIL_ADDR.get(), 0)
            .unwrap();

        call_runtime_subroutine(&mut emu, symbol_addr("keyboard", "step"), 0);

        assert_eq!(emu.peek(keyboard::PROMPT_CURSOR_ADDR.get()).unwrap(), 20);
        assert_eq!(
            emu.peek(keyboard::PROMPT_BUFFER_BASE_ADDR.get() + 20)
                .unwrap(),
            b' '
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_TAIL_ADDR.get())
                .unwrap(),
            1
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get())
                .unwrap(),
            video_commit::UiCommitOpKind::PutGlyphCell as u8
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 2)
                .unwrap(),
            0
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 3)
                .unwrap(),
            1
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_BASE_ADDR.get() + 4)
                .unwrap(),
            b' '
        );
    }

    #[test]
    fn video_commit_drains_glyph_cell_to_bg_map_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.run_until_pc(boot::SCHEDULER_MAIN_LOOP_ADDR, EMU_BOOT_BUDGET)
            .expect("runtime reaches scheduler");
        let op = video_commit::UiCommitWireOp::encode(video_commit::UiCommitOp::PutGlyphCell {
            x: 3,
            y: 2,
            glyph: b'Z',
        })
        .bytes();
        for (idx, byte) in op.into_iter().enumerate() {
            emu.poke(
                video_commit::COMMIT_QUEUE_BASE_ADDR.get() + idx as u16,
                byte,
            )
            .unwrap();
        }
        emu.poke(video_commit::COMMIT_QUEUE_HEAD_ADDR.get(), 0)
            .unwrap();
        emu.poke(video_commit::COMMIT_QUEUE_TAIL_ADDR.get(), 1)
            .unwrap();
        emu.bus_write(gbf_hw::lcd::LCDC_REG, 0)
            .expect("LCD can be disabled for unrestricted VRAM commit");

        call_runtime_subroutine(&mut emu, symbol_addr("video_commit", "drain_vblank"), 0);

        assert_eq!(
            emu.peek(video_commit::BOOTSTRAP_BG_MAP_ORIGIN + 2 * 32 + 3)
                .unwrap(),
            b'Z'
        );
        assert_eq!(
            emu.peek(video_commit::COMMIT_QUEUE_HEAD_ADDR.get())
                .unwrap(),
            1
        );
    }

    #[test]
    fn video_commit_illegal_mode_raises_fault_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.run_until_pc(boot::SCHEDULER_MAIN_LOOP_ADDR, EMU_BOOT_BUDGET)
            .expect("runtime reaches scheduler");
        emu.bus_write(
            gbf_hw::lcd::LCDC_REG,
            boot::BootInitPolicy::bring_up().default_lcdc,
        )
        .expect("LCD can be enabled");
        run_until(
            &mut emu,
            EMU_STEP_BUDGET,
            |emu| {
                emu.bus_read(gbf_hw::lcd::LY_REG)
                    .is_ok_and(|ly| ly < gbf_hw::lcd::VBLANK_FIRST_LY)
            },
            "non-VBlank scanline",
        );

        let mut regs = emu.regs();
        regs.pc = symbol_addr("video_commit", "drain_vblank");
        regs.sp = TEST_STACK_SP;
        regs.ime = ImeSnapshot::Disabled;
        emu.set_regs(regs).unwrap();

        let expected = FaultCode::UiCommitOutsideLegalMode as u16;
        run_until(
            &mut emu,
            EMU_STEP_BUDGET,
            |emu| last_fault_word(emu) == Some(expected),
            "illegal-mode panic fault",
        );
    }

    #[test]
    fn panic_entry_renders_fault_code_in_real_emu() {
        let mut emu = load_demo_emu();
        emu.run_until_pc(boot::SCHEDULER_MAIN_LOOP_ADDR, EMU_BOOT_BUDGET)
            .expect("runtime reaches scheduler");

        let mut regs = emu.regs();
        regs.pc = symbol_addr("panic", "entry");
        regs.h = ((FaultCode::UiCommitQueueFull as u16) >> 8) as u8;
        regs.l = (FaultCode::UiCommitQueueFull as u16 & 0x00FF) as u8;
        regs.ime = ImeSnapshot::Disabled;
        emu.set_regs(regs).unwrap();

        run_until(
            &mut emu,
            gbf_emu::DMG_FRAME_CLOCK_CYCLES.saturating_mul(4),
            |emu| {
                emu.peek(panic::PANIC_SCREEN_BG_ADDR + 9)
                    .is_ok_and(|byte| byte == b'1')
                    && emu
                        .bus_read(gbf_hw::lcd::LCDC_REG)
                        .is_ok_and(|lcdc| lcdc == panic::PANIC_VISIBLE_LCDC)
            },
            "panic screen LCD re-enable",
        );

        let rendered = emu
            .peek_range(panic::PANIC_SCREEN_BG_ADDR, 10)
            .expect("panic BG map readable");
        assert_eq!(&rendered[..10], b"FAULT 0041");
        assert_eq!(
            emu.bus_read(gbf_hw::lcd::LCDC_REG).expect("LCDC readable"),
            panic::PANIC_VISIBLE_LCDC
        );
    }

    fn load_demo_emu() -> Emulator {
        Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .load_rom(&demo_bank0_rom_image().expect("demo ROM assembles"))
            .expect("demo ROM loads")
    }

    fn symbol_addr(module: &'static str, target: &'static str) -> u16 {
        let (_, symbols, layout) = normalized_bank0_image_and_symbols_for_test();
        let name = SymbolName::runtime(module, target).expect("static runtime symbol");
        let address = symbols.resolve(&name).expect("runtime symbol resolves");
        let placed = layout
            .placement_for(address.section)
            .expect("symbol section has placement");
        u16::try_from(u32::from(placed.cpu_start) + address.offset)
            .expect("runtime symbol is in 16-bit CPU space")
    }

    fn call_runtime_subroutine(emu: &mut Emulator, addr: u16, a: u8) {
        emu.poke(TEST_STACK_SP, (TEST_RETURN_PC & 0x00FF) as u8)
            .unwrap();
        emu.poke(TEST_STACK_SP + 1, (TEST_RETURN_PC >> 8) as u8)
            .unwrap();
        let mut regs = emu.regs();
        regs.pc = addr;
        regs.sp = TEST_STACK_SP;
        regs.a = a;
        regs.ime = ImeSnapshot::Disabled;
        emu.set_regs(regs).unwrap();
        run_until_pc_allow_idle(emu, TEST_RETURN_PC, EMU_STEP_BUDGET);
    }

    fn last_fault_word(emu: &mut Emulator) -> Option<u16> {
        let lo = emu.peek(panic::WRAM_LAST_FAULT_ADDR.get()).ok()?;
        let hi = emu.peek(panic::WRAM_LAST_FAULT_HI_ADDR.get()).ok()?;
        Some(u16::from_le_bytes([lo, hi]))
    }

    fn run_until_pc_allow_idle(emu: &mut Emulator, pc: u16, budget: ClockCycles) {
        run_until(
            emu,
            budget,
            |emu| emu.regs().pc == pc,
            "runtime subroutine return",
        );
    }

    fn run_until(
        emu: &mut Emulator,
        budget: ClockCycles,
        mut predicate: impl FnMut(&mut Emulator) -> bool,
        label: &str,
    ) {
        let deadline = emu.clock_count().0.saturating_add(budget.0);
        while emu.clock_count().0 < deadline {
            if predicate(emu) {
                return;
            }
            emu.step().expect("emulator step succeeds");
        }
        panic!("{label} not reached within {budget:?}");
    }
}
