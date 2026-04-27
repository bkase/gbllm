//! Backend-independent model topology configuration.

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;

use serde::{Deserialize, Deserializer};

use crate::qat::SharedDenseBranchConfig;
use crate::sequence::{SequenceExportFacts, SequenceSemanticsSpec};

const BOUNDED_KV_SLOT_BYTES: u16 = 4;
const BOUNDED_KV_MIN_RECORD_BYTES: u16 = 2 * BOUNDED_KV_SLOT_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTopologyConfig {
    blocks: Vec<MoeBlockConfig>,
}

impl ModelTopologyConfig {
    pub fn new(blocks: Vec<MoeBlockConfig>) -> Result<Self, ModelConfigError> {
        if blocks.is_empty() {
            return Err(ModelConfigError::EmptyBlockSet);
        }

        let d_model = blocks[0].d_model();
        let sequence_semantics = blocks[0].shared_sequence_block().sequence_semantics();
        for (block_index, block) in blocks.iter().enumerate() {
            if block.d_model() != d_model {
                return Err(ModelConfigError::BlockModelDimMismatch {
                    block_index,
                    expected: d_model,
                    actual: block.d_model(),
                });
            }
            if block.shared_sequence_block().sequence_semantics() != sequence_semantics {
                return Err(ModelConfigError::BlockSequenceSemanticsMismatch {
                    block_index,
                    expected: sequence_semantics,
                    actual: block.shared_sequence_block().sequence_semantics(),
                });
            }
        }

        Ok(Self { blocks })
    }

    pub fn from_moe_block_selection(
        total_blocks: usize,
        block_selection: &MoeBlockSelection,
        shared_sequence_block: SharedSequenceConfig,
        dense_ffn: Option<DenseFfnConfig>,
        moe_ffn: MoeFfnConfig,
    ) -> Result<Self, ModelConfigError> {
        validate_nonzero("total_blocks", total_blocks)?;
        validate_same_d_model(
            "moe_ffn",
            shared_sequence_block.d_model(),
            moe_ffn.d_model(),
        )?;

        let selected_blocks = block_selection.selected_blocks(total_blocks)?;
        let needs_dense_ffn = selected_blocks.len() < total_blocks;
        let dense_ffn = if needs_dense_ffn {
            let dense_ffn = dense_ffn.ok_or(ModelConfigError::MissingDenseFfnForNonMoeBlocks)?;
            validate_same_d_model(
                "dense_ffn",
                shared_sequence_block.d_model(),
                dense_ffn.d_model(),
            )?;
            if dense_ffn.d_ff() != moe_ffn.d_ff() {
                return Err(ModelConfigError::FfnHiddenDimMismatch {
                    dense_d_ff: dense_ffn.d_ff(),
                    moe_d_ff: moe_ffn.d_ff(),
                });
            }
            Some(dense_ffn)
        } else {
            dense_ffn
        };

        let mut blocks = Vec::with_capacity(total_blocks);
        for block_index in 0..total_blocks {
            let block = if selected_blocks.contains(&block_index) {
                MoeBlockConfig::moe_ffn(shared_sequence_block.clone(), moe_ffn.clone())?
            } else {
                MoeBlockConfig::dense_ffn(
                    shared_sequence_block.clone(),
                    dense_ffn
                        .clone()
                        .expect("dense FFN was validated for non-MoE blocks"),
                )?
            };
            blocks.push(block);
        }

        Self::new(blocks)
    }

    pub fn blocks(&self) -> &[MoeBlockConfig] {
        &self.blocks
    }

    pub fn d_model(&self) -> usize {
        self.blocks[0].d_model()
    }

    pub fn sequence_semantics(&self) -> SequenceSemanticsSpec {
        self.blocks[0].shared_sequence_block().sequence_semantics()
    }

