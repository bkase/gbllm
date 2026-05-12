//! S2 training loss composer and inert-loss classification.
//!
//! This module consumes already-computed raw diagnostics. It does not invoke
//! raw loss helpers and it keeps unit selection explicit at the composition
//! boundary.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::phase::TrainPhaseKind;

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{BurnBackend, BurnFloatTensor, float_tensor_into_vec};

const LM_LOSS_NEXT_BYTE_NATS: &str = "lm_loss_next_byte_nats";
const DISTILL: &str = "distill";
const BALANCE: &str = "balance";
const ZROUTER: &str = "zrouter";
const SWITCH: &str = "switch";
const RANGE: &str = "range";
const ZERO: &str = "zero";
const SHAPE: &str = "shape";
const OVERFLOW: &str = "overflow";

/// Unit of the language-model and auxiliary losses entering S2 composition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingLossUnit {
    /// Natural-log nats. This is the only supported S2 closure unit.
    Nats,
    /// Test/config-parse sentinel used only to prove unsupported units fail
    /// closed. S2 public configs and artifacts should continue to serialize
    /// only `Nats` unless a pass-versioned contract adds another real unit.
    #[doc(hidden)]
    Unsupported(String),
}

impl TrainingLossUnit {
    /// Construct a rejected unit label for boundary tests and config parsing.
    #[must_use]
    pub fn unsupported(label: impl Into<String>) -> Self {
        Self::Unsupported(label.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Nats => "nats",
            Self::Unsupported(label) => label.as_str(),
        }
    }
}

/// Raw per-term losses consumed by the composer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossTerms {
    pub lm_loss_next_byte_nats: f32,
    pub distill_loss_raw_nats: Option<f32>,
    pub balance_loss_raw: Option<f32>,
    pub zrouter_loss_raw: Option<f32>,
    pub switch_loss_raw: Option<f32>,
    pub range_loss_raw: Option<f32>,
    pub zero_loss_raw: Option<f32>,
    pub shape_loss_raw: Option<f32>,
    pub overflow_loss_raw: Option<f32>,
}

/// Phase-effective lambda values for every term the composer can consume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseEffectiveLossWeights {
    pub lambda_distill: f32,
    pub lambda_balance: f32,
    pub lambda_zrouter: f32,
    pub lambda_switch: f32,
    pub lambda_range: f32,
    pub lambda_zero: f32,
    pub lambda_shape: f32,
    pub lambda_overflow: f32,
}

impl PhaseEffectiveLossWeights {
    pub fn new(values: PhaseEffectiveLossWeightsValues) -> Result<Self, ComposeError> {
        let weights = Self {
            lambda_distill: values.lambda_distill,
            lambda_balance: values.lambda_balance,
            lambda_zrouter: values.lambda_zrouter,
            lambda_switch: values.lambda_switch,
            lambda_range: values.lambda_range,
            lambda_zero: values.lambda_zero,
            lambda_shape: values.lambda_shape,
            lambda_overflow: values.lambda_overflow,
        };
        weights.validate()?;
        Ok(weights)
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self {
            lambda_distill: 0.0,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range: 0.0,
            lambda_zero: 0.0,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        }
    }

    pub fn validate(self) -> Result<(), ComposeError> {
        validate_lambda("lambda_distill", self.lambda_distill)?;
        validate_lambda("lambda_balance", self.lambda_balance)?;
        validate_lambda("lambda_zrouter", self.lambda_zrouter)?;
        validate_lambda("lambda_switch", self.lambda_switch)?;
        validate_lambda("lambda_range", self.lambda_range)?;
        validate_lambda("lambda_zero", self.lambda_zero)?;
        validate_lambda("lambda_shape", self.lambda_shape)?;
        validate_lambda("lambda_overflow", self.lambda_overflow)
    }
}

/// Input bag for `PhaseEffectiveLossWeights::new`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseEffectiveLossWeightsValues {
    pub lambda_distill: f32,
    pub lambda_balance: f32,
    pub lambda_zrouter: f32,
    pub lambda_switch: f32,
    pub lambda_range: f32,
    pub lambda_zero: f32,
    pub lambda_shape: f32,
    pub lambda_overflow: f32,
}

