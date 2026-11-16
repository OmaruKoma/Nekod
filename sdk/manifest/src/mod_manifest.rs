//! L4 `ModManifest` v2 (`.nmod`) types, dependency specs, compatibility matrix entries,
//! and lossless `TrainerManifest` v1 upgrader (ADR-019 / ADR-020).

use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::common::{
    AntiCheatKind, CompatibilityStatus, ManifestError, is_valid_feature_id, is_valid_kebab_slug,
    is_valid_namespaced_id, is_valid_sha256_hex, validate_https_url, validate_relative_path,
    validate_schema_v2,
};
use crate::permissions::{PermissionDeclaration, PermissionScope};
use crate::trainer::{
    AuthorInfo, RuntimeKind, RuntimeRequirement, SupportedGameVersion, TargetGameRef,
    TrainerArtifact, TrainerFeature, TrainerManifest,
};

const DISALLOWED_EXECUTABLE_EXTENSIONS: &[&str] = &[".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"];

/// Packaging convention for `.nmod` source-first mod bundles (ADR-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackageFormat {
    #[serde(rename = "nmod_v1")]
    NmodV1,
}

/// High-level mod category supported by the unified Nekod platform (ADR-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModKind {
    Trainer,
    ScriptMod,
    PluginMod,
    AssetMod,
    ConfigurationMod,
    PatchMod,
    EmulatorMod,
}

/// Category of an external runtime, loader, or protocol dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    ModLoader,
    RuntimeBridge,
    ScriptEngine,
    EmulatorProtocol,
}

/// Redistribution rule governing how Nekod acquires or detects an external dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRedistribution {
    LocalDetectOnly,
    OfficialUpstreamFetch,
    BundledRecipeOnly,
}

/// Pinned upstream download coordinates (`url` + `sha256` + `license`) per ADR-020.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedUpstreamSource {
    pub url: String,
    pub sha256: String,
    pub license: String,
}

/// Explicit dependency declaration resolved by `Runtime Dependency Manager`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDependencySpec {
    pub dependency_id: String,
    pub kind: DependencyKind,
    pub version_req: String,
    pub optional: bool,
    pub redistribution: DependencyRedistribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<PinnedUpstreamSource>,
}

/// Storefront filter inside a multi-dimensional compatibility record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorePlatformTag {
    Steam,
    Gog,
    Epic,
    Xbox,
    CustomPath,
    Any,
}

/// Multi-dimensional compatibility tuple (`Game Version × Store × Runtime Adapter × Status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityMatrixEntry {
    pub game_version: String,
    pub store_platform: StorePlatformTag,
    pub runtime_adapter: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    pub status: CompatibilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Policy and multi-dimensional compatibility declaration for `ModManifest` v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModCompatibility {
    pub singleplayer_only: bool,
    pub requires_offline_mode: bool,
    pub anti_cheat_incompatible: Vec<AntiCheatKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatibility_matrix: Vec<CompatibilityMatrixEntry>,
}

/// Unified Mod Manifest (`v2`) supporting `.nmod` source-first packages across all runtimes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModManifest {
    pub schema_version: Version,
    pub package_format: PackageFormat,
    pub mod_kind: ModKind,
    pub id: String,
    pub version: Version,
    pub name: String,
    pub description: String,
    pub authors: Vec<AuthorInfo>,
    pub license: String,
    pub source_repository: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub target_game: TargetGameRef,
    pub supported_versions: Vec<SupportedGameVersion>,
    pub runtime: RuntimeRequirement,
    #[serde(default)]
    pub dependencies: Vec<RuntimeDependencySpec>,
    pub permissions: Vec<PermissionDeclaration>,
    #[serde(default)]
    pub features: Vec<TrainerFeature>,
    pub compatibility: ModCompatibility,
    pub artifacts: Vec<TrainerArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

impl ModManifest {
    /// Checks whether the mod declares a given capability scope.
    pub fn has_permission(&self, scope: PermissionScope) -> bool {
        self.permissions.iter().any(|p| p.scope == scope)
    }

    /// Losslessly upgrades a legacy v1 [`TrainerManifest`] into a `ModManifest` v2 (`mod_kind = trainer`).
    pub fn from_trainer_v1(trainer: &TrainerManifest) -> Self {
        Self {
            schema_version: Version::new(2, 0, 0),
            package_format: PackageFormat::NmodV1,
            mod_kind: ModKind::Trainer,
            id: trainer.id.clone(),
            version: trainer.version.clone(),
            name: trainer.name.clone(),
            description: trainer.description.clone(),
            authors: trainer.authors.clone(),
            license: trainer.license.clone(),
            source_repository: trainer.source_repository.clone(),
            homepage: trainer.homepage.clone(),
            target_game: trainer.target_game.clone(),
            supported_versions: trainer.supported_versions.clone(),
            runtime: trainer.runtime.clone(),
            dependencies: Vec::new(),
            permissions: trainer.permissions.clone(),
            features: trainer.features.clone(),
            compatibility: ModCompatibility {
                singleplayer_only: trainer.compatibility.singleplayer_only,
                requires_offline_mode: trainer.compatibility.requires_offline_mode,
                anti_cheat_incompatible: trainer.compatibility.anti_cheat_incompatible.clone(),
                last_verified_at: trainer.compatibility.last_verified_at.clone(),
                compatibility_matrix: Vec::new(),
            },
            artifacts: trainer.artifacts.clone(),
            extensions: trainer.extensions.clone(),
        }
    }

