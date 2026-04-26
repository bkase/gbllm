//! Backend-independent sequence-state block contracts.

pub mod bounded_kv;
pub mod linear_state;
pub mod spec;

pub use bounded_kv::{
    BoundedKvBlock, BoundedKvBlockConfig, BoundedKvBlockError, BoundedKvForwardOptions,
};
pub use linear_state::{
    LinearStateBlock, LinearStateBlockConfig, LinearStateBlockError, LinearStateForwardOptions,
};
pub use spec::{
    SequenceActivation, SequenceActivationError, SequenceBlock, SequenceExportFacts,
    SequenceSemanticsError, SequenceSemanticsSpec, SequenceState, SequenceStateSize,
};
