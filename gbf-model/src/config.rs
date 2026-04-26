//! Backend-independent model topology configuration.

use std::error::Error;
use std::fmt;

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
        for (block_index, block) in blocks.iter().enumerate() {
            if block.d_model() != d_model {
                return Err(ModelConfigError::BlockModelDimMismatch {
                    block_index,
                    expected: d_model,
                    actual: block.d_model(),
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
    kind: SharedSequenceKind,
    d_model: usize,
    state_width: usize,
}

impl SharedSequenceConfig {
    pub fn linear_state(d_model: usize, state_width: usize) -> Result<Self, ModelConfigError> {
        Self::new(SharedSequenceKind::LinearState, d_model, state_width)
    }

    pub fn bounded_kv(d_model: usize, state_width: usize) -> Result<Self, ModelConfigError> {
        Self::new(SharedSequenceKind::BoundedKv, d_model, state_width)
    }

    fn new(
        kind: SharedSequenceKind,
        d_model: usize,
        state_width: usize,
    ) -> Result<Self, ModelConfigError> {
        validate_nonzero("d_model", d_model)?;
        validate_nonzero("state_width", state_width)?;
        Ok(Self {
            kind,
            d_model,
            state_width,
        })
    }

    pub fn kind(&self) -> SharedSequenceKind {
        self.kind
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn state_width(&self) -> usize {
        self.state_width
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
}

impl fmt::Display for ModelConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBlockSet => f.write_str("model topology must contain at least one block"),
            Self::EmptyDimension { field } => write!(f, "{field} must be nonzero"),
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
