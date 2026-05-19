#![cfg(feature = "s3")]

use std::collections::{BTreeMap, BTreeSet};

use gbf_experiments::s3::schema::S3Hypothesis;

const TARGETS: [(&str, &[S3Hypothesis]); 9] = [
    ("F1-broken-S3", &[S3Hypothesis::H1]),
    ("F2-broken-S3", &[S3Hypothesis::H2]),
    ("F3-broken-S3", &[S3Hypothesis::H3]),
    ("F4-broken-S3", &[S3Hypothesis::H4, S3Hypothesis::H6]),
    ("F5-broken-S3", &[S3Hypothesis::H5]),
    ("F6-broken-S3", &[S3Hypothesis::H5]),
    ("F7-broken-S3", &[S3Hypothesis::H3]),
    ("F8-broken-S3", &[S3Hypothesis::H4]),
    ("F9-broken-S3", &[S3Hypothesis::H7]),
];

#[test]
fn falsification_s3_covers_every_closure_hypothesis() {
    let covered = TARGETS
        .iter()
        .flat_map(|(_, hypotheses)| hypotheses.iter().copied())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        covered,
        S3Hypothesis::ALL.into_iter().collect::<BTreeSet<_>>()
    );
}

#[test]
fn falsification_s3_declares_exactly_nine_unique_substitutes() {
    let names = TARGETS
        .iter()
        .map(|(name, _)| *name)
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), 9);
    assert_eq!(TARGETS.len(), 9);
}

#[test]
fn falsification_s3_records_multi_hypothesis_f4_only() {
    let multi = TARGETS
        .iter()
        .filter(|(_, hypotheses)| hypotheses.len() > 1)
        .map(|(name, hypotheses)| (*name, *hypotheses))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        multi,
        BTreeMap::from([("F4-broken-S3", &[S3Hypothesis::H4, S3Hypothesis::H6][..])])
    );
}
