#![cfg(feature = "s3")]

use gbf_experiments::s3::oracle_re_run::{
    S1_D7_METRIC_IDS, S3_ORACLE_RE_RUN_SCHEMA, s3_oracle_re_run,
};

#[test]
fn oracle_re_run_s3() {
    let report = s3_oracle_re_run().expect("S3 oracle re-run succeeds");

    assert_eq!(report.schema, S3_ORACLE_RE_RUN_SCHEMA);
    assert!(report.s1_oracle_re_run_passed);
    assert!(report.s2_oracle_re_run_passed);
    report.validate_closure().expect("closure validates");
    assert_eq!(report.per_metric.len(), S1_D7_METRIC_IDS.len());
    assert_eq!(
        report.oracle_re_run_self_hash,
        report.computed_self_hash().expect("self hash computes")
    );

    for metric_id in S1_D7_METRIC_IDS {
        let row = report
            .per_metric
            .get(&metric_id.into())
            .expect("metric id present");
        assert_eq!(row.s1_baseline, 1.0);
        assert_eq!(row.s2_baseline, 1.0);
        assert_eq!(row.s3_observed, 1.0);
        assert_eq!(row.delta_vs_s1, 0.0);
        assert_eq!(row.delta_vs_s2, 0.0);
        assert_eq!(row.tolerance, 0.0);
        assert!(row.passed);
    }
}
