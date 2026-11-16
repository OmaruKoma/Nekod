use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::common::{
    AntiCheatKind, BinaryArch, ManifestError, TargetPlatform, is_valid_kebab_slug,
    is_valid_sha256_hex, validate_relative_path, validate_schema_v1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StoreIdentifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic_catalog_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gog_product_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbox_product_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub itch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_install_hints: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableSignature {
    pub platform: TargetPlatform,
    pub relative_path: String,
    pub process_name: String,
    pub architecture: BinaryArch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownGameVersion {
    pub version_id: String,
    pub display_version: String,
    pub platform: TargetPlatform,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_build_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiplayerMode {
    SingleplayerOnly,
    CoopOfflineCapable,
    OnlineMultiplayer,
    CompetitiveOnline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainerPolicy {
    AllowedSingleplayer,
    RestrictedOfflineOnly,
    ProhibitedOnlineCompetitive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameCompatibilityPolicy {
    pub multiplayer_mode: MultiplayerMode,
    pub anti_cheat_detected: Vec<AntiCheatKind>,
    pub trainer_policy: TrainerPolicy,
}

/// Canonical Game Manifest (`v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameManifest {
    pub schema_version: Version,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    pub platforms: Vec<TargetPlatform>,
    pub store_ids: StoreIdentifiers,
    pub executables: Vec<ExecutableSignature>,
    pub known_versions: Vec<KnownGameVersion>,
    pub compatibility_policy: GameCompatibilityPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

impl GameManifest {
    /// Performs full semantic and defense-in-depth structural validation.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        validate_schema_v1(&self.schema_version)?;

        if !is_valid_kebab_slug(&self.id) {
            return Err(ManifestError::InvalidIdentifier(
                self.id.clone(),
                "expected strict lowercase kebab-case slug (U11)",
            ));
        }

        if self.name.trim().is_empty() || self.name.len() > 200 {
            return Err(ManifestError::InconsistentManifest(
                "game name must be 1..=200 characters".to_owned(),
            ));
        }

        if self.platforms.is_empty() {
            return Err(ManifestError::InconsistentManifest(
                "game manifest must declare at least one platform".to_owned(),
            ));
        }
        let mut seen_platforms = BTreeSet::new();
        for &platform in &self.platforms {
            if !seen_platforms.insert(platform) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate platform '{platform:?}'"
                )));
            }
        }

        if self.executables.is_empty() {
            return Err(ManifestError::InconsistentManifest(
                "game manifest must declare at least one executable".to_owned(),
            ));
        }
        for exe in &self.executables {
            validate_relative_path(&exe.relative_path)?;
            if exe.process_name.is_empty()
                || exe.process_name.contains('/')
                || exe.process_name.contains('\\')
                || exe.process_name.chars().any(char::is_control)
            {
                return Err(ManifestError::InconsistentManifest(format!(
                    "invalid process_name '{}'",
                    exe.process_name
                )));
            }
        }
        if let Some(hints) = &self.store_ids.custom_install_hints {
            for hint in hints {
                validate_relative_path(hint)?;
            }
        }
        for known in &self.known_versions {
            if known.version_id.trim().is_empty() || known.display_version.trim().is_empty() {
                return Err(ManifestError::InconsistentManifest(
                    "known_versions entries must have non-empty version_id and display_version"
                        .to_owned(),
                ));
            }
            if let Some(digest) = &known.executable_sha256 {
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
    fn parses_and_validates_example_game_manifest() {
        let raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let manifest: GameManifest = serde_json::from_str(raw).expect("valid example JSON");
        manifest
            .validate_semantic()
            .expect("semantic validation passes");
        assert_eq!(manifest.id, "sample-singleplayer-rpg");
        assert_eq!(manifest.store_ids.steam_app_id, Some(999999));
    }

    #[test]
    fn rejects_path_traversal_in_executable() {
        let raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let mut manifest: GameManifest = serde_json::from_str(raw).unwrap();
        manifest.executables[0].relative_path = "../evil.exe".to_owned();
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::InvalidRelativePath(_))
        ));
    }

    #[test]
    fn rejects_non_kebab_game_id_per_u11() {
        let raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let mut manifest: GameManifest = serde_json::from_str(raw).unwrap();
        manifest.id = "steam:999999".to_owned();
        assert!(matches!(
            manifest.validate_semantic(),
            Err(ManifestError::InvalidIdentifier(_, _))
        ));
    }
}