    pub fn sequence_export_facts(&self) -> SequenceExportFacts {
        SequenceExportFacts::for_spec(self.sequence_semantics())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoeBlockSelection {
    AllBlocks,
    Alternating(MoeAlternatingBlockSelection),
    Middle(MoeMiddleBlockSelection),
    Explicit(MoeExplicitBlockSelection),
}

impl MoeBlockSelection {
    pub fn all_blocks() -> Self {
        Self::AllBlocks
    }

    pub fn alternating(start: usize, stride: usize) -> Result<Self, ModelConfigError> {
        Ok(Self::Alternating(MoeAlternatingBlockSelection::new(
            start, stride,
        )?))
    }

    pub fn middle(first_moe: usize, last_moe: usize) -> Result<Self, ModelConfigError> {
        Ok(Self::Middle(MoeMiddleBlockSelection::new(
            first_moe, last_moe,
        )?))
    }

    pub fn explicit(block_indices: Vec<usize>) -> Result<Self, ModelConfigError> {
        Ok(Self::Explicit(MoeExplicitBlockSelection::new(
            block_indices,
        )?))
    }

    pub fn contains(
        &self,
        block_idx: usize,
        total_blocks: usize,
    ) -> Result<bool, ModelConfigError> {
        Ok(self.selected_blocks(total_blocks)?.contains(&block_idx))
    }

    pub fn selected_blocks(&self, total_blocks: usize) -> Result<Vec<usize>, ModelConfigError> {
        validate_nonzero("total_blocks", total_blocks)?;

        let block_indices = match self {
            Self::AllBlocks => (0..total_blocks).collect(),
            Self::Alternating(selection) => {
                let mut block_indices = Vec::new();
                let mut block_idx = selection.start();
                while block_idx < total_blocks {
                    block_indices.push(block_idx);
                    let Some(next_block_idx) = block_idx.checked_add(selection.stride().get())
                    else {
                        break;
                    };
                    block_idx = next_block_idx;
                }
                block_indices
            }
            Self::Middle(selection) => {
                validate_block_index(selection.last_moe(), total_blocks)?;
                (selection.first_moe()..=selection.last_moe()).collect()
            }
            Self::Explicit(selection) => {
                for block_idx in selection.block_indices() {
                    validate_block_index(*block_idx, total_blocks)?;
                }
                selection.block_indices().to_vec()
            }
        };

        if block_indices.is_empty() {
            return Err(ModelConfigError::EmptyBlockSelection);
        }

        Ok(block_indices)
    }
}

impl<'de> Deserialize<'de> for MoeBlockSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = MoeBlockSelectionToml::deserialize(deserializer)?;
        match raw {
            MoeBlockSelectionToml::AllBlocks => Ok(Self::all_blocks()),
            MoeBlockSelectionToml::Alternating { start, stride } => {
                Self::alternating(start, stride).map_err(serde::de::Error::custom)
            }
            MoeBlockSelectionToml::Middle {
                first_moe,
                last_moe,
            } => Self::middle(first_moe, last_moe).map_err(serde::de::Error::custom),
            MoeBlockSelectionToml::Explicit { block_indices } => {
                Self::explicit(block_indices).map_err(serde::de::Error::custom)
            }
        }
    }
}

