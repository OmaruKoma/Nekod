//! L4 Authoring SDK for declarative Nekod trainers (`declarative_patch_v1`).
//!
//! Types in this crate map 1:1 to `schemas/declarative-patch.v1.schema.json` and
//! `docs/specs/declarative-patch.md`.

use std::collections::BTreeSet;

pub mod builders;
pub use builders::*;

use nekod_manifest::{
    FeatureControlType, ManifestError, TrainerManifest, is_valid_feature_id, validate_schema_v1,
};
use semver::Version;
use serde::{Deserialize, Serialize};

/// Maximum number of bytes permitted in a single `byte_patch` instruction.
pub const MAX_PATCH_BYTES: usize = 64;
/// Maximum byte length of an AOB signature pattern (`aob_pattern`).
pub const MAX_AOB_BYTES: usize = 128;
/// Maximum pointer dereference depth permitted in `pointer_chain`.
pub const MAX_POINTER_CHAIN_DEPTH: usize = 8;
/// Maximum operations permitted inside a single feature entry.
pub const MAX_OPERATIONS_PER_FEATURE: usize = 16;
/// Minimum allowed freeze tick interval in milliseconds (ADR-013).
pub const MIN_FREEZE_INTERVAL_MS: u32 = 50;
/// Maximum allowed freeze tick interval in milliseconds (ADR-013).
pub const MAX_FREEZE_INTERVAL_MS: u32 = 1000;
/// Default freeze tick interval in milliseconds when omitted.
pub const DEFAULT_FREEZE_INTERVAL_MS: u32 = 100;

/// Top-level `declarative_patch_v1` trainer payload document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarativePatchPayload {
    pub schema_version: Version,
    pub features: Vec<DeclarativeFeatureEntry>,
}

impl DeclarativePatchPayload {
    /// Looks up the declarative entry for a manifest feature ID.
    pub fn feature(&self, feature_id: &str) -> Option<&DeclarativeFeatureEntry> {
        self.features.iter().find(|f| f.feature_id == feature_id)
    }

    /// Validates semantic constraints beyond JSON Schema syntax.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        validate_schema_v1(&self.schema_version)?;

        if self.features.is_empty() {
            return Err(ManifestError::InconsistentManifest(
                "declarative patch payload must define at least one feature".to_owned(),
            ));
        }

        let mut seen_features = BTreeSet::new();
        for feature in &self.features {
            if !is_valid_feature_id(&feature.feature_id) {
                return Err(ManifestError::InvalidIdentifier(
                    feature.feature_id.clone(),
                    "must match ^[a-z0-9][a-z0-9_-]{1,63}$",
                ));
            }
            if !seen_features.insert(feature.feature_id.clone()) {
                return Err(ManifestError::InconsistentManifest(format!(
                    "duplicate declarative feature '{}'",
                    feature.feature_id
                )));
            }
            feature.validate_semantic()?;
        }

        Ok(())
    }
}

/// Declarative patch entry bound to a `TrainerFeature.id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarativeFeatureEntry {
    pub feature_id: String,
    pub operations: Vec<DeclarativeOperation>,
}

impl DeclarativeFeatureEntry {
    /// Validates operations within a single feature entry.
    pub fn validate_semantic(&self) -> Result<(), ManifestError> {
        if self.operations.is_empty() || self.operations.len() > MAX_OPERATIONS_PER_FEATURE {
            return Err(ManifestError::InconsistentManifest(format!(
                "feature '{}' must contain between 1 and {MAX_OPERATIONS_PER_FEATURE} operations",
                self.feature_id
            )));
        }

        for op in &self.operations {
            op.validate_semantic(&self.feature_id)?;
        }

        Ok(())
    }
}

/// Bounded declarative instruction executed by `DeclarativeRuntime`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeclarativeOperation {
    /// Replaces a bounded slice of memory with static patch bytes.
    BytePatch {
        locator: AddressLocator,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_original_bytes: Option<String>,
        patch_bytes: String,
    },
    /// Writes a typed scalar value to a located address, optionally frozen.
    WriteValue {
        locator: AddressLocator,
        value_type: ScalarValueType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        freeze_interval_ms: Option<u32>,
    },
}

impl DeclarativeOperation {
    /// Returns the address locator used by this operation.
    pub fn locator(&self) -> &AddressLocator {
        match self {
            Self::BytePatch { locator, .. } | Self::WriteValue { locator, .. } => locator,
        }
    }

    /// Returns the target module name referenced by the locator.
    pub fn target_module(&self) -> &str {
        self.locator().module()
    }

