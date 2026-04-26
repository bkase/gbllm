//! Structured provenance for assembly instructions and data directives.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Source value identifier carried from compiler IR into assembly provenance.
///
/// `gbf-asm` cannot depend on later IR crates without creating a cycle, so this
/// is an ASM provenance adapter handle. IR/codegen layers should map their
/// native value IDs into this type at the assembly boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AsmSourceValueId(u32);

impl AsmSourceValueId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for AsmSourceValueId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<AsmSourceValueId> for u32 {
    fn from(value: AsmSourceValueId) -> Self {
        value.get()
    }
}

impl fmt::Display for AsmSourceValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compiler stage that emitted or last transformed an assembly item.
///
/// The explicit discriminants are part of the report/schema contract. Append
/// new stages only with a migration decision and a test update.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStage {
    QuantGraph = 0,
    StoragePlan = 1,
    RomWindowPlan = 2,
    OverlayPlan = 3,
    ArenaPlan = 4,
    GbSchedIr = 5,
    Backend = 6,
}

impl PlanningStage {
    pub const ALL: [Self; 7] = [
        Self::QuantGraph,
        Self::StoragePlan,
        Self::RomWindowPlan,
        Self::OverlayPlan,
        Self::ArenaPlan,
        Self::GbSchedIr,
        Self::Backend,
    ];

    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::QuantGraph => "quant_graph",
            Self::StoragePlan => "storage_plan",
            Self::RomWindowPlan => "rom_window_plan",
            Self::OverlayPlan => "overlay_plan",
            Self::ArenaPlan => "arena_plan",
            Self::GbSchedIr => "gb_sched_ir",
            Self::Backend => "backend",
        }
    }
}

/// Required provenance payload attached to each instruction or data directive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstrProvenance {
    pub stage: PlanningStage,
    pub source_node: Option<AsmSourceValueId>,
    pub source_op: Option<Cow<'static, str>>,
    pub note: Option<Cow<'static, str>>,
}

impl InstrProvenance {
    #[must_use]
    pub const fn new(stage: PlanningStage) -> Self {
        Self {
            stage,
            source_node: None,
            source_op: None,
            note: None,
        }
    }

    #[must_use]
    pub fn with_source_node(mut self, source_node: AsmSourceValueId) -> Self {
        self.source_node = Some(source_node);
        self
    }

    #[must_use]
    pub fn with_source_op(mut self, source_op: impl Into<Cow<'static, str>>) -> Self {
        self.source_op = Some(source_op.into());
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<Cow<'static, str>>) -> Self {
        self.note = Some(note.into());
        self
    }
}

#[cfg(test)]
#[test]
fn stage_enum_stable() {
    assert_eq!(
        PlanningStage::ALL,
        [
            PlanningStage::QuantGraph,
            PlanningStage::StoragePlan,
            PlanningStage::RomWindowPlan,
            PlanningStage::OverlayPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIr,
            PlanningStage::Backend,
        ]
    );
    assert_eq!(
        PlanningStage::ALL.map(PlanningStage::code),
        [0, 1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        PlanningStage::ALL.map(PlanningStage::canonical_name),
        [
            "quant_graph",
            "storage_plan",
            "rom_window_plan",
            "overlay_plan",
            "arena_plan",
            "gb_sched_ir",
            "backend",
        ]
    );

    let provenance = InstrProvenance::new(PlanningStage::GbSchedIr)
        .with_source_node(AsmSourceValueId::new(42))
        .with_source_op("tile_matvec")
        .with_note("lowered after storage placement");
    let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
    let decoded: InstrProvenance = serde_json::from_str(&encoded).expect("provenance deserializes");

    assert_eq!(decoded, provenance);
    assert_eq!(
        serde_json::to_string(&PlanningStage::GbSchedIr).expect("stage serializes"),
        r#""gb_sched_ir""#
    );
    assert_eq!(
        serde_json::from_str::<PlanningStage>(r#""rom_window_plan""#)
            .expect("stage deserializes from canonical name"),
        PlanningStage::RomWindowPlan
    );
}
