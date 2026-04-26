//! Backend-independent sequence-state block contracts.

pub mod spec;

pub use spec::{
    SequenceActivation, SequenceActivationError, SequenceBlock, SequenceExportFacts,
    SequenceSemanticsError, SequenceSemanticsSpec, SequenceState, SequenceStateSize,
};
