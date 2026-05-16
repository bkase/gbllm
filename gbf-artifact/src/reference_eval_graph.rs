//! Canonical reference-program evaluation graph.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;
use crate::opset_v1::ReferenceOp;

/// Stable operation identifier inside a reference program graph.
pub type OpId = ArtifactPath;

/// Tensor reference consumed or produced by a reference graph node.
pub type TensorRef = ArtifactPath;

/// A single operation node in a reference evaluation graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceNode {
    pub op_id: OpId,
    pub op: ReferenceOp,
    pub inputs: Vec<TensorRef>,
    pub outputs: Vec<TensorRef>,
}

impl ReferenceNode {
    #[must_use]
    pub fn new(
        op_id: OpId,
        op: ReferenceOp,
        inputs: Vec<TensorRef>,
        outputs: Vec<TensorRef>,
    ) -> Self {
        Self {
            op_id,
            op,
            inputs,
            outputs,
        }
    }
}

/// Explicit data-flow dependency between two reference nodes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceEdge {
    pub from: OpId,
    pub to: OpId,
    pub tensor: TensorRef,
}

impl ReferenceEdge {
    #[must_use]
    pub fn new(from: OpId, to: OpId, tensor: TensorRef) -> Self {
        Self { from, to, tensor }
    }
}

/// Evaluation graph with nodes stored in deterministic topological order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceEvalGraph {
    pub nodes: Vec<ReferenceNode>,
    pub edges: Vec<ReferenceEdge>,
}

impl ReferenceEvalGraph {
    pub fn new(
        nodes: Vec<ReferenceNode>,
        edges: Vec<ReferenceEdge>,
    ) -> Result<Self, ReferenceGraphError> {
        Self { nodes, edges }.canonicalized()
    }

    pub fn canonicalized(&self) -> Result<Self, ReferenceGraphError> {
        let mut node_by_id = BTreeMap::new();
        for node in &self.nodes {
            if node_by_id
                .insert(node.op_id.clone(), node.clone())
                .is_some()
            {
                return Err(ReferenceGraphError::DuplicateNode {
                    op_id: node.op_id.clone(),
                });
            }
        }

        let mut sorted_edges = self.edges.clone();
        sorted_edges.sort();
        let mut unique_edges = BTreeSet::new();
        let mut outgoing: BTreeMap<OpId, BTreeSet<OpId>> = BTreeMap::new();
        let mut indegree: BTreeMap<OpId, usize> =
            node_by_id.keys().cloned().map(|op_id| (op_id, 0)).collect();

        for edge in &sorted_edges {
            if !node_by_id.contains_key(&edge.from) {
                return Err(ReferenceGraphError::MissingEdgeEndpoint {
                    op_id: edge.from.clone(),
                });
            }
            if !node_by_id.contains_key(&edge.to) {
                return Err(ReferenceGraphError::MissingEdgeEndpoint {
                    op_id: edge.to.clone(),
                });
            }
            if !unique_edges.insert(edge.clone()) {
                return Err(ReferenceGraphError::DuplicateEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    tensor: edge.tensor.clone(),
                });
            }

            let inserted_dependency = outgoing
                .entry(edge.from.clone())
                .or_default()
                .insert(edge.to.clone());
            if inserted_dependency {
                *indegree
                    .get_mut(&edge.to)
                    .expect("edge endpoint was validated") += 1;
            }
        }

        let mut ready: BTreeSet<OpId> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(op_id, _)| op_id.clone())
            .collect();
        let mut ordered_nodes = Vec::with_capacity(node_by_id.len());
        while let Some(op_id) = ready.pop_first() {
            ordered_nodes.push(
                node_by_id
                    .get(&op_id)
                    .expect("ready op id exists in node index")
                    .clone(),
            );
            if let Some(successors) = outgoing.get(&op_id) {
                for successor in successors {
                    let degree = indegree
                        .get_mut(successor)
                        .expect("successor exists in indegree map");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(successor.clone());
                    }
                }
            }
        }

        if ordered_nodes.len() != node_by_id.len() {
            return Err(ReferenceGraphError::Cycle);
        }

        Ok(Self {
            nodes: ordered_nodes,
            edges: sorted_edges,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceGraphError {
    DuplicateNode {
        op_id: OpId,
    },
    DuplicateEdge {
        from: OpId,
        to: OpId,
        tensor: TensorRef,
    },
    MissingEdgeEndpoint {
        op_id: OpId,
    },
    Cycle,
}

impl fmt::Display for ReferenceGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode { op_id } => {
                write!(f, "reference graph contains duplicate node {op_id}")
            }
            Self::DuplicateEdge { from, to, tensor } => write!(
                f,
                "reference graph contains duplicate edge {from} -> {to} for tensor {tensor}"
            ),
            Self::MissingEdgeEndpoint { op_id } => {
                write!(f, "reference graph edge references missing node {op_id}")
            }
            Self::Cycle => f.write_str("reference graph contains a cycle"),
        }
    }
}

impl Error for ReferenceGraphError {}
