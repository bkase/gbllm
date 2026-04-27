//! Slow independent reference implementation for ternary weight packing.
//!
//! This module deliberately stays byte-oriented and dependency-light. It does
//! not call into the model/export packer; tests can use it as an oracle for
//! deployed ternary bytes once the production materializer exists.
//!
//! The public helpers split projection, weight packing, and scale packing on
//! purpose. `TernaryWeightPlan` names threshold strategies, but it does not
//! carry concrete threshold or scale values, so callers must pass canonical
//! ternary values and raw Q8.8 scale payloads at the byte-packing boundary.

use std::error::Error;
use std::fmt;

use gbf_artifact::weight_plan::{ScaleFormat, ScaleGranularity, TernaryWeightPlan, WeightEncoding};

const TERNARY_VALUES_PER_BYTE: usize = 4;
const BITS_PER_BYTE: usize = 8;
const Q8_8_SCALE: f32 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceTernaryShape {
    rows: u32,
    cols: u32,
    len: usize,
}

impl ReferenceTernaryShape {
    pub fn new(rows: u32, cols: u32) -> Result<Self, TernaryReferenceError> {
        if rows == 0 || cols == 0 {
            return Err(TernaryReferenceError::EmptyShape { rows, cols });
        }

        let element_count = u128::from(rows) * u128::from(cols);
        let len = usize::try_from(element_count)
            .map_err(|_| TernaryReferenceError::ElementCountOverflow { rows, cols })?;

        Ok(Self { rows, cols, len })
    }

    #[must_use]
    pub const fn rows(self) -> u32 {
        self.rows
    }

