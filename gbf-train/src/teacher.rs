//! Dense teacher freezing boundary.
//!
//! The freezer owns the pre-export invariant for Phase A: clone the current
//! training model, detach the clone from optimizer/autodiff state, and keep the
//! frozen snapshot immutable for distillation. Exporting that snapshot as a
//! `ReferenceModelBundle` is a later artifact boundary.

use std::error::Error;
use std::fmt;
use std::time::Instant;

use crate::logging::{LoggingEventError, TeacherFreezeEvent, TrainingLogEmitter};

pub trait DenseTeacherModel: Clone {
    type Input;
    type Output;
    type ForwardError;

    fn detach_for_teacher(&mut self);
    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError>;
    fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint;
    fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint;
    fn teacher_requires_grad(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TeacherWeightFingerprint {
    bytes: Vec<u8>,
}

impl TeacherWeightFingerprint {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, TeacherFreezeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(TeacherFreezeError::EmptyFingerprint);
        }

        Ok(Self { bytes })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(self.bytes.len() * 2);
        for byte in &self.bytes {
            use std::fmt::Write as _;
            write!(&mut hex, "{byte:02x}").expect("writing to a String should not fail");
        }
        hex
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TeacherStorageFingerprint {
    bytes: Vec<u8>,
}

impl TeacherStorageFingerprint {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, TeacherFreezeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(TeacherFreezeError::EmptyStorageFingerprint);
        }

        Ok(Self { bytes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeacherFreezeMetadata {
    step: u64,
    teacher_checkpoint_id: String,
}

impl TeacherFreezeMetadata {
    pub fn new(
        step: u64,
        teacher_checkpoint_id: impl Into<String>,
    ) -> Result<Self, TeacherFreezeError> {
        let teacher_checkpoint_id = teacher_checkpoint_id.into();
        if teacher_checkpoint_id.trim().is_empty() {
            return Err(TeacherFreezeError::EmptyMetadataField {
                name: "teacher_checkpoint_id",
            });
        }

        Ok(Self {
            step,
            teacher_checkpoint_id,
        })
    }

    #[must_use]
    pub const fn step(&self) -> u64 {
        self.step
    }

    #[must_use]
    pub fn teacher_checkpoint_id(&self) -> &str {
        &self.teacher_checkpoint_id
    }
}

#[derive(Debug, Clone)]
pub struct FrozenTeacher<M: DenseTeacherModel> {
    snapshot: M,
    weight_fingerprint: TeacherWeightFingerprint,
}

impl<M: DenseTeacherModel> FrozenTeacher<M> {
    pub fn forward_no_grad(&self, input: M::Input) -> Result<M::Output, M::ForwardError> {
        self.snapshot.forward_no_grad(input)
    }

    #[must_use]
    pub fn weight_fingerprint(&self) -> &TeacherWeightFingerprint {
        &self.weight_fingerprint
    }
}

pub fn freeze_teacher<M>(model: &M) -> Result<FrozenTeacher<M>, TeacherFreezeError>
where
    M: DenseTeacherModel,
{
    let source_fingerprint = model.teacher_weight_fingerprint();
    let source_storage = model.teacher_storage_fingerprint();
    let mut snapshot = model.clone();
    snapshot.detach_for_teacher();
    let frozen_fingerprint = snapshot.teacher_weight_fingerprint();
    let frozen_storage = snapshot.teacher_storage_fingerprint();

    if frozen_fingerprint != source_fingerprint {
        return Err(TeacherFreezeError::FingerprintMismatch {
            source: source_fingerprint,
            frozen: frozen_fingerprint,
        });
    }

    if frozen_storage == source_storage {
        return Err(TeacherFreezeError::SharedStorage);
    }

    if snapshot.teacher_requires_grad() {
        return Err(TeacherFreezeError::FrozenSnapshotRequiresGrad);
    }

    Ok(FrozenTeacher {
        snapshot,
        weight_fingerprint: frozen_fingerprint,
    })
}

pub fn freeze_teacher_with_logging<M>(
    model: &M,
    metadata: TeacherFreezeMetadata,
    log_emitter: &TrainingLogEmitter,
) -> Result<FrozenTeacher<M>, TeacherFreezeError>
where
    M: DenseTeacherModel,
{
    let started_at = Instant::now();
    let frozen_teacher = freeze_teacher(model)?;
    let weight_fingerprint = frozen_teacher.weight_fingerprint().to_hex();
    let duration_ms = started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);

    log_emitter.teacher_freeze(&TeacherFreezeEvent {
        step: metadata.step(),
        teacher_checkpoint_id: metadata.teacher_checkpoint_id().to_owned(),
        source_weight_fingerprint: weight_fingerprint.clone(),
        frozen_weight_fingerprint: weight_fingerprint,
        weights_match: true,
        duration_ms,
    })?;

    Ok(frozen_teacher)
}

#[derive(Debug, Clone, PartialEq)]
pub enum TeacherFreezeError {
    EmptyFingerprint,
    EmptyStorageFingerprint,
    EmptyMetadataField {
        name: &'static str,
    },
    FingerprintMismatch {
        source: TeacherWeightFingerprint,
        frozen: TeacherWeightFingerprint,
    },
    SharedStorage,
    FrozenSnapshotRequiresGrad,
    Logging(LoggingEventError),
}

impl fmt::Display for TeacherFreezeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFingerprint => f.write_str("teacher weight fingerprint must not be empty"),
            Self::EmptyStorageFingerprint => {
                f.write_str("teacher storage fingerprint must not be empty")
            }
            Self::EmptyMetadataField { name } => write!(f, "{name} must not be empty"),
            Self::FingerprintMismatch { source, frozen } => write!(
                f,
                "frozen teacher fingerprint {} did not match source fingerprint {}",
                frozen.to_hex(),
                source.to_hex()
            ),
            Self::SharedStorage => {
                f.write_str("frozen teacher snapshot must not share source model storage")
            }
            Self::FrozenSnapshotRequiresGrad => {
                f.write_str("frozen teacher snapshot still requires gradients")
            }
            Self::Logging(error) => write!(f, "failed to log teacher freeze: {error}"),
        }
    }
}

