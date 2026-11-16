//! Ergonomic authoring builders, signature testing helpers, and package generation
//! utilities for `declarative_patch_v1` Nekod trainers.

use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::{
    AuthorInfo, CompatibilityStatus, FeatureCategory, FeatureControlType, FeatureRiskLevel,
    FeatureValueRange, ManifestError, PermissionConstraints, PermissionDeclaration,
    PermissionScope, RuntimeKind, RuntimeRequirement, SupportedGameVersion, TargetGameRef,
    TargetPlatform, TargetStoreIds, TrainerArtifact, TrainerCompatibility, TrainerFeature,
    TrainerManifest, validate_relative_path,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AddressLocator, DeclarativeFeatureEntry, DeclarativeOperation, DeclarativePatchPayload,
    ScalarValueType, TrainerPackageDefinition, parse_aob_pattern,
};

/// Computes the lowercase hexadecimal SHA-256 digest of `bytes`.
pub fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Result of offline AOB pattern matching against a captured module byte slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureMatch {
    pub total_matches: usize,
    pub selected_match_index: u32,
    pub pattern_offset: usize,
    pub resolved_offset: usize,
}

/// Finds every starting byte offset matching `pattern` (`XX` or `??`) inside `haystack`.
pub fn count_aob_matches_in_bytes(
    haystack: &[u8],
    pattern: &str,
) -> Result<Vec<usize>, ManifestError> {
    let parsed = parse_aob_pattern(pattern)?;
    if parsed.iter().all(Option::is_none) {
        return Err(ManifestError::InconsistentManifest(
            "aob_pattern must contain at least one concrete byte".to_owned(),
        ));
    }
    if parsed.len() > haystack.len() {
        return Ok(Vec::new());
    }

    let mut matches = Vec::new();
    for start in 0..=(haystack.len() - parsed.len()) {
        let matched = parsed
            .iter()
            .enumerate()
            .all(|(i, expected)| expected.is_none_or(|b| haystack[start + i] == b));
        if matched {
            matches.push(start);
        }
    }
    Ok(matches)
}

/// Evaluates an `aob_pattern` with `match_index` and signed `delta` against a captured
/// module slice so trainer authors can test signatures offline before runtime execution.
pub fn test_aob_pattern_in_bytes(
    haystack: &[u8],
    pattern: &str,
    match_index: u32,
    delta: i64,
) -> Result<SignatureMatch, ManifestError> {
    let offsets = count_aob_matches_in_bytes(haystack, pattern)?;
    let idx = match_index as usize;
    let Some(&pattern_offset) = offsets.get(idx) else {
        return Err(ManifestError::InconsistentManifest(format!(
            "AOB pattern match_index {match_index} out of range (found {} match(es))",
            offsets.len()
        )));
    };

    let signed_offset = i64::try_from(pattern_offset)
        .map_err(|_| ManifestError::InconsistentManifest("pattern offset overflow".to_owned()))?
        .checked_add(delta)
        .ok_or_else(|| ManifestError::InconsistentManifest("delta overflow".to_owned()))?;
    let resolved_offset = usize::try_from(signed_offset).map_err(|_| {
        ManifestError::InconsistentManifest("resolved AOB offset cannot be negative".to_owned())
    })?;
    if resolved_offset >= haystack.len() {
        return Err(ManifestError::InconsistentManifest(format!(
            "resolved AOB offset {resolved_offset} exceeds buffer size {}",
            haystack.len()
        )));
    }

    Ok(SignatureMatch {
        total_matches: offsets.len(),
        selected_match_index: match_index,
        pattern_offset,
        resolved_offset,
    })
}

/// Fluent builder for constructing validated `DeclarativePatchPayload` documents.
#[derive(Debug, Clone, Default)]
pub struct DeclarativePatchBuilder {
    features: Vec<DeclarativeFeatureEntry>,
}

impl DeclarativePatchBuilder {
    /// Creates an empty `declarative_patch_v1` builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a bounded operation to `feature_id`, creating the feature entry if needed.
    pub fn push_operation(
        mut self,
        feature_id: impl Into<String>,
        op: DeclarativeOperation,
    ) -> Self {
        let id = feature_id.into();
        if let Some(entry) = self.features.iter_mut().find(|f| f.feature_id == id) {
            entry.operations.push(op);
        } else {
            self.features.push(DeclarativeFeatureEntry {
                feature_id: id,
                operations: vec![op],
            });
        }
        self
    }

    /// Adds a reversible `byte_patch` operation for `feature_id`.
    pub fn byte_patch(
        self,
        feature_id: impl Into<String>,
        locator: AddressLocator,
        expected_original_bytes: Option<String>,
        patch_bytes: impl Into<String>,
    ) -> Self {
        self.push_operation(
            feature_id,
            DeclarativeOperation::BytePatch {
                locator,
                expected_original_bytes,
                patch_bytes: patch_bytes.into(),
            },
        )
    }

