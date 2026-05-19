//! Workload/train split contamination checks for S3.

use std::collections::BTreeMap;
use std::time::Instant;

use gbf_artifact::TextCharSeq;
use gbf_workload::{PromptId, WorkloadManifest_v0};
use serde::{Deserialize, Serialize};

use crate::s3::workload::S3_WORKLOAD_LOG_TARGET;

/// Contamination check event name.
pub const EVENT_NAME_CONTAMINATION_CHECKED: &str = "s3::contamination::checked";
/// Cross-corpus contamination owner bead named by RFC O-8.
pub const CONTAMINATION_MOVED_OWNER_BEADS: [&str; 2] = ["bd-tmaw", "bd-pso7"];

/// Per-prompt contamination result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptContaminationRecord {
    /// Prompt id.
    pub prompt_id: PromptId,
    /// Prompt length in normalized chars.
    pub prompt_char_count: u32,
    /// Whether the prompt appeared verbatim in train_post.
    pub contamination_found: bool,
    /// First matching train offset, if any.
    pub first_train_offset: Option<u32>,
}

/// O-8 prompt/train contamination report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContaminationReport {
    /// Prompt count checked.
    pub prompt_count: u32,
    /// Train split length in normalized chars.
    pub train_post_char_count: u32,
    /// Whether any prompt appeared verbatim in train_post.
    pub contamination_found: bool,
    /// Per-prompt records.
    pub per_prompt: Vec<PromptContaminationRecord>,
    /// Runtime in milliseconds.
    pub runtime_ms: u64,
    /// Named owner beads for richer cross-corpus contamination work.
    pub moved_owner_beads: Vec<String>,
}

/// Check that no v0_success prompt appears verbatim in train_post.
#[must_use]
pub fn check_no_prompt_in_train_post(
    workload: &WorkloadManifest_v0,
    train_post: &TextCharSeq,
) -> ContaminationReport {
    let started = Instant::now();
    let train = train_post.as_slice();
    let mut windows_by_len = BTreeMap::<usize, BTreeMap<u64, Vec<usize>>>::new();
    let mut per_prompt = Vec::new();

    for prompt in &workload.prompts {
        let prompt_chars = prompt.prompt_chars.as_slice();
        let prompt_len = prompt_chars.len();
        let windows = windows_by_len
            .entry(prompt_len)
            .or_insert_with(|| rolling_windows_by_hash(train, prompt_len));
        let prompt_hash = rolling_hash(prompt_chars);
        let first_train_offset = windows
            .get(&prompt_hash)
            .and_then(|offsets| {
                offsets
                    .iter()
                    .copied()
                    .find(|offset| &train[*offset..*offset + prompt_len] == prompt_chars)
            })
            .and_then(|offset| u32::try_from(offset).ok());
        per_prompt.push(PromptContaminationRecord {
            prompt_id: prompt.id.clone(),
            prompt_char_count: prompt_len as u32,
            contamination_found: first_train_offset.is_some(),
            first_train_offset,
        });
    }

    let contamination_found = per_prompt.iter().any(|record| record.contamination_found);
    let runtime_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let report = ContaminationReport {
        prompt_count: per_prompt.len() as u32,
        train_post_char_count: train.len() as u32,
        contamination_found,
        per_prompt,
        runtime_ms,
        moved_owner_beads: CONTAMINATION_MOVED_OWNER_BEADS
            .iter()
            .map(|bead| (*bead).to_owned())
            .collect(),
    };
    tracing::info!(
        target: S3_WORKLOAD_LOG_TARGET,
        event_name = EVENT_NAME_CONTAMINATION_CHECKED,
        prompt_count = report.prompt_count as u64,
        train_post_char_count = report.train_post_char_count as u64,
        contamination_found = report.contamination_found,
        runtime_ms = report.runtime_ms,
    );
    report
}

fn rolling_windows_by_hash(train: &[u8], window_len: usize) -> BTreeMap<u64, Vec<usize>> {
    let mut windows = BTreeMap::<u64, Vec<usize>>::new();
    if window_len == 0 || window_len > train.len() {
        return windows;
    }
    for offset in 0..=train.len() - window_len {
        windows
            .entry(rolling_hash(&train[offset..offset + window_len]))
            .or_default()
            .push(offset);
    }
    windows
}

fn rolling_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(*byte)
    })
}
