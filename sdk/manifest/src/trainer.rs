use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::common::{
    AntiCheatKind, CompatibilityStatus, ManifestError, TargetPlatform, is_valid_feature_id,
    is_valid_kebab_slug, is_valid_namespaced_id, is_valid_sha256_hex, validate_https_url,
    validate_relative_path, validate_schema_v1,
};
use crate::permissions::{PermissionDeclaration, PermissionScope};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TargetStoreIds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic_catalog_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gog_product_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetGameRef {
    pub game_id: String,
    pub game_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_ids: Option<TargetStoreIds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedGameVersion {
    pub platform: TargetPlatform,
    pub version_pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<Vec<String>>,
    pub status: CompatibilityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeKind {
    #[serde(rename = "declarative_patch_v1")]
    DeclarativePatchV1,
    #[serde(rename = "wasm_sandbox_v1")]
    WasmSandboxV1,
    #[serde(rename = "adapter_ipc_v1")]
    AdapterIpcV1,
    #[serde(rename = "ce_subset_v1")]
    CeSubsetV1,
    #[serde(rename = "unity_bridge_v1")]
    UnityBridgeV1,
    #[serde(rename = "ue4ss_bridge_v1")]
    Ue4ssBridgeV1,
    #[serde(rename = "native_provider_v1")]
    NativeProviderV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequirement {
    pub kind: RuntimeKind,
    pub api_version: Version,
    pub min_nekod_version: Version,
    pub entrypoint: String,
    pub supported_platforms: Vec<TargetPlatform>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureCategory {
    Player,
    Inventory,
    World,
    Time,
    Economy,
    Camera,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureControlType {
    Toggle,
    Action,
    Integer,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureValueRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerFeature {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: FeatureCategory,
    pub control_type: FeatureControlType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_hotkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<FeatureValueRange>,
    pub mutates_memory: bool,
    pub supports_rollback: bool,
    pub risk_level: FeatureRiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerCompatibility {
    pub singleplayer_only: bool,
    pub requires_offline_mode: bool,
    pub anti_cheat_incompatible: Vec<AntiCheatKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerArtifact {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<TargetPlatform>,
}

/// Authoritative Trainer Manifest (`v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainerManifest {
    pub schema_version: Version,
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
    pub permissions: Vec<PermissionDeclaration>,
    pub features: Vec<TrainerFeature>,
    pub compatibility: TrainerCompatibility,
    pub artifacts: Vec<TrainerArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

impl TrainerManifest {
    /// Checks whether the manifest declares a given permission scope.
    pub fn has_permission(&self, scope: PermissionScope) -> bool {
        self.permissions.iter().any(|p| p.scope == scope)
    }

    /// Performs semantic, policy, and defense-in-depth structural validation on the manifest.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        validate_schema_v1(&self.schema_version)?;

        if !is_valid_namespaced_id(&self.id) {
            return Err(ManifestError::InvalidIdentifier(
                self.id.clone(),
                "expected lowercase namespaced trainer ID",
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
            || self.features.is_empty()
            || self.artifacts.is_empty()
            || self.runtime.supported_platforms.is_empty()
        {
            return Err(ManifestError::InconsistentManifest(
                "required metadata fields and collections must be non-empty".to_owned(),
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
                "trainers must declare compatibility.singleplayer_only = true".to_owned(),
            ));
        }

        validate_relative_path(&self.runtime.entrypoint)?;
        for artifact in &self.artifacts {
            validate_relative_path(&artifact.filename)?;
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

        if self.runtime.kind == RuntimeKind::NativeProviderV1
            && !self.has_permission(PermissionScope::RuntimeNativeModule)
        {
            return Err(ManifestError::InconsistentManifest(
                "native_provider_v1 runtime requires explicit runtime.native_module permission"
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
            if let Some(constraints) = &perm.constraints {
                if let Some(paths) = &constraints.path_scope {
                    for path in paths {
                        validate_relative_path(path)?;
                    }
                }
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
            if let Some(range) = &feature.value_range {
                if !range.min.is_finite()
                    || !range.max.is_finite()
                    || !range.step.is_finite()
                    || !range.default.is_finite()
                    || range.min > range.max
                    || range.step <= 0.0
                    || range.default < range.min
                    || range.default > range.max
                {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "feature '{}' has invalid value_range",
                        feature.id
                    )));
                }
            }
        }

        let any_memory_mutation = self.features.iter().any(|f| f.mutates_memory);
        if any_memory_mutation && !self.has_permission(PermissionScope::ProcessWriteMemory) {
            return Err(ManifestError::InconsistentManifest(
                "features with mutates_memory=true require process.write_memory permission"
                    .to_owned(),
            ));
        }

        for supported in &self.supported_versions {
            if let Some(hashes) = &supported.executable_sha256 {
                for digest in hashes {
                    if !is_valid_sha256_hex(digest) {
                        return Err(ManifestError::InvalidSha256(digest.clone()));
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_example_trainer_manifest() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let manifest: TrainerManifest = serde_json::from_str(raw).expect("valid example JSON");
        manifest
            .validate_semantic()
            .expect("semantic validation passes");
        assert_eq!(manifest.id, "dev.example.sample-singleplayer-rpg");
        assert_eq!(manifest.runtime.kind, RuntimeKind::DeclarativePatchV1);
    }

    #[test]
    fn rejects_memory_mutation_without_write_permission() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut manifest: TrainerManifest = serde_json::from_str(raw).unwrap();
        manifest
            .permissions
            .retain(|p| p.scope != PermissionScope::ProcessWriteMemory);
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::InconsistentManifest(_))
        ));
    }

    #[test]
    fn rejects_non_singleplayer_trainer() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut manifest: TrainerManifest = serde_json::from_str(raw).unwrap();
        manifest.compatibility.singleplayer_only = false;
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::PolicyViolation(_))
        ));
    }

    #[test]
    fn rejects_localhost_url_and_duplicate_features() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut manifest: TrainerManifest = serde_json::from_str(raw).unwrap();
        manifest.source_repository = "https://localhost/evil".to_owned();
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::InvalidHttpsUrl(_))
        ));

        let mut dup = serde_json::from_str::<TrainerManifest>(raw).unwrap();
        dup.features.push(dup.features[0].clone());
        assert!(matches!(
            dup.validate_semantic(),
            Err(ManifestError::InconsistentManifest(_))
        ));
    }
}
