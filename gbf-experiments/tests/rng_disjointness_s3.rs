#![cfg(feature = "s3")]

mod common;

use gbf_experiments::s3::rng::{S3RngStreams, assert_no_new_rng_domains, s3_stream_domains};

#[test]
fn s3_rng_streams_are_the_s2_streams() {
    assert_eq!(
        std::any::type_name::<S3RngStreams>(),
        "gbf_experiments::s2::rng::S2RngStreams"
    );
}

#[test]
fn s3_declares_no_new_rng_stream_domains() {
    assert_no_new_rng_domains();
    assert_eq!(
        s3_stream_domains(),
        ["init", "batch", "shuffle", "threshold_init"]
    );
}

#[test]
fn s3_stream_constructor_emits_only_inherited_stream_domains() {
    use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};

    let capture = TraceCapture::default();
    with_trace_capture(&capture, || {
        let _streams = S3RngStreams::new(7);
    });

    let domains = captured_events(&capture)
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

    assert_eq!(
        domains,
        s3_stream_domains()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
}
