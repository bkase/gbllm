#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod denotational_s3_support;

use denotational_s3_support::evaluate;
use gbf_oracle::denotational::RealDenotationalOracle;

#[test]
fn oracle_determinism_s3() {
    let first = evaluate(RealDenotationalOracle);
    let first_bytes = first.observations.canonical_bytes().unwrap();

    for _ in 0..10 {
        let replay = evaluate(RealDenotationalOracle);
        assert_eq!(replay.oracle_self_hash, first.oracle_self_hash);
        assert_eq!(replay.observations.canonical_bytes().unwrap(), first_bytes);
    }
}
