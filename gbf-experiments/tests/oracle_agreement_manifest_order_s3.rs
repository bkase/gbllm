#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use gbf_oracle::denotational::SemanticCheckpoint;
use gbf_oracle::phase_surface_agreement::PhaseId;
use oracle_agreement_s3_support::{
    run_default_agreement_with_workload, workload_with_first_three_prompt_ids,
};

#[test]
fn oracle_agreement_manifest_order_s3() {
    let workload = workload_with_first_three_prompt_ids(["manifest-z", "manifest-a", "manifest-m"]);
    let product = run_default_agreement_with_workload(&workload);

    let observed = product
        .records
        .iter()
        .filter(|record| {
            record.seed == 0
                && record.phase == PhaseId::PhaseA
                && record.checkpoint == SemanticCheckpoint::PostLogits
                && record.step == 0
        })
        .map(|record| record.prompt_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(observed, vec!["manifest-z", "manifest-a", "manifest-m"]);
}
