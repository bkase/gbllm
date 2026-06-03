//! Typed Game Boy interrupt-vector slots.

use std::fmt;

use gbf_hw::interrupts::{InterruptSource, vector_for};
use serde::{Deserialize, Serialize};

/// Fixed DMG/CGB interrupt-vector slots in ROM0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptVectorSlot {
    VBlank,
    LcdStat,
    Timer,
    Serial,
    Joypad,
}

impl InterruptVectorSlot {
    pub const ALL: [Self; 5] = [
        Self::VBlank,
        Self::LcdStat,
        Self::Timer,
        Self::Serial,
        Self::Joypad,
    ];

    pub const SLOT_BYTES: u16 = 8;

    #[must_use]
    pub const fn source(self) -> InterruptSource {
        match self {
            Self::VBlank => InterruptSource::VBlank,
            Self::LcdStat => InterruptSource::LcdStat,
            Self::Timer => InterruptSource::Timer,
            Self::Serial => InterruptSource::Serial,
            Self::Joypad => InterruptSource::Joypad,
        }
    }

    #[must_use]
    pub const fn address(self) -> u16 {
        vector_for(self.source())
    }

    #[must_use]
    pub const fn end_exclusive(self) -> u16 {
        self.address() + Self::SLOT_BYTES
    }

    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::VBlank => "vblank",
            Self::LcdStat => "lcd_stat",
            Self::Timer => "timer",
            Self::Serial => "serial",
            Self::Joypad => "joypad",
        }
    }

    pub const fn from_address(address: u16) -> Result<Self, InterruptVectorSlotError> {
        if address == vector_for(InterruptSource::VBlank) {
            Ok(Self::VBlank)
        } else if address == vector_for(InterruptSource::LcdStat) {
            Ok(Self::LcdStat)
        } else if address == vector_for(InterruptSource::Timer) {
            Ok(Self::Timer)
        } else if address == vector_for(InterruptSource::Serial) {
            Ok(Self::Serial)
        } else if address == vector_for(InterruptSource::Joypad) {
            Ok(Self::Joypad)
        } else {
            Err(InterruptVectorSlotError::NonVectorAddress { address })
        }
    }
}

impl fmt::Display for InterruptVectorSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.canonical_name())
    }
}

/// Address validation error for typed interrupt-vector slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptVectorSlotError {
    NonVectorAddress { address: u16 },
}

impl fmt::Display for InterruptVectorSlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonVectorAddress { address } => {
                write!(f, "${address:04X} is not a Game Boy interrupt vector slot")
            }
        }
    }
}

