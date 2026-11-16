//! Versioned L4 manifest models and semantic validation rules for Nekod.
//!
//! These types map 1:1 to the JSON Schemas in `schemas/*.v1.schema.json`.

pub mod common;
pub mod game;
pub mod mod_manifest;
pub mod permissions;
pub mod registry;
pub mod trainer;

pub use common::{
    AntiCheatKind, BinaryArch, CompatibilityStatus, ManifestError, TargetPlatform, TrustTier,
    is_valid_feature_id, is_valid_kebab_slug, is_valid_namespaced_id, is_valid_sha256_hex,
    validate_https_url, validate_relative_path, validate_schema_v1, validate_schema_v2,
};
pub use game::{
    ExecutableSignature, GameCompatibilityPolicy, GameManifest, KnownGameVersion, MultiplayerMode,
    StoreIdentifiers, TrainerPolicy,
};
pub use mod_manifest::{
    CompatibilityMatrixEntry, DependencyKind, DependencyRedistribution, ModCompatibility, ModKind,
    ModManifest, PackageFormat, PinnedUpstreamSource, RuntimeDependencySpec, StorePlatformTag,
};
pub use permissions::{PermissionConstraints, PermissionDeclaration, PermissionScope};
pub use registry::{
    CompatibilitySummary, MaintainerInfo, RegistryFeedType, RegistryManifest, RegistryTrainerEntry,
    RegistryTrainerStatus, ReleaseChannel,
};
pub use trainer::{
    AuthorInfo, FeatureCategory, FeatureControlType, FeatureRiskLevel, FeatureValueRange,
    RuntimeKind, RuntimeRequirement, SupportedGameVersion, TargetGameRef, TargetStoreIds,
    TrainerArtifact, TrainerCompatibility, TrainerFeature, TrainerManifest,
};

/// Current major version supported by `nekod-manifest` v1 models.
pub const SUPPORTED_SCHEMA_MAJOR: u64 = 1;
