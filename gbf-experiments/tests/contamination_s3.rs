#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod v0_success_s3_support;

use gbf_artifact::TextCharSeq;
use gbf_experiments::s3::contamination::{
    CONTAMINATION_MOVED_OWNER_BEADS, check_no_prompt_in_train_post,
};
use v0_success_s3_support::fixture_workload;

#[test]
fn contamination_s3() {
    let workload = fixture_workload();
    let clean_train = TextCharSeq::new(vec![74; 512]).expect("clean train chars validate");
    let clean = check_no_prompt_in_train_post(&workload, &clean_train);

    assert_eq!(clean.prompt_count, 8);
    assert_eq!(clean.train_post_char_count, 512);
    assert!(!clean.contamination_found);
    assert!(
        clean
            .per_prompt
            .iter()
            .all(|record| { !record.contamination_found && record.first_train_offset.is_none() })
    );
    assert!(
        clean.runtime_ms < 60_000,
        "tiny contamination check exceeded runtime budget"
    );
    assert_eq!(
        clean.moved_owner_beads,
        CONTAMINATION_MOVED_OWNER_BEADS
            .iter()
            .map(|bead| (*bead).to_owned())
            .collect::<Vec<_>>()
    );

    let mut contaminated = vec![74; 10];
    contaminated.extend_from_slice(workload.prompts[0].prompt_chars.as_slice());
    contaminated.extend_from_slice(&[73; 10]);
    let contaminated = TextCharSeq::new(contaminated).expect("contaminated train chars validate");
    let report = check_no_prompt_in_train_post(&workload, &contaminated);

    assert!(report.contamination_found);
    let first = report
        .per_prompt
        .iter()
        .find(|record| record.prompt_id == workload.prompts[0].id)
        .expect("prompt record exists");
    assert!(first.contamination_found);
    assert_eq!(first.first_train_offset, Some(10));
}
