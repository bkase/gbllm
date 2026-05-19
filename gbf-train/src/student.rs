//! Hard-ternary student freezing boundary for S3 artifact export.
//!
//! The freezer owns the pre-export invariant for S3: clone the current student
//! model after step 10000, detach the clone from optimizer/autodiff state, and
//! verify that detaching preserved payload bytes while breaking storage aliasing.

use std::error::Error;
use std::fmt;

use crate::logging::{LoggingEventError, StudentFreezeEvent, TrainingLogEmitter};

/// Phase-log/training-log boundary step for the S3 student snapshot.
pub const STUDENT_FREEZE_EVENT_STEP: u64 = 10_001;

/// Model contract required by `freeze_student_as_artifact`.
pub trait HardTernaryStudentModel: Clone {
    /// Detach the cloned student from optimizer/autodiff state.
    fn detach_for_student(&mut self);
    /// Stable fingerprint over student trainable weight payloads.
    fn student_weight_fingerprint(&self) -> StudentWeightFingerprint;
    /// Stable fingerprint over frozen-student storage contents/provenance.
    fn student_storage_fingerprint(&self) -> StudentStorageFingerprint;
    /// Process-local identity for detecting source/frozen storage aliasing.
    fn student_storage_identity(&self) -> usize;
    /// Whether any student parameter in this model still requires gradients.
    fn student_requires_grad(&self) -> bool;
}

/// Stable fingerprint over student weight payloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StudentWeightFingerprint {
    bytes: Vec<u8>,
}

impl StudentWeightFingerprint {
    /// Construct a non-empty student weight fingerprint.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, StudentFreezeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(StudentFreezeError::EmptyFingerprint);
        }

        Ok(Self { bytes })
    }

    /// Raw fingerprint bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Lowercase hex rendering used by structured logs.
    #[must_use]
    pub fn to_hex(&self) -> String {
        bytes_to_hex(&self.bytes)
    }
}

/// Stable fingerprint over student storage contents/provenance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StudentStorageFingerprint {
    bytes: Vec<u8>,
}

impl StudentStorageFingerprint {
    /// Construct a non-empty student storage fingerprint.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, StudentFreezeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(StudentFreezeError::EmptyStorageFingerprint);
        }

        Ok(Self { bytes })
    }

    /// Raw fingerprint bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Lowercase hex rendering used by structured logs.
    #[must_use]
    pub fn to_hex(&self) -> String {
        bytes_to_hex(&self.bytes)
    }
}

/// Frozen student snapshot and its verified fingerprints.
#[derive(Debug, Clone)]
pub struct FrozenStudent<M: HardTernaryStudentModel> {
    snapshot: M,
    storage_fingerprint: StudentStorageFingerprint,
    weight_fingerprint: StudentWeightFingerprint,
    requires_grad: bool,
}

impl<M: HardTernaryStudentModel> FrozenStudent<M> {
    /// Borrow the detached snapshot for later artifact export.
    #[must_use]
    pub fn snapshot(&self) -> &M {
        &self.snapshot
    }

    /// Consume the wrapper and return the detached snapshot.
    #[must_use]
    pub fn into_snapshot(self) -> M {
        self.snapshot
    }

    /// Frozen student weight fingerprint.
    #[must_use]
    pub fn weight_fingerprint(&self) -> &StudentWeightFingerprint {
        &self.weight_fingerprint
    }

    /// Frozen student storage fingerprint.
    #[must_use]
    pub fn storage_fingerprint(&self) -> &StudentStorageFingerprint {
        &self.storage_fingerprint
    }

    /// Frozen snapshots must never require gradients.
    #[must_use]
    pub const fn requires_grad(&self) -> bool {
        self.requires_grad
    }
}

/// Per-run single-fire guard for the S3 student freeze boundary.
#[derive(Debug, Default, Clone)]
pub struct StudentFreezeGuard {
    fired: bool,
}

impl StudentFreezeGuard {
    /// Create an unfired student freeze guard.
    #[must_use]
    pub const fn new() -> Self {
        Self { fired: false }
    }

    /// True after this guard has frozen a student snapshot.
    #[must_use]
    pub const fn has_fired(&self) -> bool {
        self.fired
    }

    /// Freeze the student once, emitting the canonical S3 tracing event.
    pub fn freeze<M>(&mut self, model: &M) -> Result<FrozenStudent<M>, StudentFreezeError>
    where
        M: HardTernaryStudentModel,
    {
        self.ensure_not_fired();
        self.fired = true;
        freeze_student_as_artifact(model)
    }

    /// Freeze the student once through a caller-owned training log emitter.
    pub fn freeze_with_logging<M>(
        &mut self,
        model: &M,
        log_emitter: &TrainingLogEmitter,
    ) -> Result<FrozenStudent<M>, StudentFreezeError>
    where
        M: HardTernaryStudentModel,
    {
        self.ensure_not_fired();
        self.fired = true;
        let outcome = freeze_student_core(model)?;
        let event = student_freeze_event(
            &outcome.frozen,
            outcome.source_storage_identity,
            outcome.frozen_storage_identity,
        );
        log_emitter.student_freeze(&event)?;
        Ok(outcome.frozen)
    }

    fn ensure_not_fired(&self) {
        assert!(
            !self.fired,
            "student freeze guard fired more than once for one run"
        );
    }
}

