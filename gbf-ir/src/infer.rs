//! Minimal inference IR node owned by F-B1.
//!
//! This is a deliberately skeletal, single-op IR. M1 replaces/extends this
//! with effect edges, observation visitors, range annotations, and additional
//! model op kinds.

use std::fmt;

use gbf_abi::compute_shape::SquareDim;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferGraph {
    nodes: Vec<InferNode>,
}

impl InferGraph {
    pub fn f_b1_single_matmul(node: MatmulI8Node) -> Result<Self, InferGraphError> {
        let graph = Self {
            nodes: vec![InferNode::MatmulI8(node), InferNode::End],
        };
        graph.validate_f_b1()?;
        Ok(graph)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(nodes: Vec<InferNode>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn nodes(&self) -> &[InferNode] {
        &self.nodes
    }

    pub fn validate_f_b1(&self) -> Result<(), InferGraphError> {
        if self.nodes.len() != 2 {
            return Err(InferGraphError::WrongNodeCount {
                expected: 2,
                found: self.nodes.len(),
            });
        }
        let Some(InferNode::MatmulI8(node)) = self.nodes.first() else {
            return Err(InferGraphError::MissingMatmul);
        };
        if !node.a.read_only || !node.b.read_only {
            return Err(InferGraphError::InputMustBeReadOnly);
        }
        if node.out.storage != TensorStorage::HarnessStreamedOutput {
            return Err(InferGraphError::OutputMustBeHarnessStreamed);
        }
        if !matches!(self.nodes.get(1), Some(InferNode::End)) {
            return Err(InferGraphError::MissingEnd);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InferNode {
    MatmulI8(MatmulI8Node),
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatmulI8Node {
    pub a: TensorBinding,
    pub b: TensorBinding,
    pub out: TensorBinding,
    pub dim: SquareDim,
    pub tile_size: FusedTileSize,
    // M1: deferred fields and stages intentionally absent here:
    // effect-edge graph, ObservationPlan visitor hooks, RangePlan annotations,
    // LifetimeClass on TensorBinding, quantization metadata, and all non-i8
    // matmul/model op kinds.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorBinding {
    pub name: String,
    pub read_only: bool,
    pub storage: TensorStorage,
}

impl TensorBinding {
    #[must_use]
    pub fn read_only(name: impl Into<String>, storage: TensorStorage) -> Self {
        Self {
            name: name.into(),
            read_only: true,
            storage,
        }
    }

    #[must_use]
    pub fn writable(name: impl Into<String>, storage: TensorStorage) -> Self {
        Self {
            name: name.into(),
            read_only: false,
            storage,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorStorage {
    WramSmoke,
    SwitchableRom,
    HarnessStreamedOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FusedTileSize {
    pub m: u16,
    pub n: u16,
    pub k: u16,
}

impl FusedTileSize {
    #[must_use]
    pub const fn f_b1() -> Self {
        Self {
            m: 16,
            n: 16,
            k: 16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferGraphError {
    WrongNodeCount { expected: usize, found: usize },
    MissingMatmul,
    MissingEnd,
    InputMustBeReadOnly,
    OutputMustBeHarnessStreamed,
}

impl fmt::Display for InferGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongNodeCount { expected, found } => {
                write!(f, "expected {expected} F-B1 IR nodes, found {found}")
            }
            Self::MissingMatmul => f.write_str("F-B1 IR must start with MatmulI8"),
            Self::MissingEnd => f.write_str("F-B1 IR must end with End"),
            Self::InputMustBeReadOnly => f.write_str("F-B1 MatmulI8 inputs must be read-only"),
            Self::OutputMustBeHarnessStreamed => {
                f.write_str("F-B1 MatmulI8 output must be harness-streamed")
            }
        }
    }
}

impl std::error::Error for InferGraphError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> MatmulI8Node {
        MatmulI8Node {
            a: TensorBinding::read_only("a", TensorStorage::WramSmoke),
            b: TensorBinding::read_only("b", TensorStorage::WramSmoke),
            out: TensorBinding::writable("out", TensorStorage::HarnessStreamedOutput),
            dim: SquareDim::new(16).expect("valid"),
            tile_size: FusedTileSize::f_b1(),
        }
    }

    #[test]
    fn f_b1_matmul_ir_single_node_shape() {
        let graph = InferGraph::f_b1_single_matmul(node()).expect("valid");
        assert!(matches!(graph.nodes()[0], InferNode::MatmulI8(_)));
        assert!(matches!(graph.nodes()[1], InferNode::End));
    }

    #[test]
    fn f_b1_matmul_ir_rejects_multi_op_graph() {
        let graph = InferGraph::new_for_test(vec![
            InferNode::MatmulI8(node()),
            InferNode::MatmulI8(node()),
            InferNode::End,
        ]);
        assert!(matches!(
            graph.validate_f_b1(),
            Err(InferGraphError::WrongNodeCount {
                expected: 2,
                found: 3
            })
        ));
    }

    #[test]
    fn f_b1_matmul_ir_a_b_are_read_only() {
        let mut bad = node();
        bad.a.read_only = false;
        let graph = InferGraph::new_for_test(vec![InferNode::MatmulI8(bad), InferNode::End]);
        assert_eq!(
            graph.validate_f_b1(),
            Err(InferGraphError::InputMustBeReadOnly)
        );
    }

    #[test]
    fn f_b1_matmul_ir_no_effect_edges() {
        let graph = InferGraph::f_b1_single_matmul(node()).expect("valid");
        let json = serde_json::to_value(graph).expect("serializes");
        assert!(json.get("effect_edges").is_none());
        assert!(json.get("range_plan").is_none());
        assert!(json.get("observation_plan").is_none());
    }
}