    /// Returns the maximum contiguous byte length this operation will write.
    pub fn write_byte_len(&self) -> Result<usize, ManifestError> {
        match self {
            Self::BytePatch { patch_bytes, .. } => Ok(parse_hex_bytes(patch_bytes)?.len()),
            Self::WriteValue { value_type, .. } => Ok(value_type.byte_len()),
        }
    }

    /// Validates operation invariants and bounds.
    pub fn validate_semantic(&self, feature_id: &str) -> Result<(), ManifestError> {
        self.locator().validate_semantic(feature_id)?;

        match self {
            Self::BytePatch {
                expected_original_bytes,
                patch_bytes,
                ..
            } => {
                let patch = parse_hex_bytes(patch_bytes)?;
                if let Some(expected_raw) = expected_original_bytes {
                    let expected = parse_hex_bytes(expected_raw)?;
                    if expected.len() != patch.len() {
                        return Err(ManifestError::InconsistentManifest(format!(
                            "feature '{feature_id}': expected_original_bytes length ({}) must equal patch_bytes length ({})",
                            expected.len(),
                            patch.len()
                        )));
                    }
                }
            }
            Self::WriteValue {
                value_type,
                value,
                freeze_interval_ms,
                ..
            } => {
                if let Some(interval) = freeze_interval_ms
                    && !(MIN_FREEZE_INTERVAL_MS..=MAX_FREEZE_INTERVAL_MS).contains(interval)
                {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "feature '{feature_id}': freeze_interval_ms ({interval}) must be within {MIN_FREEZE_INTERVAL_MS}..={MAX_FREEZE_INTERVAL_MS}"
                    )));
                }
                if let Some(raw_val) = value {
                    value_type.encode_value(*raw_val)?;
                }
            }
        }

        Ok(())
    }
}

/// Strategy used to resolve a runtime target address inside a verified module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AddressLocator {
    /// Fixed offset relative to a loaded module's base address.
    ModuleOffset { module: String, offset: u64 },
    /// Byte signature scan within a loaded module's committed readable regions.
    AobPattern {
        module: String,
        pattern: String,
        #[serde(default)]
        match_index: u32,
        #[serde(default)]
        offset: i64,
    },
    /// Bounded multi-level pointer dereference starting at `module + base_offset`.
    PointerChain {
        module: String,
        base_offset: u64,
        offsets: Vec<i64>,
    },
}

impl AddressLocator {
    /// Returns the target module name.
    pub fn module(&self) -> &str {
        match self {
            Self::ModuleOffset { module, .. }
            | Self::AobPattern { module, .. }
            | Self::PointerChain { module, .. } => module,
        }
    }

    /// Validates locator syntax and safety bounds.
    pub fn validate_semantic(&self, feature_id: &str) -> Result<(), ManifestError> {
        if !is_valid_module_name(self.module()) {
            return Err(ManifestError::InvalidIdentifier(
                self.module().to_owned(),
                "module name must be 1..=128 characters matching ^[A-Za-z0-9._-]+$",
            ));
        }

        match self {
            Self::ModuleOffset { .. } => Ok(()),
            Self::AobPattern { pattern, .. } => {
                let parsed = parse_aob_pattern(pattern)?;
                if parsed.iter().all(Option::is_none) {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "feature '{feature_id}': aob_pattern must include at least one concrete byte"
                    )));
                }
                Ok(())
            }
            Self::PointerChain { offsets, .. } => {
                if offsets.is_empty() || offsets.len() > MAX_POINTER_CHAIN_DEPTH {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "feature '{feature_id}': pointer_chain offsets length ({}) must be 1..={MAX_POINTER_CHAIN_DEPTH}",
                        offsets.len()
                    )));
                }
                Ok(())
            }
        }
    }
}

/// Scalar memory types supported by `write_value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalarValueType {
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl ScalarValueType {
    /// Returns the little-endian byte width of this scalar type.
    pub const fn byte_len(self) -> usize {
        match self {
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }

    /// Encodes a numeric JSON/UI value into little-endian bytes with range checking.
    pub fn encode_value(self, raw: f64) -> Result<Vec<u8>, ManifestError> {
        if !raw.is_finite() {
            return Err(ManifestError::InconsistentManifest(
                "write_value numeric payload must be finite".to_owned(),
            ));
        }

        match self {
            Self::I32 => {
                let v = checked_integral_i64(raw)?;
                let narrowed = i32::try_from(v).map_err(|_| {
                    ManifestError::InconsistentManifest(format!(
                        "value {raw} is out of range for i32"
                    ))
                })?;
                Ok(narrowed.to_le_bytes().to_vec())
            }
            Self::U32 => {
                let v = checked_integral_u64(raw)?;
                let narrowed = u32::try_from(v).map_err(|_| {
                    ManifestError::InconsistentManifest(format!(
                        "value {raw} is out of range for u32"
                    ))
                })?;
                Ok(narrowed.to_le_bytes().to_vec())
            }
            Self::I64 => {
                let v = checked_integral_i64(raw)?;
                Ok(v.to_le_bytes().to_vec())
            }
            Self::U64 => {
                let v = checked_integral_u64(raw)?;
                Ok(v.to_le_bytes().to_vec())
            }
            Self::F32 => {
                let narrowed = raw as f32;
                if !narrowed.is_finite() {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "value {raw} overflows f32"
                    )));
                }
                Ok(narrowed.to_le_bytes().to_vec())
            }
            Self::F64 => Ok(raw.to_le_bytes().to_vec()),
        }
    }
}

