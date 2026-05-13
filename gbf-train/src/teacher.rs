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
    fn teacher_storage_identity(&self) -> TeacherStorageIdentity;
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
pub struct TeacherStorageIdentity {
    bytes: Vec<u8>,
}

impl TeacherStorageIdentity {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, TeacherFreezeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(TeacherFreezeError::EmptyStorageIdentity);
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
    storage_fingerprint: TeacherStorageFingerprint,
    weight_fingerprint: TeacherWeightFingerprint,
    requires_grad: bool,
}

impl<M: DenseTeacherModel> FrozenTeacher<M> {
    pub fn forward_no_grad(&self, input: M::Input) -> Result<M::Output, M::ForwardError> {
        self.snapshot.forward_no_grad(input)
    }

    #[must_use]
    pub fn weight_fingerprint(&self) -> &TeacherWeightFingerprint {
        &self.weight_fingerprint
    }

    #[must_use]
    pub fn storage_fingerprint(&self) -> &TeacherStorageFingerprint {
        &self.storage_fingerprint
    }

    #[must_use]
    pub const fn requires_grad(&self) -> bool {
        self.requires_grad
    }
}

#[derive(Debug, Default, Clone)]
pub struct TeacherFreezeGuard {
    fired: bool,
}

impl TeacherFreezeGuard {
    #[must_use]
    pub const fn new() -> Self {
        Self { fired: false }
    }

    #[must_use]
    pub const fn has_fired(&self) -> bool {
        self.fired
    }

    pub fn freeze<M>(&mut self, model: &M) -> Result<FrozenTeacher<M>, TeacherFreezeError>
    where
        M: DenseTeacherModel,
    {
        self.ensure_not_fired();
        self.fired = true;
        freeze_teacher(model)
    }

    pub fn freeze_with_logging<M>(
        &mut self,
        model: &M,
        metadata: TeacherFreezeMetadata,
        log_emitter: &TrainingLogEmitter,
    ) -> Result<FrozenTeacher<M>, TeacherFreezeError>
    where
        M: DenseTeacherModel,
    {
        self.ensure_not_fired();
        self.fired = true;
        freeze_teacher_with_logging(model, metadata, log_emitter)
    }

    fn ensure_not_fired(&self) {
        assert!(
            !self.fired,
            "teacher freeze guard fired more than once for one run"
        );
    }
}

