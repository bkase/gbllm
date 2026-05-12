//! S2 ablation build comparison artifact helpers.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorPayload, canonical_tensor_payload_hash,
};
use gbf_foundation::Hash256;

use crate::S2_LOG_TARGET;
use crate::s1::schema::{S1CanonicalJson, S1SchemaError};
use crate::s2::schema::{S2AblationReport, S2ReportWriteError, S2TensorMismatch};

/// Checkpoint-side inputs needed by the S2 H4 ablation verifier.
#[derive(Debug, Clone, Copy)]
pub struct AblationInputs<'a> {
    /// S2 seed. H4 is normative only for seed 0.
    pub seed: u64,
    /// Ternary-full Phase A checkpoint hash.
    pub s2_ternary_phase_a_checkpoint_sha: Hash256,
    /// Ablation Phase A checkpoint hash.
    pub s2_ablation_phase_a_checkpoint_sha: Hash256,
    /// Ternary-full Phase A canonical tensors.
    pub s2_ternary_tensors: &'a [CanonicalTensor],
    /// Ablation Phase A canonical tensors.
    pub s2_ablation_tensors: &'a [CanonicalTensor],
}

/// Verify H4 Phase A cleanliness using canonical tensor payload hashes.
pub fn verify_h4(inputs: AblationInputs<'_>) -> Result<S2AblationReport, H4VerifierError> {
    if inputs.seed != 0 {
        return Err(H4VerifierError::InvalidSeed {
            observed: inputs.seed,
        });
    }

    let ternary = filtered_tensor_set(
        H4Side::S2TernaryFull,
        inputs.seed,
        inputs.s2_ternary_tensors,
    )?;
    let ablation =
        filtered_tensor_set(H4Side::S2Ablation, inputs.seed, inputs.s2_ablation_tensors)?;

    let first_mismatch = first_tensor_mismatch(&ternary.tensors, &ablation.tensors);
    if let Some(mismatch) = &first_mismatch {
        let (ternary_byte, ablation_byte) =
            mismatch_bytes(&ternary.tensors, &ablation.tensors, mismatch);
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "h4_first_mismatch",
            tensor = mismatch.tensor.as_str(),
            byte_offset = mismatch.byte_offset,
            ternary_byte = u64::from(ternary_byte),
            ablation_byte = u64::from(ablation_byte),
        );
    }

    let s2_ternary_tensor_payload_sha = canonical_tensor_payload_hash(&ternary.tensors);
    let s2_ablation_tensor_payload_sha = canonical_tensor_payload_hash(&ablation.tensors);
    let report = S2AblationReport::new(
        inputs.seed,
        inputs.s2_ternary_phase_a_checkpoint_sha,
        inputs.s2_ablation_phase_a_checkpoint_sha,
        s2_ternary_tensor_payload_sha,
        s2_ablation_tensor_payload_sha,
        first_mismatch,
    )?;

    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h4_verdict",
        phase_a_eq_ablation = report.phase_a_eq_ablation,
        first_mismatch_present = report.first_mismatch.is_some(),
        first_mismatch_tensor = report
            .first_mismatch
            .as_ref()
            .map(|mismatch| mismatch.tensor.as_str())
            .unwrap_or(""),
        first_mismatch_offset = report
            .first_mismatch
            .as_ref()
            .map(|mismatch| mismatch.byte_offset)
            .unwrap_or(0),
    );

    Ok(report)
}

/// Write `s2_ablation.v1` as canonical JSON and emit comparator events.
pub fn write_ablation_report(
    path: impl AsRef<Path>,
    report: &S2AblationReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    report.validate()?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "ablation_comparator_run",
        event = "ablation_comparator_run",
        seed = report.seed,
        ternary_payload_sha = %report.s2_ternary_tensor_payload_sha,
        ablation_payload_sha = %report.s2_ablation_tensor_payload_sha,
        phase_a_eq_ablation = report.phase_a_eq_ablation,
        "s2 ablation comparator run"
    );
    if let Some(first_mismatch) = &report.first_mismatch {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "ablation_first_mismatch",
            event = "ablation_first_mismatch",
            tensor = first_mismatch.tensor.as_str(),
            byte_offset = first_mismatch.byte_offset,
            "s2 ablation first mismatch"
        );
    }
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    Ok(())
}

fn filtered_tensor_set(
    side: H4Side,
    seed: u64,
    tensors: &[CanonicalTensor],
) -> Result<FilteredTensorSet, H4VerifierError> {
    let mut filtered = Vec::new();
    let mut excluded_qat_buffers = 0_u32;
    for tensor in tensors {
        if is_qat_only_buffer(tensor) {
            excluded_qat_buffers += 1;
        } else {
            filtered.push(tensor.clone());
        }
    }

    let mut names = BTreeMap::new();
    for tensor in &filtered {
        let name = tensor.id.as_str();
        if names.insert(name, ()).is_some() {
            return Err(H4VerifierError::DuplicateTensorName {
                side,
                tensor: name.to_owned(),
            });
        }
    }

    let payload_hash = canonical_tensor_payload_hash(&filtered);
    let payload_hash_string = payload_hash.to_string();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h4_payload_extract",
        build = side.as_str(),
        seed,
        tensor_count = filtered.len() as u64,
        excluded_qat_buffers = u64::from(excluded_qat_buffers),
        canonical_tensor_payload_sha = payload_hash_string.as_str(),
    );

    Ok(FilteredTensorSet {
        tensors: filtered,
        excluded_qat_buffers,
    })
}

