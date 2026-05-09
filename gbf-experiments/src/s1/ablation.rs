//! Ablation build comparison plumbing for S1.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorPayload, canonical_tensor_payload_hash,
};
use gbf_foundation::Hash256;

use crate::s1::logging::{
    AblationCompareStartEvent, AblationCompleteEvent, AblationMetadataCheckFailEvent,
    AblationMismatchEvent, AblationTensorCompareEvent, LoggingEventError, S1LogEmitter,
};
use crate::s1::schema::{
    AblationReport, CheckpointMetadata, S1BuildKind, S1SchemaError, TensorMismatch,
};

/// Checkpoint-side inputs needed by the S1 ablation comparator.
///
/// The current S1 checkpoint bytes are metadata-free SafeTensors, so callers
/// provide already materialized trainable [`CanonicalTensor`] values alongside
/// the metadata sidecar and checkpoint file hash. Production SafeTensors
/// loading/lowering can use this same comparator once that reader owns tensor
/// kind reconstruction.
#[derive(Debug, Clone, Copy)]
pub struct AblationCheckpoint<'a> {
    /// Checkpoint metadata sidecar.
    pub metadata: &'a CheckpointMetadata,
    /// SHA-256 of the checkpoint SafeTensors bytes.
    pub checkpoint_sha: Hash256,
    /// Trainable tensors extracted from the checkpoint.
    pub tensors: &'a [CanonicalTensor],
}

/// Compare seed-0 Phase A and ablation checkpoint tensor payloads.
pub fn compare(
    phase_a: AblationCheckpoint<'_>,
    ablation: AblationCheckpoint<'_>,
) -> Result<AblationReport, AblationError> {
    let emitter = S1LogEmitter::new();
    let ablation_span = emitter.ablation_span(phase_a.metadata.seed);
    let _ablation_guard = ablation_span.enter();
    emitter.ablation_compare_start(AblationCompareStartEvent {
        seed: phase_a.metadata.seed,
    })?;

    if let Err(error) = validate_metadata(phase_a.metadata, ablation.metadata) {
        emitter.ablation_metadata_check_fail(&AblationMetadataCheckFailEvent {
            seed: phase_a.metadata.seed,
            reason: error.to_string(),
        })?;
        return Err(error);
    }

    let phase_a_tensors = tensor_map(phase_a.tensors, AblationSide::PhaseA)?;
    let ablation_tensors = tensor_map(ablation.tensors, AblationSide::Ablation)?;
    let first_mismatch = first_tensor_mismatch(
        phase_a.metadata.seed,
        &emitter,
        &phase_a_tensors,
        &ablation_tensors,
    )?;
    if let Some(mismatch) = &first_mismatch {
        emitter.ablation_mismatch(&AblationMismatchEvent {
            seed: phase_a.metadata.seed,
            tensor_name: mismatch.tensor.clone(),
            byte_offset: mismatch.byte_offset,
        })?;
    }
    let phase_a_tensor_payload_sha = canonical_tensor_payload_hash(phase_a.tensors);
    let ablation_tensor_payload_sha = canonical_tensor_payload_hash(ablation.tensors);
    let phase_a_eq_ablation =
        phase_a_tensor_payload_sha == ablation_tensor_payload_sha && first_mismatch.is_none();

    let report = AblationReport {
        schema: "s1_ablation.v1".to_owned(),
        seed: phase_a.metadata.seed,
        phase_a_checkpoint_sha: phase_a.checkpoint_sha,
        ablation_checkpoint_sha: ablation.checkpoint_sha,
        phase_a_tensor_payload_sha,
        ablation_tensor_payload_sha,
        phase_a_eq_ablation,
        first_mismatch,
        ablation_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?;
    emitter.ablation_complete(&AblationCompleteEvent {
        seed: report.seed,
        phase_a_eq_ablation: report.phase_a_eq_ablation,
        ablation_self_hash: report.ablation_self_hash.to_string(),
    })?;
    Ok(report)
}

fn validate_metadata(
    phase_a: &CheckpointMetadata,
    ablation: &CheckpointMetadata,
) -> Result<(), AblationError> {
    if phase_a.build_kind != S1BuildKind::PhaseA {
        return Err(AblationError::InvalidBuildKind {
            side: AblationSide::PhaseA,
            expected: S1BuildKind::PhaseA,
            observed: phase_a.build_kind,
        });
    }
    if ablation.build_kind != S1BuildKind::Ablation {
        return Err(AblationError::InvalidBuildKind {
            side: AblationSide::Ablation,
            expected: S1BuildKind::Ablation,
            observed: ablation.build_kind,
        });
    }

    require_seed_zero(AblationSide::PhaseA, phase_a.seed)?;
    require_seed_zero(AblationSide::Ablation, ablation.seed)?;
    compare_metadata_field("seed", phase_a.seed, ablation.seed)?;
    compare_metadata_field(
        "corpus_train_sha",
        phase_a.corpus_train_sha,
        ablation.corpus_train_sha,
    )?;
    compare_metadata_field(
        "corpus_val_sha",
        phase_a.corpus_val_sha,
        ablation.corpus_val_sha,
    )?;
    compare_metadata_field(
        "model_config_hash",
        phase_a.model_config_hash,
        ablation.model_config_hash,
    )?;
    compare_metadata_field(
        "train_config_hash",
        phase_a.train_config_hash,
        ablation.train_config_hash,
    )?;
    compare_metadata_field(
        "device_profile_hash",
        phase_a.device_profile_hash,
        ablation.device_profile_hash,
    )?;
    compare_metadata_field(
        "rng_stream_def_hash",
        phase_a.rng_stream_def_hash,
        ablation.rng_stream_def_hash,
    )?;

    Ok(())
}

fn require_seed_zero(side: AblationSide, observed: u64) -> Result<(), AblationError> {
    if observed == 0 {
        Ok(())
    } else {
        Err(AblationError::InvalidSeed { side, observed })
    }
}

fn compare_metadata_field<T>(
    field: &'static str,
    phase_a: T,
    ablation: T,
) -> Result<(), AblationError>
where
    T: Eq + fmt::Display,
{
    if phase_a == ablation {
        Ok(())
    } else {
        Err(AblationError::MetadataMismatch {
            field,
            phase_a: phase_a.to_string(),
            ablation: ablation.to_string(),
        })
    }
}

fn tensor_map(
    tensors: &[CanonicalTensor],
    side: AblationSide,
) -> Result<BTreeMap<&str, &CanonicalTensor>, AblationError> {
    let mut map = BTreeMap::new();
    for tensor in tensors {
        let name = tensor.id.as_str();
        if map.insert(name, tensor).is_some() {
            return Err(AblationError::DuplicateTensorName {
                side,
                tensor: name.to_owned(),
            });
        }
    }
    Ok(map)
}

fn first_tensor_mismatch(
    seed: u64,
    emitter: &S1LogEmitter,
    phase_a: &BTreeMap<&str, &CanonicalTensor>,
    ablation: &BTreeMap<&str, &CanonicalTensor>,
) -> Result<Option<TensorMismatch>, AblationError> {
    let mut tensor_names = phase_a
        .keys()
        .chain(ablation.keys())
        .copied()
        .collect::<Vec<_>>();
    tensor_names.sort_unstable();
    tensor_names.dedup();

    for name in tensor_names {
        emitter.ablation_tensor_compare(&AblationTensorCompareEvent {
            seed,
            tensor_name: name.to_owned(),
        })?;
        let Some(left) = phase_a.get(name) else {
            return Ok(Some(TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            }));
        };
        let Some(right) = ablation.get(name) else {
            return Ok(Some(TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            }));
        };

        if left.kind != right.kind
            || left.layout != right.layout
            || left.payload.element_type() != right.payload.element_type()
        {
            return Ok(Some(TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: 0,
            }));
        }

        let first_diff = first_payload_mismatch(&left.payload, &right.payload);
        if let Some(byte_offset) = first_diff {
            return Ok(Some(TensorMismatch {
                tensor: name.to_owned(),
                byte_offset: byte_offset as u64,
            }));
        }
    }

    Ok(None)
}

