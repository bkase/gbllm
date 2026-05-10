use gbf_experiments::s1::oracle::MetricOracleResults;
use gbf_experiments::s1::schema::S1Outcome;

#[test]
fn f3_no_reset_scorer_refutes_h5_and_fails_metric() {
    let broken_context_lengths = (0_usize..=128).collect::<Vec<_>>();
    let expected = (0_usize..128).chain([0]).collect::<Vec<_>>();
    assert_ne!(
        broken_context_lengths, expected,
        "no-reset scorer must diverge from O-metric-3 context trace"
    );

    let results = MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: true,
        o_metric_3: false,
        o_metric_4: true,
    };

    let mut input = crate::confirmed_input();
    input.h5 = results.h5_status();
    crate::assert_falsification_outcome(
        "F3",
        input,
        S1Outcome::FailMetric,
        crate::fail_metric_decision(),
    );
}
