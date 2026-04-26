//! Backend-independent model topology configuration.

use std::error::Error;
use std::fmt;

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
}

impl MoeFfnConfig {
    pub fn new(d_model: usize, d_ff: usize, n_experts: usize) -> Result<Self, ModelConfigError> {
        validate_nonzero("d_model", d_model)?;
        validate_nonzero("d_ff", d_ff)?;
        validate_nonzero("n_experts", n_experts)?;
        Ok(Self {
            d_model,
            d_ff,
            n_experts,
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
