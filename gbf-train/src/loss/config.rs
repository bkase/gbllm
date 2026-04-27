//! Training loss configuration and composition helpers.
//!
//! This module owns the `[training.loss]` TOML contract and the diagnostic
//! arithmetic that turns raw per-term losses into a weighted scalar for logs.
//! The individual loss modules keep their raw loss formulas, and the future
//! Burn training-loop composer must build the differentiable tensor loss.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::logging::{LoggingEventError, LossBreakdown, TrainingLogEmitter};
use crate::phase::TrainPhaseKind;

use super::distillation::lambda_distill_for_phase;
use super::router::{lambda_balance_for_phase, lambda_zrouter_for_phase};

const LAMBDA_DISTILL: &str = "lambda_distill";
const LAMBDA_BALANCE: &str = "lambda_balance";
const LAMBDA_ZROUTER: &str = "lambda_zrouter";
const LAMBDA_SWITCH: &str = "lambda_switch";
const LAMBDA_RANGE: &str = "lambda_range";
const LAMBDA_ZERO: &str = "lambda_zero";
const LAMBDA_SHAPE: &str = "lambda_shape";
const LAMBDA_OVERFLOW: &str = "lambda_overflow";

const LM_LOSS: &str = "lm_loss";
const DISTILL_LOSS: &str = "distill_loss";
const BALANCE_LOSS: &str = "balance_loss";
const ZROUTER_LOSS: &str = "zrouter_loss";
const SWITCH_LOSS: &str = "switch_loss";
const RANGE_LOSS: &str = "range_loss";
const ZERO_LOSS: &str = "zero_loss";
const SHAPE_LOSS: &str = "shape_loss";
const OVERFLOW_LOSS: &str = "overflow_loss";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LossConfig {
    lambda_distill: f32,
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_switch: f32,
    lambda_range: f32,
    lambda_zero: f32,
    lambda_shape: f32,
    lambda_overflow: f32,
}

impl LossConfig {
    pub fn from_toml_str(input: &str) -> Result<Self, LossConfigError> {
        let value: toml::Value = toml::from_str(input).map_err(LossConfigError::Toml)?;
        let loss_value = if let Some(training) = value.get("training") {
            match training.get("loss") {
                Some(loss) => loss.clone(),
                None => return Ok(Self::default()),
            }
        } else {
            value
        };

        let raw: RawLossConfig = loss_value.try_into().map_err(LossConfigError::Toml)?;
        Self::from_raw(raw)
    }

    pub fn validate(&self) -> Result<(), LossConfigError> {
        validate_lambda(LAMBDA_DISTILL, self.lambda_distill)?;
        validate_lambda(LAMBDA_BALANCE, self.lambda_balance)?;
        validate_lambda(LAMBDA_ZROUTER, self.lambda_zrouter)?;
        validate_lambda(LAMBDA_SWITCH, self.lambda_switch)?;
        validate_lambda(LAMBDA_RANGE, self.lambda_range)?;
        validate_lambda(LAMBDA_ZERO, self.lambda_zero)?;
        validate_lambda(LAMBDA_SHAPE, self.lambda_shape)?;
        validate_lambda(LAMBDA_OVERFLOW, self.lambda_overflow)
    }

    fn from_raw(raw: RawLossConfig) -> Result<Self, LossConfigError> {
        let config = Self {
            lambda_distill: raw.lambda_distill,
            lambda_balance: raw.lambda_balance,
            lambda_zrouter: raw.lambda_zrouter,
            lambda_switch: raw.lambda_switch,
            lambda_range: raw.lambda_range,
            lambda_zero: raw.lambda_zero,
            lambda_shape: raw.lambda_shape,
            lambda_overflow: raw.lambda_overflow,
        };
        config.validate()?;
        Ok(config)
    }

