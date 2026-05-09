mod common;

use common::tempdir::fresh_isolated_env;
use common::tracing_capture::{TraceCapture, captured_events};
use gbf_experiments::s1::device_profile::{
    DeviceProfileViolation, S1CpuDeterministic, device_profile_hash, enforce,
};
use gbf_experiments::s1::logging::event;
use serde_json::json;
use tracing_subscriber::prelude::*;

#[test]
fn enforce_reads_current_process_env_from_clean_launch_context() {
    let _guard = fresh_isolated_env(&[
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "1"),
    ]);

    let enforcement = enforce(&S1CpuDeterministic::canonical()).unwrap();

    assert_eq!(
        enforcement.device_profile_hash(),
        device_profile_hash(&S1CpuDeterministic::canonical()).unwrap()
    );
}

#[test]
fn enforce_rejects_extra_current_process_env_var_before_tensor_allocation() {
    let _guard = fresh_isolated_env(&[
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS_HACK", "1"),
    ]);

    let err = enforce(&S1CpuDeterministic::canonical()).unwrap_err();

    assert!(matches!(
        err.violations(),
        [DeviceProfileViolation::ForbiddenEnv { var, observed }]
            if var == "RAYON_NUM_THREADS_HACK" && observed == "1"
    ));
}

#[test]
fn enforce_failure_emits_device_profile_and_precondition_events() {
    let _guard = fresh_isolated_env(&[
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "8"),
    ]);
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        let _ = enforce(&S1CpuDeterministic::canonical());
    });

    let events = captured_events(&capture);
    assert!(events.iter().any(|record| {
        record.name == event::DEVICE_PROFILE_ENFORCE_START
            && record
                .fields
                .get("device_profile_hash")
                .and_then(|value| value.as_str())
                == Some("sha256:24a3f310d912f21f542d3eba8f42120fd835e964683c64dadc2515652888845d")
    }));
    assert!(events.iter().any(|record| {
        record.name == event::DEVICE_PROFILE_ENFORCE_FAIL
            && record.fields.get("rejected_var") == Some(&json!("RAYON_NUM_THREADS"))
            && record.fields.get("expected") == Some(&json!("1"))
            && record.fields.get("observed") == Some(&json!("8"))
    }));
    assert!(events.iter().any(|record| {
        record.name == event::RUN_PRECONDITION_FAILED
            && record
                .fields
                .get("reason")
                .and_then(|value| value.as_str())
                .is_some_and(|reason| reason.contains("RAYON_NUM_THREADS"))
    }));
}
