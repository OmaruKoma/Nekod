//! L5 manifest validator combining JSON Schema Draft 2020-12 checks
//! with semantic and security policy invariants from `nekod-manifest`.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use jsonschema::Validator;
use nekod_manifest::{
    GameManifest, ManifestError, RegistryManifest, RuntimeKind, TrainerManifest,
    validate_relative_path,
};
pub use nekod_trainer_sdk::sha256_hex_bytes;
use nekod_trainer_sdk::{DeclarativePatchPayload, TrainerPackageDefinition};
use serde_json::Value;
use thiserror::Error;

const GAME_SCHEMA_STR: &str = include_str!("../../../schemas/game-manifest.v1.schema.json");
const TRAINER_SCHEMA_STR: &str = include_str!("../../../schemas/trainer-manifest.v1.schema.json");
const REGISTRY_SCHEMA_STR: &str = include_str!("../../../schemas/registry-manifest.v1.schema.json");
const DECLARATIVE_PATCH_SCHEMA_STR: &str =
    include_str!("../../../schemas/declarative-patch.v1.schema.json");

static GAME_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
static TRAINER_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
static REGISTRY_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
static DECLARATIVE_PATCH_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    Game,
    Trainer,
    Registry,
    Patch,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("invalid JSON syntax: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("schema compilation failed: {0}")]
    SchemaCompile(String),

    #[error("JSON Schema validation failed:\n{}", .0.join("\n"))]
    SchemaViolation(Vec<String>),

    #[error("semantic/policy validation failed: {0}")]
    Semantic(#[from] ManifestError),

    #[error("package validation failed: {0}")]
    Package(String),
}

fn compile_validator(schema_str: &str) -> Result<Validator, String> {
    let schema: Value = serde_json::from_str(schema_str).map_err(|e| e.to_string())?;
    jsonschema::draft202012::new(&schema).map_err(|e| e.to_string())
}

fn validate_with(
    cache: &OnceLock<Result<Validator, String>>,
    schema_str: &str,
    instance: &Value,
) -> Result<(), ValidationError> {
    let validator = cache
        .get_or_init(|| compile_validator(schema_str))
        .as_ref()
        .map_err(|e| ValidationError::SchemaCompile(e.clone()))?;

    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|err| format!("- {err} (at {})", err.instance_path()))
        .collect();

    if !errors.is_empty() {
        return Err(ValidationError::SchemaViolation(errors));
    }
    Ok(())
}

/// Validates a raw JSON string as a Game Manifest (`v1`).
pub fn validate_game_manifest_str(raw: &str) -> Result<GameManifest, ValidationError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_with(&GAME_VALIDATOR, GAME_SCHEMA_STR, &value)?;
    let manifest: GameManifest = serde_json::from_value(value)?;
    manifest.validate_semantic()?;
    Ok(manifest)
}

/// Validates a raw JSON string as a Trainer Manifest (`v1`).
pub fn validate_trainer_manifest_str(raw: &str) -> Result<TrainerManifest, ValidationError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_with(&TRAINER_VALIDATOR, TRAINER_SCHEMA_STR, &value)?;
    let manifest: TrainerManifest = serde_json::from_value(value)?;
    manifest.validate_semantic()?;
    Ok(manifest)
}

/// Validates a raw JSON string as a Registry Manifest (`v1`).
pub fn validate_registry_manifest_str(raw: &str) -> Result<RegistryManifest, ValidationError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_with(&REGISTRY_VALIDATOR, REGISTRY_SCHEMA_STR, &value)?;
    let manifest: RegistryManifest = serde_json::from_value(value)?;
    manifest.validate_semantic()?;
    Ok(manifest)
}

/// Validates a raw JSON string as a `declarative_patch_v1` trainer payload.
pub fn validate_declarative_patch_str(
    raw: &str,
) -> Result<DeclarativePatchPayload, ValidationError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_with(
        &DECLARATIVE_PATCH_VALIDATOR,
        DECLARATIVE_PATCH_SCHEMA_STR,
        &value,
    )?;
    let payload: DeclarativePatchPayload = serde_json::from_value(value)?;
    payload.validate_semantic()?;
    Ok(payload)
}

