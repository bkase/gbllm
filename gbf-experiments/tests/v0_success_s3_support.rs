#![cfg(feature = "s3")]
#![allow(dead_code)]

use std::collections::BTreeMap;

use gbf_artifact::TextCharSeq;
use gbf_experiments::s3::artifact::s3_export_fixture_model_artifact;
use gbf_experiments::s3::baseline::{CountsSummary, KnBaselineProduct, KnDiscounts};
use gbf_experiments::s3::bundle::s3_export_fixture_reference_bundle;
use gbf_experiments::s3::workload::{
    ConservativeChromeBudget, GenerationDecodeStep, GenerationRecord, RomBudgetSlot,
    V0SuccessPerSeed, V0SuccessProduct,
};
use gbf_foundation::{Hash256, WorkloadId, sha256};
use gbf_workload::{
    AcceptanceMatrix_S3, ExecutionMatrix_S3, ObservationPolicy_S3, PromptCase, SessionProfile_S3,
    V0_SUCCESS_HELD_OUT_CHAPTER_SHA, V0_SUCCESS_PROMPT_COUNT, WorkloadClass, WorkloadManifest_v0,
};

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

pub fn tiny_val_post() -> TextCharSeq {
    TextCharSeq::new((0..24).map(|index| (index % 75) as u8).collect()).expect("val_post validates")
}

pub fn high_baseline() -> KnBaselineProduct {
    KnBaselineProduct {
        schema: "s3_baseline_kn5.v1".to_owned(),
        train_post_sha256: sha256(b"v0-success-test-train"),
        val_post_sha256: sha256(b"v0-success-test-val"),
        order: 5,
        discounts: BTreeMap::from([
            (2, discounts()),
            (3, discounts()),
            (4, discounts()),
            (5, discounts()),
        ]),
        bpc_kn1_val: 100.0,
        bpc_kn2_val: 100.0,
        bpc_kn3_val: 100.0,
        bpc_kn4_val: 100.0,
        bpc_kn5_val: 100.0,
        counts_summary: CountsSummary {
            train_chars: 128,
            val_chars: 24,
            c2_unique_count: 1,
            c3_unique_count: 1,
            c4_unique_count: 1,
            c5_unique_count: 1,
        },
        counts_blob_sha256: sha256(b"v0-success-test-counts"),
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("baseline self-hash computes")
}

pub fn budget(bytes: u64) -> ConservativeChromeBudget {
    ConservativeChromeBudget::new(vec![RomBudgetSlot::new("fixture", bytes)])
        .expect("budget constructs")
}

pub fn generation_record(
    prompt_id: &str,
    generated_char_count: u32,
    max_run: u32,
    charset_validity_rate: f64,
    terminal_eos_seen: bool,
) -> GenerationRecord {
    let chars = varied_chars(generated_char_count as usize);
    GenerationRecord {
        prompt_id: prompt_id.into(),
        generated_chars: TextCharSeq::new(chars).expect("generated chars validate"),
        generated_char_count,
        max_consecutive_same_token: max_run,
        charset_validity_rate,
        terminal_eos_seen,
        decode_mode: gbf_artifact::DecodeMode::Argmax,
        decode_log: vec![GenerationDecodeStep {
            step: 0,
            token: 1,
            logit_max: 1.0,
        }],
    }
}

pub fn passing_per_seed(seed: u64) -> V0SuccessPerSeed {
    V0SuccessPerSeed::from_quality_bits(seed, true, true, true, true, true, true)
}

pub fn passing_product() -> V0SuccessProduct {
    let per_seed = (0..5).map(passing_per_seed).collect::<Vec<_>>();
    V0SuccessProduct::new(
        sha256(b"workload"),
        sha256(b"baseline"),
        sha256(b"chrome-budget"),
        per_seed,
    )
    .expect("v0_success product builds")
}

pub fn runner_artifacts() -> Vec<gbf_artifact::ModelArtifact> {
    (0..5)
        .map(|seed| {
            s3_export_fixture_model_artifact(seed)
                .expect("artifact exports")
                .artifact
        })
        .collect()
}

pub fn runner_bundles() -> Vec<gbf_artifact::ReferenceModelBundle> {
    (0..5)
        .map(|seed| {
            s3_export_fixture_reference_bundle(seed)
                .expect("bundle exports")
                .bundle
        })
        .collect()
}

fn discounts() -> KnDiscounts {
    KnDiscounts {
        y_k: 0.5,
        d_1: 0.5,
        d_2: 1.0,
        d_3p: 1.5,
    }
}

fn varied_chars(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 75) as u8).collect()
}
