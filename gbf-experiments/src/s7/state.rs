//! S7 train-run state helpers.

use std::fmt;

use gbf_artifact::S7Topology;
use gbf_foundation::{CanonicalJsonError, DomainHash, Hash256, SemVer};
use serde::Serialize;

use crate::S7_LOG_TARGET;

/// Phase A end step in the S7 scheduler.
pub const S7_PHASE_A_END_STEP: u64 = 4_000;

/// Structured tracing event emitted when a run freezes its Phase A teacher.
pub const S7_TEACHER_FREEZE_BOUNDARY_EVENT: &str = "s7.teacher_freeze.boundary";

/// Schema id for the S7 teacher-freeze boundary event.
pub const S7_TEACHER_FREEZE_BOUNDARY_SCHEMA: &str = "s7_teacher_freeze_boundary.v1";

/// Version carried by the S7 teacher-freeze boundary event.
pub const S7_TEACHER_FREEZE_BOUNDARY_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

const S7_TEACHER_CHECKPOINT_DOMAIN_VERSION: &str = "1";

/// Identity of one S7 train run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7TrainRunId {
    /// Topology trained by this run.
    pub topology: S7Topology,
    /// Experiment seed for this run.
    pub seed: u64,
}

impl S7TrainRunId {
    /// Construct a run identity from topology and seed.
    #[must_use]
    pub const fn new(topology: S7Topology, seed: u64) -> Self {
        Self { topology, seed }
    }

    /// Stable human-readable topology spelling used in tracing fields.
    #[must_use]
    pub fn topology_name(&self) -> &'static str {
        s7_topology_name(&self.topology)
    }
}

/// Teacher freeze boundary produced inside one S7 train run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7TeacherFreezeBoundary {
    /// Run that produced the frozen teacher.
    pub run_id: S7TrainRunId,
    /// Step at which Phase A ended and the teacher was frozen.
    pub phase_a_end_step: u64,
    /// Deterministic checkpoint hash for the frozen teacher.
    pub teacher_checkpoint_sha: Hash256,
}

impl S7TeacherFreezeBoundary {
    /// Build the deterministic Phase A teacher-freeze boundary for a run.
    pub fn new(run_id: S7TrainRunId) -> Result<Self, S7StateError> {
        let phase_a_end_step = S7_PHASE_A_END_STEP;
        let teacher_checkpoint_sha = teacher_checkpoint_sha(&run_id, phase_a_end_step)?;
        Ok(Self {
            run_id,
            phase_a_end_step,
            teacher_checkpoint_sha,
        })
    }

    /// Emit the structured tracing boundary event.
    pub fn emit_trace(&self) {
        tracing::info!(
            target: S7_LOG_TARGET,
            event_name = S7_TEACHER_FREEZE_BOUNDARY_EVENT,
            schema = S7_TEACHER_FREEZE_BOUNDARY_SCHEMA,
            schema_version_major = S7_TEACHER_FREEZE_BOUNDARY_SCHEMA_VERSION.major,
            schema_version_minor = S7_TEACHER_FREEZE_BOUNDARY_SCHEMA_VERSION.minor,
            schema_version_patch = S7_TEACHER_FREEZE_BOUNDARY_SCHEMA_VERSION.patch,
            topology = self.run_id.topology_name(),
            seed = self.run_id.seed,
            phase = "PhaseA",
            boundary = "PhaseAEnd",
            phase_a_end_step = self.phase_a_end_step,
            teacher_checkpoint_sha = %self.teacher_checkpoint_sha,
            frozen_teacher_checkpoint_sha = %self.teacher_checkpoint_sha,
            "s7 phase-a teacher freeze boundary"
        );
    }
}

/// Minimal S7-local train-run state for the internal teacher-freeze boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S7TrainRunState {
    /// Run is ready to start from the matched baseline.
    BaselineMatched {
        /// Identity of the run waiting to train.
        run_id: S7TrainRunId,
    },
    /// Run has attempted training and carries its same-topology, same-seed teacher.
    TrainAttempted {
        /// Identity of the attempted run.
        run_id: S7TrainRunId,
        /// Teacher frozen at the internal Phase A boundary.
        phase_a_teacher: S7TeacherFreezeBoundary,
    },
}