/// Validates an unpacked trainer package directory end-to-end:
/// `trainer-manifest.json` schema + semantics, artifact path containment + size + SHA-256,
/// and `declarative_patch_v1` payload cross-reference checks.
pub fn validate_trainer_package_dir(
    package_dir: impl AsRef<Path>,
) -> Result<(TrainerManifest, String), ValidationError> {
    let root = fs::canonicalize(package_dir.as_ref())
        .map_err(|e| ValidationError::Package(format!("cannot open package dir: {e}")))?;

    let manifest_bytes = fs::read(root.join("trainer-manifest.json"))
        .map_err(|e| ValidationError::Package(format!("missing trainer-manifest.json: {e}")))?;
    let manifest_raw = std::str::from_utf8(&manifest_bytes).map_err(|e| {
        ValidationError::Package(format!("trainer-manifest.json is not UTF-8: {e}"))
    })?;
    let manifest = validate_trainer_manifest_str(manifest_raw)?;
    let manifest_sha256 = sha256_hex_bytes(&manifest_bytes);

    let mut entrypoint_listed = false;
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.filename)?;
        let canonical_file = fs::canonicalize(root.join(&artifact.filename)).map_err(|e| {
            ValidationError::Package(format!("missing artifact '{}': {e}", artifact.filename))
        })?;
        if !canonical_file.starts_with(&root) {
            return Err(ValidationError::Package(format!(
                "artifact '{}' escapes package root",
                artifact.filename
            )));
        }
        let bytes = fs::read(&canonical_file).map_err(|e| {
            ValidationError::Package(format!("cannot read '{}': {e}", artifact.filename))
        })?;
        if bytes.len() as u64 != artifact.size_bytes {
            return Err(ValidationError::Package(format!(
                "artifact '{}' expected {} bytes, found {}",
                artifact.filename,
                artifact.size_bytes,
                bytes.len()
            )));
        }
        let actual_sha = sha256_hex_bytes(&bytes);
        if actual_sha != artifact.sha256 {
            return Err(ValidationError::Package(format!(
                "artifact '{}' sha256 mismatch (expected {}, got {actual_sha})",
                artifact.filename, artifact.sha256
            )));
        }
        if artifact.filename == manifest.runtime.entrypoint {
            entrypoint_listed = true;
        }
    }

    if !entrypoint_listed {
        return Err(ValidationError::Package(format!(
            "runtime.entrypoint '{}' must be declared in artifacts with SHA-256",
            manifest.runtime.entrypoint
        )));
    }

    if manifest.runtime.kind == RuntimeKind::DeclarativePatchV1 {
        let entry_raw =
            fs::read_to_string(root.join(&manifest.runtime.entrypoint)).map_err(|e| {
                ValidationError::Package(format!(
                    "cannot read entrypoint '{}': {e}",
                    manifest.runtime.entrypoint
                ))
            })?;
        let payload = validate_declarative_patch_str(&entry_raw)?;
        TrainerPackageDefinition {
            manifest: manifest.clone(),
            payload,
        }
        .validate()?;
    }

    Ok((manifest, manifest_sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_example_game_manifest_against_schema() {
        let raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        validate_game_manifest_str(raw).expect("game example passes schema + semantic validation");
    }

    #[test]
    fn validates_example_trainer_manifest_against_schema() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        validate_trainer_manifest_str(raw)
            .expect("trainer example passes schema + semantic validation");
    }

    #[test]
    fn validates_example_registry_manifest_against_schema() {
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json");
        validate_registry_manifest_str(raw)
            .expect("registry example passes schema + semantic validation");
    }

    #[test]
    fn rejects_unknown_top_level_field_in_trainer_manifest() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut value: Value = serde_json::from_str(raw).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected_field".to_owned(), Value::Bool(true));

        let modified = serde_json::to_string(&value).unwrap();
        let err = validate_trainer_manifest_str(&modified).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaViolation(_)));
    }

    #[test]
    fn schema_rejects_relative_path_traversal_directly() {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut value: Value = serde_json::from_str(raw).unwrap();
        value["runtime"]["entrypoint"] = Value::String("../evil.json".to_owned());

        let modified = serde_json::to_string(&value).unwrap();
        let err = validate_trainer_manifest_str(&modified).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaViolation(_)));
    }

    #[test]
    fn schema_rejects_store_prefixed_game_id_per_u11() {
        let raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let mut value: Value = serde_json::from_str(raw).unwrap();
        value["id"] = Value::String("steam:999999".to_owned());

        let modified = serde_json::to_string(&value).unwrap();
        let err = validate_game_manifest_str(&modified).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaViolation(_)));
    }

    #[test]
    fn validates_example_declarative_patch_against_schema() {
        let raw = include_str!("../../../schemas/examples/declarative-patch.example.json");
        let payload = validate_declarative_patch_str(raw)
            .expect("declarative patch example passes schema + semantic validation");
        assert_eq!(payload.features.len(), 2);
    }

    #[test]
    fn schema_rejects_out_of_range_freeze_interval_and_bad_hex() {
        let raw = include_str!("../../../schemas/examples/declarative-patch.example.json");
        let mut value: Value = serde_json::from_str(raw).unwrap();
        value["features"][1]["operations"][0]["freeze_interval_ms"] = serde_json::json!(15);

        let modified = serde_json::to_string(&value).unwrap();
        let err = validate_declarative_patch_str(&modified).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaViolation(_)));

        let mut bad_hex: Value = serde_json::from_str(raw).unwrap();
        bad_hex["features"][0]["operations"][0]["patch_bytes"] =
            Value::String("90 ZZ 90".to_owned());
        let err_hex = validate_declarative_patch_str(&bad_hex.to_string()).unwrap_err();
        assert!(matches!(err_hex, ValidationError::SchemaViolation(_)));
    }

    #[test]
    fn validates_full_trainer_package_directory_and_rejects_mismatch() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekod_val_pkg_{nanos}"));
        fs::create_dir_all(&dir).unwrap();

        let payload = include_bytes!("../../../schemas/examples/declarative-patch.example.json");
        fs::write(dir.join("trainer.json"), payload).unwrap();

        let mut trainer: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        trainer.artifacts[0].sha256 = sha256_hex_bytes(payload);
        trainer.artifacts[0].size_bytes = payload.len() as u64;
        fs::write(
            dir.join("trainer-manifest.json"),
            serde_json::to_vec_pretty(&trainer).unwrap(),
        )
        .unwrap();

        let (validated, sha) = validate_trainer_package_dir(&dir).unwrap();
        assert_eq!(validated.id, trainer.id);
        assert_eq!(sha.len(), 64);

        // Tamper payload after packaging -> must fail SHA-256 check
        fs::write(dir.join("trainer.json"), b"{}").unwrap();
        assert!(matches!(
            validate_trainer_package_dir(&dir).unwrap_err(),
            ValidationError::Package(_)
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
