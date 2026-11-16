use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::common::{
    CompatibilityStatus, ManifestError, TrustTier, is_valid_kebab_slug, is_valid_namespaced_id,
    is_valid_sha256_hex, validate_https_url, validate_schema_v1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintainerInfo {
    pub name: String,
    pub github_handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    Stable,
    Beta,
    Nightly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryFeedType {
    GithubReleases,
    StaticIndexJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryTrainerStatus {
    Active,
    Deprecated,
    Archived,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilitySummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_game_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CompatibilityStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryTrainerEntry {
    pub trainer_id: String,
    pub game_id: String,
    pub latest_version: Version,
    pub manifest_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_tag: Option<String>,
    pub status: RegistryTrainerStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_summary: Option<CompatibilitySummary>,
}

/// Registry Manifest (`v1`) indexing an official or third-party trainer repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryManifest {
    pub schema_version: Version,
    pub repository_id: String,
    pub name: String,
    pub description: String,
    pub url: String,
    pub maintainers: Vec<MaintainerInfo>,
    pub trust_tier: TrustTier,
    pub channel: ReleaseChannel,
    pub feed_type: RegistryFeedType,
    pub trainers: Vec<RegistryTrainerEntry>,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

impl RegistryManifest {
    /// Performs semantic and structural validation on repository URLs and indexed trainer entries.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        validate_schema_v1(&self.schema_version)?;

        if !is_valid_namespaced_id(&self.repository_id) {
            return Err(ManifestError::InvalidIdentifier(
                self.repository_id.clone(),
                "expected lowercase namespaced repository_id",
            ));
        }
        if self.maintainers.is_empty() || self.name.trim().is_empty() {
            return Err(ManifestError::InconsistentManifest(
                "registry manifest requires non-empty name and maintainers".to_owned(),
            ));
        }
        validate_https_url(&self.url)?;

        let mut seen_trainers = BTreeSet::new();
        for entry in &self.trainers {
            if !is_valid_namespaced_id(&entry.trainer_id) {
                return Err(ManifestError::InvalidIdentifier(
                    entry.trainer_id.clone(),
                    "expected lowercase namespaced trainer_id",
                ));
            }
            if !is_valid_kebab_slug(&entry.game_id) {
                return Err(ManifestError::InvalidIdentifier(
                    entry.game_id.clone(),
                    "expected strict lowercase kebab-case game_id (U11)",
                ));
            }
            if !seen_trainers.insert(entry.trainer_id.as_str()) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate trainer_id '{}' in registry manifest",
                    entry.trainer_id
                )));
            }
            validate_https_url(&entry.manifest_url)?;
            if let Some(digest) = &entry.manifest_sha256 {
                if !is_valid_sha256_hex(digest) {
                    return Err(ManifestError::InvalidSha256(digest.clone()));
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
    fn parses_and_validates_example_registry_manifest() {
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json");
        let manifest: RegistryManifest = serde_json::from_str(raw).expect("valid example JSON");
        manifest
            .validate_semantic()
            .expect("semantic validation passes");
        assert_eq!(
            manifest.repository_id,
            "com.github.example-contributor.sample-trainers"
        );
        assert_eq!(manifest.trainers.len(), 1);
    }

    #[test]
    fn rejects_insecure_http_manifest_url() {
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json");
        let mut manifest: RegistryManifest = serde_json::from_str(raw).unwrap();
        manifest.trainers[0].manifest_url = "http://example.com/trainer-manifest.json".to_owned();
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::InvalidHttpsUrl(_))
        ));
    }
}