    #[must_use]
    pub fn for_phase(self, phase: TrainPhaseKind) -> PhaseLossConfig {
        PhaseLossConfig {
            phase,
            lambda_distill: lambda_distill_for_phase(phase, self.lambda_distill),
            lambda_balance: lambda_balance_for_phase(phase, self.lambda_balance),
            lambda_zrouter: lambda_zrouter_for_phase(phase, self.lambda_zrouter),
            lambda_switch: self.lambda_switch,
            lambda_range: self.lambda_range,
            lambda_zero: self.lambda_zero,
            lambda_shape: self.lambda_shape,
            lambda_overflow: self.lambda_overflow,
        }
    }

    #[must_use]
    pub const fn lambda_distill(self) -> f32 {
        self.lambda_distill
    }

    #[must_use]
    pub const fn lambda_balance(self) -> f32 {
        self.lambda_balance
    }

    #[must_use]
    pub const fn lambda_zrouter(self) -> f32 {
        self.lambda_zrouter
    }

    #[must_use]
    pub const fn lambda_switch(self) -> f32 {
        self.lambda_switch
    }

    #[must_use]
    pub const fn lambda_range(self) -> f32 {
        self.lambda_range
    }

    #[must_use]
    pub const fn lambda_zero(self) -> f32 {
        self.lambda_zero
    }

    #[must_use]
    pub const fn lambda_shape(self) -> f32 {
        self.lambda_shape
    }

