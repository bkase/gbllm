//! Durable model-lineage contracts and target-data lowering schemas.

/// Compile-time marker that the F-S3 schema surface is enabled.
#[cfg(feature = "s3-schemas")]
pub const S3_SCHEMAS_FEATURE_ENABLED: bool = true;

/// Compile-time marker that the F-S4 schema surface is enabled.
#[cfg(feature = "s4-schemas")]
pub const S4_SCHEMAS_FEATURE_ENABLED: bool = true;

pub mod artifact;
pub mod aux;
pub mod bundle;
pub mod bundle_program_evaluator;
pub mod canonical_artifact_write;
pub mod canonical_bundle_write;
pub mod canonical_conformance_write;
pub mod canonical_gutenberg_manifest_write;
pub mod canonical_tensor;
pub mod conformance;
pub mod core;
pub mod decode;
pub mod export_facts;
pub mod frontier;
pub mod golden;
pub mod gutenberg_manifest;
pub mod hint_bundle;
pub mod ids;
pub mod interaction;
pub mod lexical;
pub mod lexical_spec;
pub mod lowerings;
pub mod luts;
pub mod manifest;
pub mod model_spec;
pub mod norm_plan;
pub mod opset_v1;
pub mod preferences;
pub mod quant;
pub mod reference_eval_graph;
pub mod semantic_checkpoint;
pub mod sequence;
pub mod session;
pub mod shadow;
pub mod tensor;
pub mod tied_alias;
pub mod weight_plan;

pub use artifact::*;
pub use aux::*;
pub use bundle::*;
pub use bundle_program_evaluator::*;
pub use canonical_artifact_write::*;
pub use canonical_bundle_write::*;
pub use canonical_conformance_write::*;
pub use canonical_gutenberg_manifest_write::*;
pub use canonical_tensor::*;
pub use conformance::*;
pub use frontier::*;
pub use gbf_foundation::{GoldenVectorId, GoldenVectorRef};
pub use gutenberg_manifest::*;
pub use hint_bundle::*;
pub use lexical::*;
pub use lexical_spec::*;
pub use lowerings::*;
pub use manifest::*;
pub use opset_v1::*;
pub use quant::*;
pub use reference_eval_graph::*;
pub use semantic_checkpoint::*;
pub use sequence::*;
pub use shadow::*;
pub use tied_alias::*;