fn is_qat_only_buffer(tensor: &CanonicalTensor) -> bool {
    if matches!(tensor.kind, CanonicalTensorKind::TernaryScale) {
        return true;
    }

    let name = tensor.id.as_str().to_ascii_lowercase();
    name.contains("threshold")
        || name.contains("scale")
        || name.contains("fake_quant")
        || name.contains("fake-quant")
        || name.contains("fakequant")
}

fn first_tensor_mismatch(
    ternary: &[CanonicalTensor],
    ablation: &[CanonicalTensor],
) -> Option<S2TensorMismatch> {
    let ternary = tensor_map(ternary);
    let ablation = tensor_map(ablation);
    let mut tensor_names = ternary
        .keys()
        .chain(ablation.keys())
        .copied()
        .collect::<Vec<_>>();
    tensor_names.sort_unstable();
    tensor_names.dedup();

    for name in tensor_names {
        let Some(left) = ternary.get(name) else {
            return Some(S2TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            });
        };
        let Some(right) = ablation.get(name) else {
            return Some(S2TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            });
        };

        if left.kind != right.kind
            || left.layout != right.layout
            || left.payload.element_type() != right.payload.element_type()
        {
            return Some(S2TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            });
        }

        if let Some(byte_offset) = first_payload_mismatch(&left.payload, &right.payload) {
            return Some(S2TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: byte_offset as u64,
            });
        }
    }

    None
}

fn tensor_map(tensors: &[CanonicalTensor]) -> BTreeMap<&str, &CanonicalTensor> {
    tensors
        .iter()
        .map(|tensor| (tensor.id.as_str(), tensor))
        .collect()
}

fn first_payload_mismatch(
    left: &CanonicalTensorPayload,
    right: &CanonicalTensorPayload,
) -> Option<usize> {
    if left == right {
        return None;
    }
    let left_len = payload_byte_len(left);
    let right_len = payload_byte_len(right);
    let shared_len = left_len.min(right_len);

    (0..shared_len)
        .find(|&offset| payload_byte_at(left, offset) != payload_byte_at(right, offset))
        .or_else(|| (left_len != right_len).then_some(shared_len))
}

fn payload_byte_len(payload: &CanonicalTensorPayload) -> usize {
    match payload {
        CanonicalTensorPayload::F32(values) => values.len() * std::mem::size_of::<f32>(),
        CanonicalTensorPayload::I8(values) => values.len(),
        CanonicalTensorPayload::U16(values) => values.len() * std::mem::size_of::<u16>(),
    }
}

fn payload_byte_at(payload: &CanonicalTensorPayload, offset: usize) -> u8 {
    match payload {
        CanonicalTensorPayload::F32(values) => {
            values[offset / 4].to_bits().to_le_bytes()[offset % 4]
        }
        CanonicalTensorPayload::I8(values) => values[offset].to_le_bytes()[0],
        CanonicalTensorPayload::U16(values) => values[offset / 2].to_le_bytes()[offset % 2],
    }
}

fn mismatch_bytes(
    ternary: &[CanonicalTensor],
    ablation: &[CanonicalTensor],
    mismatch: &S2TensorMismatch,
) -> (u8, u8) {
    let offset = mismatch.byte_offset as usize;
    let ternary_byte = ternary
        .iter()
        .find(|tensor| tensor.id.as_str() == mismatch.tensor)
        .and_then(|tensor| {
            (offset < payload_byte_len(&tensor.payload))
                .then(|| payload_byte_at(&tensor.payload, offset))
        })
        .unwrap_or(0);
    let ablation_byte = ablation
        .iter()
        .find(|tensor| tensor.id.as_str() == mismatch.tensor)
        .and_then(|tensor| {
            (offset < payload_byte_len(&tensor.payload))
                .then(|| payload_byte_at(&tensor.payload, offset))
        })
        .unwrap_or(0);
    (ternary_byte, ablation_byte)
}

#[derive(Debug, Clone)]
struct FilteredTensorSet {
    tensors: Vec<CanonicalTensor>,
    #[allow(dead_code)]
    excluded_qat_buffers: u32,
}

/// H4 comparison side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H4Side {
    /// `s2_ternary_full` Phase A checkpoint.
    S2TernaryFull,
    /// `s2_ablation` Phase A checkpoint.
    S2Ablation,
}

impl H4Side {
    fn as_str(self) -> &'static str {
        match self {
            Self::S2TernaryFull => "s2_ternary_full",
            Self::S2Ablation => "s2_ablation",
        }
    }
}

/// Errors returned before H4 can emit an ablation report.
#[derive(Debug)]
pub enum H4VerifierError {
    /// H4 is defined only for seed 0.
    InvalidSeed {
        /// Observed seed.
        observed: u64,
    },
    /// One side contained duplicate canonical tensor names after QAT-buffer filtering.
    DuplicateTensorName {
        /// Side with the duplicate.
        side: H4Side,
        /// Duplicate tensor name.
        tensor: String,
    },
    /// Canonical schema/self-hash construction failed.
    Schema(S1SchemaError),
}

impl fmt::Display for H4VerifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSeed { observed } => {
                write!(
                    f,
                    "S2 H4 ablation comparison requires seed 0, observed {observed}"
                )
            }
            Self::DuplicateTensorName { side, tensor } => {
                write!(f, "{side:?} contains duplicate tensor {tensor:?}")
            }
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl Error for H4VerifierError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::InvalidSeed { .. } | Self::DuplicateTensorName { .. } => None,
        }
    }
}

impl From<S1SchemaError> for H4VerifierError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}