impl std::error::Error for InterruptVectorSlotError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{Builder, BuilderError};
    use crate::encoder::encode_section;
    use crate::isa::Instr;
    use crate::layout::{
        BankIndex, LayoutError, PinnedPlacement, PlacementProfile, layout_into_banks,
    };
    use crate::listing::{ListingOptions, emit_listing};
    use crate::lowering::{StubPreLayoutOpLowering, lower_pre_layout_ops};
    use crate::relax::relax_and_legalize;
    use crate::section::{InterruptVector, SectionId, SectionRole, SymbolicBranch};
    use crate::symbols::{SymbolAddress, SymbolName, SymbolTable};

    #[test]
    fn interrupt_vector_slot_accepts_only_fixed_addresses() {
        assert_eq!(
            InterruptVectorSlot::from_address(vector_for(InterruptSource::VBlank)),
            Ok(InterruptVectorSlot::VBlank)
        );
        assert_eq!(
            InterruptVectorSlot::from_address(vector_for(InterruptSource::LcdStat)),
            Ok(InterruptVectorSlot::LcdStat)
        );
        let non_vector_address = vector_for(InterruptSource::VBlank) + 1;
        assert!(matches!(
            InterruptVectorSlot::from_address(non_vector_address),
            Err(InterruptVectorSlotError::NonVectorAddress { address }) if address == non_vector_address
        ));
    }

    #[test]
    fn interrupt_vector_vblank_emits_typed_jp_stub_and_listing_visibility() {
        let slot = InterruptVectorSlot::VBlank;
        let owner = sym("runtime.interrupt.vblank_vector");
        let target = sym("runtime.interrupt.vblank_handler");
        let vector = Builder::interrupt_vector_with_id(
            SectionId::new(1),
            slot,
            owner.clone(),
            target.clone(),
        )
        .finish();
        let mut handler =
            Builder::new_with_id(SectionId::new(2), SectionRole::Bank0Nucleus, target.clone());
        handler.label(target.clone());
        handler.emit(Instr::Nop);

        let lowered = lower_pre_layout_ops(
            vec![vector, handler.finish()],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("pre-layout lowering succeeds");
        let layout = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &[])
            .expect("layout succeeds");
        let placed_vector = layout
            .placement_for(SectionId::new(1))
            .expect("vector placement");
        assert_eq!(placed_vector.bank, BankIndex::Rom(0));
        assert_eq!(placed_vector.cpu_start, slot.address());
        assert_eq!(placed_vector.final_size, 3);

        let relaxed = relax_and_legalize(&lowered, &layout).expect("relax succeeds");
        let legalized_vector = relaxed
            .sections
            .iter()
            .find(|section| section.id == SectionId::new(1))
            .expect("legalized vector");
        assert_eq!(
            legalized_vector
                .interrupt_vector
                .as_ref()
                .map(|vector| vector.slot),
            Some(slot)
        );
        assert!(matches!(
            legalized_vector.instrs[0].data,
            Instr::JpAbs { cond: None, .. }
        ));

        let target_addr = resolved_cpu_addr(&relaxed.layout, &relaxed.symbols, &target);
        let encoded = encode_section(legalized_vector, placed_vector).expect("vector encodes");
        assert_eq!(
            encoded.bytes,
            vec![0xC3, target_addr as u8, (target_addr >> 8) as u8]
        );
        assert_eq!(
            relaxed.symbols.resolve(&owner),
            Some(SymbolAddress::new(SectionId::new(1), 0))
        );

        let listing = emit_listing(
            legalized_vector,
            &encoded,
            placed_vector,
            &relaxed.symbols,
            &ListingOptions::default(),
        )
        .expect("listing emits");
        assert!(listing.contains("interrupt_vector: vblank"));
        assert!(listing.contains("target=runtime.interrupt.vblank_handler"));
        assert!(listing.contains("<runtime.interrupt.vblank_vector>:"));
        assert!(listing.contains("jp"));
    }

    #[test]
    fn interrupt_vector_duplicate_slot_is_rejected() {
        let target = sym("runtime.interrupt.handler");
        let vector_a = Builder::interrupt_vector_with_id(
            SectionId::new(1),
            InterruptVectorSlot::VBlank,
            sym("runtime.interrupt.vector_a"),
            target.clone(),
        )
        .finish();
        let vector_b = Builder::interrupt_vector_with_id(
            SectionId::new(2),
            InterruptVectorSlot::VBlank,
            sym("runtime.interrupt.vector_b"),
            target,
        )
        .finish();
        let lowered = lower_pre_layout_ops(
            vec![vector_a, vector_b],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("pre-layout lowering succeeds");

        let err = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &[])
            .expect_err("duplicate vector slots rejected");
        assert!(matches!(
            err,
            LayoutError::DuplicateInterruptVectorSlot {
                slot: InterruptVectorSlot::VBlank,
                ..
            }
        ));
    }

    #[test]
    fn interrupt_vector_duplicate_owner_is_rejected() {
        let owner = sym("runtime.interrupt.shared_vector");
        let vector_a = Builder::interrupt_vector_with_id(
            SectionId::new(1),
            InterruptVectorSlot::VBlank,
            owner.clone(),
            sym("runtime.interrupt.vblank_handler"),
        )
        .finish();
        let vector_b = Builder::interrupt_vector_with_id(
            SectionId::new(2),
            InterruptVectorSlot::Timer,
            owner,
            sym("runtime.interrupt.timer_handler"),
        )
        .finish();
        let lowered = lower_pre_layout_ops(
            vec![vector_a, vector_b],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("pre-layout lowering succeeds");

        let err = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &[])
            .expect_err("duplicate vector owners rejected");
        assert!(matches!(
            err,
            LayoutError::DuplicateInterruptVectorOwner { .. }
        ));
    }

    #[test]
    fn interrupt_vector_builder_rejects_post_stub_payload() {
        let target = sym("runtime.interrupt.vblank_handler");
        let mut vector = Builder::interrupt_vector_with_id(
            SectionId::new(1),
            InterruptVectorSlot::VBlank,
            sym("runtime.interrupt.vblank_vector"),
            target,
        );

        let err = vector
            .try_emit(Instr::Nop)
            .expect_err("canonical vector payload is sealed");
        assert!(matches!(
            err,
            BuilderError::InterruptVectorPayloadSealed {
                role: SectionRole::InterruptVector,
            }
        ));
    }

    #[test]
    fn interrupt_vector_stub_overflow_is_rejected() {
        let target = sym("runtime.interrupt.vblank_handler");
        let owner = sym("runtime.interrupt.vblank_vector");
        let mut vector = Builder::new_with_id(
            SectionId::new(1),
            SectionRole::InterruptVector,
            owner.clone(),
        );
        vector.label(owner);
        vector.branch(SymbolicBranch::interrupt_vector_jp(target.clone()));
        for _ in 0..InterruptVectorSlot::SLOT_BYTES {
            vector.emit(Instr::Nop);
        }
        let vector = vector.finish().with_interrupt_vector(InterruptVector {
            slot: InterruptVectorSlot::VBlank,
            target,
        });
        let lowered = lower_pre_layout_ops(
            vec![vector],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("pre-layout lowering succeeds");

        let err = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &[])
            .expect_err("oversized vector stub rejected");
        assert!(matches!(
            err,
            LayoutError::InterruptVectorStubTooLarge {
                slot: InterruptVectorSlot::VBlank,
                ..
            }
        ));
    }

    #[test]
    fn interrupt_vector_reserved_slot_rejects_bank0_overlap() {
        let mut bank0 = Builder::new_with_id(
            SectionId::new(9),
            SectionRole::Bank0Nucleus,
            sym("runtime.interrupt.overlap"),
        );
        bank0.emit(Instr::Nop);
        let lowered = lower_pre_layout_ops(
            vec![bank0.finish()],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("pre-layout lowering succeeds");

        let err = layout_into_banks(
            &lowered,
            PlacementProfile::PackedExperts,
            &[PinnedPlacement {
                section_id: SectionId::new(9),
                bank: BankIndex::Rom(0),
                cpu_start: InterruptVectorSlot::VBlank.address(),
            }],
        )
        .expect_err("pinned section cannot overlap vector slot");
        assert!(matches!(err, LayoutError::PlacementCollision { .. }));
    }

    #[test]
    fn interrupt_vector_reserved_bank0_neighbors_reject_overlap() {
        for (section_id, name, cpu_start) in [
            (
                SectionId::new(10),
                "runtime.interrupt.reset_overlap",
                0x0000,
            ),
            (
                SectionId::new(11),
                "runtime.interrupt.header_overlap",
                0x0100,
            ),
        ] {
            let mut bank0 = Builder::new_with_id(section_id, SectionRole::Bank0Nucleus, sym(name));
            bank0.emit(Instr::Nop);
            let lowered = lower_pre_layout_ops(
                vec![bank0.finish()],
                &StubPreLayoutOpLowering::default(),
                &SymbolTable::new(),
            )
            .expect("pre-layout lowering succeeds");

            let err = layout_into_banks(
                &lowered,
                PlacementProfile::PackedExperts,
                &[PinnedPlacement {
                    section_id,
                    bank: BankIndex::Rom(0),
                    cpu_start,
                }],
            )
            .expect_err("pinned section cannot overlap reserved Bank0 neighbor");
            assert!(matches!(
                err,
                LayoutError::PlacementCollision {
                    section_id: actual_section_id,
                    bank: BankIndex::Rom(0),
                    start,
                    ..
                } if actual_section_id == section_id && start == cpu_start
            ));
        }
    }

    fn sym(value: &'static str) -> SymbolName {
        SymbolName::new(value).expect("valid symbol")
    }

    fn resolved_cpu_addr(
        layout: &crate::layout::LayoutPlan,
        symbols: &SymbolTable,
        name: &SymbolName,
    ) -> u16 {
        let address = symbols.resolve(name).expect("symbol resolves");
        let placed = layout
            .placement_for(address.section)
            .expect("symbol section placed");
        placed.cpu_start + u16::try_from(address.offset).expect("offset fits")
    }
}
