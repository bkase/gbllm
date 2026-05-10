//! Cross-cutting identifiers, hashes, version wrappers, blob references, and shared newtypes.

pub mod blob;
pub mod cost;
pub mod hash;
pub mod ids;
pub mod schema_carriers;
pub mod semver;

pub use blob::{BlobCodec, BlobRef};
pub use cost::ByteCost;
pub use hash::{Hash256, Hash256ParseError, sha256};
pub use ids::{
    BudgetSlotId, CalibrationCohortId, CheckpointId, CompileProfileId, ExpertId, FieldPath,
    KernelCalibrationId, KernelImplId, KernelSpecId, LayerId, PlatformCalibrationId,
    RuntimeCalibrationId, RuntimeNucleusId, TargetFamilyId, TargetProfileId, WorkloadId,
};
pub use schema_carriers::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentId, DataLoweringProfileId, EvidenceRef,
    GoldenVectorId, LineageId, LoweringShardId, LoweringShardRef, ManifestInvariant, SidecarKind,
};
pub use semver::{PackerVersion, SemVer, SemVerParseError};