/// Clone, detach, verify, and log the S3 student snapshot for artifact export.
pub fn freeze_student_as_artifact<M>(model: &M) -> Result<FrozenStudent<M>, StudentFreezeError>
where
    M: HardTernaryStudentModel,
{
    let outcome = freeze_student_core(model)?;
    let event = student_freeze_event(
        &outcome.frozen,
        outcome.source_storage_identity,
        outcome.frozen_storage_identity,
    );
    TrainingLogEmitter::new().student_freeze(&event)?;
    Ok(outcome.frozen)
}

#[derive(Debug)]
struct StudentFreezeOutcome<M: HardTernaryStudentModel> {
    frozen: FrozenStudent<M>,
    source_storage_identity: usize,
    frozen_storage_identity: usize,
}

fn freeze_student_core<M>(model: &M) -> Result<StudentFreezeOutcome<M>, StudentFreezeError>
where
    M: HardTernaryStudentModel,
{
    let source_fingerprint = model.student_weight_fingerprint();
    let source_storage = model.student_storage_fingerprint();
    let source_storage_identity = model.student_storage_identity();
    let mut snapshot = model.clone();
    snapshot.detach_for_student();
    let frozen_fingerprint = snapshot.student_weight_fingerprint();
    let frozen_storage = snapshot.student_storage_fingerprint();
    let frozen_storage_identity = snapshot.student_storage_identity();

    if frozen_fingerprint != source_fingerprint {
        return Err(StudentFreezeError::FingerprintMismatch {
            source: source_fingerprint,
            frozen: frozen_fingerprint,
        });
    }

    if frozen_storage != source_storage {
        return Err(StudentFreezeError::StorageFingerprintMismatch {
            source: source_storage,
            frozen: frozen_storage,
        });
    }

    if source_storage_identity == frozen_storage_identity {
        return Err(StudentFreezeError::SharedStorage);
    }

    if snapshot.student_requires_grad() {
        return Err(StudentFreezeError::FrozenSnapshotRequiresGrad);
    }

    Ok(StudentFreezeOutcome {
        frozen: FrozenStudent {
            snapshot,
            storage_fingerprint: frozen_storage,
            weight_fingerprint: frozen_fingerprint,
            requires_grad: false,
        },
        source_storage_identity,
        frozen_storage_identity,
    })
}

fn student_freeze_event<M>(
    frozen: &FrozenStudent<M>,
    source_storage_identity: usize,
    frozen_storage_identity: usize,
) -> StudentFreezeEvent
where
    M: HardTernaryStudentModel,
{
    StudentFreezeEvent {
        step: STUDENT_FREEZE_EVENT_STEP,
        student_storage_fingerprint: frozen.storage_fingerprint().to_hex(),
        student_weight_fingerprint: frozen.weight_fingerprint().to_hex(),
        source_storage_identity: identity_to_u64(source_storage_identity),
        frozen_storage_identity: identity_to_u64(frozen_storage_identity),
    }
}

fn identity_to_u64(identity: usize) -> u64 {
    u64::try_from(identity).unwrap_or(u64::MAX)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("writing to a String should not fail");
    }
    hex
}

/// Errors raised while freezing the S3 student snapshot.
#[derive(Debug, Clone, PartialEq)]
pub enum StudentFreezeError {
    /// Weight fingerprints must be non-empty.
    EmptyFingerprint,
    /// Storage fingerprints must be non-empty.
    EmptyStorageFingerprint,
    /// Detaching changed trainable student weight payloads.
    FingerprintMismatch {
        /// Source model fingerprint before cloning.
        source: StudentWeightFingerprint,
        /// Frozen snapshot fingerprint after detaching.
        frozen: StudentWeightFingerprint,
    },
    /// Detaching changed storage content/provenance bytes.
    StorageFingerprintMismatch {
        /// Source model storage fingerprint before cloning.
        source: StudentStorageFingerprint,
        /// Frozen snapshot storage fingerprint after detaching.
        frozen: StudentStorageFingerprint,
    },
    /// Source and frozen snapshot still point at the same storage identity.
    SharedStorage,
    /// The detached snapshot still requires gradients.
    FrozenSnapshotRequiresGrad,
    /// Structured logging failed.
    Logging(LoggingEventError),
}

impl fmt::Display for StudentFreezeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFingerprint => f.write_str("student weight fingerprint must not be empty"),
            Self::EmptyStorageFingerprint => {
                f.write_str("student storage fingerprint must not be empty")
            }
            Self::FingerprintMismatch { source, frozen } => write!(
                f,
                "frozen student fingerprint {} did not match source fingerprint {}",
                frozen.to_hex(),
                source.to_hex()
            ),
            Self::StorageFingerprintMismatch { source, frozen } => write!(
                f,
                "frozen student storage fingerprint {} did not match source storage fingerprint {}",
                frozen.to_hex(),
                source.to_hex()
            ),
            Self::SharedStorage => {
                f.write_str("frozen student snapshot must not share source model storage")
            }
            Self::FrozenSnapshotRequiresGrad => {
                f.write_str("frozen student snapshot still requires gradients")
            }
            Self::Logging(error) => write!(f, "failed to log student freeze: {error}"),
        }
    }
}

impl Error for StudentFreezeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Logging(error) => Some(error),
            Self::EmptyFingerprint
            | Self::EmptyStorageFingerprint
            | Self::FingerprintMismatch { .. }
            | Self::StorageFingerprintMismatch { .. }
            | Self::SharedStorage
            | Self::FrozenSnapshotRequiresGrad => None,
        }
    }
}

impl From<LoggingEventError> for StudentFreezeError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}