fn first_payload_mismatch(
    left: &CanonicalTensorPayload,
    right: &CanonicalTensorPayload,
) -> Option<usize> {
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

/// Which side of the ablation comparison produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AblationSide {
    /// Phase A checkpoint.
    PhaseA,
    /// Ablation checkpoint.
    Ablation,
}

impl fmt::Display for AblationSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PhaseA => f.write_str("phase_a"),
            Self::Ablation => f.write_str("ablation"),
        }
    }
}

/// Errors returned before an ablation report can be emitted.
#[derive(Debug)]
pub enum AblationError {
    /// A checkpoint metadata sidecar had the wrong build kind for its side.
    InvalidBuildKind {
        /// Side being validated.
        side: AblationSide,
        /// Expected build kind.
        expected: S1BuildKind,
        /// Observed build kind.
        observed: S1BuildKind,
    },
    /// Ablation reports are normative seed-0 comparisons only.
    InvalidSeed {
        /// Side being validated.
        side: AblationSide,
        /// Observed seed.
        observed: u64,
    },
    /// Phase A and ablation metadata differed on a field that must be shared.
    MetadataMismatch {
        /// Field name.
        field: &'static str,
        /// Phase A value.
        phase_a: String,
        /// Ablation value.
        ablation: String,
    },
    /// One side contained duplicate canonical tensor names.
    DuplicateTensorName {
        /// Side with the duplicate.
        side: AblationSide,
        /// Duplicate tensor name.
        tensor: String,
    },
    /// Structured logging event construction failed.
    Logging(LoggingEventError),
    /// Canonical schema/self-hash construction failed.
    Schema(S1SchemaError),
}

impl fmt::Display for AblationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBuildKind {
                side,
                expected,
                observed,
            } => write!(
                f,
                "{side} checkpoint build_kind must be {expected:?}, observed {observed:?}"
            ),
            Self::InvalidSeed { side, observed } => {
                write!(f, "{side} checkpoint seed must be 0, observed {observed}")
            }
            Self::MetadataMismatch {
                field,
                phase_a,
                ablation,
            } => write!(
                f,
                "ablation metadata mismatch on {field}: phase_a={phase_a}, ablation={ablation}"
            ),
            Self::DuplicateTensorName { side, tensor } => {
                write!(f, "{side} checkpoint contains duplicate tensor {tensor:?}")
            }
            Self::Logging(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl Error for AblationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::InvalidBuildKind { .. }
            | Self::InvalidSeed { .. }
            | Self::MetadataMismatch { .. }
            | Self::DuplicateTensorName { .. } => None,
        }
    }
}

impl From<S1SchemaError> for AblationError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for AblationError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}
