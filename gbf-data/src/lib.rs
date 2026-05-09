//! Corpus ingestion, normalization, governance, splits, sampling policies, and contamination checks.

pub mod contamination;
pub mod corpus;
pub mod normalization;
pub mod sampling;
pub mod splits;

pub use corpus::{
    CorpusFile, CorpusManifestError, CorpusSource, SplitRole, TinyStoriesManifest,
    TinyStoriesSplits, load_train_bytes, load_val_bytes, read_tinystories_manifest,
};