    /// Enforces L4 schema, security, dependency provenance, and ADR-020 `.nmod` source-first rules.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        validate_schema_v2(&self.schema_version)?;

        if !is_valid_namespaced_id(&self.id) {
            return Err(ManifestError::InvalidIdentifier(
                self.id.clone(),
                "expected lowercase namespaced mod ID",
            ));
        }
        if !is_valid_kebab_slug(&self.target_game.game_id) {
            return Err(ManifestError::InvalidIdentifier(
                self.target_game.game_id.clone(),
                "expected strict lowercase kebab-case target_game.game_id (U11)",
            ));
        }
        if self.name.trim().is_empty()
            || self.description.trim().is_empty()
            || self.license.trim().is_empty()
            || self.authors.is_empty()
            || self.supported_versions.is_empty()
            || self.permissions.is_empty()
            || self.artifacts.is_empty()
            || self.runtime.supported_platforms.is_empty()
        {
            return Err(ManifestError::InconsistentManifest(
                "required mod metadata fields and collections must be non-empty".to_owned(),
            ));
        }

        if matches!(
            self.mod_kind,
            ModKind::Trainer | ModKind::ScriptMod | ModKind::EmulatorMod
        ) && self.features.is_empty()
        {
            return Err(ManifestError::InconsistentManifest(
                "interactive trainer/script/emulator mods must declare at least one feature"
                    .to_owned(),
            ));
        }

        validate_https_url(&self.source_repository)?;
        if let Some(homepage) = &self.homepage {
            validate_https_url(homepage)?;
        }
        for author in &self.authors {
            if author.name.trim().is_empty() {
                return Err(ManifestError::InconsistentManifest(
                    "author name must be non-empty".to_owned(),
                ));
            }
            if let Some(url) = &author.url {
                validate_https_url(url)?;
            }
        }

        if !self.compatibility.singleplayer_only {
            return Err(ManifestError::PolicyViolation(
                "mods must declare compatibility.singleplayer_only = true".to_owned(),
            ));
        }

        validate_relative_path(&self.runtime.entrypoint)?;
        for artifact in &self.artifacts {
            validate_relative_path(&artifact.filename)?;
            let lower = artifact.filename.to_ascii_lowercase();
            if DISALLOWED_EXECUTABLE_EXTENSIONS
                .iter()
                .any(|ext| lower.ends_with(ext))
            {
                return Err(ManifestError::PolicyViolation(format!(
                    "source-first .nmod package refuses raw executable artifact '{}' (ADR-020)",
                    artifact.filename
                )));
            }
            if artifact.size_bytes == 0 {
                return Err(ManifestError::InconsistentManifest(format!(
                    "artifact '{}' must have size_bytes >= 1",
                    artifact.filename
                )));
            }
            if !is_valid_sha256_hex(&artifact.sha256) {
                return Err(ManifestError::InvalidSha256(artifact.sha256.clone()));
            }
        }

        if !self
            .artifacts
            .iter()
            .any(|a| a.filename == self.runtime.entrypoint)
        {
            return Err(ManifestError::InconsistentManifest(format!(
                "runtime.entrypoint '{}' is not listed in artifacts",
                self.runtime.entrypoint
            )));
        }

