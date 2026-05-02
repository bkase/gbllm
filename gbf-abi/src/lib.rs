//! Live execution ABI shared by compiler, runtime, harnesses, and emulator adapters.
//!
//! See `history/rfcs/F-A3-gbf-abi.md` for the contract rationale.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(feature = "alloc", feature = "host"))]
extern crate alloc;

pub mod checkpoint;
pub mod continuation;
pub mod fault;
pub mod harness;
pub mod interrupt;
pub mod liveness;
pub mod trace;
pub mod version;

#[cfg(feature = "host")]
pub use checkpoint::{CheckpointEntry, SchemaValidationError, SemanticCheckpointSchema};
#[cfg(feature = "alloc")]
pub use checkpoint::{CheckpointIdError, CheckpointResolver, SemanticCheckpointId};
pub use checkpoint::{CompactCheckpointId, SemanticStratum};
pub use continuation::{
    ContinuationError, FaultCodeOptional, InferenceStateHeader, UnknownFaultCode, decode_header,
    header_size_bytes, split_header_tail, total_continuation_bytes,
};
pub use fault::{
    BootValidationPlan, FaultCode, FaultDomain, FaultSnapshot, PersistScanPolicy, RecoveryAction,
    RegisterSnapshot, SnapshotDecodeError, classify_fault,
};
#[cfg(feature = "host")]
pub use fault::{FaultPolicy, FaultPolicyError};
pub use harness::{
    HarnessCommandBlock, HarnessOp, HarnessProtocolError, HarnessResultBlock, HarnessResultKind,
};
pub use interrupt::{
    InterruptPolicy, LeaseId, OverlayId, ResourceLease, ResourceLeaseKind, RomWindowBinding,
    SliceId, SramPageBinding,
};
pub use liveness::LivenessCounters;
pub use trace::{
    ProbeBudgetClass, ProbeLevel, TraceBudget, TraceBudgetError, TraceDropPolicy, TraceEvent,
    TraceProbeId,
};
#[cfg(feature = "host")]
pub use trace::{TraceProbeEntry, TraceProbeRegistry, TraceProbeRegistryError};
pub use version::{
    AbiVersion, AbiVersionError, BuildIdentityArgs, BuildIdentityBlock, BuildIdentityError,
    CURRENT_ABI,
};
#[cfg(feature = "host")]
pub use version::{CompatibilityEnvelope, CompatibilityError};
