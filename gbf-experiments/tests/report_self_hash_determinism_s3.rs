#![cfg(feature = "s3")]

mod report_s3_support;

use gbf_experiments::s3::report::{S3Report, report_self_hash, validate_r_self_hash};

#[test]
fn report_self_hash_is_deterministic_across_replays() {
    let report = report_s3_support::pass_clean_report();
    let hashes = (0..10)
        .map(|_| report_self_hash(&report.front_matter, &report.body).expect("hash"))
        .collect::<Vec<_>>();

    assert!(
        hashes
            .iter()
            .all(|hash| *hash == report.front_matter.report_self_hash)
    );
    validate_r_self_hash(&report).expect("self hash validates");
}

#[test]
fn generated_at_commit_time_is_excluded_from_report_self_hash() {
    let report = report_s3_support::pass_clean_report();
    let mut front_matter = report.front_matter.clone();
    front_matter.generated_at_commit_time = "2099-01-01T00:00:00Z".to_owned();
    front_matter.report_self_hash = gbf_foundation::Hash256::ZERO;
    front_matter.report_self_hash =
        report_self_hash(&front_matter, &report.body).expect("changed timestamp rehashes");
    let changed = S3Report::new(front_matter, report.body.clone()).expect("report validates");

    assert_eq!(
        report.front_matter.report_self_hash,
        changed.front_matter.report_self_hash
    );
}