fn checked_integral_i64(raw: f64) -> Result<i64, ManifestError> {
    if raw.fract() != 0.0 || raw < i64::MIN as f64 || raw >= 9_223_372_036_854_775_808.0 {
        return Err(ManifestError::InconsistentManifest(format!(
            "integer write_value '{raw}' must be an exact in-range integer"
        )));
    }
    Ok(raw as i64)
}

fn checked_integral_u64(raw: f64) -> Result<u64, ManifestError> {
    if raw.fract() != 0.0 || raw < 0.0 || raw >= 18_446_744_073_709_551_616.0 {
        return Err(ManifestError::InconsistentManifest(format!(
            "unsigned integer write_value '{raw}' must be a non-negative exact integer"
        )));
    }
    Ok(raw as u64)
}

/// Checks whether a module name matches `^[A-Za-z0-9._-]+$` and `1..=128`.
pub fn is_valid_module_name(module: &str) -> bool {
    !module.is_empty()
        && module.len() <= 128
        && module
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Parses whitespace-separated 2-character hex bytes (`<= MAX_PATCH_BYTES`).
pub fn parse_hex_bytes(input: &str) -> Result<Vec<u8>, ManifestError> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > MAX_PATCH_BYTES {
        return Err(ManifestError::InconsistentManifest(format!(
            "hex byte sequence must contain between 1 and {MAX_PATCH_BYTES} bytes"
        )));
    }

    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token.len() != 2 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ManifestError::InconsistentManifest(format!(
                "invalid hex byte token '{token}'"
            )));
        }
        let byte = u8::from_str_radix(token, 16).map_err(|_| {
            ManifestError::InconsistentManifest(format!("invalid hex byte token '{token}'"))
        })?;
        out.push(byte);
    }
    Ok(out)
}

/// Parses whitespace-separated AOB pattern tokens (`XX` or `??`, `<= MAX_AOB_BYTES`).
pub fn parse_aob_pattern(input: &str) -> Result<Vec<Option<u8>>, ManifestError> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > MAX_AOB_BYTES {
        return Err(ManifestError::InconsistentManifest(format!(
            "AOB pattern must contain between 1 and {MAX_AOB_BYTES} tokens"
        )));
    }

    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token == "??" {
            out.push(None);
        } else if token.len() == 2 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            let byte = u8::from_str_radix(token, 16).map_err(|_| {
                ManifestError::InconsistentManifest(format!("invalid AOB byte token '{token}'"))
            })?;
            out.push(Some(byte));
        } else {
            return Err(ManifestError::InconsistentManifest(format!(
                "invalid AOB token '{token}': expected two hex digits or '??'"
            )));
        }
    }
    Ok(out)
}

/// Container pairing a validated `TrainerManifest` with its `declarative_patch_v1` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainerPackageDefinition {
    pub manifest: TrainerManifest,
    pub payload: DeclarativePatchPayload,
}