/// Whether each loss term applies at the current phase/topology boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossTermApplicability {
    pub distill: bool,
    pub balance: bool,
    pub zrouter: bool,
    pub switch: bool,
    pub range: bool,
    pub zero: bool,
    pub shape: bool,
    pub overflow: bool,
}

impl LossTermApplicability {
    #[must_use]
    pub const fn all_enabled() -> Self {
        Self {
            distill: true,
            balance: true,
            zrouter: true,
            switch: true,
            range: true,
            zero: true,
            shape: true,
            overflow: true,
        }
    }

    #[must_use]
    pub const fn toy0_phase_cd() -> Self {
        Self {
            distill: true,
            balance: false,
            zrouter: false,
            switch: false,
            range: true,
            zero: true,
            shape: false,
            overflow: false,
        }
    }

    #[must_use]
    pub const fn toy0_phase_a_without_distill_call() -> Self {
        Self {
            distill: false,
            balance: false,
            zrouter: false,
            switch: false,
            range: false,
            zero: false,
            shape: false,
            overflow: false,
        }
    }

    /// S2/Toy0-only applicability table for phases reached by the S2 run.
    ///
    /// `HardenAndSelect` belongs to the canonical five-phase scheduler but is
    /// not executable in S2's A-D Toy0 run, so this helper returns `None`
    /// instead of silently claiming an S2 loss composition contract for it.
    #[must_use]
    pub const fn s2_toy0_for_train_phase_kind(phase: TrainPhaseKind) -> Option<Self> {
        match phase {
            TrainPhaseKind::DenseTeacherWarmup | TrainPhaseKind::RouterWarmup => {
                Some(Self::toy0_phase_a_without_distill_call())
            }
            TrainPhaseKind::ExpertTernaryQat => Some(Self {
                distill: true,
                balance: false,
                zrouter: false,
                switch: false,
                range: false,
                zero: true,
                shape: false,
                overflow: false,
            }),
            TrainPhaseKind::FullNumericQat => Some(Self::toy0_phase_cd()),
            TrainPhaseKind::HardenAndSelect => None,
        }
    }
}

/// Per-term inert-loss classification.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum InertClassification {
    ComputedDisabled { raw: f32, weighted: f32 },
    StructurallyInert,
    Enabled { raw: f32, weighted: f32 },
}

impl InertClassification {
    #[must_use]
    pub const fn kind(self) -> InertClassificationKind {
        match self {
            Self::ComputedDisabled { .. } => InertClassificationKind::ComputedDisabled,
            Self::StructurallyInert => InertClassificationKind::StructurallyInert,
            Self::Enabled { .. } => InertClassificationKind::Enabled,
        }
    }

    #[must_use]
    pub const fn raw(self) -> Option<f32> {
        match self {
            Self::ComputedDisabled { raw, .. } | Self::Enabled { raw, .. } => Some(raw),
            Self::StructurallyInert => None,
        }
    }

