//! S2 distillation executable fixture helpers.

use gbf_foundation::{Hash256, sha256};
use gbf_train::loss::distillation::{
    DEFAULT_DISTILLATION_TEMPERATURE, DistillInputs, DistillProduct, DistillationLossError,
    distillation_product,
};

/// Pinned student logits consumed by the `gbf s2 distill-once` smoke path.
pub const PINNED_STUDENT_LOGITS: [f32; 4] = [0.125, -0.25, 0.5, -0.75];

/// Pinned dense-teacher logits consumed by the `gbf s2 distill-once` smoke path.
pub const PINNED_TEACHER_LOGITS: [f32; 4] = [0.0, -0.125, 0.625, -0.875];

/// Number of classes in the pinned distillation fixture.
pub const PINNED_CLASS_COUNT: usize = 4;

/// Pinned `lambda_distill` used to produce both raw and weighted diagnostics.
pub const PINNED_LAMBDA_DISTILL: f32 = 1.0;

/// Output from one executable S2 distillation smoke step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistillOnceOutput {
    /// Raw KL distillation loss in nats.
    pub distill_loss_raw: f32,
    /// Raw KL bits, used as the bytewise determinism evidence.
    pub distill_loss_raw_bits: u32,
    /// SHA-256 of the raw KL bits in big-endian order.
    pub distill_loss_raw_sha: Hash256,
    /// Pre-clamp KL diagnostic retained from the distillation helper.
    pub pre_clamp_kl_loss: Option<f32>,
    /// Weighted distillation loss after applying `lambda_distill`.
    pub distill_loss_weighted: f32,
    /// Distillation temperature used by the fixture.
    pub temperature: f32,
    /// Number of classes in each logit row.
    pub class_count: usize,
    /// Number of logit rows reduced by the helper.
    pub row_count: usize,
}

/// Run the pinned, executable S2 distillation step.
pub fn distill_once_pinned() -> Result<DistillOnceOutput, DistillationLossError> {
    let product = distillation_product(DistillInputs {
        student_logits: &PINNED_STUDENT_LOGITS,
        teacher_logits: &PINNED_TEACHER_LOGITS,
        class_count: PINNED_CLASS_COUNT,
        temperature: DEFAULT_DISTILLATION_TEMPERATURE,
        lambda_distill: PINNED_LAMBDA_DISTILL,
    })?;
    Ok(output_for_product(product))
}

fn output_for_product(product: DistillProduct) -> DistillOnceOutput {
    let bits = product.distill_loss_raw.to_bits();
    let sha = sha256(bits.to_be_bytes());
    DistillOnceOutput {
        distill_loss_raw: product.distill_loss_raw,
        distill_loss_raw_bits: bits,
        distill_loss_raw_sha: sha,
        pre_clamp_kl_loss: product.pre_clamp_kl_loss,
        distill_loss_weighted: product.distill_loss_weighted,
        temperature: DEFAULT_DISTILLATION_TEMPERATURE,
        class_count: PINNED_CLASS_COUNT,
        row_count: PINNED_STUDENT_LOGITS.len() / PINNED_CLASS_COUNT,
    }
}
