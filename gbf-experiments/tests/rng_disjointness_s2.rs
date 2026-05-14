mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::rng::{S1Rng, seed128};
use gbf_experiments::s2::rng::{
    S2RngStreams, ThresholdInitRng, audit_disjointness, audited_domains,
};
use serde_json::json;

#[test]
fn threshold_init_domain_is_disjoint_from_inherited_and_linearstate_domains() {
    let threshold = seed128("threshold_init", 0);

    for domain in [
        "init",
        "batch",
        "shuffle",
        "linearstate_smoke/linearstate_input_v1",
        "linearstate_smoke/linearstate_params_v1",
    ] {
        assert_ne!(threshold, seed128(domain, 0), "{domain} collided");
    }
}

#[test]
fn threshold_init_rng_is_deterministic_across_replays() {
    let mut first = ThresholdInitRng::new(0);
    let mut second = ThresholdInitRng::new(0);

    assert_eq!(first.state(), seed128("threshold_init", 0) | 1);
    assert_eq!(first.next_u64(), second.next_u64());
    assert_eq!(first.next_u64(), second.next_u64());
    assert_eq!(first.draw_count(), 2);
    assert_eq!(second.draw_count(), 2);
}

#[test]
fn threshold_init_rng_v1_consumes_zero_draws_until_future_randomized_init() {
    let rng = ThresholdInitRng::new(4);

    assert_eq!(rng.seed(), 4);
    assert_eq!(rng.draw_count(), 0);
}

#[test]
fn rng_disjointness_audit_covers_all_s2_domains() {
    let audit = audit_disjointness(0);

    assert_eq!(audit.collision_count, 0);
    assert_eq!(audit.domains, audited_domains().to_vec());
}

#[test]
fn rng_stream_init_events_cover_inherited_and_threshold_streams() {
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        let _streams = S2RngStreams::new(7);
    });

    let events = captured_events(&capture);
    let domains = events
        .iter()
        .filter(|event| event.name == "rng_stream_init")
        .map(|event| {
            event
                .fields
                .get("domain")
                .and_then(serde_json::Value::as_str)
                .expect("domain field")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(domains, ["init", "batch", "shuffle", "threshold_init"]);
    for event in events
        .iter()
        .filter(|event| event.name == "rng_stream_init")
    {
        assert_eq!(event.fields.get("seed"), Some(&json!(7)));
        assert!(event.fields.contains_key("seed128"));
    }
}