    #[must_use]
    pub const fn lambda_overflow(self) -> f32 {
        self.lambda_overflow
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseLossConfig {
    phase: TrainPhaseKind,
    lambda_distill: f32,
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_switch: f32,
    lambda_range: f32,
    lambda_zero: f32,
    lambda_shape: f32,
    lambda_overflow: f32,
}

impl PhaseLossConfig {
    pub fn validate(&self) -> Result<(), LossConfigError> {
        validate_lambda(LAMBDA_DISTILL, self.lambda_distill)?;
        validate_lambda(LAMBDA_BALANCE, self.lambda_balance)?;
        validate_lambda(LAMBDA_ZROUTER, self.lambda_zrouter)?;
        validate_lambda(LAMBDA_SWITCH, self.lambda_switch)?;
        validate_lambda(LAMBDA_RANGE, self.lambda_range)?;
        validate_lambda(LAMBDA_ZERO, self.lambda_zero)?;
        validate_lambda(LAMBDA_SHAPE, self.lambda_shape)?;
        validate_lambda(LAMBDA_OVERFLOW, self.lambda_overflow)
    }

    pub fn weighted_diagnostic_total_loss(
        self,
        lm_loss: f32,
        raw_terms: LossTermValues,
    ) -> Result<f32, LossConfigError> {
        self.validate()?;
        raw_terms.validate()?;
        validate_loss_value(LM_LOSS, lm_loss)?;

        normalize_total_loss(
            f64::from(lm_loss)
                + weighted(raw_terms.distill_loss, self.lambda_distill)
                + weighted(raw_terms.balance_loss, self.lambda_balance)
                + weighted(raw_terms.zrouter_loss, self.lambda_zrouter)
                + weighted(raw_terms.switch_loss, self.lambda_switch)
                + weighted(raw_terms.range_loss, self.lambda_range)
                + weighted(raw_terms.zero_loss, self.lambda_zero)
                + weighted(raw_terms.shape_loss, self.lambda_shape)
                + weighted(raw_terms.overflow_loss, self.lambda_overflow),
        )
    }

    pub fn diagnostic_loss_breakdown(
        self,
        step: u64,
        lm_loss: f32,
        raw_terms: LossTermValues,
    ) -> Result<LossBreakdown, LossConfigError> {
        let total_loss = self.weighted_diagnostic_total_loss(lm_loss, raw_terms)?;

        Ok(LossBreakdown {
            step,
            lm_loss,
            distill_loss: raw_terms.distill_loss,
            balance_loss: raw_terms.balance_loss,
            zrouter_loss: raw_terms.zrouter_loss,
            switch_loss: raw_terms.switch_loss,
            range_loss: raw_terms.range_loss,
            zero_loss: raw_terms.zero_loss,
            shape_loss: raw_terms.shape_loss,
            overflow_loss: raw_terms.overflow_loss,
            total_loss,
        })
    }

    pub fn log_diagnostic_loss_step(
        self,
        emitter: &TrainingLogEmitter,
        step: u64,
        lm_loss: f32,
        raw_terms: LossTermValues,
    ) -> Result<LossBreakdown, LossConfigError> {
        let breakdown = self.diagnostic_loss_breakdown(step, lm_loss, raw_terms)?;
        emitter.loss_step(&breakdown)?;
        Ok(breakdown)
    }

    #[must_use]
    pub const fn phase(self) -> TrainPhaseKind {
        self.phase
    }

    #[must_use]
    pub const fn lambda_distill(self) -> f32 {
        self.lambda_distill
    }

    #[must_use]
    pub const fn lambda_balance(self) -> f32 {
        self.lambda_balance
    }

    #[must_use]
    pub const fn lambda_zrouter(self) -> f32 {
        self.lambda_zrouter
    }

    #[must_use]
    pub const fn lambda_switch(self) -> f32 {
        self.lambda_switch
    }

    #[must_use]
    pub const fn lambda_range(self) -> f32 {
        self.lambda_range
    }

    #[must_use]
    pub const fn lambda_zero(self) -> f32 {
        self.lambda_zero
    }

    #[must_use]
    pub const fn lambda_shape(self) -> f32 {
        self.lambda_shape
    }

    #[must_use]
    pub const fn lambda_overflow(self) -> f32 {
        self.lambda_overflow
    }
}

impl Default for LossConfig {
    fn default() -> Self {
        Self {
            lambda_distill: 0.0,
            lambda_balance: 0.01,
            lambda_zrouter: 0.001,
            lambda_switch: 0.0,
            lambda_range: 0.0,
            lambda_zero: 0.0,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        }
    }
}

impl<'de> Deserialize<'de> for LossConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLossConfig::deserialize(deserializer)?;
        Self::from_raw(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LossTermValues {
    pub distill_loss: f32,
    pub balance_loss: f32,
    pub zrouter_loss: f32,
    pub switch_loss: f32,
    pub range_loss: f32,
    pub zero_loss: f32,
    pub shape_loss: f32,
    pub overflow_loss: f32,
}

impl LossTermValues {
    pub fn validate(self) -> Result<(), LossConfigError> {
        validate_loss_value(DISTILL_LOSS, self.distill_loss)?;
        validate_loss_value(BALANCE_LOSS, self.balance_loss)?;
        validate_loss_value(ZROUTER_LOSS, self.zrouter_loss)?;
        validate_loss_value(SWITCH_LOSS, self.switch_loss)?;
        validate_loss_value(RANGE_LOSS, self.range_loss)?;
        validate_loss_value(ZERO_LOSS, self.zero_loss)?;
        validate_loss_value(SHAPE_LOSS, self.shape_loss)?;
        validate_loss_value(OVERFLOW_LOSS, self.overflow_loss)
    }
}

#[derive(Debug)]
pub enum LossConfigError {
    Toml(toml::de::Error),
    NonFiniteLossWeight { name: &'static str, value: f32 },
    NegativeLossWeight { name: &'static str, value: f32 },
    NonFiniteLoss { name: &'static str, value: f32 },
    NegativeLoss { name: &'static str, value: f32 },
    TotalLossOverflow { value: f64 },
    Logging(LoggingEventError),
}

impl PartialEq for LossConfigError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Toml(left), Self::Toml(right)) => left.to_string() == right.to_string(),
            (
                Self::NonFiniteLossWeight {
                    name: left_name,
                    value: left_value,
                },
                Self::NonFiniteLossWeight {
                    name: right_name,
                    value: right_value,
                },
            )
            | (
                Self::NegativeLossWeight {
                    name: left_name,
                    value: left_value,
                },
                Self::NegativeLossWeight {
                    name: right_name,
                    value: right_value,
                },
            )
            | (
                Self::NonFiniteLoss {
                    name: left_name,
                    value: left_value,
                },
                Self::NonFiniteLoss {
                    name: right_name,
                    value: right_value,
                },
            )
            | (
                Self::NegativeLoss {
                    name: left_name,
                    value: left_value,
                },
                Self::NegativeLoss {
                    name: right_name,
                    value: right_value,
                },
            ) => left_name == right_name && float_error_value_eq(*left_value, *right_value),
            (
                Self::TotalLossOverflow { value: left_value },
                Self::TotalLossOverflow { value: right_value },
            ) => left_value == right_value,
            (Self::Logging(left), Self::Logging(right)) => left == right,
            _ => false,
        }
    }
}

impl fmt::Display for LossConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(error) => write!(f, "{error}"),
            Self::NonFiniteLossWeight { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            Self::NegativeLossWeight { name, value } => {
                write!(f, "{name} must be non-negative, got {value}")
            }
            Self::NonFiniteLoss { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            Self::NegativeLoss { name, value } => {
                write!(f, "{name} must be non-negative, got {value}")
            }
            Self::TotalLossOverflow { value } => {
                write!(f, "weighted total loss must fit in f32, got {value}")
            }
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for LossConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Toml(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::NonFiniteLossWeight { .. }
            | Self::NegativeLossWeight { .. }
            | Self::NonFiniteLoss { .. }
            | Self::NegativeLoss { .. }
            | Self::TotalLossOverflow { .. } => None,
        }
    }
}

impl From<LoggingEventError> for LossConfigError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawLossConfig {
    lambda_distill: f32,
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_switch: f32,
    lambda_range: f32,
    lambda_zero: f32,
    lambda_shape: f32,
    lambda_overflow: f32,
}

impl Default for RawLossConfig {
    fn default() -> Self {
        let defaults = LossConfig::default();
        Self {
            lambda_distill: defaults.lambda_distill,
            lambda_balance: defaults.lambda_balance,
            lambda_zrouter: defaults.lambda_zrouter,
            lambda_switch: defaults.lambda_switch,
            lambda_range: defaults.lambda_range,
            lambda_zero: defaults.lambda_zero,
            lambda_shape: defaults.lambda_shape,
            lambda_overflow: defaults.lambda_overflow,
        }
    }
}

fn validate_lambda(name: &'static str, value: f32) -> Result<(), LossConfigError> {
    if !value.is_finite() {
        return Err(LossConfigError::NonFiniteLossWeight { name, value });
    }

    if value < 0.0 {
        return Err(LossConfigError::NegativeLossWeight { name, value });
    }

    Ok(())
}

fn validate_loss_value(name: &'static str, value: f32) -> Result<(), LossConfigError> {
    if !value.is_finite() {
        return Err(LossConfigError::NonFiniteLoss { name, value });
    }

    if value < 0.0 {
        return Err(LossConfigError::NegativeLoss { name, value });
    }

    Ok(())
}

fn weighted(raw_loss: f32, lambda: f32) -> f64 {
    f64::from(raw_loss) * f64::from(lambda)
}

fn normalize_total_loss(value: f64) -> Result<f32, LossConfigError> {
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(LossConfigError::TotalLossOverflow { value });
    }

    Ok(value as f32)
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(test)]
mod tests {
    use crate::logging::{TestEventCollector, TestFieldValue};

    use super::*;

    #[test]
    fn loss_config_defaults_match_training_contract() {
        let config = LossConfig::default();

        assert_eq!(config.lambda_distill(), 0.0);
        assert_eq!(config.lambda_balance(), 0.01);
        assert_eq!(config.lambda_zrouter(), 0.001);
        assert_eq!(config.lambda_switch(), 0.0);
        assert_eq!(config.lambda_range(), 0.0);
        assert_eq!(config.lambda_zero(), 0.0);
        assert_eq!(config.lambda_shape(), 0.0);
        assert_eq!(config.lambda_overflow(), 0.0);
    }

    #[test]
    fn loss_config_parses_training_toml_and_round_trips() {
        let config = LossConfig::from_toml_str(
            r#"
            [training.loss]
            lambda_distill = 0.5
            lambda_balance = 0.25
            lambda_zrouter = 0.125
            lambda_switch = 0.0625
            lambda_range = 0.03125
            lambda_zero = 0.015625
            lambda_shape = 0.0078125
            lambda_overflow = 0.00390625
            "#,
        )
        .unwrap();

        assert_eq!(config.lambda_distill(), 0.5);
        assert_eq!(config.lambda_balance(), 0.25);
        assert_eq!(config.lambda_zrouter(), 0.125);
        assert_eq!(config.lambda_switch(), 0.0625);
        assert_eq!(config.lambda_range(), 0.03125);
        assert_eq!(config.lambda_zero(), 0.015625);
        assert_eq!(config.lambda_shape(), 0.0078125);
        assert_eq!(config.lambda_overflow(), 0.00390625);

        let encoded = toml::to_string(&config).unwrap();
        let decoded = LossConfig::from_toml_str(&encoded).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn loss_config_defaults_when_training_toml_omits_loss_section() {
        let config = LossConfig::from_toml_str(
            r#"
            [training]
            seed = 42
            "#,
        )
        .unwrap();

        assert_eq!(config, LossConfig::default());
    }

    #[test]
    fn loss_config_rejects_invalid_weights() {
        let negative = LossConfig::from_toml_str(
            r#"
            [training.loss]
            lambda_balance = -0.1
            "#,
        )
        .unwrap_err();

        assert_eq!(
            negative,
            LossConfigError::NegativeLossWeight {
                name: LAMBDA_BALANCE,
                value: -0.1,
            }
        );

        let non_finite = LossConfig::from_toml_str(
            r#"
            [training.loss]
            lambda_zero = inf
            "#,
        )
        .unwrap_err();

        assert_eq!(
            non_finite,
            LossConfigError::NonFiniteLossWeight {
                name: LAMBDA_ZERO,
                value: f32::INFINITY,
            }
        );

        let unknown = LossConfig::from_toml_str(
            r#"
            [training.loss]
            lambda_bank = 1.0
            "#,
        )
        .unwrap_err();

        assert!(matches!(unknown, LossConfigError::Toml(_)));
    }

    #[test]
    fn loss_config_phase_gates_implemented_phase_owned_terms() {
        let config = LossConfig::from_toml_str(
            r#"
            [training.loss]
            lambda_distill = 0.5
            lambda_balance = 0.25
            lambda_zrouter = 0.125
            lambda_switch = 0.0625
            "#,
        )
        .unwrap();

        let dense = config.for_phase(TrainPhaseKind::DenseTeacherWarmup);
        assert_eq!(dense.phase(), TrainPhaseKind::DenseTeacherWarmup);
        assert_eq!(dense.lambda_distill(), 0.0);
        assert_eq!(dense.lambda_balance(), 0.0);
        assert_eq!(dense.lambda_zrouter(), 0.0);
        assert_eq!(dense.lambda_switch(), 0.0625);

        let router = config.for_phase(TrainPhaseKind::RouterWarmup);
        assert_eq!(router.lambda_distill(), 0.0);
        assert_eq!(router.lambda_balance(), 0.25);
        assert_eq!(router.lambda_zrouter(), 0.125);

        let expert = config.for_phase(TrainPhaseKind::ExpertTernaryQat);
        assert_eq!(expert.lambda_distill(), 0.5);
        assert_eq!(expert.lambda_balance(), 0.25);
        assert_eq!(expert.lambda_zrouter(), 0.125);

        let harden = config.for_phase(TrainPhaseKind::HardenAndSelect);
        assert_eq!(harden.lambda_distill(), 0.0);
        assert_eq!(harden.lambda_balance(), 0.25);
        assert_eq!(harden.lambda_zrouter(), 0.125);
    }

    #[test]
    fn default_phase_b_onward_config_only_enables_router_terms() {
        let router = LossConfig::default().for_phase(TrainPhaseKind::RouterWarmup);

        assert_eq!(router.lambda_distill(), 0.0);
        assert_eq!(router.lambda_balance(), 0.01);
        assert_eq!(router.lambda_zrouter(), 0.001);
        assert_eq!(router.lambda_switch(), 0.0);
        assert_eq!(router.lambda_range(), 0.0);
        assert_eq!(router.lambda_zero(), 0.0);
        assert_eq!(router.lambda_shape(), 0.0);
        assert_eq!(router.lambda_overflow(), 0.0);
    }

    #[test]
    fn weighted_diagnostic_total_loss_uses_all_non_zero_lambdas() {
        let config = LossConfig::from_toml_str(
            r#"
            lambda_distill = 0.5
            lambda_balance = 0.25
            lambda_zrouter = 0.125
            lambda_switch = 0.0625
            lambda_range = 0.03125
            lambda_zero = 0.015625
            lambda_shape = 0.0078125
            lambda_overflow = 0.00390625
            "#,
        )
        .unwrap();
        let terms = nonzero_terms();

        assert_eq!(
            config
                .for_phase(TrainPhaseKind::ExpertTernaryQat)
                .weighted_diagnostic_total_loss(1.0, terms)
                .unwrap(),
            9.0
        );
    }

    #[test]
    fn weighted_diagnostic_total_loss_validates_raw_diagnostics_even_with_zero_lambdas() {
        let config = LossConfig::default().for_phase(TrainPhaseKind::DenseTeacherWarmup);
        let terms = LossTermValues {
            distill_loss: 0.0,
            balance_loss: 0.0,
            zrouter_loss: 0.0,
            switch_loss: 0.0,
            range_loss: 0.0,
            zero_loss: f32::NAN,
            shape_loss: 0.0,
            overflow_loss: 0.0,
        };

        assert_eq!(
            config
                .weighted_diagnostic_total_loss(1.0, terms)
                .unwrap_err(),
            LossConfigError::NonFiniteLoss {
                name: ZERO_LOSS,
                value: f32::NAN,
            }
        );
    }

    #[test]
    fn loss_config_builds_and_logs_structured_raw_breakdown_with_weighted_diagnostic_total() {
        let config = LossConfig::from_toml_str(
            r#"
            lambda_distill = 0.5
            lambda_balance = 0.25
            lambda_zrouter = 0.125
            "#,
        )
        .unwrap();
        let terms = nonzero_terms();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let breakdown = config
            .for_phase(TrainPhaseKind::ExpertTernaryQat)
            .log_diagnostic_loss_step(&emitter, 7, 1.0, terms)
            .unwrap();

        assert_eq!(breakdown.total_loss, 4.0);
        assert_eq!(breakdown.zero_loss, 64.0);
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("zero_loss"),
            Some(&TestFieldValue::F32(64.0))
        );
        assert_eq!(
            events[0].field("total_loss"),
            Some(&TestFieldValue::F32(4.0))
        );
    }

    fn nonzero_terms() -> LossTermValues {
        LossTermValues {
            distill_loss: 2.0,
            balance_loss: 4.0,
            zrouter_loss: 8.0,
            switch_loss: 16.0,
            range_loss: 32.0,
            zero_loss: 64.0,
            shape_loss: 128.0,
            overflow_loss: 256.0,
        }
    }
}
