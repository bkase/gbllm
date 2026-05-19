mod artifact_b5_support;

use artifact_b5_support::artifact_core;

#[test]
fn artifact_core_self_hash_is_deterministic_across_replays() {
    let first = artifact_core()
        .compute_core_hash()
        .expect("artifact core hash computes");

    for _ in 0..10 {
        assert_eq!(
            artifact_core()
                .compute_core_hash()
                .expect("artifact core hash computes"),
            first
        );
    }
}