    #[must_use]
    pub const fn weighted(self) -> Option<f32> {
        match self {
            Self::ComputedDisabled { weighted, .. } | Self::Enabled { weighted, .. } => {
                Some(weighted)
            }
            Self::StructurallyInert => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InertClassificationKind {
    ComputedDisabled,
    StructurallyInert,
    Enabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossTermEvalPoint {
    pub class: InertClassificationKind,
    pub raw: Option<f32>,
    pub weighted: Option<f32>,
}

impl From<InertClassification> for LossTermEvalPoint {
    fn from(classification: InertClassification) -> Self {
        Self {
            class: classification.kind(),
            raw: classification.raw(),
            weighted: classification.weighted(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InertClassificationPerTerm {
    pub distill: InertClassification,
    pub balance: InertClassification,
    pub zrouter: InertClassification,
    pub switch: InertClassification,
    pub range: InertClassification,
    pub zero: InertClassification,
    pub shape: InertClassification,
    pub overflow: InertClassification,
}

impl InertClassificationPerTerm {
    #[must_use]
    pub fn eval_points(self) -> LossTermEvalPoints {
        LossTermEvalPoints {
            distill: self.distill.into(),
            balance: self.balance.into(),
            zrouter: self.zrouter.into(),
            switch: self.switch.into(),
            range: self.range.into(),
            zero: self.zero.into(),
            shape: self.shape.into(),
            overflow: self.overflow.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossTermEvalPoints {
    pub distill: LossTermEvalPoint,
    pub balance: LossTermEvalPoint,
    pub zrouter: LossTermEvalPoint,
    pub switch: LossTermEvalPoint,
    pub range: LossTermEvalPoint,
    pub zero: LossTermEvalPoint,
    pub shape: LossTermEvalPoint,
    pub overflow: LossTermEvalPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerTermWeightedLosses {
    pub distill: Option<f32>,
    pub balance: Option<f32>,
    pub zrouter: Option<f32>,
    pub switch: Option<f32>,
    pub range: Option<f32>,
    pub zero: Option<f32>,
    pub shape: Option<f32>,
    pub overflow: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedLoss {
    pub total_loss: f32,
    pub weighted: PerTermWeightedLosses,
    pub inert_classification: InertClassificationPerTerm,
}

#[cfg(feature = "burn-adapter")]
pub struct BurnLossTerms<B: BurnBackend> {
    pub lm_loss_next_byte_nats: BurnFloatTensor<B, 1>,
    pub distill_loss_raw_nats: Option<BurnFloatTensor<B, 1>>,
    pub balance_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub zrouter_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub switch_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub range_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub zero_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub shape_loss_raw: Option<BurnFloatTensor<B, 1>>,
    pub overflow_loss_raw: Option<BurnFloatTensor<B, 1>>,
}

#[cfg(feature = "burn-adapter")]
pub struct BurnPerTermWeightedLosses<B: BurnBackend> {
    pub distill: Option<BurnFloatTensor<B, 1>>,
    pub balance: Option<BurnFloatTensor<B, 1>>,
    pub zrouter: Option<BurnFloatTensor<B, 1>>,
    pub switch: Option<BurnFloatTensor<B, 1>>,
    pub range: Option<BurnFloatTensor<B, 1>>,
    pub zero: Option<BurnFloatTensor<B, 1>>,
    pub shape: Option<BurnFloatTensor<B, 1>>,
    pub overflow: Option<BurnFloatTensor<B, 1>>,
}

#[cfg(feature = "burn-adapter")]
pub struct BurnComposedLoss<B: BurnBackend> {
    pub total_loss: BurnFloatTensor<B, 1>,
    pub scalar: ComposedLoss,
    pub weighted: BurnPerTermWeightedLosses<B>,
}

pub fn compose(
    terms: LossTerms,
    lambdas: PhaseEffectiveLossWeights,
    applicability: LossTermApplicability,
    unit: TrainingLossUnit,
) -> Result<ComposedLoss, ComposeError> {
    if unit != TrainingLossUnit::Nats {
        tracing::error!(event_name = "loss_compose_unit_error", got = unit.as_str());
        return Err(ComposeError::UnitMismatch {
            got: unit.as_str().to_owned(),
        });
    }

    validate_loss(LM_LOSS_NEXT_BYTE_NATS, terms.lm_loss_next_byte_nats)?;
    lambdas.validate()?;

    let inert_classification = InertClassificationPerTerm {
        distill: classify(
            DISTILL,
            terms.distill_loss_raw_nats,
            lambdas.lambda_distill,
            applicability.distill,
        )?,
        balance: classify(
            BALANCE,
            terms.balance_loss_raw,
            lambdas.lambda_balance,
            applicability.balance,
        )?,
        zrouter: classify(
            ZROUTER,
            terms.zrouter_loss_raw,
            lambdas.lambda_zrouter,
            applicability.zrouter,
        )?,
        switch: classify(
            SWITCH,
            terms.switch_loss_raw,
            lambdas.lambda_switch,
            applicability.switch,
        )?,
        range: classify(
            RANGE,
            terms.range_loss_raw,
            lambdas.lambda_range,
            applicability.range,
        )?,
        zero: classify(
            ZERO,
            terms.zero_loss_raw,
            lambdas.lambda_zero,
            applicability.zero,
        )?,
        shape: classify(
            SHAPE,
            terms.shape_loss_raw,
            lambdas.lambda_shape,
            applicability.shape,
        )?,
        overflow: classify(
            OVERFLOW,
            terms.overflow_loss_raw,
            lambdas.lambda_overflow,
            applicability.overflow,
        )?,
    };

    let weighted = PerTermWeightedLosses {
        distill: inert_classification.distill.weighted(),
        balance: inert_classification.balance.weighted(),
        zrouter: inert_classification.zrouter.weighted(),
        switch: inert_classification.switch.weighted(),
        range: inert_classification.range.weighted(),
        zero: inert_classification.zero.weighted(),
        shape: inert_classification.shape.weighted(),
        overflow: inert_classification.overflow.weighted(),
    };

    let total = normalize_total_loss(
        f64::from(terms.lm_loss_next_byte_nats)
            + weighted.distill.map(f64::from).unwrap_or(0.0)
            + weighted.balance.map(f64::from).unwrap_or(0.0)
            + weighted.zrouter.map(f64::from).unwrap_or(0.0)
            + weighted.switch.map(f64::from).unwrap_or(0.0)
            + weighted.range.map(f64::from).unwrap_or(0.0)
            + weighted.zero.map(f64::from).unwrap_or(0.0)
            + weighted.shape.map(f64::from).unwrap_or(0.0)
            + weighted.overflow.map(f64::from).unwrap_or(0.0),
    )?;

    Ok(ComposedLoss {
        total_loss: total,
        weighted,
        inert_classification,
    })
}

#[cfg(feature = "burn-adapter")]
pub fn burn_compose<B>(
    terms: BurnLossTerms<B>,
    lambdas: PhaseEffectiveLossWeights,
    applicability: LossTermApplicability,
    unit: TrainingLossUnit,
) -> Result<BurnComposedLoss<B>, ComposeError>
where
    B: BurnBackend,
{
    let scalar_terms = LossTerms {
        lm_loss_next_byte_nats: scalar_tensor_value(
            LM_LOSS_NEXT_BYTE_NATS,
            terms.lm_loss_next_byte_nats.clone(),
        )?,
        distill_loss_raw_nats: optional_scalar_tensor_value(
            DISTILL,
            terms.distill_loss_raw_nats.clone(),
        )?,
        balance_loss_raw: optional_scalar_tensor_value(BALANCE, terms.balance_loss_raw.clone())?,
        zrouter_loss_raw: optional_scalar_tensor_value(ZROUTER, terms.zrouter_loss_raw.clone())?,
        switch_loss_raw: optional_scalar_tensor_value(SWITCH, terms.switch_loss_raw.clone())?,
        range_loss_raw: optional_scalar_tensor_value(RANGE, terms.range_loss_raw.clone())?,
        zero_loss_raw: optional_scalar_tensor_value(ZERO, terms.zero_loss_raw.clone())?,
        shape_loss_raw: optional_scalar_tensor_value(SHAPE, terms.shape_loss_raw.clone())?,
        overflow_loss_raw: optional_scalar_tensor_value(OVERFLOW, terms.overflow_loss_raw.clone())?,
    };
    let scalar = compose(scalar_terms, lambdas, applicability, unit)?;

    let weighted = BurnPerTermWeightedLosses {
        distill: burn_classify_weighted(
            terms.distill_loss_raw_nats,
            lambdas.lambda_distill,
            scalar.inert_classification.distill,
        ),
        balance: burn_classify_weighted(
            terms.balance_loss_raw,
            lambdas.lambda_balance,
            scalar.inert_classification.balance,
        ),
        zrouter: burn_classify_weighted(
            terms.zrouter_loss_raw,
            lambdas.lambda_zrouter,
            scalar.inert_classification.zrouter,
        ),
        switch: burn_classify_weighted(
            terms.switch_loss_raw,
            lambdas.lambda_switch,
            scalar.inert_classification.switch,
        ),
        range: burn_classify_weighted(
            terms.range_loss_raw,
            lambdas.lambda_range,
            scalar.inert_classification.range,
        ),
        zero: burn_classify_weighted(
            terms.zero_loss_raw,
            lambdas.lambda_zero,
            scalar.inert_classification.zero,
        ),
        shape: burn_classify_weighted(
            terms.shape_loss_raw,
            lambdas.lambda_shape,
            scalar.inert_classification.shape,
        ),
        overflow: burn_classify_weighted(
            terms.overflow_loss_raw,
            lambdas.lambda_overflow,
            scalar.inert_classification.overflow,
        ),
    };

    let total_loss = add_optional(
        add_optional(
            add_optional(
                add_optional(
                    add_optional(
                        add_optional(
                            add_optional(
                                add_optional(terms.lm_loss_next_byte_nats, &weighted.distill),
                                &weighted.balance,
                            ),
                            &weighted.zrouter,
                        ),
                        &weighted.switch,
                    ),
                    &weighted.range,
                ),
                &weighted.zero,
            ),
            &weighted.shape,
        ),
        &weighted.overflow,
    );

    Ok(BurnComposedLoss {
        total_loss,
        scalar,
        weighted,
    })
}

pub fn classify(
    term: &'static str,
    raw: Option<f32>,
    lambda: f32,
    applies: bool,
) -> Result<InertClassification, ComposeError> {
    validate_lambda(lambda_name(term), lambda)?;

    if !applies {
        if lambda != 0.0 {
            tracing::warn!(
                event_name = "loss_structurally_inert_nonzero_lambda",
                term,
                lambda,
            );
        }
        return Ok(InertClassification::StructurallyInert);
    }

    let raw = raw.ok_or(ComposeError::MissingRawLoss { term })?;
    validate_loss(raw_name(term), raw)?;

    if lambda == 0.0 {
        return Ok(InertClassification::ComputedDisabled { raw, weighted: 0.0 });
    }

    let weighted = normalize_total_loss(f64::from(raw) * f64::from(lambda))?;
    Ok(InertClassification::Enabled { raw, weighted })
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComposeError {
    UnitMismatch {
        got: String,
    },
    NonFiniteLossWeight {
        name: &'static str,
        value: f32,
    },
    NegativeLossWeight {
        name: &'static str,
        value: f32,
    },
    MissingRawLoss {
        term: &'static str,
    },
    NonFiniteLoss {
        name: &'static str,
        value: f32,
    },
    NegativeLoss {
        name: &'static str,
        value: f32,
    },
    TotalLossOverflow {
        value: f64,
    },
    #[cfg(feature = "burn-adapter")]
    ScalarTensorShape {
        name: &'static str,
        len: usize,
    },
    #[cfg(feature = "burn-adapter")]
    TensorRead {
        name: &'static str,
    },
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnitMismatch { got } => {
                write!(f, "S2 training loss unit must be nats, got {got}")
            }
            Self::NonFiniteLossWeight { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            Self::NegativeLossWeight { name, value } => {
                write!(f, "{name} must be non-negative, got {value}")
            }
            Self::MissingRawLoss { term } => {
                write!(f, "{term} raw loss is required when the term applies")
            }
            Self::NonFiniteLoss { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            Self::NegativeLoss { name, value } => {
                write!(f, "{name} must be non-negative, got {value}")
            }
            Self::TotalLossOverflow { value } => {
                write!(f, "composed total loss must fit in f32, got {value}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::ScalarTensorShape { name, len } => {
                write!(
                    f,
                    "{name} must be a scalar tensor with one element, got {len}"
                )
            }
            #[cfg(feature = "burn-adapter")]
            Self::TensorRead { name } => {
                write!(f, "{name} tensor value could not be read for validation")
            }
        }
    }
}

impl Error for ComposeError {}

fn validate_lambda(name: &'static str, value: f32) -> Result<(), ComposeError> {
    if !value.is_finite() {
        return Err(ComposeError::NonFiniteLossWeight { name, value });
    }
    if value < 0.0 {
        return Err(ComposeError::NegativeLossWeight { name, value });
    }
    Ok(())
}

fn validate_loss(name: &'static str, value: f32) -> Result<(), ComposeError> {
    if !value.is_finite() {
        return Err(ComposeError::NonFiniteLoss { name, value });
    }
    if value < 0.0 {
        return Err(ComposeError::NegativeLoss { name, value });
    }
    Ok(())
}

fn normalize_total_loss(value: f64) -> Result<f32, ComposeError> {
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(ComposeError::TotalLossOverflow { value });
    }
    Ok(value as f32)
}

fn lambda_name(term: &'static str) -> &'static str {
    match term {
        DISTILL => "lambda_distill",
        BALANCE => "lambda_balance",
        ZROUTER => "lambda_zrouter",
        SWITCH => "lambda_switch",
        RANGE => "lambda_range",
        ZERO => "lambda_zero",
        SHAPE => "lambda_shape",
        OVERFLOW => "lambda_overflow",
        _ => "lambda_unknown",
    }
}

fn raw_name(term: &'static str) -> &'static str {
    match term {
        DISTILL => "distill_loss_raw_nats",
        BALANCE => "balance_loss_raw",
        ZROUTER => "zrouter_loss_raw",
        SWITCH => "switch_loss_raw",
        RANGE => "range_loss_raw",
        ZERO => "zero_loss_raw",
        SHAPE => "shape_loss_raw",
        OVERFLOW => "overflow_loss_raw",
        _ => "unknown_loss_raw",
    }
}

#[cfg(feature = "burn-adapter")]
fn burn_classify_weighted<B>(
    raw: Option<BurnFloatTensor<B, 1>>,
    lambda: f32,
    classification: InertClassification,
) -> Option<BurnFloatTensor<B, 1>>
where
    B: BurnBackend,
{
    match classification {
        InertClassification::ComputedDisabled { .. } => raw.map(|loss| loss * 0.0),
        InertClassification::StructurallyInert => None,
        InertClassification::Enabled { .. } => raw.map(|loss| loss * lambda),
    }
}

#[cfg(feature = "burn-adapter")]
fn add_optional<B>(
    total: BurnFloatTensor<B, 1>,
    term: &Option<BurnFloatTensor<B, 1>>,
) -> BurnFloatTensor<B, 1>
where
    B: BurnBackend,
{
    match term {
        Some(term) => total + term.clone(),
        None => total,
    }
}

#[cfg(feature = "burn-adapter")]
fn optional_scalar_tensor_value<B>(
    term: &'static str,
    tensor: Option<BurnFloatTensor<B, 1>>,
) -> Result<Option<f32>, ComposeError>
where
    B: BurnBackend,
{
    tensor
        .map(|tensor| scalar_tensor_value(raw_name(term), tensor))
        .transpose()
}

#[cfg(feature = "burn-adapter")]
fn scalar_tensor_value<B>(
    name: &'static str,
    tensor: BurnFloatTensor<B, 1>,
) -> Result<f32, ComposeError>
where
    B: BurnBackend,
{
    let shape = tensor.shape();
    let len = shape.num_elements();
    if len != 1 {
        return Err(ComposeError::ScalarTensorShape { name, len });
    }
    let values = float_tensor_into_vec(tensor).map_err(|_| ComposeError::TensorRead { name })?;
    values
        .first()
        .copied()
        .ok_or(ComposeError::ScalarTensorShape { name, len: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_sums_lm_and_weighted_auxiliary_nats() {
        let terms = LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(2.0),
            ..LossTerms::default()
        };
        let lambdas = PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
            lambda_distill: 1.0,
            ..zero_values()
        })
        .unwrap();

        let composed = compose(
            terms,
            lambdas,
            LossTermApplicability {
                distill: true,
                ..LossTermApplicability::toy0_phase_a_without_distill_call()
            },
            TrainingLossUnit::Nats,
        )
        .unwrap();

        assert_eq!(composed.total_loss, 2.5);
        assert_eq!(
            composed.inert_classification.distill,
            InertClassification::Enabled {
                raw: 2.0,
                weighted: 2.0
            }
        );
    }

    #[test]
    fn phase_a_omitted_distill_call_is_structurally_inert() {
        let composed = compose(
            LossTerms {
                lm_loss_next_byte_nats: 0.5,
                ..LossTerms::default()
            },
            PhaseEffectiveLossWeights::zero(),
            LossTermApplicability::toy0_phase_a_without_distill_call(),
            TrainingLossUnit::Nats,
        )
        .unwrap();

        assert_eq!(composed.total_loss, 0.5);
        assert_eq!(
            composed.inert_classification.distill,
            InertClassification::StructurallyInert
        );
        assert_eq!(composed.weighted.distill, None);
    }

    #[test]
    fn nodistill_phase_c_records_computed_disabled_raw_distill() {
        let composed = compose(
            LossTerms {
                lm_loss_next_byte_nats: 0.5,
                distill_loss_raw_nats: Some(2.5),
                range_loss_raw: Some(0.0),
                zero_loss_raw: Some(0.0),
                ..LossTerms::default()
            },
            PhaseEffectiveLossWeights::zero(),
            LossTermApplicability::toy0_phase_cd(),
            TrainingLossUnit::Nats,
        )
        .unwrap();

        assert_eq!(composed.total_loss, 0.5);
        assert_eq!(
            composed.inert_classification.distill,
            InertClassification::ComputedDisabled {
                raw: 2.5,
                weighted: 0.0
            }
        );
    }

    #[test]
    fn unsupported_units_fail_closed() {
        assert_eq!(
            compose(
                LossTerms {
                    lm_loss_next_byte_nats: 0.5,
                    ..LossTerms::default()
                },
                PhaseEffectiveLossWeights::zero(),
                LossTermApplicability::toy0_phase_a_without_distill_call(),
                TrainingLossUnit::unsupported("bits")
            )
            .unwrap_err(),
            ComposeError::UnitMismatch {
                got: "bits".to_owned()
            }
        );
    }

    #[test]
    fn missing_enabled_raw_loss_is_an_error() {
        let lambdas = PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
            lambda_range: 0.01,
            ..zero_values()
        })
        .unwrap();

        assert_eq!(
            compose(
                LossTerms {
                    lm_loss_next_byte_nats: 0.5,
                    distill_loss_raw_nats: Some(0.0),
                    zero_loss_raw: Some(0.0),
                    ..LossTerms::default()
                },
                lambdas,
                LossTermApplicability::toy0_phase_cd(),
                TrainingLossUnit::Nats
            )
            .unwrap_err(),
            ComposeError::MissingRawLoss { term: RANGE }
        );
    }

    #[test]
    fn s2_toy0_loss_applicability_is_canonical_by_train_phase_kind() {
        let cases = [
            (
                TrainPhaseKind::DenseTeacherWarmup,
                Some(LossTermApplicability::toy0_phase_a_without_distill_call()),
            ),
            (
                TrainPhaseKind::RouterWarmup,
                Some(LossTermApplicability::toy0_phase_a_without_distill_call()),
            ),
            (
                TrainPhaseKind::ExpertTernaryQat,
                Some(LossTermApplicability {
                    distill: true,
                    zero: true,
                    ..LossTermApplicability::toy0_phase_a_without_distill_call()
                }),
            ),
            (
                TrainPhaseKind::FullNumericQat,
                Some(LossTermApplicability::toy0_phase_cd()),
            ),
            (TrainPhaseKind::HardenAndSelect, None),
        ];

        for (phase, expected) in cases {
            let applicability = LossTermApplicability::s2_toy0_for_train_phase_kind(phase);
            assert_eq!(applicability, expected, "{phase:?}");
            let Some(applicability) = applicability else {
                assert_eq!(phase, TrainPhaseKind::HardenAndSelect);
                continue;
            };
            assert!(!applicability.balance, "{phase:?} keeps balance inert");
            assert!(!applicability.zrouter, "{phase:?} keeps zrouter inert");
            assert!(!applicability.switch, "{phase:?} keeps switch inert");
            assert!(!applicability.shape, "{phase:?} keeps shape inert");
            assert!(!applicability.overflow, "{phase:?} keeps overflow inert");
        }
    }

    fn zero_values() -> PhaseEffectiveLossWeightsValues {
        PhaseEffectiveLossWeightsValues {
            lambda_distill: 0.0,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range: 0.0,
            lambda_zero: 0.0,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        }
    }
}
