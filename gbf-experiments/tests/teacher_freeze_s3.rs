#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::fixtures::ToyHardTernaryStudent;
use gbf_experiments::s2::run::scheduler::{
    PhaseEvent as S2SchedulerEvent, PhasePlan, events_for_global_step,
};
use gbf_experiments::s3::schema::{S3_STUDENT_FREEZE_EVENT_STEP, S3PhaseLogError, S3PhaseLogEvent};
use gbf_train::student::StudentFreezeGuard;

#[test]
fn teacher_freeze_remains_at_step_4001_and_student_freeze_is_step_10001() {
    let plan = PhasePlan::full_s2();
    let teacher_events = events_for_global_step(4_001, &plan, Some("teacher-checkpoint"));
    let early_events = events_for_global_step(4_000, &plan, Some("teacher-checkpoint"));

    assert!(
        early_events
            .iter()
            .all(|event| !matches!(event, S2SchedulerEvent::TeacherFreeze { .. }))
    );
    assert!(teacher_events.iter().any(|event| matches!(
        event,
        S2SchedulerEvent::TeacherFreeze {
            teacher_checkpoint_sha
        } if teacher_checkpoint_sha == "teacher-checkpoint"
    )));

    let student_event = S3PhaseLogEvent::student_freeze("student-storage", "student-weight")
        .expect("student freeze event builds");
    assert_eq!(student_event.step(), S3_STUDENT_FREEZE_EVENT_STEP);
    assert_eq!(student_event.event_kind(), "student_freeze");
}

#[test]
fn student_freeze_phase_log_rejects_the_step_10000_source_boundary() {
    let event = S3PhaseLogEvent::StudentFreeze {
        schema: "s3_phase_log.v1".to_owned(),
        step: 10_000,
        student_storage_fingerprint: "student-storage".to_owned(),
        student_weight_fingerprint: "student-weight".to_owned(),
    };

    assert!(matches!(
        event.validate().unwrap_err(),
        S3PhaseLogError::InvalidStudentFreezeStep { observed: 10_000 }
    ));
}

#[test]
#[should_panic(expected = "student freeze guard fired more than once for one run")]
fn student_freeze_guard_panics_on_second_invocation() {
    let student = ToyHardTernaryStudent::new(vec![1.0, 0.0, -1.0], true);
    let mut guard = StudentFreezeGuard::new();

    let _ = guard.freeze(&student).unwrap();
    assert!(guard.has_fired());

    let _ = guard.freeze(&student);
}
