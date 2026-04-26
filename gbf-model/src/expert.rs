//! Expert feed-forward semantics.
//!
//! The deployable expert core currently lives in `qat::expert` because expert
//! projections are ternary-QAT projections. This module provides the model-level
//! import surface without duplicating that implementation.

pub use crate::qat::{
    ClippedActivation, ClippedActivationKind, ExpertBatchOutput, ExpertBlockQat,
    ExpertBlockQatError, ExpertForwardOptions, ExpertMlpConfig, ExpertMlpConfigEvent,
    ExpertMlpConfigEventCode, ExpertMlpConfigEventLevel, ExpertMlpVariant, ExpertQat,
    ExpertQatForwardMode,
};