        let mut seen_deps = BTreeSet::new();
        for dep in &self.dependencies {
            if !is_valid_kebab_slug(&dep.dependency_id) {
                return Err(ManifestError::InvalidIdentifier(
                    dep.dependency_id.clone(),
                    "dependency_id must be lowercase kebab-case",
                ));
            }
            if !seen_deps.insert(dep.dependency_id.as_str()) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate dependency_id '{}'",
                    dep.dependency_id
                )));
            }
            if semver::VersionReq::parse(&dep.version_req).is_err() {
                return Err(ManifestError::InconsistentManifest(format!(
                    "invalid SemVer range '{}' on dependency '{}'",
                    dep.version_req, dep.dependency_id
                )));
            }
            match dep.redistribution {
                DependencyRedistribution::OfficialUpstreamFetch => {
                    let Some(upstream) = &dep.upstream else {
                        return Err(ManifestError::InconsistentManifest(format!(
                            "dependency '{}' with official_upstream_fetch must declare pinned upstream coordinates",
                            dep.dependency_id
                        )));
                    };
                    validate_https_url(&upstream.url)?;
                    if !is_valid_sha256_hex(&upstream.sha256) {
                        return Err(ManifestError::InvalidSha256(upstream.sha256.clone()));
                    }
                    if upstream.license.trim().is_empty() {
                        return Err(ManifestError::InconsistentManifest(format!(
                            "upstream source for '{}' must declare license metadata",
                            dep.dependency_id
                        )));
                    }
                }
                DependencyRedistribution::LocalDetectOnly
                | DependencyRedistribution::BundledRecipeOnly => {
                    if let Some(upstream) = &dep.upstream {
                        validate_https_url(&upstream.url)?;
                        if !is_valid_sha256_hex(&upstream.sha256) {
                            return Err(ManifestError::InvalidSha256(upstream.sha256.clone()));
                        }
                    }
                }
            }
        }

        if matches!(
            self.runtime.kind,
            RuntimeKind::NativeProviderV1 | RuntimeKind::FridaGumV1
        ) && !self.has_permission(PermissionScope::RuntimeNativeModule)
        {
            return Err(ManifestError::InconsistentManifest(
                "native/dynamic instrumentation runtime requires explicit runtime.native_module permission"
                    .to_owned(),
            ));
        }

        let mut seen_scopes = BTreeSet::new();
        for perm in &self.permissions {
            if !seen_scopes.insert(perm.scope) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate permission scope '{:?}'",
                    perm.scope
                )));
            }
            if perm.reason.trim().len() < 4 {
                return Err(ManifestError::InconsistentManifest(format!(
                    "permission '{:?}' must provide a meaningful reason",
                    perm.scope
                )));
            }
        }

        let mut seen_features = BTreeSet::new();
        for feature in &self.features {
            if !is_valid_feature_id(&feature.id) {
                return Err(ManifestError::InvalidIdentifier(
                    feature.id.clone(),
                    "invalid feature id",
                ));
            }
            if !seen_features.insert(feature.id.as_str()) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate feature id '{}'",
                    feature.id
                )));
            }
        }

        if self.features.iter().any(|f| f.mutates_memory)
            && !self.has_permission(PermissionScope::ProcessWriteMemory)
        {
            return Err(ManifestError::InconsistentManifest(
                "features with mutates_memory=true require process.write_memory permission"
                    .to_owned(),
            ));
        }

        Ok(())
    }
}

impl From<TrainerManifest> for ModManifest {
    fn from(trainer: TrainerManifest) -> Self {
        Self::from_trainer_v1(&trainer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_example_mod_manifest_v2_and_upgrades_v1_trainer() {
        let raw_v2 = include_str!("../../../schemas/examples/mod-manifest.v2.example.json");
        let mod_v2: ModManifest = serde_json::from_str(raw_v2).expect("valid v2 JSON");
        mod_v2.validate_semantic().expect("semantic valid v2");
        assert_eq!(mod_v2.package_format, PackageFormat::NmodV1);
        assert_eq!(mod_v2.mod_kind, ModKind::EmulatorMod);

        let raw_v1 = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let trainer_v1: TrainerManifest = serde_json::from_str(raw_v1).expect("valid v1 JSON");
        let upgraded = ModManifest::from_trainer_v1(&trainer_v1);
        upgraded
            .validate_semantic()
            .expect("upgraded v1 -> v2 valid");
        assert_eq!(upgraded.mod_kind, ModKind::Trainer);
        assert_eq!(upgraded.id, trainer_v1.id);
    }

    #[test]
    fn rejects_exe_artifacts_and_unpinned_upstream_fetch_dependencies() {
        let raw_v2 = include_str!("../../../schemas/examples/mod-manifest.v2.example.json");
        let mut with_exe: ModManifest = serde_json::from_str(raw_v2).unwrap();
        with_exe.artifacts.push(TrainerArtifact {
            filename: "cheat.exe".to_owned(),
            sha256: "b".repeat(64),
            size_bytes: 1024,
            platform: None,
        });
        assert!(matches!(
            with_exe.validate_semantic(),
            Err(ManifestError::PolicyViolation(_))
        ));

        let mut unpinned_dep: ModManifest = serde_json::from_str(raw_v2).unwrap();
        unpinned_dep.dependencies = vec![RuntimeDependencySpec {
            dependency_id: "bepinex-mono".to_owned(),
            kind: DependencyKind::ModLoader,
            version_req: "^5.4.0".to_owned(),
            optional: false,
            redistribution: DependencyRedistribution::OfficialUpstreamFetch,
            upstream: None,
        }];
        assert!(matches!(
            unpinned_dep.validate_semantic(),
            Err(ManifestError::InconsistentManifest(_))
        ));
    }
}
