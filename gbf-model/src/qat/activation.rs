//! Backend-independent activation fake-quantization core.

use std::error::Error;
use std::fmt;

use super::{QatHardnessControl, QuantHardness};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivationRange {
    lo: f32,
    hi: f32,
}

impl ActivationRange {
    pub fn new(lo: f32, hi: f32) -> Result<Self, ActFakeQuantError> {
        if !lo.is_finite() {
            return Err(ActFakeQuantError::InvalidRangeBound {
                name: "lo",
                value: lo,
            });
        }

        if !hi.is_finite() {
            return Err(ActFakeQuantError::InvalidRangeBound {
                name: "hi",
                value: hi,
            });
        }

        if lo >= hi {
            return Err(ActFakeQuantError::InvalidRangeOrder { lo, hi });
        }

        Ok(Self { lo, hi })
    }

    pub fn lo(self) -> f32 {
        self.lo
    }

    pub fn hi(self) -> f32 {
        self.hi
    }

    pub fn clamp(self, value: f32) -> f32 {
        value.clamp(self.lo, self.hi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmaDecay {
    value: f32,
}

impl EmaDecay {
    pub fn new(value: f32) -> Result<Self, ActFakeQuantError> {
        if !value.is_finite() || !(0.0..1.0).contains(&value) {
            return Err(ActFakeQuantError::InvalidEmaDecay(value));
        }

        Ok(Self { value })
    }

    pub fn value(self) -> f32 {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivationRangeMode {
    Fixed(ActivationRange),
    Learned(ActivationRange),
    Ema {
        range: ActivationRange,
        decay: EmaDecay,
    },
}

impl ActivationRangeMode {
    pub fn range(self) -> ActivationRange {
        match self {
            Self::Fixed(range) | Self::Learned(range) | Self::Ema { range, .. } => range,
        }
    }

    pub fn kind(self) -> ActivationRangeModeKind {
        match self {
            Self::Fixed(_) => ActivationRangeModeKind::Fixed,
            Self::Learned(_) => ActivationRangeModeKind::Learned,
            Self::Ema { .. } => ActivationRangeModeKind::Ema,
        }
    }

    pub fn with_range(self, range: ActivationRange) -> Self {
        match self {
            Self::Fixed(_) => Self::Fixed(range),
            Self::Learned(_) => Self::Learned(range),
            Self::Ema { decay, .. } => Self::Ema { range, decay },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationRangeModeKind {
    Fixed,
    Learned,
    Ema,
}

impl fmt::Display for ActivationRangeModeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed => f.write_str("fixed"),
            Self::Learned => f.write_str("learned"),
            Self::Ema => f.write_str("ema"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationQuantFormat {
    /// Symmetric signed 8-bit quantization with zero preserved exactly.
    Int8,
    /// Affine unsigned 8-bit quantization over the configured activation range.
    UInt8,
    /// Affine unsigned 4-bit quantization over the configured activation range.
    UInt4,
}

impl ActivationQuantFormat {
    pub fn quant_steps(self) -> u16 {
        match self {
            Self::UInt8 => 255,
            Self::Int8 => 127,
            Self::UInt4 => 15,
        }
    }

    fn is_signed(self) -> bool {
        matches!(self, Self::Int8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationForwardMode {
    Passthrough,
    Train,
    Eval,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivationFakeQuantSpec {
    range: ActivationRange,
    quant_format: ActivationQuantFormat,
    hardness: QuantHardness,
    enabled: bool,
}

impl ActivationFakeQuantSpec {
    pub fn range(self) -> ActivationRange {
        self.range
    }

    pub fn quant_format(self) -> ActivationQuantFormat {
        self.quant_format
    }

    pub fn hardness(self) -> QuantHardness {
        self.hardness
    }

    pub fn quant_steps(self) -> u16 {
        self.quant_format.quant_steps()
    }

    pub fn enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActFakeQuant {
    range_mode: ActivationRangeMode,
    quant_format: ActivationQuantFormat,
    eval_passthrough: bool,
    hardness: QuantHardness,
}

impl ActFakeQuant {
    pub fn new(
        range_mode: ActivationRangeMode,
        quant_format: ActivationQuantFormat,
    ) -> Result<Self, ActFakeQuantError> {
        // Force validation through the range accessor even if future variants
        // carry richer state.
        let _ = range_mode.range();

        Ok(Self {
            range_mode,
            quant_format,
            eval_passthrough: false,
            hardness: QuantHardness::Hard,
        })
    }

    pub fn with_eval_passthrough(mut self, eval_passthrough: bool) -> Self {
        self.eval_passthrough = eval_passthrough;
        self
    }

    pub fn inference_forward(
        &self,
        input: &[f32],
        mode: ActivationForwardMode,
    ) -> Result<Vec<f32>, ActFakeQuantError> {
        validate_input(input)?;

        if self.hardness == QuantHardness::Off
            || mode == ActivationForwardMode::Passthrough
            || mode == ActivationForwardMode::Eval && self.eval_passthrough
        {
            return Ok(input.to_vec());
        }

        Ok(input
            .iter()
            .copied()
            .map(|value| match self.hardness {
                QuantHardness::Off => unreachable!("off hardness returns before quantized path"),
                QuantHardness::Soft => {
                    soft_fake_quantize_value(value, self.export_range(), self.quant_format)
                }
                QuantHardness::Hard => {
                    fake_quantize_value(value, self.export_range(), self.quant_format)
                }
            })
            .collect())
    }

    pub fn export_range(&self) -> ActivationRange {
        self.range_mode.range()
    }

    pub fn range_mode(&self) -> ActivationRangeMode {
        self.range_mode
    }

    pub fn quant_format(&self) -> ActivationQuantFormat {
        self.quant_format
    }

    pub fn eval_passthrough(&self) -> bool {
        self.eval_passthrough
    }

    pub fn forward_spec(&self, mode: ActivationForwardMode) -> ActivationFakeQuantSpec {
        ActivationFakeQuantSpec {
            range: self.range_mode.range(),
            quant_format: self.quant_format,
            hardness: self.hardness,
            enabled: self.hardness != QuantHardness::Off
                && mode != ActivationForwardMode::Passthrough
                && (mode == ActivationForwardMode::Train || !self.eval_passthrough),
        }
    }

    pub fn update_ema_range(&mut self, observed: ActivationRange) -> Result<(), ActFakeQuantError> {
        if let ActivationRangeMode::Ema { range, decay } = self.range_mode {
            let keep = decay.value();
            let update = 1.0 - keep;
            let next = ActivationRange {
                lo: range.lo() * keep + observed.lo() * update,
                hi: range.hi() * keep + observed.hi() * update,
            };
            self.range_mode = ActivationRangeMode::Ema { range: next, decay };
            Ok(())
        } else {
            Err(ActFakeQuantError::RangeModeMismatch {
                expected: ActivationRangeModeKind::Ema,
                actual: self.range_mode.kind(),
            })
        }
    }

    pub fn update_learned_range(
        &mut self,
        learned: ActivationRange,
    ) -> Result<(), ActFakeQuantError> {
        if matches!(self.range_mode, ActivationRangeMode::Learned(_)) {
            self.range_mode = ActivationRangeMode::Learned(learned);
            Ok(())
        } else {
            Err(ActFakeQuantError::RangeModeMismatch {
                expected: ActivationRangeModeKind::Learned,
                actual: self.range_mode.kind(),
            })
        }
    }
}

impl QatHardnessControl for ActFakeQuant {
    fn hardness(&self) -> QuantHardness {
        self.hardness
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        self.hardness = hardness;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActFakeQuantError {
    InvalidRangeBound {
        name: &'static str,
        value: f32,
    },
    InvalidRangeOrder {
        lo: f32,
        hi: f32,
    },
    InvalidEmaDecay(f32),
    RangeModeMismatch {
        expected: ActivationRangeModeKind,
        actual: ActivationRangeModeKind,
    },
    NonFiniteInput {
        index: usize,
    },
}

impl fmt::Display for ActFakeQuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRangeBound { name, value } => {
                write!(f, "activation range bound {name} is not finite: {value}")
            }
            Self::InvalidRangeOrder { lo, hi } => {
                write!(f, "activation range must satisfy lo < hi, got {lo} >= {hi}")
            }
            Self::InvalidEmaDecay(value) => {
                write!(f, "EMA decay must be finite and in [0, 1), got {value}")
            }
            Self::RangeModeMismatch { expected, actual } => {
                write!(
                    f,
                    "activation range update expected {expected} mode, got {actual}"
                )
            }
            Self::NonFiniteInput { index } => {
                write!(f, "activation input at index {index} is not finite")
            }
        }
    }
}

impl Error for ActFakeQuantError {}

fn validate_input(input: &[f32]) -> Result<(), ActFakeQuantError> {
    if let Some(index) = input.iter().position(|value| !value.is_finite()) {
        return Err(ActFakeQuantError::NonFiniteInput { index });
    }

    Ok(())
}

fn fake_quantize_value(
    value: f32,
    range: ActivationRange,
    quant_format: ActivationQuantFormat,
) -> f32 {
    let clamped = range.clamp(value);
    if quant_format.is_signed() {
        let qmax = f64::from(quant_format.quant_steps());
        let max_abs = f64::from(range.lo().abs().max(range.hi().abs()));
        let quantized = (f64::from(clamped) * qmax / max_abs)
            .round()
            .clamp(-qmax, qmax);
        range.clamp((quantized * max_abs / qmax) as f32)
    } else {
        let qmax = f64::from(quant_format.quant_steps());
        let lo = f64::from(range.lo());
        let width = f64::from(range.hi()) - lo;
        let quantized = ((f64::from(clamped) - lo) * qmax / width)
            .round()
            .clamp(0.0, qmax);
        range.clamp((quantized * width / qmax + lo) as f32)
    }
}

fn soft_fake_quantize_value(
    value: f32,
    range: ActivationRange,
    quant_format: ActivationQuantFormat,
) -> f32 {
    let clamped = range.clamp(value);
    let quantized = fake_quantize_value(value, range, quant_format);

    range.clamp((clamped + quantized) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qat_activation_forward_clamps_and_quantizes_in_range() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let input = vec![-2.0, -0.25, 0.25, 2.0];

        let output = quant
            .inference_forward(&input, ActivationForwardMode::Train)
            .unwrap();

        assert!(output.iter().all(|value| (-1.0..=1.0).contains(value)));
        assert_eq!(output[0], -1.0);
        assert_eq!(output[3], 1.0);
    }

    #[test]
    fn qat_activation_signed_int8_preserves_zero() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        let output = quant
            .inference_forward(&[0.0], ActivationForwardMode::Train)
            .unwrap();

        assert_eq!(output, vec![0.0]);
    }

    #[test]
    fn qat_activation_eval_passthrough_is_explicit() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap()
        .with_eval_passthrough(true);
        let input = vec![-1.0, 0.25, 2.0];

        assert_eq!(
            quant
                .inference_forward(&input, ActivationForwardMode::Eval)
                .unwrap(),
            input
        );
    }

    #[test]
    fn qat_activation_exports_current_range_and_updates_ema_at_boundary() {
        let mut quant = ActFakeQuant::new(
            ActivationRangeMode::Ema {
                range: ActivationRange::new(-1.0, 1.0).unwrap(),
                decay: EmaDecay::new(0.5).unwrap(),
            },
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        quant
            .update_ema_range(ActivationRange::new(-3.0, 5.0).unwrap())
            .unwrap();

        let range = quant.export_range();
        assert_eq!(range.lo(), -2.0);
        assert_eq!(range.hi(), 3.0);
    }

    #[test]
    fn qat_activation_rejects_invalid_contracts() {
        assert!(ActivationRange::new(1.0, 1.0).is_err());
        assert!(ActivationRange::new(f32::NAN, 1.0).is_err());
        assert!(ActivationRange::new(0.0, f32::INFINITY).is_err());
        assert!(EmaDecay::new(-0.1).is_err());
        assert!(EmaDecay::new(1.0).is_err());

        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        assert_eq!(
            quant.inference_forward(&[0.0, f32::NAN], ActivationForwardMode::Train),
            Err(ActFakeQuantError::NonFiniteInput { index: 1 })
        );
    }

    #[test]
    fn qat_activation_learned_range_updates_explicitly() {
        let mut quant = ActFakeQuant::new(
            ActivationRangeMode::Learned(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        quant
            .update_learned_range(ActivationRange::new(-2.0, 3.0).unwrap())
            .unwrap();

        let range = quant.export_range();
        assert_eq!(range.lo(), -2.0);
        assert_eq!(range.hi(), 3.0);
    }

    #[test]
    fn qat_activation_signed_int8_clamps_after_asymmetric_dequantization() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 0.5).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        let output = quant
            .inference_forward(&[0.5], ActivationForwardMode::Train)
            .unwrap();

        assert_eq!(output, vec![0.5]);
    }

    #[test]
    fn qat_activation_tiny_ranges_do_not_produce_nan() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0e-40, 1.0e-40).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        let output = quant
            .inference_forward(&[1.0e-40], ActivationForwardMode::Train)
            .unwrap();

        assert!(output[0].is_finite());
        assert!(output[0] <= 1.0e-40);
    }

    #[test]
    fn qat_activation_range_updates_reject_wrong_mode() {
        let mut quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();

        assert_eq!(
            quant.update_ema_range(ActivationRange::new(-2.0, 2.0).unwrap()),
            Err(ActFakeQuantError::RangeModeMismatch {
                expected: ActivationRangeModeKind::Ema,
                actual: ActivationRangeModeKind::Fixed,
            })
        );
        assert_eq!(
            quant.update_learned_range(ActivationRange::new(-2.0, 2.0).unwrap()),
            Err(ActFakeQuantError::RangeModeMismatch {
                expected: ActivationRangeModeKind::Learned,
                actual: ActivationRangeModeKind::Fixed,
            })
        );
    }

    #[test]
    fn qat_activation_spec_exposes_quant_steps_from_model_contract() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt4,
        )
        .unwrap();

        let spec = quant.forward_spec(ActivationForwardMode::Train);

        assert!(spec.enabled());
        assert_eq!(spec.quant_steps(), 15);
        assert_eq!(spec.quant_format(), ActivationQuantFormat::UInt4);
    }
}
