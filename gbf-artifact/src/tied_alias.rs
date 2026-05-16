//! F-S3 tied embedding/classifier alias schema.

use serde::{Deserialize, Serialize};

use crate::canonical_tensor::CanonicalTensorId;

/// Classifier view over a tied embedding tensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierView {
    SameTensor,
    TransposedView,
}

/// Alias metadata proving embedding/classifier sharing is represented once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TiedEmbeddingAlias {
    pub embedding_canonical_id: CanonicalTensorId,
    pub classifier_canonical_id: CanonicalTensorId,
    pub shared: bool,
    pub classifier_view: ClassifierView,
}

impl TiedEmbeddingAlias {
    #[must_use]
    pub const fn new(
        embedding_canonical_id: CanonicalTensorId,
        classifier_canonical_id: CanonicalTensorId,
        shared: bool,
        classifier_view: ClassifierView,
    ) -> Self {
        Self {
            embedding_canonical_id,
            classifier_canonical_id,
            shared,
            classifier_view,
        }
    }
}
