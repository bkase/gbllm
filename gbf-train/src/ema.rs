//! Exponential moving average weights for shadow artifact export.
//!
//! This module owns the pure EMA state/update boundary and the handoff from an
//! EMA shadow snapshot into the existing artifact `ExportVisitor`. Shadow
//! compile scheduling, temp-dir lifecycle, and compiler execution are owned by
//! bd-1f7.

use std::error::Error;
use std::fmt;

use gbf_artifact::ModelArtifact;
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

use crate::export_visitor::{ArtifactExportModel, ExportVisitor, ExportVisitorError};
use crate::student::{HardTernaryStudentModel, StudentFreezeError, freeze_student_snapshot};

pub const DEFAULT_EMA_DECAY_VALUE: f32 = 0.999;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmaDecay(f32);

impl EmaDecay {
    pub fn new(value: f32) -> Result<Self, EmaExportError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(EmaExportError::InvalidDecay { value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for EmaDecay {
    fn default() -> Self {
        Self(DEFAULT_EMA_DECAY_VALUE)
    }
}

impl Serialize for EmaDecay {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f32(self.0)
    }
}

impl<'de> Deserialize<'de> for EmaDecay {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Model-owned EMA update hook.
///
/// Implementors expose only the trainable weight payloads that should
/// participate in the shadow average. Scheduling decides when to call this.
pub trait EmaUpdate: Clone {
    fn ema_update_from(&mut self, current: &Self, decay: EmaDecay) -> Result<(), EmaExportError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmaWeights<M: EmaUpdate> {
    decay: EmaDecay,
    shadow: M,
    update_count: u64,
}

impl<M: EmaUpdate> EmaWeights<M> {
    pub fn new(initial: M, decay: EmaDecay) -> Self {
        Self {
            decay,
            shadow: initial,
            update_count: 0,
        }
    }

    pub fn with_default_decay(initial: M) -> Self {
        Self::new(initial, EmaDecay::default())
    }

    #[must_use]
    pub const fn decay(&self) -> EmaDecay {
        self.decay
    }

    #[must_use]
    pub const fn update_count(&self) -> u64 {
        self.update_count
    }

    #[must_use]
    pub fn shadow(&self) -> &M {
        &self.shadow
    }

    pub fn update(&mut self, current: &M) -> Result<(), EmaExportError> {
        self.shadow.ema_update_from(current, self.decay)?;
        self.update_count = self.update_count.saturating_add(1);
        Ok(())
    }

    #[must_use]
    pub fn into_shadow(self) -> M {
        self.shadow
    }

    pub fn export_checkpoint_artifact(
        &self,
        visitor: &ExportVisitor,
    ) -> Result<EmaCheckpointArtifact, EmaExportError>
    where
        M: ArtifactExportModel + HardTernaryStudentModel,
    {
        let frozen = freeze_student_snapshot(&self.shadow)?;
        let artifact = visitor.visit_for_artifact(&frozen)?;
        Ok(EmaCheckpointArtifact {
            decay: self.decay,
            update_count: self.update_count,
            artifact,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmaCheckpointArtifact {
    pub decay: EmaDecay,
    pub update_count: u64,
    pub artifact: ModelArtifact,
}

pub fn update_ema_slice(
    shadow: &mut [f32],
    current: &[f32],
    decay: EmaDecay,
) -> Result<(), EmaExportError> {
    if shadow.len() != current.len() {
        return Err(EmaExportError::WeightLengthMismatch {
            shadow_len: shadow.len(),
            current_len: current.len(),
        });
    }

    for (index, value) in shadow.iter().copied().enumerate() {
        validate_finite_weight("shadow", index, value)?;
    }
    for (index, value) in current.iter().copied().enumerate() {
        validate_finite_weight("current", index, value)?;
    }

    let decay = decay.as_f32();
    let current_scale = 1.0 - decay;
    for (index, (shadow_value, current_value)) in shadow.iter_mut().zip(current).enumerate() {
        *shadow_value = decay.mul_add(*shadow_value, current_scale * *current_value);
        validate_finite_weight("updated", index, *shadow_value)?;
    }

    Ok(())
}

#[derive(Debug)]
pub enum EmaExportError {
    InvalidDecay {
        value: f32,
    },
    WeightLengthMismatch {
        shadow_len: usize,
        current_len: usize,
    },
    NonFiniteWeight {
        source: &'static str,
        index: usize,
        value: f32,
    },
    StudentFreeze(StudentFreezeError),
    ExportVisitor(ExportVisitorError),
}

impl fmt::Display for EmaExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecay { value } => {
                write!(f, "EMA decay must be finite and in [0, 1], got {value}")
            }
            Self::WeightLengthMismatch {
                shadow_len,
                current_len,
            } => write!(
                f,
                "EMA shadow/current weight lengths differ: shadow={shadow_len}, current={current_len}"
            ),
            Self::NonFiniteWeight {
                source,
                index,
                value,
            } => write!(
                f,
                "EMA {source} weight at index {index} must be finite, got {value}"
            ),
            Self::StudentFreeze(error) => write!(f, "{error}"),
            Self::ExportVisitor(error) => write!(f, "{error}"),
        }
    }
}

impl Error for EmaExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::StudentFreeze(error) => Some(error),
            Self::ExportVisitor(error) => Some(error),
            Self::InvalidDecay { .. }
            | Self::WeightLengthMismatch { .. }
            | Self::NonFiniteWeight { .. } => None,
        }
    }
}

impl From<StudentFreezeError> for EmaExportError {
    fn from(error: StudentFreezeError) -> Self {
        Self::StudentFreeze(error)
    }
}

impl From<ExportVisitorError> for EmaExportError {
    fn from(error: ExportVisitorError) -> Self {
        Self::ExportVisitor(error)
    }
}

fn validate_finite_weight(
    source: &'static str,
    index: usize,
    value: f32,
) -> Result<(), EmaExportError> {
    if !value.is_finite() {
        return Err(EmaExportError::NonFiniteWeight {
            source,
            index,
            value,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gbf_artifact::ids::ArtifactPath;
    use gbf_artifact::{
        CanonicalTensor, Dtype, PayloadRole, QuantSpec_S3, WeightQuant, canonical_payload_sha,
    };
    use gbf_foundation::sha256;

    use super::*;
    use crate::student::{StudentStorageFingerprint, StudentWeightFingerprint};

    #[test]
    fn ema_decay_defaults_to_shadow_compile_value_and_validates_bounds() {
        assert_eq!(EmaDecay::default().as_f32(), DEFAULT_EMA_DECAY_VALUE);
        assert_eq!(EmaDecay::new(0.25).unwrap().as_f32(), 0.25);
        let decoded: EmaDecay = serde_json::from_str("0.25").unwrap();
        assert_eq!(decoded.as_f32(), 0.25);
        assert!(serde_json::from_str::<EmaDecay>("1.25").is_err());
        assert!(serde_json::from_str::<EmaDecay>("-0.25").is_err());
        assert!(matches!(
            EmaDecay::new(f32::NAN),
            Err(EmaExportError::InvalidDecay { .. })
        ));
        assert!(matches!(
            EmaDecay::new(1.25),
            Err(EmaExportError::InvalidDecay { .. })
        ));
    }

    #[test]
    fn ema_decay_one_preserves_initial_weights() {
        let initial = ToyEmaStudent::new(vec![1.0, 3.0], true);
        let current = ToyEmaStudent::new(vec![9.0, 11.0], true);
        let mut ema = EmaWeights::new(initial.clone(), EmaDecay::new(1.0).unwrap());

        ema.update(&current).unwrap();

        assert_eq!(ema.shadow().weights, initial.weights);
        assert_eq!(ema.update_count(), 1);
    }

    #[test]
    fn ema_decay_zero_tracks_current_weights_exactly() {
        let initial = ToyEmaStudent::new(vec![1.0, 3.0], true);
        let current = ToyEmaStudent::new(vec![9.0, 11.0], true);
        let mut ema = EmaWeights::new(initial, EmaDecay::new(0.0).unwrap());

        ema.update(&current).unwrap();

        assert_eq!(ema.shadow().weights, current.weights);
    }

    #[test]
    fn ema_update_uses_weighted_average_and_rejects_bad_payloads() {
        let mut shadow = vec![1.0, 3.0];
        update_ema_slice(&mut shadow, &[3.0, 7.0], EmaDecay::new(0.5).unwrap()).unwrap();
        assert_eq!(shadow, vec![2.0, 5.0]);

        let mut short_shadow = vec![1.0];
        assert!(matches!(
            update_ema_slice(&mut short_shadow, &[1.0, 2.0], EmaDecay::new(0.5).unwrap()),
            Err(EmaExportError::WeightLengthMismatch { .. })
        ));

        let mut non_finite = vec![1.0];
        assert!(matches!(
            update_ema_slice(
                &mut non_finite,
                &[f32::INFINITY],
                EmaDecay::new(0.5).unwrap()
            ),
            Err(EmaExportError::NonFiniteWeight {
                source: "current",
                index: 0,
                ..
            })
        ));
    }

    #[test]
    fn ema_export_uses_export_visitor_and_produces_valid_artifact() {
        let mut ema = EmaWeights::new(
            ToyEmaStudent::new(vec![1.0, 3.0], true),
            EmaDecay::new(0.5).unwrap(),
        );
        ema.update(&ToyEmaStudent::new(vec![3.0, 7.0], true))
            .unwrap();

        let exported = ema
            .export_checkpoint_artifact(&ExportVisitor::pinned())
            .unwrap();

        assert_eq!(exported.update_count, 1);
        assert_eq!(exported.decay.as_f32(), 0.5);
        assert_eq!(exported.artifact.core.tensors.len(), 1);
        assert_eq!(
            exported.artifact.core.tensors[0].payload_sha,
            canonical_payload_sha(weight_bytes(&[2.0, 5.0]))
        );
        assert!(exported.artifact.validate().is_ok());
    }

    #[derive(Debug, Clone, PartialEq)]
    struct ToyEmaStudent {
        weights: Vec<f32>,
        requires_grad: bool,
    }

    impl ToyEmaStudent {
        fn new(weights: Vec<f32>, requires_grad: bool) -> Self {
            Self {
                weights,
                requires_grad,
            }
        }
    }

    impl EmaUpdate for ToyEmaStudent {
        fn ema_update_from(
            &mut self,
            current: &Self,
            decay: EmaDecay,
        ) -> Result<(), EmaExportError> {
            update_ema_slice(&mut self.weights, &current.weights, decay)
        }
    }

    impl HardTernaryStudentModel for ToyEmaStudent {
        fn detach_for_student(&mut self) {
            self.requires_grad = false;
        }

        fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
            StudentWeightFingerprint::new(weight_bytes(&self.weights)).unwrap()
        }

        fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
            let mut bytes = Vec::from("toy-ema-student:f32:");
            bytes.extend_from_slice(&self.weights.len().to_le_bytes());
            bytes.extend_from_slice(&weight_bytes(&self.weights));
            StudentStorageFingerprint::new(bytes).unwrap()
        }

        fn student_storage_identity(&self) -> usize {
            self.weights.as_ptr() as usize
        }

        fn student_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    impl ArtifactExportModel for ToyEmaStudent {
        fn artifact_seed(&self) -> u64 {
            42
        }

        fn artifact_semantic_core_hash(
            &self,
            frozen: &crate::student::FrozenStudent<Self>,
        ) -> gbf_foundation::Hash256 {
            sha256(frozen.weight_fingerprint().bytes())
        }

        fn artifact_quant_spec(&self) -> QuantSpec_S3 {
            QuantSpec_S3::new(BTreeMap::from([(weight_tensor_id(), WeightQuant::Fp32)]))
        }

        fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError> {
            let payload = weight_bytes(&self.weights);
            Ok(vec![
                CanonicalTensor::new(
                    weight_tensor_id(),
                    Dtype::Fp32,
                    vec![
                        u32::try_from(self.weights.len())
                            .map_err(|_| ExportVisitorError::model("too many weights"))?,
                    ],
                    canonical_payload_sha(&payload),
                    PayloadRole::DeployableWeight,
                )
                .map_err(|error| ExportVisitorError::model(error.to_string()))?,
            ])
        }
    }

    fn weight_tensor_id() -> ArtifactPath {
        ArtifactPath::new("ema.weight").unwrap()
    }

    fn weight_bytes(weights: &[f32]) -> Vec<u8> {
        weights
            .iter()
            .flat_map(|weight| weight.to_le_bytes())
            .collect()
    }
}