impl Error for TeacherFreezeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Logging(error) => Some(error),
            Self::EmptyFingerprint
            | Self::EmptyStorageFingerprint
            | Self::EmptyMetadataField { .. }
            | Self::FingerprintMismatch { .. }
            | Self::SharedStorage
            | Self::FrozenSnapshotRequiresGrad => None,
        }
    }
}

impl From<LoggingEventError> for TeacherFreezeError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{TestEventCollector, TestEventKind, TestFieldValue};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn freeze_teacher_clones_weights_independent_of_student_mutation() {
        let mut student = ToyTeacherModel::new([1.0, 2.0], true);
        let teacher = freeze_teacher(&student).unwrap();

        student.weights[0] = 10.0;
        student.requires_grad = true;

        assert_eq!(
            teacher.weight_fingerprint().bytes(),
            &[0, 0, 128, 63, 0, 0, 0, 64]
        );
        assert_eq!(
            teacher.forward_no_grad(vec![2.0, 3.0]).unwrap(),
            ToyForwardOutput {
                value: 8.0,
                requires_grad: false,
            }
        );
        assert_eq!(student.forward_with_grad(vec![2.0, 3.0]).value, 26.0);
    }

    #[test]
    fn frozen_teacher_forward_no_grad_detaches_snapshot() {
        let student = ToyTeacherModel::new([3.0, -1.0], true);
        let teacher = freeze_teacher(&student).unwrap();

        let output = teacher.forward_no_grad(vec![2.0, 4.0]).unwrap();

        assert_eq!(output.value, 2.0);
        assert!(!output.requires_grad);
        assert!(student.forward_with_grad(vec![2.0, 4.0]).requires_grad);
    }

    #[test]
    fn freeze_teacher_rejects_snapshot_that_changes_weights_during_detach() {
        let student = BuggyDetachModel(ToyTeacherModel::new([1.0, 2.0], true));

        assert!(matches!(
            freeze_teacher(&student).unwrap_err(),
            TeacherFreezeError::FingerprintMismatch { .. }
        ));
    }

    #[test]
    fn freeze_teacher_rejects_snapshot_that_shares_student_storage() {
        let student = SharedCloneModel {
            weights: Rc::new(RefCell::new(vec![1.0, 2.0])),
            requires_grad: false,
        };

        assert_eq!(
            freeze_teacher(&student).unwrap_err(),
            TeacherFreezeError::SharedStorage
        );
    }

    #[test]
    fn freeze_teacher_rejects_snapshot_that_still_requires_grad() {
        let student = NoDetachModel(ToyTeacherModel::new([1.0, 2.0], true));

        assert_eq!(
            freeze_teacher(&student).unwrap_err(),
            TeacherFreezeError::FrozenSnapshotRequiresGrad
        );
    }

    #[test]
    fn freeze_teacher_with_logging_emits_canonical_teacher_freeze_event() {
        let student = ToyTeacherModel::new([1.0, 2.0], true);
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
        let metadata = TeacherFreezeMetadata::new(10, "teacher-10").unwrap();

        let teacher = freeze_teacher_with_logging(&student, metadata, &emitter).unwrap();

        assert_eq!(
            teacher.weight_fingerprint(),
            &student.teacher_weight_fingerprint()
        );
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::TeacherFreeze);
        assert_eq!(events[0].field("step"), Some(&TestFieldValue::U64(10)));
        assert_eq!(
            events[0].field("teacher_checkpoint_id"),
            Some(&TestFieldValue::String("teacher-10".to_owned()))
        );
        assert_eq!(
            events[0].field("source_weight_fingerprint"),
            Some(&TestFieldValue::String(
                student.teacher_weight_fingerprint().to_hex()
            ))
        );
        assert_eq!(
            events[0].field("frozen_weight_fingerprint"),
            Some(&TestFieldValue::String(
                student.teacher_weight_fingerprint().to_hex()
            ))
        );
        assert_eq!(
            events[0].field("weights_match"),
            Some(&TestFieldValue::Bool(true))
        );
    }

    #[test]
    fn freeze_teacher_validates_metadata_and_fingerprint_inputs() {
        assert_eq!(
            TeacherWeightFingerprint::new(Vec::new()).unwrap_err(),
            TeacherFreezeError::EmptyFingerprint
        );
        assert_eq!(
            TeacherStorageFingerprint::new(Vec::new()).unwrap_err(),
            TeacherFreezeError::EmptyStorageFingerprint
        );
        assert_eq!(
            TeacherFreezeMetadata::new(0, "   ").unwrap_err(),
            TeacherFreezeError::EmptyMetadataField {
                name: "teacher_checkpoint_id",
            }
        );
    }

    #[derive(Debug, Clone)]
    struct ToyTeacherModel {
        weights: Vec<f32>,
        requires_grad: bool,
    }

    impl ToyTeacherModel {
        fn new<const N: usize>(weights: [f32; N], requires_grad: bool) -> Self {
            Self {
                weights: weights.to_vec(),
                requires_grad,
            }
        }

        fn forward_with_grad(&self, input: Vec<f32>) -> ToyForwardOutput {
            ToyForwardOutput {
                value: dot(&self.weights, &input),
                requires_grad: self.requires_grad,
            }
        }
    }

    impl DenseTeacherModel for ToyTeacherModel {
        type Input = Vec<f32>;
        type Output = ToyForwardOutput;
        type ForwardError = ToyForwardError;

        fn detach_for_teacher(&mut self) {
            self.requires_grad = false;
        }

        fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
            Ok(ToyForwardOutput {
                value: dot(&self.weights, &input),
                requires_grad: false,
            })
        }

        fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
            let bytes = self
                .weights
                .iter()
                .flat_map(|weight| weight.to_le_bytes())
                .collect::<Vec<_>>();
            TeacherWeightFingerprint::new(bytes).unwrap()
        }

        fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
            TeacherStorageFingerprint::new((self.weights.as_ptr() as usize).to_le_bytes()).unwrap()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    #[derive(Debug, Clone)]
    struct BuggyDetachModel(ToyTeacherModel);

    impl DenseTeacherModel for BuggyDetachModel {
        type Input = Vec<f32>;
        type Output = ToyForwardOutput;
        type ForwardError = ToyForwardError;

        fn detach_for_teacher(&mut self) {
            self.0.detach_for_teacher();
            self.0.weights[0] += 1.0;
        }

        fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
            self.0.forward_no_grad(input)
        }

        fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
            self.0.teacher_weight_fingerprint()
        }

        fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
            self.0.teacher_storage_fingerprint()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.0.teacher_requires_grad()
        }
    }

    #[derive(Debug, Clone)]
    struct NoDetachModel(ToyTeacherModel);

    impl DenseTeacherModel for NoDetachModel {
        type Input = Vec<f32>;
        type Output = ToyForwardOutput;
        type ForwardError = ToyForwardError;

        fn detach_for_teacher(&mut self) {}

        fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
            self.0.forward_no_grad(input)
        }

        fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
            self.0.teacher_weight_fingerprint()
        }

        fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
            self.0.teacher_storage_fingerprint()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.0.teacher_requires_grad()
        }
    }

    #[derive(Debug, Clone)]
    struct SharedCloneModel {
        weights: Rc<RefCell<Vec<f32>>>,
        requires_grad: bool,
    }

    impl DenseTeacherModel for SharedCloneModel {
        type Input = Vec<f32>;
        type Output = ToyForwardOutput;
        type ForwardError = ToyForwardError;

        fn detach_for_teacher(&mut self) {
            self.requires_grad = false;
        }

        fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
            Ok(ToyForwardOutput {
                value: dot(&self.weights.borrow(), &input),
                requires_grad: false,
            })
        }

        fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
            let bytes = self
                .weights
                .borrow()
                .iter()
                .flat_map(|weight| weight.to_le_bytes())
                .collect::<Vec<_>>();
            TeacherWeightFingerprint::new(bytes).unwrap()
        }

        fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
            TeacherStorageFingerprint::new((Rc::as_ptr(&self.weights) as usize).to_le_bytes())
                .unwrap()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct ToyForwardOutput {
        value: f32,
        requires_grad: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ToyForwardError;

    fn dot(weights: &[f32], input: &[f32]) -> f32 {
        weights
            .iter()
            .zip(input.iter())
            .map(|(weight, input)| weight * input)
            .sum()
    }
}
