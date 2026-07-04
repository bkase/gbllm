//! Kernel selection: map each op of the legalized model program onto a
//! measured kernel family (bd-1skgm).
//!
//! Selection is real but the menu is currently one deep per op class: the
//! kernel bake-off (bd-rzq5n, `docs/experiments/kernel-bakeoff`) measured
//! V1 (data-driven), V2 (unrolled), and V3 (weights-as-code) ternary matvec
//! variants and V3 won on M-cycles/MAC, so block matvecs select the banked
//! V3 family; the tied head selects the lane-major i8 product-LUT kernel and
//! the scalar ops select the integer routines proven byte-exact by the
//! one-token/multi-token gates (`docs/experiments/one-token`,
//! `docs/experiments/multi-token`). The backend
//! ([`crate::compile`]) verifies the selected families are exactly the ones
//! the `gbf_kernel::asm_impl_model` builder emits, so a future second family
//! cannot be silently ignored.

use serde::Serialize;

use crate::lower_infer::DenseBigramProgram;

/// The kernel families implemented by the `gbf-kernel` model-ROM backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KernelFamily {
    /// Banked straight-line add/sub code over u8 zp128 activations walked
    /// with SP; i16 accumulators seeded with `-128 * sum(row)`.
    V3WeightsAsCodeBanked,
    /// Q11.5 embedding rows copied from banked ROM tables.
    EmbeddingTableQ11_5Banked,
    /// Integer full-vector RMS norm + activation quant (sum of squares,
    /// floor isqrt, per-lane rounded division).
    IntNormQuantFullVector,
    /// Q8.8 row-scale multiply with round-half-away requantization.
    ScaleEpilogueQ8_8,
    /// 255-entry GELU LUT on the activation grid.
    GeluLut255,
    /// Q8.8 scale epilogue to the Q11.5 residual grid + wrapping add.
    ScaleEpilogueQ8_8ResidualAdd,
    /// Lane-major transposed i8 tied head via per-lane product LUT pages,
    /// i24 accumulation.
    TiedHeadLaneMajorI8ProductLut,
    /// 256-way i24 argmax scan, lowest index wins ties.
    ArgmaxI24LowestIndex,
}

/// One op-to-kernel decision with its measured-evidence rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelSelection {
    pub op: String,
    pub kernel: KernelFamily,
    pub rationale: &'static str,
}

/// The complete kernel plan for a model program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelPlan {
    pub selections: Vec<KernelSelection>,
}

impl KernelPlan {
    /// Kernel selected for a named op, if any.
    #[must_use]
    pub fn kernel_for(&self, op: &str) -> Option<KernelFamily> {
        self.selections
            .iter()
            .find(|s| s.op == op)
            .map(|s| s.kernel)
    }
}

const MATVEC_RATIONALE: &str = "kernel bake-off (bd-rzq5n): V3 weights-as-code won \
     M-cycles/MAC over V1/V2; byte-exact under the one-token and multi-token gates";
const TABLE_RATIONALE: &str =
    "banked data table proven byte-exact by the one-token bring-up (bd-59qiq)";
const ROUTINE_RATIONALE: &str =
    "bank-0 integer routine proven byte-exact by the one-token bring-up (bd-59qiq)";

/// Select a kernel for every op in the program.
#[must_use]
pub fn select_kernels(program: &DenseBigramProgram) -> KernelPlan {
    let mut selections = Vec::new();
    let mut select = |op: String, kernel: KernelFamily, rationale: &'static str| {
        selections.push(KernelSelection {
            op,
            kernel,
            rationale,
        });
    };

    select(
        "embed_lookup".to_string(),
        KernelFamily::EmbeddingTableQ11_5Banked,
        TABLE_RATIONALE,
    );
    for block in &program.blocks {
        let k = block.index;
        select(
            format!("block{k}.norm_quant"),
            KernelFamily::IntNormQuantFullVector,
            ROUTINE_RATIONALE,
        );
        select(
            format!("block{k}.up.matvec"),
            KernelFamily::V3WeightsAsCodeBanked,
            MATVEC_RATIONALE,
        );
        select(
            format!("block{k}.up.scale_epilogue"),
            KernelFamily::ScaleEpilogueQ8_8,
            ROUTINE_RATIONALE,
        );
        select(
            format!("block{k}.gelu"),
            KernelFamily::GeluLut255,
            ROUTINE_RATIONALE,
        );
        select(
            format!("block{k}.down.matvec"),
            KernelFamily::V3WeightsAsCodeBanked,
            MATVEC_RATIONALE,
        );
        select(
            format!("block{k}.down.scale_epilogue_residual_add"),
            KernelFamily::ScaleEpilogueQ8_8ResidualAdd,
            ROUTINE_RATIONALE,
        );
    }
    select(
        "final.norm_quant".to_string(),
        KernelFamily::IntNormQuantFullVector,
        ROUTINE_RATIONALE,
    );
    select(
        "head.tied_matvec".to_string(),
        KernelFamily::TiedHeadLaneMajorI8ProductLut,
        TABLE_RATIONALE,
    );
    select(
        "head.argmax".to_string(),
        KernelFamily::ArgmaxI24LowestIndex,
        ROUTINE_RATIONALE,
    );

    KernelPlan { selections }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_checkpoint_export::{
        import_checkpoint_export, write_synthetic_checkpoint_export,
    };
    use crate::lower_infer::lower_infer;

    #[test]
    fn kernel_plan_covers_every_program_op() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 17).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        let program = lower_infer(&imported.core, &imported.topology).expect("lowers");
        let plan = select_kernels(&program);
        let op_names = program.op_names();
        assert_eq!(plan.selections.len(), op_names.len());
        for op in &op_names {
            assert!(plan.kernel_for(op).is_some(), "no kernel selected for {op}");
        }
        assert_eq!(
            plan.kernel_for("block2.up.matvec"),
            Some(KernelFamily::V3WeightsAsCodeBanked)
        );
        assert_eq!(
            plan.kernel_for("head.tied_matvec"),
            Some(KernelFamily::TiedHeadLaneMajorI8ProductLut)
        );
    }
}