    /// Adds a typed `write_value` operation for `feature_id`.
    pub fn write_value(
        self,
        feature_id: impl Into<String>,
        locator: AddressLocator,
        value_type: ScalarValueType,
        value: Option<f64>,
        freeze_interval_ms: Option<u32>,
    ) -> Self {
        self.push_operation(
            feature_id,
            DeclarativeOperation::WriteValue {
                locator,
                value_type,
                value,
                freeze_interval_ms,
            },
        )
    }

    /// Builds and semantically validates the `DeclarativePatchPayload`.
    pub fn build(self) -> Result<DeclarativePatchPayload, ManifestError> {
        let payload = DeclarativePatchPayload {
            schema_version: Version::new(1, 0, 0),
            features: self.features,
        };
        payload.validate_semantic()?;
        Ok(payload)
    }
}

/// Fluent builder for constructing policy-compliant `TrainerManifest` drafts.
#[derive(Debug, Clone)]
pub struct TrainerManifestBuilder {
    manifest: TrainerManifest,
}

impl TrainerManifestBuilder {
    /// Initializes a `declarative_patch_v1`, singleplayer-only `TrainerManifest` builder.
    pub fn new(
        trainer_id: impl Into<String>,
        name: impl Into<String>,
        game_id: impl Into<String>,
        game_name: impl Into<String>,
        source_repository: impl Into<String>,
    ) -> Self {
        Self {
            manifest: TrainerManifest {
                schema_version: Version::new(1, 0, 0),
                id: trainer_id.into(),
                version: Version::new(1, 0, 0),
                name: name.into(),
                description: "Community declarative trainer for singleplayer gameplay.".to_owned(),
                authors: Vec::new(),
                license: "MIT".to_owned(),
                source_repository: source_repository.into(),
                homepage: None,
                target_game: TargetGameRef {
                    game_id: game_id.into(),
                    game_name: game_name.into(),
                    store_ids: None,
                },
                supported_versions: vec![SupportedGameVersion {
                    platform: TargetPlatform::WindowsX64,
                    version_pattern: "*".to_owned(),
                    executable_sha256: None,
                    status: CompatibilityStatus::Untested,
                }],
                runtime: RuntimeRequirement {
                    kind: RuntimeKind::DeclarativePatchV1,
                    api_version: Version::new(1, 0, 0),
                    min_nekod_version: Version::new(0, 1, 0),
                    entrypoint: "trainer.json".to_owned(),
                    supported_platforms: vec![TargetPlatform::WindowsX64],
                },
                permissions: Vec::new(),
                features: Vec::new(),
                compatibility: TrainerCompatibility {
                    singleplayer_only: true,
                    requires_offline_mode: false,
                    anti_cheat_incompatible: Vec::new(),
                    last_verified_at: None,
                },
                artifacts: Vec::new(),
                extensions: None,
            },
        }
    }

    pub fn version(mut self, version: Version) -> Self {
        self.manifest.version = version;
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.manifest.description = description.into();
        self
    }

    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.manifest.license = license.into();
        self
    }

    pub fn author(
        mut self,
        name: impl Into<String>,
        github: Option<String>,
        url: Option<String>,
    ) -> Self {
        self.manifest.authors.push(AuthorInfo {
            name: name.into(),
            github,
            url,
        });
        self
    }

    pub fn target_store_ids(mut self, store_ids: TargetStoreIds) -> Self {
        self.manifest.target_game.store_ids = Some(store_ids);
        self
    }

    pub fn replace_supported_versions(mut self, versions: Vec<SupportedGameVersion>) -> Self {
        self.manifest.supported_versions = versions;
        self
    }

    pub fn entrypoint(mut self, entrypoint: impl Into<String>) -> Self {
        self.manifest.runtime.entrypoint = entrypoint.into();
        self
    }

    pub fn requires_offline_mode(mut self, requires_offline: bool) -> Self {
        self.manifest.compatibility.requires_offline_mode = requires_offline;
        self
    }

    /// Declares minimal scoped memory permissions (`discover`, `read`, `write`) constrained to
    /// `allowed_modules` and `max_write_bytes`.
    pub fn default_memory_permissions(
        mut self,
        allowed_modules: Vec<String>,
        max_write_bytes: u64,
    ) -> Self {
        self.manifest.permissions = vec![
            PermissionDeclaration {
                scope: PermissionScope::ProcessDiscover,
                reason: "Locate the running game process.".to_owned(),
                required: true,
                constraints: None,
            },
            PermissionDeclaration {
                scope: PermissionScope::ProcessReadMemory,
                reason: "Verify signatures and read original memory before patching.".to_owned(),
                required: true,
                constraints: Some(PermissionConstraints {
                    allowed_modules: Some(allowed_modules.clone()),
                    ..Default::default()
                }),
            },
            PermissionDeclaration {
                scope: PermissionScope::ProcessWriteMemory,
                reason: "Apply reversible declarative trainer patches for enabled features."
                    .to_owned(),
                required: true,
                constraints: Some(PermissionConstraints {
                    max_write_bytes: Some(max_write_bytes),
                    allowed_modules: Some(allowed_modules),
                    path_scope: None,
                }),
            },
        ];
        self
    }

    pub fn feature(mut self, feature: TrainerFeature) -> Self {
        self.manifest.features.push(feature);
        self
    }

    /// Helper to add a simple toggle feature backed by reversible memory mutation.
    pub fn toggle_feature(
        self,
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        category: FeatureCategory,
        default_hotkey: Option<String>,
    ) -> Self {
        self.feature(TrainerFeature {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            category,
            control_type: FeatureControlType::Toggle,
            default_hotkey,
            value_range: None,
            mutates_memory: true,
            supports_rollback: true,
            risk_level: FeatureRiskLevel::Low,
        })
    }

    /// Helper to add an integer parameter feature backed by reversible memory mutation.
    pub fn integer_feature(
        self,
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        category: FeatureCategory,
        default_hotkey: Option<String>,
        value_range: FeatureValueRange,
    ) -> Self {
        self.feature(TrainerFeature {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            category,
            control_type: FeatureControlType::Integer,
            default_hotkey,
            value_range: Some(value_range),
            mutates_memory: true,
            supports_rollback: true,
            risk_level: FeatureRiskLevel::Low,
        })
    }

    /// Returns the draft `TrainerManifest` without verifying artifacts yet (used before
    /// `build_trainer_package` populates computed artifact digests).
    pub fn build_draft(self) -> TrainerManifest {
        self.manifest
    }

    /// Validates all fields including artifact declarations.
    pub fn build(self) -> Result<TrainerManifest, ManifestError> {
        self.manifest.validate_semantic()?;
        Ok(self.manifest)
    }
}

