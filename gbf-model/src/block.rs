//! Model block topology for shared sequence state and FFN-only MoE routing.

use crate::config::{
    DenseFfnConfig, FfnPathConfig, MoeBlockConfig, MoeFfnConfig, SharedSequenceConfig,
    SharedSequenceKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelBlockTopology {
    config: MoeBlockConfig,
}

impl ModelBlockTopology {
    pub fn new(config: MoeBlockConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &MoeBlockConfig {
        &self.config
    }

    pub fn plan(&self) -> BlockTopologyPlan {
        let shared_sequence = SharedSequenceStage::from_config(self.config.shared_sequence_block());
        let ffn_path = match self.config.ffn_path() {
            FfnPathConfig::Dense(config) => {
                FfnExecutionPath::Dense(DenseFfnStage::from_config(config))
            }
            FfnPathConfig::Moe(config) => FfnExecutionPath::Moe {
                router: RouterStage {
                    d_model: config.d_model(),
                    n_experts: config.n_experts(),
                },
                ffn: MoeFfnStage::from_config(config),
            },
        };

        BlockTopologyPlan {
            shared_sequence,
            ffn_path,
        }
    }

    pub fn stages(&self) -> Vec<BlockExecutionStage> {
        self.plan().stages()
    }

    pub fn sequence_state_update_count(&self) -> usize {
        self.plan().sequence_state_update_count()
    }

    pub fn router_invocations_per_token(&self) -> usize {
        self.plan().router_invocations_per_token()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTopologyPlan {
    shared_sequence: SharedSequenceStage,
    ffn_path: FfnExecutionPath,
}

impl BlockTopologyPlan {
    pub fn shared_sequence(&self) -> SharedSequenceStage {
        self.shared_sequence
    }

    pub fn ffn_path(&self) -> &FfnExecutionPath {
        &self.ffn_path
    }

    pub fn sequence_state_update_count(&self) -> usize {
        1
    }

    pub fn router_invocations_per_token(&self) -> usize {
        usize::from(matches!(self.ffn_path, FfnExecutionPath::Moe { .. }))
    }

    pub fn stages(&self) -> Vec<BlockExecutionStage> {
        let mut stages = vec![BlockExecutionStage::SharedSequence(self.shared_sequence)];
        match self.ffn_path {
            FfnExecutionPath::Dense(stage) => stages.push(BlockExecutionStage::DenseFfn(stage)),
            FfnExecutionPath::Moe { router, ffn } => {
                stages.push(BlockExecutionStage::Router(router));
                stages.push(BlockExecutionStage::MoeFfn(ffn));
            }
        }
        stages
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfnExecutionPath {
    Dense(DenseFfnStage),
    Moe {
        router: RouterStage,
        ffn: MoeFfnStage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSequenceStage {
    pub kind: SharedSequenceKind,
    pub d_model: usize,
    pub state_width: usize,
}

impl SharedSequenceStage {
    fn from_config(config: &SharedSequenceConfig) -> Self {
        Self {
            kind: config.kind(),
            d_model: config.d_model(),
            state_width: config.state_width(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DenseFfnStage {
    pub d_model: usize,
    pub d_ff: usize,
}

impl DenseFfnStage {
    fn from_config(config: &DenseFfnConfig) -> Self {
        Self {
            d_model: config.d_model(),
            d_ff: config.d_ff(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterStage {
    pub d_model: usize,
    pub n_experts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoeFfnStage {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_experts: usize,
}

impl MoeFfnStage {
    fn from_config(config: &MoeFfnConfig) -> Self {
        Self {
            d_model: config.d_model(),
            d_ff: config.d_ff(),
            n_experts: config.n_experts(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockExecutionStage {
    SharedSequence(SharedSequenceStage),
    DenseFfn(DenseFfnStage),
    Router(RouterStage),
    MoeFfn(MoeFfnStage),
}

#[cfg(test)]
mod tests {
    use crate::config::{DenseFfnConfig, ModelConfigError, ModelTopologyConfig, MoeFfnConfig};

    use super::*;

    #[test]
    fn block_topology_routes_only_ffn_path_for_moe_blocks() {
        let block = ModelBlockTopology::new(moe_block());

        assert_eq!(
            block.stages(),
            vec![
                BlockExecutionStage::SharedSequence(SharedSequenceStage {
                    kind: SharedSequenceKind::LinearState,
                    d_model: 8,
                    state_width: 4,
                }),
                BlockExecutionStage::Router(RouterStage {
                    d_model: 8,
                    n_experts: 2,
                }),
                BlockExecutionStage::MoeFfn(MoeFfnStage {
                    d_model: 8,
                    d_ff: 16,
                    n_experts: 2,
                }),
            ]
        );
        assert!(matches!(
            block.plan().ffn_path(),
            FfnExecutionPath::Moe { .. }
        ));
        assert_eq!(block.sequence_state_update_count(), 1);
        assert_eq!(block.router_invocations_per_token(), 1);
        assert!(block.config().has_moe_ffn());
    }

    #[test]
    fn block_topology_uses_dense_ffn_when_moe_is_disabled() {
        let block = ModelBlockTopology::new(dense_block());

        assert_eq!(
            block.stages(),
            vec![
                BlockExecutionStage::SharedSequence(SharedSequenceStage {
                    kind: SharedSequenceKind::BoundedKv,
                    d_model: 8,
                    state_width: 6,
                }),
                BlockExecutionStage::DenseFfn(DenseFfnStage {
                    d_model: 8,
                    d_ff: 16,
                }),
            ]
        );
        assert!(matches!(
            block.plan().ffn_path(),
            FfnExecutionPath::Dense(_)
        ));
        assert_eq!(block.sequence_state_update_count(), 1);
        assert_eq!(block.router_invocations_per_token(), 0);
        assert!(!block.config().has_moe_ffn());
    }

    #[test]
    fn block_config_rejects_moe_sequence_and_ffn_model_dim_mismatch() {
        let err = MoeBlockConfig::moe_ffn(
            SharedSequenceConfig::linear_state(8, 4).unwrap(),
            MoeFfnConfig::new(7, 16, 2).unwrap(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ModelConfigError::FfnModelDimMismatch {
                path: "moe_ffn",
                expected: 8,
                actual: 7,
            }
        );
    }

    #[test]
    fn block_config_rejects_dense_sequence_and_ffn_model_dim_mismatch() {
        let err = MoeBlockConfig::dense_ffn(
            SharedSequenceConfig::linear_state(8, 4).unwrap(),
            DenseFfnConfig::new(9, 16).unwrap(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ModelConfigError::FfnModelDimMismatch {
                path: "dense_ffn",
                expected: 8,
                actual: 9,
            }
        );
    }

    #[test]
    fn block_config_rejects_zero_dimensions_and_empty_expert_sets() {
        assert_eq!(
            SharedSequenceConfig::linear_state(0, 4),
            Err(ModelConfigError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            SharedSequenceConfig::bounded_kv(8, 0, 4),
            Err(ModelConfigError::EmptyDimension {
                field: "max_context"
            })
        );
        assert_eq!(
            DenseFfnConfig::new(8, 0),
            Err(ModelConfigError::EmptyDimension { field: "d_ff" })
        );
        assert_eq!(
            MoeFfnConfig::new(8, 16, 0),
            Err(ModelConfigError::EmptyDimension { field: "n_experts" })
        );
    }

    #[test]
    fn block_model_topology_rejects_empty_block_sets() {
        assert_eq!(
            ModelTopologyConfig::new(vec![]),
            Err(ModelConfigError::EmptyBlockSet)
        );
    }

    #[test]
    fn block_model_topology_requires_consistent_model_dim() {
        let err =
            ModelTopologyConfig::new(vec![dense_block(), mismatched_dense_block()]).unwrap_err();

        assert_eq!(
            err,
            ModelConfigError::BlockModelDimMismatch {
                block_index: 1,
                expected: 8,
                actual: 9,
            }
        );
    }

    #[test]
    fn block_model_topology_requires_consistent_sequence_semantics() {
        let err = ModelTopologyConfig::new(vec![dense_block(), linear_dense_block()]).unwrap_err();

        assert_eq!(
            err,
            ModelConfigError::BlockSequenceSemanticsMismatch {
                block_index: 1,
                expected: dense_block().shared_sequence_block().sequence_semantics(),
                actual: linear_dense_block()
                    .shared_sequence_block()
                    .sequence_semantics(),
            }
        );
    }

    fn moe_block() -> MoeBlockConfig {
        MoeBlockConfig::moe_ffn(
            SharedSequenceConfig::linear_state(8, 4).unwrap(),
            MoeFfnConfig::new(8, 16, 2).unwrap(),
        )
        .unwrap()
    }

    fn dense_block() -> MoeBlockConfig {
        MoeBlockConfig::dense_ffn(
            SharedSequenceConfig::bounded_kv(8, 16, 6).unwrap(),
            DenseFfnConfig::new(8, 16).unwrap(),
        )
        .unwrap()
    }

    fn linear_dense_block() -> MoeBlockConfig {
        MoeBlockConfig::dense_ffn(
            SharedSequenceConfig::linear_state(8, 6).unwrap(),
            DenseFfnConfig::new(8, 16).unwrap(),
        )
        .unwrap()
    }

    fn mismatched_dense_block() -> MoeBlockConfig {
        MoeBlockConfig::dense_ffn(
            SharedSequenceConfig::linear_state(9, 6).unwrap(),
            DenseFfnConfig::new(9, 16).unwrap(),
        )
        .unwrap()
    }
}
