//! Corpus ingestion, normalization, governance, splits, sampling policies, and contamination checks.

pub mod charset_v1;
pub mod contamination;
pub mod corpus;
pub mod gutenberg;
pub mod normalization;
pub mod sampling;
pub mod splits;
pub mod tinystories_v2;

pub use charset_v1::*;
pub use corpus::{
    CorpusFile, CorpusManifestError, CorpusSource, SplitRole, TinyStoriesManifest,
    TinyStoriesSplits, load_train_bytes, load_val_bytes, read_tinystories_manifest,
};
pub use gutenberg::*;
pub use tinystories_v2::*;
