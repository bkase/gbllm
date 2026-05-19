#![cfg(feature = "s3")]
#![allow(dead_code)]

use gbf_artifact::{TextCharSeq, UNK_ID, VOCAB_SIZE};
use gbf_experiments::s3::artifact::s3_export_fixture_model_artifact;
use gbf_foundation::{Hash256, WorkloadId};
use gbf_workload::{
    AcceptanceMatrix_S3, ExecutionMatrix_S3, ObservationPolicy_S3, PromptCase, SessionProfile_S3,
    V0_SUCCESS_HELD_OUT_CHAPTER_SHA, V0_SUCCESS_PROMPT_COUNT, WorkloadClass, WorkloadManifest_v0,
};

pub fn fixture_artifact(seed: u64) -> gbf_artifact::ModelArtifact {
    s3_export_fixture_model_artifact(seed)
        .expect("fixture artifact exports")
        .artifact
}

pub fn fixture_workload() -> WorkloadManifest_v0 {
    let held_out_sha = V0_SUCCESS_HELD_OUT_CHAPTER_SHA
        .parse()
        .expect("held-out hash parses");
    let prompts = (0..V0_SUCCESS_PROMPT_COUNT)
        .map(|index| {
            let chars = (0..64)
                .map(|offset| ((index + offset) % 75) as u8)
                .collect::<Vec<_>>();
            PromptCase::new(format!("prompt-{index:02}"), chars, held_out_sha)
                .expect("prompt case builds")
        })
        .collect::<Vec<_>>();
    let mut workload = WorkloadManifest_v0 {
        schema: "workload_manifest.v1".to_owned(),
        id: WorkloadId::from("v0_success"),
        class: WorkloadClass::Conformance,
        prompts,
        seeds: vec![0, 1, 2, 3, 4],
        session: SessionProfile_S3::pinned(),
        observation: ObservationPolicy_S3::pinned(),
        execution: ExecutionMatrix_S3::pinned(),
        acceptance: AcceptanceMatrix_S3::pinned(),
        workload_self_hash: Hash256::ZERO,
    };
    workload.workload_self_hash = workload.compute_self_hash().expect("workload self-hash");
    workload.validate().expect("workload fixture validates");
    workload
}

pub fn fixture_policy() -> ObservationPolicy_S3 {
    ObservationPolicy_S3::pinned()
}

pub fn prompt_for_logits() -> TextCharSeq {
    TextCharSeq::new(vec![1, 4, 7, 10, 13, 16, 19]).expect("prompt validates")
}

pub fn prompt_for_decode() -> TextCharSeq {
    TextCharSeq::new(vec![1, 2, 3, 4, 5]).expect("prompt validates")
}

pub fn eos_trigger_prompt() -> TextCharSeq {
    TextCharSeq::new(vec![UNK_ID]).expect("eos trigger prompt validates")
}

pub fn max_abs_diff(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max)
}

pub fn argmax(logits: &[f32]) -> (u8, f32) {
    assert_eq!(logits.len(), VOCAB_SIZE);
    let mut best_index = 0_usize;
    let mut best_value = logits[0];
    for (index, value) in logits.iter().copied().enumerate().skip(1) {
        if value > best_value {
            best_index = index;
            best_value = value;
        }
    }
    (best_index as u8, best_value)
}