impl Default for MoeBlockSelection {
    fn default() -> Self {
        Self::alternating(2, 2).expect("default MoE block selection has nonzero stride")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoeAlternatingBlockSelection {
    start: usize,
    stride: NonZeroUsize,
}

impl MoeAlternatingBlockSelection {
    pub fn new(start: usize, stride: usize) -> Result<Self, ModelConfigError> {
        let stride = NonZeroUsize::new(stride).ok_or(ModelConfigError::ZeroBlockSelectionStride)?;
        Ok(Self { start, stride })
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn stride(&self) -> NonZeroUsize {
        self.stride
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoeMiddleBlockSelection {
    first_moe: usize,
    last_moe: usize,
}

impl MoeMiddleBlockSelection {
    pub fn new(first_moe: usize, last_moe: usize) -> Result<Self, ModelConfigError> {
        if first_moe > last_moe {
            return Err(ModelConfigError::InvalidBlockSelectionRange {
                first_moe,
                last_moe,
            });
        }
        Ok(Self {
            first_moe,
            last_moe,
        })
    }

    pub fn first_moe(&self) -> usize {
        self.first_moe
    }

    pub fn last_moe(&self) -> usize {
        self.last_moe
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoeExplicitBlockSelection {
    block_indices: Vec<usize>,
}

impl MoeExplicitBlockSelection {
    pub fn new(block_indices: Vec<usize>) -> Result<Self, ModelConfigError> {
        validate_explicit_block_selection(&block_indices)?;
        Ok(Self { block_indices })
    }

    pub fn block_indices(&self) -> &[usize] {
        &self.block_indices
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum MoeBlockSelectionToml {
    AllBlocks,
    Alternating { start: usize, stride: usize },
    Middle { first_moe: usize, last_moe: usize },
    Explicit { block_indices: Vec<usize> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoeBlockConfig {
    shared_sequence_block: SharedSequenceConfig,
    ffn_path: FfnPathConfig,
}

impl MoeBlockConfig {
    pub fn dense_ffn(
        shared_sequence_block: SharedSequenceConfig,
        dense_ffn: DenseFfnConfig,
    ) -> Result<Self, ModelConfigError> {
        validate_same_d_model(
            "dense_ffn",
            shared_sequence_block.d_model(),
            dense_ffn.d_model(),
        )?;
        Ok(Self {
            shared_sequence_block,
            ffn_path: FfnPathConfig::Dense(dense_ffn),
        })
    }

    pub fn moe_ffn(
        shared_sequence_block: SharedSequenceConfig,
        moe_ffn: MoeFfnConfig,
    ) -> Result<Self, ModelConfigError> {
        validate_same_d_model(
            "moe_ffn",
            shared_sequence_block.d_model(),
            moe_ffn.d_model(),
        )?;
        Ok(Self {
            shared_sequence_block,
            ffn_path: FfnPathConfig::Moe(moe_ffn),
        })
    }

    pub fn shared_sequence_block(&self) -> &SharedSequenceConfig {
        &self.shared_sequence_block
    }

    pub fn ffn_path(&self) -> &FfnPathConfig {
        &self.ffn_path
    }

    pub fn has_moe_ffn(&self) -> bool {
        matches!(self.ffn_path, FfnPathConfig::Moe(_))
    }

    pub fn d_model(&self) -> usize {
        self.shared_sequence_block.d_model()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSequenceConfig {
    d_model: usize,
    semantics: SequenceSemanticsSpec,
}

impl SharedSequenceConfig {
    pub fn linear_state(
        d_model: usize,
        state_bytes_per_layer: u16,
    ) -> Result<Self, ModelConfigError> {
        Self::new(
            d_model,
            SequenceSemanticsSpec::linear_state(state_bytes_per_layer).map_err(|_| {
                ModelConfigError::EmptyDimension {
                    field: "state_bytes_per_layer",
                }
            })?,
        )
    }

    pub fn bounded_kv(
        d_model: usize,
        max_context: u16,
        kv_bytes_per_token: u16,
    ) -> Result<Self, ModelConfigError> {
        let semantics = SequenceSemanticsSpec::bounded_kv(max_context, kv_bytes_per_token)
            .map_err(|err| match err {
                crate::sequence::SequenceSemanticsError::ZeroField { field } => {
                    ModelConfigError::EmptyDimension { field }
                }
                crate::sequence::SequenceSemanticsError::StateSizeMismatch { .. } => {
                    unreachable!("bounded-kv constructor does not accept measured state size")
                }
            })?;
        validate_executable_bounded_kv_layout(kv_bytes_per_token)?;
        Self::new(d_model, semantics)
    }

    fn new(d_model: usize, semantics: SequenceSemanticsSpec) -> Result<Self, ModelConfigError> {
        validate_nonzero("d_model", d_model)?;
        validate_nonzero(
            "sequence_state_bytes",
            semantics.state_size().bytes_per_layer as usize,
        )?;
        Ok(Self { d_model, semantics })
    }

    pub fn kind(&self) -> SharedSequenceKind {
        match self.semantics {
            SequenceSemanticsSpec::LinearState(_) => SharedSequenceKind::LinearState,
            SequenceSemanticsSpec::BoundedKv(_) => SharedSequenceKind::BoundedKv,
        }
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn state_width(&self) -> usize {
        match self.semantics {
            SequenceSemanticsSpec::LinearState(semantics) => {
                usize::from(semantics.state_bytes_per_layer())
            }
            SequenceSemanticsSpec::BoundedKv(semantics) => {
                usize::from(semantics.kv_bytes_per_token())
            }
        }
    }

    pub fn sequence_semantics(&self) -> SequenceSemanticsSpec {
        self.semantics
    }

    pub fn sequence_export_facts(&self) -> SequenceExportFacts {
        SequenceExportFacts::for_spec(self.semantics)
    }
}

/// Topology marker for the shared sequence-state path.
///
/// This bead does not implement the math for either variant; it only records
/// that the sequence-state path is block-shared and outside expert routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedSequenceKind {
    LinearState,
    BoundedKv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfnPathConfig {
    Dense(DenseFfnConfig),
    Moe(MoeFfnConfig),
}

impl FfnPathConfig {
    pub fn d_model(&self) -> usize {
        match self {
            Self::Dense(config) => config.d_model(),
            Self::Moe(config) => config.d_model(),
        }
    }

    pub fn d_ff(&self) -> usize {
        match self {
            Self::Dense(config) => config.d_ff(),
            Self::Moe(config) => config.d_ff(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenseFfnConfig {
    d_model: usize,
    d_ff: usize,
}

impl DenseFfnConfig {
    pub fn new(d_model: usize, d_ff: usize) -> Result<Self, ModelConfigError> {
        validate_nonzero("d_model", d_model)?;
        validate_nonzero("d_ff", d_ff)?;
        Ok(Self { d_model, d_ff })
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn d_ff(&self) -> usize {
        self.d_ff
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoeFfnConfig {
    d_model: usize,
    d_ff: usize,
    n_experts: usize,
    shared_dense_branch: Option<SharedDenseBranchConfig>,
}

impl MoeFfnConfig {
    pub fn new(d_model: usize, d_ff: usize, n_experts: usize) -> Result<Self, ModelConfigError> {
        Self::with_shared_dense_branch(d_model, d_ff, n_experts, None)
    }

    pub fn with_shared_dense_branch(
        d_model: usize,
        d_ff: usize,
        n_experts: usize,
        shared_dense_branch: Option<SharedDenseBranchConfig>,
    ) -> Result<Self, ModelConfigError> {
        validate_nonzero("d_model", d_model)?;
        validate_nonzero("d_ff", d_ff)?;
        validate_nonzero("n_experts", n_experts)?;
        if let Some(shared_dense_branch) = shared_dense_branch {
            if shared_dense_branch.d_model() != d_model {
                return Err(ModelConfigError::SharedDenseBranchModelDimMismatch {
                    expected: d_model,
                    actual: shared_dense_branch.d_model(),
                });
            }
            if shared_dense_branch.d_ff_shared() >= d_ff {
                return Err(ModelConfigError::SharedDenseBranchNotSmallerThanExpert {
                    d_ff_shared: shared_dense_branch.d_ff_shared(),
                    d_ff,
                });
            }
        }

        Ok(Self {
            d_model,
            d_ff,
            n_experts,
            shared_dense_branch,
        })
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn d_ff(&self) -> usize {
        self.d_ff
    }

    pub fn n_experts(&self) -> usize {
        self.n_experts
    }

    pub fn shared_dense_branch(&self) -> Option<SharedDenseBranchConfig> {
        self.shared_dense_branch
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelConfigError {
    EmptyBlockSet,
    EmptyDimension {
        field: &'static str,
    },
    InvalidSequenceStateLayout {
        field: &'static str,
        value: u16,
        reason: &'static str,
    },
    FfnModelDimMismatch {
        path: &'static str,
        expected: usize,
        actual: usize,
    },
    FfnHiddenDimMismatch {
        dense_d_ff: usize,
        moe_d_ff: usize,
    },
    MissingDenseFfnForNonMoeBlocks,
    SharedDenseBranchModelDimMismatch {
        expected: usize,
        actual: usize,
    },
    SharedDenseBranchNotSmallerThanExpert {
        d_ff_shared: usize,
        d_ff: usize,
    },
    BlockModelDimMismatch {
        block_index: usize,
        expected: usize,
        actual: usize,
    },
    BlockSequenceSemanticsMismatch {
        block_index: usize,
        expected: SequenceSemanticsSpec,
        actual: SequenceSemanticsSpec,
    },
    EmptyBlockSelection,
    ZeroBlockSelectionStride,
    InvalidBlockSelectionRange {
        first_moe: usize,
        last_moe: usize,
    },
    BlockSelectionIndexOutOfRange {
        index: usize,
        total_blocks: usize,
    },
    DuplicateBlockSelectionIndex {
        index: usize,
    },
}

impl fmt::Display for ModelConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBlockSet => f.write_str("model topology must contain at least one block"),
            Self::EmptyDimension { field } => write!(f, "{field} must be nonzero"),
            Self::InvalidSequenceStateLayout {
                field,
                value,
                reason,
            } => write!(f, "{field}={value} has invalid sequence layout: {reason}"),
            Self::FfnModelDimMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "{path} d_model mismatch: expected shared sequence d_model {expected}, got {actual}"
            ),
            Self::FfnHiddenDimMismatch {
                dense_d_ff,
                moe_d_ff,
            } => write!(
                f,
                "dense and MoE FFN hidden dimensions must match: dense d_ff {dense_d_ff}, MoE d_ff {moe_d_ff}"
            ),
            Self::MissingDenseFfnForNonMoeBlocks => f.write_str(
                "dense FFN config is required when block selection leaves non-MoE blocks",
            ),
            Self::SharedDenseBranchModelDimMismatch { expected, actual } => write!(
                f,
                "shared dense branch d_model mismatch: expected {expected}, got {actual}"
            ),
            Self::SharedDenseBranchNotSmallerThanExpert { d_ff_shared, d_ff } => write!(
                f,
                "shared dense branch d_ff_shared {d_ff_shared} must be smaller than expert d_ff {d_ff}"
            ),
            Self::BlockModelDimMismatch {
                block_index,
                expected,
                actual,
            } => write!(
                f,
                "block {block_index} d_model mismatch: expected {expected}, got {actual}"
            ),
            Self::BlockSequenceSemanticsMismatch {
                block_index,
                expected,
                actual,
            } => write!(
                f,
                "block {block_index} sequence semantics mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::EmptyBlockSelection => {
                f.write_str("MoE block selection must select at least one block")
            }
            Self::ZeroBlockSelectionStride => {
                f.write_str("MoE block selection stride must be nonzero")
            }
            Self::InvalidBlockSelectionRange {
                first_moe,
                last_moe,
            } => write!(
                f,
                "invalid MoE block selection range: first_moe {first_moe} is after last_moe {last_moe}"
            ),
            Self::BlockSelectionIndexOutOfRange {
                index,
                total_blocks,
            } => write!(
                f,
                "MoE block selection index {index} is out of range for {total_blocks} blocks"
            ),
            Self::DuplicateBlockSelectionIndex { index } => {
                write!(f, "MoE block selection repeats block index {index}")
            }
        }
    }
}

impl Error for ModelConfigError {}

fn validate_nonzero(field: &'static str, value: usize) -> Result<(), ModelConfigError> {
    if value == 0 {
        return Err(ModelConfigError::EmptyDimension { field });
    }
    Ok(())
}

fn validate_executable_bounded_kv_layout(kv_bytes_per_token: u16) -> Result<(), ModelConfigError> {
    if kv_bytes_per_token == 0 {
        return Ok(());
    }
    if !kv_bytes_per_token.is_multiple_of(BOUNDED_KV_SLOT_BYTES) {
        return Err(ModelConfigError::InvalidSequenceStateLayout {
            field: "kv_bytes_per_token",
            value: kv_bytes_per_token,
            reason: "must be divisible by 4 for f32-backed bounded-kv records",
        });
    }
    if kv_bytes_per_token < BOUNDED_KV_MIN_RECORD_BYTES {
        return Err(ModelConfigError::InvalidSequenceStateLayout {
            field: "kv_bytes_per_token",
            value: kv_bytes_per_token,
            reason: "must reserve one f32 validity flag and one f32 tied key/value payload",
        });
    }

    Ok(())
}

fn validate_same_d_model(
    path: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), ModelConfigError> {
    if expected != actual {
        return Err(ModelConfigError::FfnModelDimMismatch {
            path,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_block_index(index: usize, total_blocks: usize) -> Result<(), ModelConfigError> {
    if index >= total_blocks {
        return Err(ModelConfigError::BlockSelectionIndexOutOfRange {
            index,
            total_blocks,
        });
    }
    Ok(())
}

fn validate_explicit_block_selection(block_indices: &[usize]) -> Result<(), ModelConfigError> {
    if block_indices.is_empty() {
        return Err(ModelConfigError::EmptyBlockSelection);
    }

    let mut sorted_indices = block_indices.to_vec();
    sorted_indices.sort_unstable();
    for duplicate_pair in sorted_indices.windows(2) {
        if duplicate_pair[0] == duplicate_pair[1] {
            return Err(ModelConfigError::DuplicateBlockSelectionIndex {
                index: duplicate_pair[0],
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod moe {
    use super::*;

    #[test]
    fn default_alternating_selection_starts_at_two_and_strides_by_two() {
        let selection = MoeBlockSelection::default();

        assert_eq!(selection.selected_blocks(12).unwrap(), vec![2, 4, 6, 8, 10]);
        assert!(selection.contains(4, 12).unwrap());
        assert!(!selection.contains(3, 12).unwrap());
    }

    #[test]
    fn all_blocks_selection_includes_every_block() {
        let selection = MoeBlockSelection::all_blocks();

        assert_eq!(selection.selected_blocks(4).unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn middle_selection_includes_closed_range() {
        let selection = MoeBlockSelection::middle(3, 8).unwrap();

        assert_eq!(
            selection.selected_blocks(12).unwrap(),
            vec![3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn explicit_selection_preserves_requested_indices() {
        let selection = MoeBlockSelection::explicit(vec![5, 1, 9]).unwrap();

        assert_eq!(selection.selected_blocks(12).unwrap(), vec![5, 1, 9]);
    }

    #[test]
    fn selection_rejects_empty_and_invalid_indices() {
        assert_eq!(
            MoeBlockSelection::explicit(vec![]),
            Err(ModelConfigError::EmptyBlockSelection)
        );
        assert_eq!(
            MoeBlockSelection::alternating(2, 0),
            Err(ModelConfigError::ZeroBlockSelectionStride)
        );
        assert_eq!(
            MoeBlockSelection::middle(8, 3),
            Err(ModelConfigError::InvalidBlockSelectionRange {
                first_moe: 8,
                last_moe: 3,
            })
        );
        assert_eq!(
            MoeBlockSelection::explicit(vec![1, 1]),
            Err(ModelConfigError::DuplicateBlockSelectionIndex { index: 1 })
        );
        assert_eq!(
            MoeBlockSelection::alternating(12, 2)
                .unwrap()
                .selected_blocks(12),
            Err(ModelConfigError::EmptyBlockSelection)
        );
        assert_eq!(
            MoeBlockSelection::explicit(vec![0, 12])
                .unwrap()
                .selected_blocks(12),
            Err(ModelConfigError::BlockSelectionIndexOutOfRange {
                index: 12,
                total_blocks: 12,
            })
        );
        assert_eq!(
            MoeBlockSelection::all_blocks().contains(0, 0),
            Err(ModelConfigError::EmptyDimension {
                field: "total_blocks"
            })
        );
    }

    #[test]
    fn topology_from_selection_assigns_moe_and_dense_ffn_paths() {
        let topology = ModelTopologyConfig::from_moe_block_selection(
            6,
            &MoeBlockSelection::explicit(vec![1, 4]).unwrap(),
            shared_sequence(),
            Some(dense_ffn(16)),
            moe_ffn(16),
        )
        .unwrap();

        let has_moe_ffn = topology
            .blocks()
            .iter()
            .map(MoeBlockConfig::has_moe_ffn)
            .collect::<Vec<_>>();

        assert_eq!(has_moe_ffn, vec![false, true, false, false, true, false]);
        assert!(
            topology
                .blocks()
                .iter()
                .all(|block| block.ffn_path().d_ff() == 16)
        );
    }

    #[test]
    fn topology_from_selection_supports_default_middle_and_all_block_masks() {
        assert_eq!(
            topology_moe_mask(6, &MoeBlockSelection::default()).unwrap(),
            vec![false, false, true, false, true, false]
        );
        assert_eq!(
            topology_moe_mask(6, &MoeBlockSelection::middle(2, 4).unwrap()).unwrap(),
            vec![false, false, true, true, true, false]
        );

        let topology = ModelTopologyConfig::from_moe_block_selection(
            4,
            &MoeBlockSelection::all_blocks(),
            shared_sequence(),
            None,
            moe_ffn(16),
        )
        .unwrap();
        assert!(topology.blocks().iter().all(MoeBlockConfig::has_moe_ffn));
    }

    #[test]
    fn moe_ffn_shared_dense_branch_is_config_gated_and_absent_by_default() {
        let default = MoeFfnConfig::new(8, 16, 2).unwrap();
        assert_eq!(default.shared_dense_branch(), None);

        let shared_dense = SharedDenseBranchConfig::new(8, 4).unwrap();
        let configured =
            MoeFfnConfig::with_shared_dense_branch(8, 16, 2, Some(shared_dense)).unwrap();
        assert_eq!(configured.shared_dense_branch(), Some(shared_dense));
    }

    #[test]
    fn moe_ffn_shared_dense_branch_rejects_wrong_shape_and_non_small_width() {
        assert_eq!(
            MoeFfnConfig::with_shared_dense_branch(
                8,
                16,
                2,
                Some(SharedDenseBranchConfig::new(7, 4).unwrap())
            ),
            Err(ModelConfigError::SharedDenseBranchModelDimMismatch {
                expected: 8,
                actual: 7,
            })
        );
        assert_eq!(
            MoeFfnConfig::with_shared_dense_branch(
                8,
                16,
                2,
                Some(SharedDenseBranchConfig::new(8, 16).unwrap())
            ),
            Err(ModelConfigError::SharedDenseBranchNotSmallerThanExpert {
                d_ff_shared: 16,
                d_ff: 16,
            })
        );
    }

    #[test]
    fn topology_from_selection_requires_same_dense_and_moe_ffn_width() {
        let err = ModelTopologyConfig::from_moe_block_selection(
            4,
            &MoeBlockSelection::explicit(vec![1]).unwrap(),
            shared_sequence(),
            Some(dense_ffn(12)),
            moe_ffn(16),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ModelConfigError::FfnHiddenDimMismatch {
                dense_d_ff: 12,
                moe_d_ff: 16,
            }
        );
    }

    #[test]
    fn topology_from_selection_rejects_missing_dense_and_invalid_depth_bounds() {
        let err = ModelTopologyConfig::from_moe_block_selection(
            4,
            &MoeBlockSelection::explicit(vec![1]).unwrap(),
            shared_sequence(),
            None,
            moe_ffn(16),
        )
        .unwrap_err();
        assert_eq!(err, ModelConfigError::MissingDenseFfnForNonMoeBlocks);

        let err = ModelTopologyConfig::from_moe_block_selection(
            0,
            &MoeBlockSelection::all_blocks(),
            shared_sequence(),
            None,
            moe_ffn(16),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelConfigError::EmptyDimension {
                field: "total_blocks"
            }
        );

        let err = ModelTopologyConfig::from_moe_block_selection(
            6,
            &MoeBlockSelection::middle(3, 8).unwrap(),
            shared_sequence(),
            Some(dense_ffn(16)),
            moe_ffn(16),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelConfigError::BlockSelectionIndexOutOfRange {
                index: 8,
                total_blocks: 6,
            }
        );

        let err = ModelTopologyConfig::from_moe_block_selection(
            6,
            &MoeBlockSelection::explicit(vec![0, 6]).unwrap(),
            shared_sequence(),
            Some(dense_ffn(16)),
            moe_ffn(16),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelConfigError::BlockSelectionIndexOutOfRange {
                index: 6,
                total_blocks: 6,
            }
        );
    }

    #[test]
    fn block_selection_parses_from_model_moe_toml() {
        #[derive(Debug, serde::Deserialize)]
        struct ModelToml {
            model: ModelSection,
        }

        #[derive(Debug, serde::Deserialize)]
        struct ModelSection {
            moe: MoeSection,
        }

        #[derive(Debug, serde::Deserialize)]
        struct MoeSection {
            block_selection: MoeBlockSelection,
        }

        let config: ModelToml = toml::from_str(
            r#"
            [model.moe]
            block_selection = { type = "alternating", start = 2, stride = 2 }
            "#,
        )
        .unwrap();

        assert_eq!(
            config
                .model
                .moe
                .block_selection
                .selected_blocks(12)
                .unwrap(),
            vec![2, 4, 6, 8, 10]
        );

        let invalid = toml::from_str::<ModelToml>(
            r#"
            [model.moe]
            block_selection = { type = "explicit", block_indices = [] }
            "#,
        )
        .unwrap_err();
        assert!(
            invalid
                .to_string()
                .contains("MoE block selection must select at least one block")
        );
    }

    fn topology_moe_mask(
        total_blocks: usize,
        selection: &MoeBlockSelection,
    ) -> Result<Vec<bool>, ModelConfigError> {
        let topology = ModelTopologyConfig::from_moe_block_selection(
            total_blocks,
            selection,
            shared_sequence(),
            Some(dense_ffn(16)),
            moe_ffn(16),
        )?;
        Ok(topology
            .blocks()
            .iter()
            .map(MoeBlockConfig::has_moe_ffn)
            .collect())
    }

    fn shared_sequence() -> SharedSequenceConfig {
        SharedSequenceConfig::linear_state(8, 4).unwrap()
    }

    fn dense_ffn(d_ff: usize) -> DenseFfnConfig {
        DenseFfnConfig::new(8, d_ff).unwrap()
    }

    fn moe_ffn(d_ff: usize) -> MoeFfnConfig {
        MoeFfnConfig::new(8, d_ff, 2).unwrap()
    }
}
