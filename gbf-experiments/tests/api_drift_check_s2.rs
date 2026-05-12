mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::api_drift::{
    ApiDriftSymbols, ApiSymbolDriftKind, check_api_drift, check_api_drift_with_allow_list,
    read_snapshot_symbols,
};
use serde_json::json;

#[test]
fn api_drift_no_drift_returns_pass_and_hashes_snapshots() {
    let snapshots_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots");
    let current = ApiDriftSymbols {
        qat: read_snapshot_symbols(snapshots_dir.join("s1_qat_public_api.txt")).unwrap(),
        linearstate: read_snapshot_symbols(snapshots_dir.join("s1_linearstate_public_api.txt"))
            .unwrap(),
    };
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || check_api_drift(&snapshots_dir, current))
        .expect("api drift check");

    assert!(report.passed);
    assert_eq!(report.drift_count, 0);
    insta::assert_snapshot!(
        "api_drift__no_drift",
        format!(
            "passed={}\ndrift_count={}\nqat_hash={}\nlinearstate_hash={}",
            report.passed,
            report.drift_count,
            report.qat_public_api_snapshot_hash,
            report.linearstate_public_api_snapshot_hash
        )
    );
    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == "api_drift_check_start")
    );
    assert!(
        events
            .iter()
            .any(|event| event.name == "api_drift_check_done"
                && event.fields.get("passed") == Some(&json!(true))
                && event.fields.contains_key("qat_snapshot_hash")
                && event.fields.contains_key("linearstate_snapshot_hash"))
    );
}

#[test]
fn api_drift_added_symbol_fails_without_allow_list() {
    let temp = temp_snapshots();
    let current = ApiDriftSymbols {
        qat: vec!["QuantHardness".to_owned(), "NewQatSymbol".to_owned()],
        linearstate: vec!["LinearStateBlock".to_owned()],
    };
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || check_api_drift(temp.path(), current))
        .expect("api drift check");

    assert!(!report.passed);
    assert_eq!(report.drift_count, 1);
    assert_eq!(report.drifts[0].symbol, "NewQatSymbol");
    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == "api_drift_violation")
    );
}

#[test]
fn api_drift_added_and_removed_symbols_fail_without_allow_list() {
    let temp = temp_snapshots();
    let current = ApiDriftSymbols {
        qat: vec!["NewQatSymbol".to_owned()],
        linearstate: vec!["LinearStateBlock".to_owned(), "LinearStateExtra".to_owned()],
    };

    let report = check_api_drift(temp.path(), current).expect("api drift check");

    assert!(!report.passed);
    assert_eq!(report.drift_count, 3);
    assert!(
        report
            .drifts
            .iter()
            .any(|drift| drift.symbol == "NewQatSymbol" && drift.kind == ApiSymbolDriftKind::Added)
    );
    assert!(
        report
            .drifts
            .iter()
            .any(|drift| drift.symbol == "QuantHardness"
                && drift.kind == ApiSymbolDriftKind::Removed)
    );
    assert!(
        report
            .drifts
            .iter()
            .any(|drift| drift.symbol == "LinearStateExtra"
                && drift.kind == ApiSymbolDriftKind::Added)
    );
}

#[test]
fn api_drift_allow_list_can_accept_added_symbol() {
    let temp = temp_snapshots();
    let current = ApiDriftSymbols {
        qat: vec!["QuantHardness".to_owned(), "NewQatSymbol".to_owned()],
        linearstate: vec!["LinearStateBlock".to_owned()],
    };

    let report = check_api_drift_with_allow_list(temp.path(), current, &["NewQatSymbol"])
        .expect("api drift check");

    assert!(report.passed);
    assert_eq!(report.drift_count, 1);
    assert!(report.drifts[0].in_allow_list);
}

fn temp_snapshots() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("s1_qat_public_api.txt"), "QuantHardness\n")
        .expect("qat snapshot");
    std::fs::write(
        temp.path().join("s1_linearstate_public_api.txt"),
        "LinearStateBlock\n",
    )
    .expect("linearstate snapshot");
    temp
}
