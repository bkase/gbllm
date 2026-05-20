use gbf_data::strip_gutenberg_d3;
use gbf_experiments::s4::falsify::{S4FalsificationCase, run_s4_falsification_case};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis};

use crate::common_falsification::suite_inputs;

#[test]
fn lossy_gutenberg_decompression_refutes_h1() {
    let inputs = suite_inputs();
    let raw = &inputs.corpus_oracle_inputs.stripper_cases[0].raw_utf8;
    assert!(
        raw.iter().any(|byte| !byte.is_ascii()),
        "F1 fixture must contain non-ASCII text"
    );

    let mut lossy = raw.clone();
    lossy.retain(u8::is_ascii);
    let clean_post_strip = strip_gutenberg_d3(raw)
        .expect("clean fixture strips")
        .post_strip_sha256;
    let lossy_post_strip = strip_gutenberg_d3(&lossy)
        .expect("lossy fixture still strips")
        .post_strip_sha256;
    assert_ne!(clean_post_strip, lossy_post_strip);

    let result =
        run_s4_falsification_case(&inputs, S4FalsificationCase::LossyGutenbergDecompression);
    assert_eq!(result.expected_refuted_hypothesis, S4Hypothesis::H1);
    assert_eq!(result.observed_status, HypothesisStatus::Refuted);
    assert!(result.refuted_as_expected());
}
