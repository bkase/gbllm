//! S7 train-run helper surface.

use gbf_artifact::S7Topology;
use gbf_foundation::Hash256;

use crate::s7::state::{S7StateError, S7TeacherFreezeBoundary, S7TrainRunId, S7TrainRunState};

/// Result of the S7-local train-run helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7TrainAttempt {
    /// Identity of the attempted run.
    pub run_id: S7TrainRunId,
    /// Internal Phase A teacher-freeze boundary.
    pub phase_a_teacher: S7TeacherFreezeBoundary,
    /// Deterministic same-topology, same-seed teacher checkpoint hash for `s7_run_log.v1`.
    pub frozen_teacher_checkpoint_sha: Hash256,
}

/// Execute the S7-local train-run helper through the internal teacher-freeze boundary.
///
/// This helper intentionally stops after proving the Phase A freeze contract;
/// the full training-loop producer is owned by the end-to-end S7 run harness.
pub fn s7_train_run(topology: S7Topology, seed: u64) -> Result<S7TrainAttempt, S7StateError> {
    let mut state = S7TrainRunState::baseline_matched(topology, seed);
    let phase_a_teacher = state.freeze_teacher_at_phase_a_boundary()?;
    phase_a_teacher.emit_trace();

    Ok(S7TrainAttempt {
        run_id: state.run_id().clone(),
        frozen_teacher_checkpoint_sha: phase_a_teacher.teacher_checkpoint_sha,
        phase_a_teacher,
    })
}
