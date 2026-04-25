//! Backend-independent activation fake-quantization core.

use std::error::Error;
use std::fmt;

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

    pub fn with_range(self, range: ActivationRange) -> Self {
        match self {
            Self::Fixed(_) => Self::Fixed(range),
            Self::Learned(_) => Self::Learned(range),
            Self::Ema { decay, .. } => Self::Ema { range, decay },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationQuantFormat {
    Int8,
    UInt8,
    Int4,
}

impl ActivationQuantFormat {
    pub fn levels(self) -> u16 {
        match self {
            Self::Int8 | Self::UInt8 => 255,
            Self::Int4 => 15,
        }
    }

    pub fn quant_scale(self, range: ActivationRange) -> f32 {
        f32::from(self.levels()) / (range.hi() - range.lo())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActivationSteSpec {
    range: ActivationRange,
    quant_format: ActivationQuantFormat,
    enabled: bool,
}

impl ActivationSteSpec {
    pub fn range(self) -> ActivationRange {
        self.range
    }

    pub fn quant_format(self) -> ActivationQuantFormat {
        self.quant_format
    }

    pub fn enabled(self) -> bool {
        self.enabled
    }
}

pub trait ActivationFakeQuantBackend {
    type Tensor;

    /// Apply clamp + fake quantization while preserving STE gradients through
    /// the backend-owned input tensor.
    fn activation_fake_quant_ste(
        &self,
        spec: ActivationSteSpec,
        input: &Self::Tensor,
    ) -> Self::Tensor;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActFakeQuant {
    range_mode: ActivationRangeMode,
    quant_format: ActivationQuantFormat,
    eval_passthrough: bool,
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
        })
    }

    pub fn with_eval_passthrough(mut self, eval_passthrough: bool) -> Self {
        self.eval_passthrough = eval_passthrough;
        self
    }

    pub fn forward<B: ActivationFakeQuantBackend>(
        &self,
        backend: &B,
        input: &B::Tensor,
        training: bool,
    ) -> B::Tensor {
        backend.activation_fake_quant_ste(self.ste_spec(training), input)
    }

    pub fn inference_forward(
        &self,
        input: &[f32],
        training: bool,
    ) -> Result<Vec<f32>, ActFakeQuantError> {
        validate_input(input)?;

        let spec = self.ste_spec(training);
        if !spec.enabled() {
            return Ok(input.to_vec());
        }

        Ok(input
            .iter()
            .copied()
            .map(|value| fake_quantize_value(value, spec.range(), spec.quant_format()))
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

    pub fn update_ema_range(&mut self, observed: ActivationRange) {
        if let ActivationRangeMode::Ema { range, decay } = self.range_mode {
            let keep = decay.value();
            let update = 1.0 - keep;
            let next = ActivationRange {
                lo: range.lo() * keep + observed.lo() * update,
                hi: range.hi() * keep + observed.hi() * update,
            };
            self.range_mode = ActivationRangeMode::Ema { range: next, decay };
        }
    }

    fn ste_spec(&self, training: bool) -> ActivationSteSpec {
        ActivationSteSpec {
            range: self.range_mode.range(),
            quant_format: self.quant_format,
            enabled: training || !self.eval_passthrough,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActFakeQuantError {
    InvalidRangeBound { name: &'static str, value: f32 },
    InvalidRangeOrder { lo: f32, hi: f32 },
    InvalidEmaDecay(f32),
    NonFiniteInput { index: usize },
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
    let scale = quant_format.quant_scale(range);
    let clamped = range.clamp(value);
    ((clamped - range.lo()) * scale).round() / scale + range.lo()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScalarActivationBackend;

    impl ActivationFakeQuantBackend for ScalarActivationBackend {
        type Tensor = Vec<f32>;

        fn activation_fake_quant_ste(
            &self,
            spec: ActivationSteSpec,
            input: &Self::Tensor,
        ) -> Self::Tensor {
            if !spec.enabled() {
                return input.clone();
            }

            input
                .iter()
                .copied()
                .map(|value| fake_quantize_value(value, spec.range(), spec.quant_format()))
                .collect()
        }
    }

    #[test]
    fn qat_activation_forward_clamps_and_quantizes_in_range() {
        let quant = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let input = vec![-2.0, -0.25, 0.25, 2.0];

        let output = quant.forward(&ScalarActivationBackend, &input, true);

        assert!(output.iter().all(|value| (-1.0..=1.0).contains(value)));
        assert_eq!(output[0], -1.0);
        assert_eq!(output[3], 1.0);
        assert_eq!(output, quant.inference_forward(&input, true).unwrap());
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
            quant.forward(&ScalarActivationBackend, &input, false),
            input
        );
        assert_eq!(quant.inference_forward(&input, false).unwrap(), input);
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

        quant.update_ema_range(ActivationRange::new(-3.0, 5.0).unwrap());

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
            quant.inference_forward(&[0.0, f32::NAN], true),
            Err(ActFakeQuantError::NonFiniteInput { index: 1 })
        );
    }
}