impl TrainerPackageDefinition {
    /// Validates manifest invariants, declarative payload invariants, and cross-references.
    pub fn validate(&self) -> Result<(), ManifestError> {
        self.manifest.validate_semantic()?;
        self.payload.validate_semantic()?;

        for entry in &self.payload.features {
            let feature = self
                .manifest
                .features
                .iter()
                .find(|f| f.id == entry.feature_id)
                .ok_or_else(|| {
                    ManifestError::InconsistentManifest(format!(
                        "declarative payload references unknown manifest feature '{}'",
                        entry.feature_id
                    ))
                })?;

            for op in &entry.operations {
                if let DeclarativeOperation::WriteValue { value: None, .. } = op
                    && !matches!(
                        feature.control_type,
                        FeatureControlType::Integer | FeatureControlType::Float
                    )
                {
                    return Err(ManifestError::InconsistentManifest(format!(
                        "feature '{}': write_value without static value requires integer or float control_type",
                        entry.feature_id
                    )));
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
    fn parses_and_validates_example_declarative_patch() {
        let raw = include_str!("../../../schemas/examples/declarative-patch.example.json");
        let payload: DeclarativePatchPayload = serde_json::from_str(raw).unwrap();
        payload.validate_semantic().unwrap();
        assert_eq!(payload.features.len(), 2);
    }

    #[test]
    fn rejects_mismatched_expected_bytes_and_patch_bytes_lengths() {
        let payload = DeclarativePatchPayload {
            schema_version: Version::new(1, 0, 0),
            features: vec![DeclarativeFeatureEntry {
                feature_id: "god-mode".to_owned(),
                operations: vec![DeclarativeOperation::BytePatch {
                    locator: AddressLocator::ModuleOffset {
                        module: "Game.exe".to_owned(),
                        offset: 0x1234,
                    },
                    expected_original_bytes: Some("90 90".to_owned()),
                    patch_bytes: "90 90 90".to_owned(),
                }],
            }],
        };
        assert!(payload.validate_semantic().is_err());
    }

    #[test]
    fn rejects_out_of_bounds_freeze_interval_and_pointer_depth() {
        let payload = DeclarativePatchPayload {
            schema_version: Version::new(1, 0, 0),
            features: vec![DeclarativeFeatureEntry {
                feature_id: "set-gold".to_owned(),
                operations: vec![DeclarativeOperation::WriteValue {
                    locator: AddressLocator::PointerChain {
                        module: "Game.exe".to_owned(),
                        base_offset: 0x100,
                        offsets: vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
                    },
                    value_type: ScalarValueType::I32,
                    value: Some(999.0),
                    freeze_interval_ms: Some(10),
                }],
            }],
        };
        assert!(payload.validate_semantic().is_err());
    }

    #[test]
    fn rejects_all_wildcard_aob_pattern() {
        let payload = DeclarativePatchPayload {
            schema_version: Version::new(1, 0, 0),
            features: vec![DeclarativeFeatureEntry {
                feature_id: "god-mode".to_owned(),
                operations: vec![DeclarativeOperation::BytePatch {
                    locator: AddressLocator::AobPattern {
                        module: "Game.exe".to_owned(),
                        pattern: "?? ?? ??".to_owned(),
                        match_index: 0,
                        offset: 0,
                    },
                    expected_original_bytes: None,
                    patch_bytes: "90".to_owned(),
                }],
            }],
        };
        assert!(payload.validate_semantic().is_err());
    }

    #[test]
    fn builders_produce_valid_package_and_aob_helper_resolves_offsets() {
        let draft = TrainerManifestBuilder::new(
            "dev.example.sdk-demo",
            "SDK Demo Trainer",
            "sample-singleplayer-rpg",
            "Sample Singleplayer RPG",
            "https://github.com/example/sdk-demo",
        )
        .author("SDK Author", Some("sdk-author".to_owned()), None)
        .default_memory_permissions(vec!["SampleRPG.exe".to_owned()], 64)
        .toggle_feature(
            "god-mode",
            "God Mode",
            "Prevents HP loss.",
            nekod_manifest::FeatureCategory::Player,
            Some("F1".to_owned()),
        )
        .build_draft();

        let payload = DeclarativePatchBuilder::new()
            .byte_patch(
                "god-mode",
                AddressLocator::AobPattern {
                    module: "SampleRPG.exe".to_owned(),
                    pattern: "48 8B ?? ?? 90".to_owned(),
                    match_index: 0,
                    offset: 4,
                },
                Some("90".to_owned()),
                "90",
            )
            .build()
            .unwrap();

        let pkg = build_trainer_package(draft, payload).unwrap();
        assert_eq!(pkg.manifest.artifacts.len(), 1);
        assert_eq!(pkg.manifest.artifacts[0].sha256, pkg.entrypoint_sha256);
        assert_eq!(
            pkg.manifest.artifacts[0].size_bytes,
            pkg.entrypoint_size_bytes
        );

        let bytes = [0x00, 0x48, 0x8B, 0x05, 0x10, 0x90, 0xC3];
        let hit = test_aob_pattern_in_bytes(&bytes, "48 8B ?? ?? 90", 0, 4).unwrap();
        assert_eq!(hit.pattern_offset, 1);
        assert_eq!(hit.resolved_offset, 5);
        assert_eq!(bytes[hit.resolved_offset], 0x90);
        assert!(test_aob_pattern_in_bytes(&bytes, "48 8B ?? ?? 90", 0, -5).is_err());
    }
}
