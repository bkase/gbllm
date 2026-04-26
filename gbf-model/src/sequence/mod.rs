//! Backend-independent sequence-state block contracts.

pub mod linear_state;
pub mod spec;

pub use linear_state::{
    LinearStateBlock, LinearStateBlockConfig, LinearStateBlockError, LinearStateForwardOptions,
};
pub use spec::{
    SequenceActivation, SequenceActivationError, SequenceBlock, SequenceExportFacts,
    SequenceSemanticsError, SequenceSemanticsSpec, SequenceState, SequenceStateSize,
};