    #[must_use]
    pub const fn cols(self) -> u32 {
        self.cols
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceTernaryPacket {
    plan: TernaryWeightPlan,
    shape: ReferenceTernaryShape,
    weight_bytes: Vec<u8>,
    scale_bytes: Vec<u8>,
}

impl ReferenceTernaryPacket {
    #[must_use]
    pub const fn plan(&self) -> TernaryWeightPlan {
        self.plan
    }

    #[must_use]
    pub const fn shape(&self) -> ReferenceTernaryShape {
        self.shape
    }

    #[must_use]
    pub fn weight_bytes(&self) -> &[u8] {
        &self.weight_bytes
    }

    #[must_use]
    pub fn scale_bytes(&self) -> &[u8] {
        &self.scale_bytes
    }

    #[must_use]
    pub fn total_byte_len(&self) -> usize {
        self.weight_bytes.len() + self.scale_bytes.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TernaryReferenceError {
    EmptyShape {
        rows: u32,
        cols: u32,
    },
    ElementCountOverflow {
        rows: u32,
        cols: u32,
    },
    ByteLenOverflow {
        rows: u32,
        cols: u32,
    },
    ValueLenMismatch {
        expected: usize,
        actual: usize,
    },
    ScaleLenMismatch {
        expected: usize,
        actual: usize,
    },
    WeightByteLenMismatch {
        expected: usize,
        actual: usize,
    },
    ScaleByteLenMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidTernaryValue {
        index: usize,
        value: i8,
    },
    BinaryCannotEncodeZero {
        index: usize,
    },
    ReservedTernaryCode {
        index: usize,
        code: u8,
    },
    NonCanonicalPadding {
        bit_index: usize,
    },
    SparseSignWithoutValue {
        index: usize,
    },
    NonFiniteWeight {
        index: usize,
    },
    ThresholdLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteThreshold {
        index: usize,
    },
    NegativeThreshold {
        index: usize,
        value: f32,
    },
    NonFiniteScale {
        index: usize,
    },
    NegativeScale {
        index: usize,
        value: f32,
    },
    ScaleOutOfRange {
        index: usize,
        value: f32,
        format: ScaleFormat,
    },
    UnsupportedScaleFormat {
        format: ScaleFormat,
    },
}

impl fmt::Display for TernaryReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyShape { rows, cols } => {
                write!(
                    f,
                    "ternary reference shape must be non-empty, got {rows}x{cols}"
                )
            }
            Self::ElementCountOverflow { rows, cols } => {
                write!(
                    f,
                    "ternary reference shape {rows}x{cols} exceeds usize elements"
                )
            }
            Self::ByteLenOverflow { rows, cols } => {
                write!(
                    f,
                    "ternary reference shape {rows}x{cols} exceeds usize bytes"
                )
            }
            Self::ValueLenMismatch { expected, actual } => {
                write!(
                    f,
                    "ternary value length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ScaleLenMismatch { expected, actual } => {
                write!(
                    f,
                    "ternary scale length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::WeightByteLenMismatch { expected, actual } => write!(
                f,
                "ternary weight byte length mismatch: expected {expected}, got {actual}"
            ),
            Self::ScaleByteLenMismatch { expected, actual } => write!(
                f,
                "ternary scale byte length mismatch: expected {expected}, got {actual}"
            ),
            Self::InvalidTernaryValue { index, value } => {
                write!(
                    f,
                    "ternary value at index {index} must be -1, 0, or 1, got {value}"
                )
            }
            Self::BinaryCannotEncodeZero { index } => {
                write!(
                    f,
                    "binary ternary encoding cannot represent zero at index {index}"
                )
            }
            Self::ReservedTernaryCode { index, code } => {
                write!(
                    f,
                    "reserved 2-bit ternary code {code:#04b} at index {index}"
                )
            }
            Self::NonCanonicalPadding { bit_index } => {
                write!(
                    f,
                    "non-zero non-canonical ternary padding at bit index {bit_index}"
                )
            }
            Self::SparseSignWithoutValue { index } => {
                write!(
                    f,
                    "sparse ternary sign bit is set for a zero value at index {index}"
                )
            }
            Self::NonFiniteWeight { index } => {
                write!(f, "ternary source weight at index {index} is not finite")
            }
            Self::ThresholdLenMismatch { expected, actual } => write!(
                f,
                "ternary threshold length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteThreshold { index } => {
                write!(f, "ternary threshold at row {index} is not finite")
            }
            Self::NegativeThreshold { index, value } => {
                write!(
                    f,
                    "ternary threshold at row {index} must be non-negative, got {value}"
                )
            }
            Self::NonFiniteScale { index } => {
                write!(f, "ternary scale at index {index} is not finite")
            }
            Self::NegativeScale { index, value } => {
                write!(
                    f,
                    "ternary scale at index {index} must be non-negative, got {value}"
                )
            }
            Self::ScaleOutOfRange {
                index,
                value,
                format,
            } => write!(
                f,
                "ternary scale at index {index} is out of range for {format:?}: {value}"
            ),
            Self::UnsupportedScaleFormat { format } => {
                write!(f, "unsupported ternary reference scale format: {format:?}")
            }
        }
    }
}

impl Error for TernaryReferenceError {}

pub fn pack_reference_ternary(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    ternary_values: &[i8],
    scale_values: &[u16],
) -> Result<ReferenceTernaryPacket, TernaryReferenceError> {
    let shape = ReferenceTernaryShape::new(rows, cols)?;
    let weight_bytes = pack_reference_ternary_values(plan, rows, cols, ternary_values)?;
    let scale_bytes = pack_reference_scale_values(plan, rows, cols, scale_values)?;

    Ok(ReferenceTernaryPacket {
        plan,
        shape,
        weight_bytes,
        scale_bytes,
    })
}

pub fn project_reference_ternary_values(
    rows: u32,
    cols: u32,
    full_precision_weights: &[f32],
    thresholds: &[f32],
) -> Result<Vec<i8>, TernaryReferenceError> {
    let shape = ReferenceTernaryShape::new(rows, cols)?;
    if full_precision_weights.len() != shape.len() {
        return Err(TernaryReferenceError::ValueLenMismatch {
            expected: shape.len(),
            actual: full_precision_weights.len(),
        });
    }

    let expected_thresholds = usize::try_from(rows)
        .map_err(|_| TernaryReferenceError::ElementCountOverflow { rows, cols })?;
    if thresholds.len() != expected_thresholds {
        return Err(TernaryReferenceError::ThresholdLenMismatch {
            expected: expected_thresholds,
            actual: thresholds.len(),
        });
    }

    for (index, &threshold) in thresholds.iter().enumerate() {
        if !threshold.is_finite() {
            return Err(TernaryReferenceError::NonFiniteThreshold { index });
        }
        if threshold < 0.0 {
            return Err(TernaryReferenceError::NegativeThreshold {
                index,
                value: threshold,
            });
        }
    }

    let cols = usize::try_from(cols)
        .map_err(|_| TernaryReferenceError::ElementCountOverflow { rows, cols })?;
    full_precision_weights
        .chunks_exact(cols)
        .enumerate()
        .flat_map(|(row_index, row)| {
            let threshold = thresholds[row_index];
            row.iter().enumerate().map(move |(col_index, &weight)| {
                let index = row_index * cols + col_index;
                if !weight.is_finite() {
                    return Err(TernaryReferenceError::NonFiniteWeight { index });
                }

                Ok(if weight > threshold {
                    1
                } else if weight < -threshold {
                    -1
                } else {
                    0
                })
            })
        })
        .collect()
}

pub fn pack_reference_ternary_values(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    ternary_values: &[i8],
) -> Result<Vec<u8>, TernaryReferenceError> {
    let shape = ReferenceTernaryShape::new(rows, cols)?;
    if ternary_values.len() != shape.len() {
        return Err(TernaryReferenceError::ValueLenMismatch {
            expected: shape.len(),
            actual: ternary_values.len(),
        });
    }

    match plan.encoding {
        WeightEncoding::Ternary2 => pack_ternary2(shape, ternary_values),
        WeightEncoding::SparseTernaryBitplanes => pack_sparse_bitplanes(shape, ternary_values),
        WeightEncoding::Binary1 => pack_binary1(shape, ternary_values),
    }
}

pub fn unpack_reference_ternary_values(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    weight_bytes: &[u8],
) -> Result<Vec<i8>, TernaryReferenceError> {
    let shape = ReferenceTernaryShape::new(rows, cols)?;
    match plan.encoding {
        WeightEncoding::Ternary2 => unpack_ternary2(shape, weight_bytes),
        WeightEncoding::SparseTernaryBitplanes => unpack_sparse_bitplanes(shape, weight_bytes),
        WeightEncoding::Binary1 => unpack_binary1(shape, weight_bytes),
    }
}

pub fn pack_reference_scale_values(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    scale_values: &[u16],
) -> Result<Vec<u8>, TernaryReferenceError> {
    ensure_supported_scale_format(plan.scale_format)?;

    let shape = ReferenceTernaryShape::new(rows, cols)?;
    let expected = scale_count(plan.scale_granularity, shape)?;
    if scale_values.len() != expected {
        return Err(TernaryReferenceError::ScaleLenMismatch {
            expected,
            actual: scale_values.len(),
        });
    }

    let capacity = expected_scale_byte_len(plan, shape)?;
    let mut bytes = Vec::with_capacity(capacity);
    for &scale in scale_values {
        bytes.extend_from_slice(&scale.to_le_bytes());
    }

    Ok(bytes)
}

pub fn unpack_reference_scale_values(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    scale_bytes: &[u8],
) -> Result<Vec<u16>, TernaryReferenceError> {
    ensure_supported_scale_format(plan.scale_format)?;

    let shape = ReferenceTernaryShape::new(rows, cols)?;
    let expected = expected_scale_byte_len(plan, shape)?;
    if scale_bytes.len() != expected {
        return Err(TernaryReferenceError::ScaleByteLenMismatch {
            expected,
            actual: scale_bytes.len(),
        });
    }

    Ok(scale_bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

pub fn quantize_reference_scales(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    scales: &[f32],
) -> Result<Vec<u16>, TernaryReferenceError> {
    ensure_supported_scale_format(plan.scale_format)?;

    let shape = ReferenceTernaryShape::new(rows, cols)?;
    let expected = scale_count(plan.scale_granularity, shape)?;
    if scales.len() != expected {
        return Err(TernaryReferenceError::ScaleLenMismatch {
            expected,
            actual: scales.len(),
        });
    }

    scales
        .iter()
        .enumerate()
        .map(|(index, &scale)| quantize_q8_8_scale(scale, index))
        .collect()
}

pub fn dequantize_reference_scales(
    plan: TernaryWeightPlan,
    rows: u32,
    cols: u32,
    scale_values: &[u16],
) -> Result<Vec<f32>, TernaryReferenceError> {
    ensure_supported_scale_format(plan.scale_format)?;

    let shape = ReferenceTernaryShape::new(rows, cols)?;
    let expected = scale_count(plan.scale_granularity, shape)?;
    if scale_values.len() != expected {
        return Err(TernaryReferenceError::ScaleLenMismatch {
            expected,
            actual: scale_values.len(),
        });
    }

    Ok(scale_values
        .iter()
        .map(|&raw| f32::from(raw) / Q8_8_SCALE)
        .collect())
}

fn pack_ternary2(
    shape: ReferenceTernaryShape,
    ternary_values: &[i8],
) -> Result<Vec<u8>, TernaryReferenceError> {
    let mut bytes = vec![0; weight_byte_len(WeightEncoding::Ternary2, shape)?];
    for (index, &value) in ternary_values.iter().enumerate() {
        let code = encode_ternary2(value, index)?;
        let byte_index = index / TERNARY_VALUES_PER_BYTE;
        let shift = (index % TERNARY_VALUES_PER_BYTE) * 2;
        bytes[byte_index] |= code << shift;
    }
    Ok(bytes)
}

fn unpack_ternary2(
    shape: ReferenceTernaryShape,
    weight_bytes: &[u8],
) -> Result<Vec<i8>, TernaryReferenceError> {
    let expected = weight_byte_len(WeightEncoding::Ternary2, shape)?;
    expect_weight_byte_len(expected, weight_bytes.len())?;

    let mut values = Vec::with_capacity(shape.len());
    for index in 0..shape.len() {
        let byte = weight_bytes[index / TERNARY_VALUES_PER_BYTE];
        let shift = (index % TERNARY_VALUES_PER_BYTE) * 2;
        let code = (byte >> shift) & 0b11;
        values.push(decode_ternary2(code, index)?);
    }

    validate_ternary2_padding(shape, weight_bytes)?;
    Ok(values)
}

fn encode_ternary2(value: i8, index: usize) -> Result<u8, TernaryReferenceError> {
    match value {
        -1 => Ok(0b10),
        0 => Ok(0b00),
        1 => Ok(0b01),
        _ => Err(TernaryReferenceError::InvalidTernaryValue { index, value }),
    }
}

fn decode_ternary2(code: u8, index: usize) -> Result<i8, TernaryReferenceError> {
    match code {
        0b00 => Ok(0),
        0b01 => Ok(1),
        0b10 => Ok(-1),
        _ => Err(TernaryReferenceError::ReservedTernaryCode { index, code }),
    }
}

fn validate_ternary2_padding(
    shape: ReferenceTernaryShape,
    weight_bytes: &[u8],
) -> Result<(), TernaryReferenceError> {
    let first_padding_value = shape.len();
    let padded_values = weight_bytes.len() * TERNARY_VALUES_PER_BYTE;
    for index in first_padding_value..padded_values {
        let byte = weight_bytes[index / TERNARY_VALUES_PER_BYTE];
        let shift = (index % TERNARY_VALUES_PER_BYTE) * 2;
        let code = (byte >> shift) & 0b11;
        if code != 0 {
            return Err(TernaryReferenceError::NonCanonicalPadding {
                bit_index: index * 2,
            });
        }
    }
    Ok(())
}

fn pack_sparse_bitplanes(
    shape: ReferenceTernaryShape,
    ternary_values: &[i8],
) -> Result<Vec<u8>, TernaryReferenceError> {
    let plane_len = bitplane_len(shape)?;
    let mut bytes = vec![0; plane_len * 2];
    for (index, &value) in ternary_values.iter().enumerate() {
        let byte_index = index / BITS_PER_BYTE;
        let mask = 1_u8 << (index % BITS_PER_BYTE);
        match value {
            -1 => {
                bytes[byte_index] |= mask;
                bytes[plane_len + byte_index] |= mask;
            }
            0 => {}
            1 => {
                bytes[byte_index] |= mask;
            }
            _ => return Err(TernaryReferenceError::InvalidTernaryValue { index, value }),
        }
    }
    Ok(bytes)
}

fn unpack_sparse_bitplanes(
    shape: ReferenceTernaryShape,
    weight_bytes: &[u8],
) -> Result<Vec<i8>, TernaryReferenceError> {
    let expected = weight_byte_len(WeightEncoding::SparseTernaryBitplanes, shape)?;
    expect_weight_byte_len(expected, weight_bytes.len())?;

    let plane_len = bitplane_len(shape)?;
    let (nonzero_plane, sign_plane) = weight_bytes.split_at(plane_len);
    let mut values = Vec::with_capacity(shape.len());
    for index in 0..shape.len() {
        let nonzero = bit_is_set(nonzero_plane, index);
        let negative = bit_is_set(sign_plane, index);
        if negative && !nonzero {
            return Err(TernaryReferenceError::SparseSignWithoutValue { index });
        }
        values.push(if !nonzero {
            0
        } else if negative {
            -1
        } else {
            1
        });
    }

    validate_bit_padding(shape, nonzero_plane, 0)?;
    validate_bit_padding(shape, sign_plane, plane_len * BITS_PER_BYTE)?;
    Ok(values)
}

fn pack_binary1(
    shape: ReferenceTernaryShape,
    ternary_values: &[i8],
) -> Result<Vec<u8>, TernaryReferenceError> {
    let mut bytes = vec![0; weight_byte_len(WeightEncoding::Binary1, shape)?];
    for (index, &value) in ternary_values.iter().enumerate() {
        match value {
            -1 => {}
            0 => return Err(TernaryReferenceError::BinaryCannotEncodeZero { index }),
            1 => {
                let byte_index = index / BITS_PER_BYTE;
                bytes[byte_index] |= 1_u8 << (index % BITS_PER_BYTE);
            }
            _ => return Err(TernaryReferenceError::InvalidTernaryValue { index, value }),
        }
    }
    Ok(bytes)
}

fn unpack_binary1(
    shape: ReferenceTernaryShape,
    weight_bytes: &[u8],
) -> Result<Vec<i8>, TernaryReferenceError> {
    let expected = weight_byte_len(WeightEncoding::Binary1, shape)?;
    expect_weight_byte_len(expected, weight_bytes.len())?;

    let mut values = Vec::with_capacity(shape.len());
    for index in 0..shape.len() {
        values.push(if bit_is_set(weight_bytes, index) {
            1
        } else {
            -1
        });
    }
    validate_bit_padding(shape, weight_bytes, 0)?;
    Ok(values)
}

fn validate_bit_padding(
    shape: ReferenceTernaryShape,
    bytes: &[u8],
    bit_index_offset: usize,
) -> Result<(), TernaryReferenceError> {
    for index in shape.len()..(bytes.len() * BITS_PER_BYTE) {
        if bit_is_set(bytes, index) {
            return Err(TernaryReferenceError::NonCanonicalPadding {
                bit_index: bit_index_offset + index,
            });
        }
    }
    Ok(())
}

fn bit_is_set(bytes: &[u8], index: usize) -> bool {
    let byte = bytes[index / BITS_PER_BYTE];
    let mask = 1_u8 << (index % BITS_PER_BYTE);
    byte & mask != 0
}

fn quantize_q8_8_scale(scale: f32, index: usize) -> Result<u16, TernaryReferenceError> {
    if !scale.is_finite() {
        return Err(TernaryReferenceError::NonFiniteScale { index });
    }
    if scale < 0.0 {
        return Err(TernaryReferenceError::NegativeScale {
            index,
            value: scale,
        });
    }

    let raw = (scale * Q8_8_SCALE).round();
    if raw > f32::from(u16::MAX) {
        return Err(TernaryReferenceError::ScaleOutOfRange {
            index,
            value: scale,
            format: ScaleFormat::Q8_8,
        });
    }

    Ok(raw as u16)
}

fn ensure_supported_scale_format(format: ScaleFormat) -> Result<(), TernaryReferenceError> {
    match format {
        ScaleFormat::Q8_8 => Ok(()),
        ScaleFormat::Q4_4 | ScaleFormat::Pow2 => {
            Err(TernaryReferenceError::UnsupportedScaleFormat { format })
        }
    }
}

fn weight_byte_len(
    encoding: WeightEncoding,
    shape: ReferenceTernaryShape,
) -> Result<usize, TernaryReferenceError> {
    let elements = u128::from(shape.rows()) * u128::from(shape.cols());
    let bytes = match encoding {
        WeightEncoding::Ternary2 => elements.saturating_mul(2).div_ceil(8),
        WeightEncoding::SparseTernaryBitplanes => elements.div_ceil(8).saturating_mul(2),
        WeightEncoding::Binary1 => elements.div_ceil(8),
    };
    usize::try_from(bytes).map_err(|_| TernaryReferenceError::ByteLenOverflow {
        rows: shape.rows(),
        cols: shape.cols(),
    })
}

fn bitplane_len(shape: ReferenceTernaryShape) -> Result<usize, TernaryReferenceError> {
    let elements = u128::from(shape.rows()) * u128::from(shape.cols());
    usize::try_from(elements.div_ceil(8)).map_err(|_| TernaryReferenceError::ByteLenOverflow {
        rows: shape.rows(),
        cols: shape.cols(),
    })
}

fn scale_count(
    granularity: ScaleGranularity,
    shape: ReferenceTernaryShape,
) -> Result<usize, TernaryReferenceError> {
    let count = match granularity {
        ScaleGranularity::PerTensor => 1,
        ScaleGranularity::PerOutputRow => u128::from(shape.rows()),
        ScaleGranularity::PerGroup(group_size) => (u128::from(shape.rows())
            * u128::from(shape.cols()))
        .div_ceil(u128::from(group_size.get())),
    };

    usize::try_from(count).map_err(|_| TernaryReferenceError::ElementCountOverflow {
        rows: shape.rows(),
        cols: shape.cols(),
    })
}

fn expected_scale_byte_len(
    plan: TernaryWeightPlan,
    shape: ReferenceTernaryShape,
) -> Result<usize, TernaryReferenceError> {
    let count = scale_count(plan.scale_granularity, shape)?;
    count
        .checked_mul(usize::from(plan.scale_format.byte_len()))
        .ok_or_else(|| TernaryReferenceError::ByteLenOverflow {
            rows: shape.rows(),
            cols: shape.cols(),
        })
}

fn expect_weight_byte_len(expected: usize, actual: usize) -> Result<(), TernaryReferenceError> {
    if actual == expected {
        Ok(())
    } else {
        Err(TernaryReferenceError::WeightByteLenMismatch { expected, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_artifact::weight_plan::ThresholdPlan;

    fn plan(
        encoding: WeightEncoding,
        granularity: ScaleGranularity,
        format: ScaleFormat,
    ) -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            encoding,
            granularity,
            format,
            ThresholdPlan::AnnealedGlobalThenPerOutputRow,
        )
    }

    #[test]
    fn ternary_reference_projects_float_weights_with_per_row_thresholds() {
        let projected = project_reference_ternary_values(
            2,
            3,
            &[-0.5, -0.25, 0.0, 0.1, 0.2, 0.7],
            &[0.25, 0.2],
        )
        .unwrap();

        assert_eq!(projected, vec![-1, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn ternary_reference_packs_known_ternary2_bytes() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        let bytes = pack_reference_ternary_values(plan, 2, 3, &[-1, 0, 1, 1, 0, -1]).unwrap();

        assert_eq!(bytes, vec![0x52, 0x08]);
        assert_eq!(
            unpack_reference_ternary_values(plan, 2, 3, &bytes).unwrap(),
            vec![-1, 0, 1, 1, 0, -1]
        );
    }

    #[test]
    fn ternary_reference_rejects_reserved_ternary2_codes_and_padding() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        assert_eq!(
            unpack_reference_ternary_values(plan, 1, 1, &[0b0000_0011]).unwrap_err(),
            TernaryReferenceError::ReservedTernaryCode {
                index: 0,
                code: 0b11,
            }
        );
        assert_eq!(
            unpack_reference_ternary_values(plan, 1, 1, &[0b0000_0100]).unwrap_err(),
            TernaryReferenceError::NonCanonicalPadding { bit_index: 2 }
        );
    }

    #[test]
    fn ternary_reference_packs_sparse_bitplanes_independently() {
        let plan = plan(
            WeightEncoding::SparseTernaryBitplanes,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        );

        let bytes = pack_reference_ternary_values(plan, 1, 5, &[0, -1, 1, 0, -1]).unwrap();

        assert_eq!(bytes, vec![0x16, 0x12]);
        assert_eq!(
            unpack_reference_ternary_values(plan, 1, 5, &bytes).unwrap(),
            vec![0, -1, 1, 0, -1]
        );
    }

    #[test]
    fn ternary_reference_rejects_sparse_signs_without_nonzero_values() {
        let plan = plan(
            WeightEncoding::SparseTernaryBitplanes,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        );

        assert_eq!(
            unpack_reference_ternary_values(plan, 1, 1, &[0x00, 0x01]).unwrap_err(),
            TernaryReferenceError::SparseSignWithoutValue { index: 0 }
        );
    }

    #[test]
    fn ternary_reference_binary_round_trips_sign_values_and_rejects_zero() {
        let plan = plan(
            WeightEncoding::Binary1,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        );

        let bytes = pack_reference_ternary_values(plan, 1, 5, &[-1, 1, -1, 1, 1]).unwrap();

        assert_eq!(bytes, vec![0b0001_1010]);
        assert_eq!(
            unpack_reference_ternary_values(plan, 1, 5, &bytes).unwrap(),
            vec![-1, 1, -1, 1, 1]
        );
        assert_eq!(
            pack_reference_ternary_values(plan, 1, 1, &[0]).unwrap_err(),
            TernaryReferenceError::BinaryCannotEncodeZero { index: 0 }
        );
    }

    #[test]
    fn ternary_reference_packs_q8_8_scale_values_little_endian() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        let values = quantize_reference_scales(plan, 2, 3, &[1.5, 0.25]).unwrap();
        let bytes = pack_reference_scale_values(plan, 2, 3, &values).unwrap();

        assert_eq!(values, vec![384, 64]);
        assert_eq!(bytes, vec![0x80, 0x01, 0x40, 0x00]);
        assert_eq!(
            unpack_reference_scale_values(plan, 2, 3, &bytes).unwrap(),
            values
        );
        assert_eq!(
            dequantize_reference_scales(plan, 2, 3, &values).unwrap(),
            vec![1.5, 0.25]
        );
    }

    #[test]
    fn ternary_reference_rejects_non_q8_8_scales_until_artifact_encoding_exists() {
        for format in [ScaleFormat::Q4_4, ScaleFormat::Pow2] {
            let plan = plan(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerTensor,
                format,
            );

            assert_eq!(
                pack_reference_scale_values(plan, 1, 1, &[0]).unwrap_err(),
                TernaryReferenceError::UnsupportedScaleFormat { format }
            );
            assert_eq!(
                quantize_reference_scales(plan, 1, 1, &[1.0]).unwrap_err(),
                TernaryReferenceError::UnsupportedScaleFormat { format }
            );
        }
    }

    #[test]
    fn ternary_reference_packet_byte_count_matches_plan_for_supported_scales() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        let packet = pack_reference_ternary(plan, 2, 3, &[-1, 0, 1, 1, 0, -1], &[384, 64]).unwrap();

        assert_eq!(packet.total_byte_len(), 6);
        assert_eq!(
            packet.total_byte_len(),
            plan.compute_byte_cost(2, 3).as_u64() as usize
        );
    }

    #[test]
    fn ternary_reference_rejects_shape_and_length_mismatches() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        assert_eq!(
            ReferenceTernaryShape::new(0, 1).unwrap_err(),
            TernaryReferenceError::EmptyShape { rows: 0, cols: 1 }
        );
        assert_eq!(
            pack_reference_ternary_values(plan, 1, 2, &[0]).unwrap_err(),
            TernaryReferenceError::ValueLenMismatch {
                expected: 2,
                actual: 1,
            }
        );
        assert_eq!(
            pack_reference_scale_values(plan, 2, 3, &[1]).unwrap_err(),
            TernaryReferenceError::ScaleLenMismatch {
                expected: 2,
                actual: 1,
            }
        );
        assert_eq!(
            unpack_reference_scale_values(plan, 2, 3, &[0]).unwrap_err(),
            TernaryReferenceError::ScaleByteLenMismatch {
                expected: 4,
                actual: 1,
            }
        );
    }

    #[test]
    fn ternary_reference_rejects_invalid_projection_and_scale_numbers() {
        let plan = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );

        assert_eq!(
            project_reference_ternary_values(1, 1, &[f32::NAN], &[0.0]).unwrap_err(),
            TernaryReferenceError::NonFiniteWeight { index: 0 }
        );
        assert_eq!(
            project_reference_ternary_values(1, 1, &[0.0], &[f32::INFINITY]).unwrap_err(),
            TernaryReferenceError::NonFiniteThreshold { index: 0 }
        );
        assert_eq!(
            project_reference_ternary_values(1, 1, &[0.0], &[-0.5]).unwrap_err(),
            TernaryReferenceError::NegativeThreshold {
                index: 0,
                value: -0.5,
            }
        );
        assert_eq!(
            quantize_reference_scales(plan, 1, 1, &[f32::NAN]).unwrap_err(),
            TernaryReferenceError::NonFiniteScale { index: 0 }
        );
        assert_eq!(
            quantize_reference_scales(plan, 1, 1, &[-1.0]).unwrap_err(),
            TernaryReferenceError::NegativeScale {
                index: 0,
                value: -1.0,
            }
        );
        assert_eq!(
            quantize_reference_scales(plan, 1, 1, &[256.0]).unwrap_err(),
            TernaryReferenceError::ScaleOutOfRange {
                index: 0,
                value: 256.0,
                format: ScaleFormat::Q8_8,
            }
        );
    }

    #[test]
    fn ternary_reference_randomized_round_trips_supported_weight_encodings() {
        let plans = [
            plan(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            plan(
                WeightEncoding::SparseTernaryBitplanes,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            plan(
                WeightEncoding::Binary1,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
        ];
        let mut rng = Lcg::new(0xc0ffee);

        for case_index in 0..128 {
            let rows = 1 + rng.next_u32() % 17;
            let cols = 1 + rng.next_u32() % 19;
            let len = usize::try_from(rows * cols).unwrap();

            for plan in plans {
                let values = (0..len)
                    .map(|_| match plan.encoding {
                        WeightEncoding::Binary1 => {
                            if rng.next_u32() & 1 == 0 {
                                -1
                            } else {
                                1
                            }
                        }
                        _ => match rng.next_u32() % 3 {
                            0 => -1,
                            1 => 0,
                            _ => 1,
                        },
                    })
                    .collect::<Vec<_>>();

                let bytes = pack_reference_ternary_values(plan, rows, cols, &values)
                    .unwrap_or_else(|error| panic!("case {case_index} pack failed: {error}"));
                let unpacked = unpack_reference_ternary_values(plan, rows, cols, &bytes)
                    .unwrap_or_else(|error| panic!("case {case_index} unpack failed: {error}"));
                assert_eq!(unpacked, values, "case {case_index} plan {plan:?}");
            }
        }
    }

    struct Lcg {
        state: u64,
    }

    impl Lcg {
        const fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        fn next_u32(&mut self) -> u32 {
            self.state = self
                .state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.state >> 32) as u32
        }
    }
}