impl S7TrainRunState {
    /// Create the run-local state after the S7 matched-baseline step.
    #[must_use]
    pub const fn baseline_matched(topology: S7Topology, seed: u64) -> Self {
        Self::BaselineMatched {
            run_id: S7TrainRunId::new(topology, seed),
        }
    }

    /// Freeze the Phase A teacher internally and transition directly to `TrainAttempted`.
    pub fn freeze_teacher_at_phase_a_boundary(
        &mut self,
    ) -> Result<S7TeacherFreezeBoundary, S7StateError> {
        match self {
            Self::BaselineMatched { run_id } => {
                let phase_a_teacher = S7TeacherFreezeBoundary::new(run_id.clone())?;
                *self = Self::TrainAttempted {
                    run_id: run_id.clone(),
                    phase_a_teacher: phase_a_teacher.clone(),
                };
                Ok(phase_a_teacher)
            }
            Self::TrainAttempted { run_id, .. } => Err(S7StateError::TeacherAlreadyFrozen {
                topology: run_id.topology_name(),
                seed: run_id.seed,
            }),
        }
    }

    /// Return the run identity carried by this state.
    #[must_use]
    pub const fn run_id(&self) -> &S7TrainRunId {
        match self {
            Self::BaselineMatched { run_id } | Self::TrainAttempted { run_id, .. } => run_id,
        }
    }

    /// Return the teacher checkpoint hash once Phase A has frozen the teacher.
    #[must_use]
    pub const fn teacher_checkpoint_sha(&self) -> Option<Hash256> {
        self.frozen_teacher_checkpoint_sha()
    }

    /// Return the run-log-facing frozen teacher checkpoint hash.
    #[must_use]
    pub const fn frozen_teacher_checkpoint_sha(&self) -> Option<Hash256> {
        match self {
            Self::BaselineMatched { .. } => None,
            Self::TrainAttempted {
                phase_a_teacher, ..
            } => Some(phase_a_teacher.teacher_checkpoint_sha),
        }
    }
}

/// Return the RFC/serde spelling for an S7 topology.
#[must_use]
pub const fn s7_topology_name(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

/// Compute the deterministic Phase A teacher checkpoint hash for one run.
pub fn teacher_checkpoint_sha(
    run_id: &S7TrainRunId,
    phase_a_end_step: u64,
) -> Result<Hash256, S7StateError> {
    let material = TeacherCheckpointMaterial {
        schema: "s7_teacher_checkpoint_material.v1",
        topology: &run_id.topology,
        seed: run_id.seed,
        phase_a_end_step,
        teacher_role: "same_topology_same_seed_phase_a_teacher",
    };
    Ok(teacher_checkpoint_domain().hash(&material)?)
}

/// Errors raised by S7 train-run state helpers.
#[derive(Debug)]
pub enum S7StateError {
    /// Canonical serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// The run already emitted its internal teacher-freeze boundary.
    TeacherAlreadyFrozen {
        /// Topology of the run that already froze.
        topology: &'static str,
        /// Seed of the run that already froze.
        seed: u64,
    },
}

impl fmt::Display for S7StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::TeacherAlreadyFrozen { topology, seed } => write!(
                f,
                "S7 run ({topology}, seed={seed}) already froze its Phase A teacher"
            ),
        }
    }
}

impl std::error::Error for S7StateError {}

impl From<CanonicalJsonError> for S7StateError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct TeacherCheckpointMaterial<'a> {
    schema: &'static str,
    topology: &'a S7Topology,
    seed: u64,
    phase_a_end_step: u64,
    teacher_role: &'static str,
}

const fn teacher_checkpoint_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "S7TeacherCheckpoint",
        "s7_teacher_checkpoint.v1",
        S7_TEACHER_CHECKPOINT_DOMAIN_VERSION,
    )
}