pub fn freeze_teacher<M>(model: &M) -> Result<FrozenTeacher<M>, TeacherFreezeError>
where
    M: DenseTeacherModel,
{
    let source_fingerprint = model.teacher_weight_fingerprint();
    let source_storage = model.teacher_storage_fingerprint();
    let source_storage_identity = model.teacher_storage_identity();
    let mut snapshot = model.clone();
    snapshot.detach_for_teacher();
    let frozen_fingerprint = snapshot.teacher_weight_fingerprint();
    let frozen_storage = snapshot.teacher_storage_fingerprint();
    let frozen_storage_identity = snapshot.teacher_storage_identity();

    if frozen_fingerprint != source_fingerprint {
        return Err(TeacherFreezeError::FingerprintMismatch {
            source: source_fingerprint,
            frozen: frozen_fingerprint,
        });
    }

    if frozen_storage != source_storage {
        return Err(TeacherFreezeError::StorageFingerprintMismatch {
            source: source_storage,
            frozen: frozen_storage,
        });
    }

    if source_storage_identity == frozen_storage_identity {
        return Err(TeacherFreezeError::SharedStorage);
    }

    if snapshot.teacher_requires_grad() {
        return Err(TeacherFreezeError::FrozenSnapshotRequiresGrad);
    }

    Ok(FrozenTeacher {
        snapshot,
        storage_fingerprint: frozen_storage,
        weight_fingerprint: frozen_fingerprint,
        requires_grad: false,
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
    EmptyStorageIdentity,
    EmptyMetadataField {
        name: &'static str,
    },
    FingerprintMismatch {
        source: TeacherWeightFingerprint,
        frozen: TeacherWeightFingerprint,
    },
    StorageFingerprintMismatch {
        source: TeacherStorageFingerprint,
        frozen: TeacherStorageFingerprint,
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
            Self::EmptyStorageIdentity => f.write_str("teacher storage identity must not be empty"),
            Self::EmptyMetadataField { name } => write!(f, "{name} must not be empty"),
            Self::FingerprintMismatch { source, frozen } => write!(
                f,
                "frozen teacher fingerprint {} did not match source fingerprint {}",
                frozen.to_hex(),
                source.to_hex()
            ),
            Self::StorageFingerprintMismatch { source, frozen } => write!(
                f,
                "frozen teacher storage fingerprint {} did not match source storage fingerprint {}",
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
            | Self::EmptyStorageIdentity
            | Self::EmptyMetadataField { .. }
            | Self::FingerprintMismatch { .. }
            | Self::StorageFingerprintMismatch { .. }
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

        assert!(!teacher.requires_grad());
        assert_eq!(
            teacher.weight_fingerprint().bytes(),
            &[0, 0, 128, 63, 0, 0, 0, 64]
        );
        assert_eq!(
            teacher.storage_fingerprint(),
            &ToyTeacherModel::new([1.0, 2.0], true).teacher_storage_fingerprint()
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
    fn freeze_teacher_records_deterministic_storage_and_weight_fingerprints() {
        let first_replay = ToyTeacherModel::new([5.0, -7.0], true);
        let second_replay = ToyTeacherModel::new([5.0, -7.0], true);

        let first_teacher = freeze_teacher(&first_replay).unwrap();
        let second_teacher = freeze_teacher(&second_replay).unwrap();

        assert_eq!(
            first_teacher.weight_fingerprint(),
            second_teacher.weight_fingerprint()
        );
        assert_eq!(
            first_teacher.storage_fingerprint(),
            second_teacher.storage_fingerprint()
        );
        assert_ne!(
            first_replay.teacher_storage_identity(),
            second_replay.teacher_storage_identity()
        );
    }

    #[test]
    #[should_panic(expected = "teacher freeze guard fired more than once for one run")]
    fn teacher_freeze_guard_panics_on_second_freeze_attempt() {
        let student = ToyTeacherModel::new([1.0, 2.0], true);
        let mut guard = TeacherFreezeGuard::new();

        let _ = guard.freeze(&student).unwrap();
        assert!(guard.has_fired());

        let _ = guard.freeze(&student);
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
    fn freeze_teacher_rejects_snapshot_that_changes_storage_fingerprint_only() {
        let student = StorageFingerprintDriftModel {
            weights: vec![1.0, 2.0],
            storage_epoch: 0,
            requires_grad: true,
        };

        let error = freeze_teacher(&student).unwrap_err();

        match error {
            TeacherFreezeError::StorageFingerprintMismatch { source, frozen } => {
                assert_ne!(source, frozen);
                let mut expected_source = storage_fingerprint_bytes(&[1.0, 2.0]);
                expected_source.extend_from_slice(&0_u8.to_le_bytes());
                let mut expected_frozen = storage_fingerprint_bytes(&[1.0, 2.0]);
                expected_frozen.extend_from_slice(&1_u8.to_le_bytes());
                assert_eq!(source.bytes(), expected_source.as_slice());
                assert_eq!(frozen.bytes(), expected_frozen.as_slice());
            }
            error => panic!("expected storage fingerprint mismatch, got {error:?}"),
        }
        assert_eq!(
            student.teacher_weight_fingerprint(),
            TeacherWeightFingerprint::new(
                [1.0_f32, 2.0_f32]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
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
            TeacherStorageIdentity::new(Vec::new()).unwrap_err(),
            TeacherFreezeError::EmptyStorageIdentity
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
            TeacherStorageFingerprint::new(storage_fingerprint_bytes(&self.weights)).unwrap()
        }

        fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
            TeacherStorageIdentity::new((self.weights.as_ptr() as usize).to_le_bytes()).unwrap()
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

        fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
            self.0.teacher_storage_identity()
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

        fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
            self.0.teacher_storage_identity()
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
            TeacherStorageFingerprint::new(storage_fingerprint_bytes(&self.weights.borrow()))
                .unwrap()
        }

        fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
            TeacherStorageIdentity::new((Rc::as_ptr(&self.weights) as usize).to_le_bytes()).unwrap()
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

    #[derive(Debug, Clone)]
    struct StorageFingerprintDriftModel {
        weights: Vec<f32>,
        storage_epoch: u8,
        requires_grad: bool,
    }

    impl DenseTeacherModel for StorageFingerprintDriftModel {
        type Input = Vec<f32>;
        type Output = ToyForwardOutput;
        type ForwardError = ToyForwardError;

        fn detach_for_teacher(&mut self) {
            self.requires_grad = false;
            self.storage_epoch = self.storage_epoch.saturating_add(1);
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
            let mut bytes = storage_fingerprint_bytes(&self.weights);
            bytes.extend_from_slice(&self.storage_epoch.to_le_bytes());
            TeacherStorageFingerprint::new(bytes).unwrap()
        }

        fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
            TeacherStorageIdentity::new((self.weights.as_ptr() as usize).to_le_bytes()).unwrap()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    fn dot(weights: &[f32], input: &[f32]) -> f32 {
        weights
            .iter()
            .zip(input.iter())
            .map(|(weight, input)| weight * input)
            .sum()
    }

    fn storage_fingerprint_bytes(weights: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::from("toy-teacher:f32:rank1:");
        bytes.extend_from_slice(&weights.len().to_le_bytes());
        for weight in weights {
            bytes.extend_from_slice(&weight.to_le_bytes());
        }
        bytes
    }
}