/// Complete, validated trainer package output with deterministic SHA-256 digests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedTrainerPackage {
    pub manifest: TrainerManifest,
    pub payload: DeclarativePatchPayload,
    pub manifest_json: String,
    pub payload_json: String,
    pub manifest_sha256: String,
    pub entrypoint_sha256: String,
    pub entrypoint_size_bytes: u64,
}

impl GeneratedTrainerPackage {
    /// Writes `trainer-manifest.json` and the entrypoint payload file cleanly into `dir`.
    pub fn write_to_dir(&self, dir: impl AsRef<Path>) -> Result<PathBuf, std::io::Error> {
        let root = dir.as_ref();
        validate_relative_path(&self.manifest.runtime.entrypoint)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
        fs::create_dir_all(root)?;

        let entrypoint_path = root.join(&self.manifest.runtime.entrypoint);
        if let Some(parent) = entrypoint_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&entrypoint_path, self.payload_json.as_bytes())?;

        let manifest_path = root.join("trainer-manifest.json");
        fs::write(&manifest_path, self.manifest_json.as_bytes())?;
        Ok(manifest_path)
    }
}

/// Serializes `payload`, computes its byte size and SHA-256 digest, updates the manifest's
/// `artifacts` entry for `runtime.entrypoint`, and validates the full package definition.
pub fn build_trainer_package(
    mut manifest: TrainerManifest,
    payload: DeclarativePatchPayload,
) -> Result<GeneratedTrainerPackage, ManifestError> {
    validate_relative_path(&manifest.runtime.entrypoint)?;
    payload.validate_semantic()?;

    let payload_json = serde_json::to_string_pretty(&payload).map_err(|e| {
        ManifestError::InconsistentManifest(format!("failed to serialize payload: {e}"))
    })?;
    let entrypoint_bytes = payload_json.as_bytes();
    let entrypoint_sha256 = sha256_hex_bytes(entrypoint_bytes);
    let entrypoint_size_bytes = entrypoint_bytes.len() as u64;

    let entry = TrainerArtifact {
        filename: manifest.runtime.entrypoint.clone(),
        sha256: entrypoint_sha256.clone(),
        size_bytes: entrypoint_size_bytes,
        platform: manifest.runtime.supported_platforms.first().copied(),
    };
    if let Some(existing) = manifest
        .artifacts
        .iter_mut()
        .find(|a| a.filename == entry.filename)
    {
        *existing = entry;
    } else {
        manifest.artifacts.push(entry);
    }

    let pkg = TrainerPackageDefinition {
        manifest: manifest.clone(),
        payload: payload.clone(),
    };
    pkg.validate()?;

    let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| {
        ManifestError::InconsistentManifest(format!("failed to serialize manifest: {e}"))
    })?;
    let manifest_sha256 = sha256_hex_bytes(manifest_json.as_bytes());

    Ok(GeneratedTrainerPackage {
        manifest,
        payload,
        manifest_json,
        payload_json,
        manifest_sha256,
        entrypoint_sha256,
        entrypoint_size_bytes,
    })
}
