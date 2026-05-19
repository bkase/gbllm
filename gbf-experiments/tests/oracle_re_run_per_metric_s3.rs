#![cfg(feature = "s3")]

use std::collections::BTreeMap;

use gbf_experiments::s3::oracle_re_run::{
    MetricId, MetricReRun, OracleReRunReport, S1_D7_METRIC_IDS,
};

#[test]
fn oracle_re_run_per_metric_s3() {
    let mut per_metric = BTreeMap::new();
    for metric_id in S1_D7_METRIC_IDS {
        per_metric.insert(
            MetricId::from(metric_id),
            MetricReRun::new(1.0, 1.0, 1.0, 0.0),
        );
    }

    let failed_metric = MetricId::from("O-metric-3");
    per_metric.insert(
        failed_metric.clone(),
        MetricReRun::new(1.0, 1.0, 1.0 + 1.0e-12, 0.0),
    );
    let report = OracleReRunReport::new(per_metric).expect("report constructs");

    let row = report
        .per_metric
        .get(&failed_metric)
        .expect("failed metric present");
    assert!(!row.passed);
    assert!(!report.s1_oracle_re_run_passed);
    assert!(!report.s2_oracle_re_run_passed);
    report
        .validate()
        .expect("failed report remains well-formed");
}
