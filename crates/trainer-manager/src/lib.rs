//! L2 Trainer Manager: SQLite catalog, local trainer package ingestion,
//! SHA-256 artifact integrity verification, and compatibility matching.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nekod_manifest::{
    CompatibilityStatus, GameManifest, TargetPlatform, TrainerManifest, TrustTier,
    validate_relative_path,
};
use nekod_validator::{validate_game_manifest_str, validate_trainer_manifest_str};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Local record of an installed or cached trainer package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledTrainer {
    pub manifest: TrainerManifest,
    pub manifest_sha256: String,
    pub trust_tier: TrustTier,
    pub package_dir: String,
    pub revoked: bool,
}

/// Result of matching a trainer against a discovered game installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityMatch {
    pub trainer_id: String,
    pub game_id: String,
    pub status: CompatibilityStatus,
    pub notes: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrainerManagerError {
    #[error("trainer '{0}' not found")]
    NotFound(String),

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("artifact integrity mismatch for '{0}': {1}")]
    IntegrityMismatch(String, String),

    #[error("trainer '{0}' has been revoked")]
    Revoked(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("storage error: {0}")]
    Storage(String),
}

/// Trait abstracting local trainer catalog storage and verification.
pub trait TrainerCatalog: Send + Sync {
    fn list_for_game(&self, game_id: &str) -> Result<Vec<InstalledTrainer>, TrainerManagerError>;
    fn get_trainer(&self, trainer_id: &str) -> Result<InstalledTrainer, TrainerManagerError>;
}

/// Computes lowercase hex SHA-256 of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// SQLite-backed local catalog for `GameManifest`s and validated `InstalledTrainer` packages.
pub struct SqliteTrainerCatalog {
    conn: Mutex<Connection>,
}

impl SqliteTrainerCatalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TrainerManagerError> {
        let conn =
            Connection::open(path).map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, TrainerManagerError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS game_manifests (
                game_id TEXT PRIMARY KEY,
                manifest_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE IF NOT EXISTS installed_trainers (
                trainer_id TEXT PRIMARY KEY,
                game_id TEXT NOT NULL,
                version TEXT NOT NULL,
                manifest_json TEXT NOT NULL,
                manifest_sha256 TEXT NOT NULL,
                trust_tier TEXT NOT NULL,
                package_dir TEXT NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0,
                installed_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            ",
        )
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Validates and persists a `GameManifest` into the local catalog.
    pub fn register_game_manifest_str(
        &self,
        raw_json: &str,
    ) -> Result<GameManifest, TrainerManagerError> {
        let manifest = validate_game_manifest_str(raw_json)
            .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
        let canonical = serde_json::to_string(&manifest)
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO game_manifests (game_id, manifest_json, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(game_id) DO UPDATE SET
                manifest_json = excluded.manifest_json,
                updated_at = unixepoch()",
            params![manifest.id, canonical],
        )
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        Ok(manifest)
    }

    /// Lists all registered `GameManifest` entries.
    pub fn list_game_manifests(&self) -> Result<Vec<GameManifest>, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare("SELECT manifest_json FROM game_manifests ORDER BY game_id ASC")
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let mut manifests = Vec::new();
        for row in rows {
            let json = row.map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
            let manifest: GameManifest = serde_json::from_str(&json)
                .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    /// Validates a local trainer directory (`trainer-manifest.json` + artifact checksums + path containment)
    /// and registers it in the local SQLite catalog.
    pub fn ingest_local_package(
        &self,
        package_dir: impl AsRef<Path>,
        trust_tier: TrustTier,
    ) -> Result<InstalledTrainer, TrainerManagerError> {
        let package_root = fs::canonicalize(package_dir.as_ref())
            .map_err(|e| TrainerManagerError::Io(e.to_string()))?;

        let manifest_path = package_root.join("trainer-manifest.json");
        let raw_bytes = fs::read(&manifest_path).map_err(|e| {
            TrainerManagerError::Io(format!("failed to read '{}': {e}", manifest_path.display()))
        })?;
        let raw_str = std::str::from_utf8(&raw_bytes).map_err(|e| {
            TrainerManagerError::InvalidManifest(format!("manifest is not valid UTF-8: {e}"))
        })?;

        let manifest = validate_trainer_manifest_str(raw_str)
            .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
        let manifest_sha256 = sha256_hex(&raw_bytes);

        verify_package_artifacts(&package_root, &manifest)?;

        let canonical_json = serde_json::to_string(&manifest)
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let trust_json = serde_json::to_string(&trust_tier)
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let dir_str = package_root.to_string_lossy().into_owned();

        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO installed_trainers
                (trainer_id, game_id, version, manifest_json, manifest_sha256, trust_tier, package_dir, revoked, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, unixepoch())
             ON CONFLICT(trainer_id) DO UPDATE SET
                game_id = excluded.game_id,
                version = excluded.version,
                manifest_json = excluded.manifest_json,
                manifest_sha256 = excluded.manifest_sha256,
                trust_tier = excluded.trust_tier,
                package_dir = excluded.package_dir,
                revoked = 0,
                installed_at = unixepoch()",
            params![
                manifest.id,
                manifest.target_game.game_id,
                manifest.version.to_string(),
                canonical_json,
                manifest_sha256,
                trust_json,
                dir_str,
            ],
        )
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        Ok(InstalledTrainer {
            manifest,
            manifest_sha256,
            trust_tier,
            package_dir: dir_str,
            revoked: false,
        })
    }

    pub fn list_all_trainers(&self) -> Result<Vec<InstalledTrainer>, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare(
                "SELECT manifest_json, manifest_sha256, trust_tier, package_dir, revoked
                 FROM installed_trainers ORDER BY game_id ASC, trainer_id ASC",
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        rows.map(|r| {
            let (m_json, sha, tier_json, pkg_dir, revoked) =
                r.map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
            row_to_installed(&m_json, sha, &tier_json, pkg_dir, revoked)
        })
        .collect()
    }

    pub fn set_revoked(&self, trainer_id: &str, revoked: bool) -> Result<(), TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let updated = conn
            .execute(
                "UPDATE installed_trainers SET revoked = ?2 WHERE trainer_id = ?1",
                params![trainer_id, i64::from(revoked)],
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        if updated == 0 {
            return Err(TrainerManagerError::NotFound(trainer_id.to_owned()));
        }
        Ok(())
    }

    /// Marks an installed trainer revoked if present locally; returns `true` when updated.
    pub fn mark_revoked_if_installed(&self, trainer_id: &str) -> Result<bool, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let updated = conn
            .execute(
                "UPDATE installed_trainers SET revoked = 1 WHERE trainer_id = ?1 AND revoked = 0",
                params![trainer_id],
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        Ok(updated > 0)
    }

    /// Persists a staged trainer directory into `<install_root>/<trainer_id>/<version>/`,
    /// re-verifying manifest + artifact checksums, then registers it in SQLite.
    pub fn install_staged_package(
        &self,
        staged_dir: &Path,
        install_root: &Path,
        trust_tier: TrustTier,
    ) -> Result<InstalledTrainer, TrainerManagerError> {
        let canonical_stage =
            fs::canonicalize(staged_dir).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
        let raw_bytes = fs::read(canonical_stage.join("trainer-manifest.json")).map_err(|e| {
            TrainerManagerError::Io(format!("missing staged trainer-manifest.json: {e}"))
        })?;
        let raw_str = std::str::from_utf8(&raw_bytes).map_err(|e| {
            TrainerManagerError::InvalidManifest(format!("manifest is not valid UTF-8: {e}"))
        })?;
        let manifest = validate_trainer_manifest_str(raw_str)
            .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
        verify_package_artifacts(&canonical_stage, &manifest)?;

        let safe_id: String = manifest
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let target_dir = install_root
            .join(safe_id)
            .join(manifest.version.to_string());
        if target_dir.exists() {
            fs::remove_dir_all(&target_dir)
                .map_err(|e| TrainerManagerError::Io(format!("clear previous version dir: {e}")))?;
        }
        fs::create_dir_all(&target_dir)
            .map_err(|e| TrainerManagerError::Io(format!("create package install dir: {e}")))?;

        fs::copy(
            canonical_stage.join("trainer-manifest.json"),
            target_dir.join("trainer-manifest.json"),
        )
        .map_err(|e| TrainerManagerError::Io(e.to_string()))?;

        for artifact in &manifest.artifacts {
            let src = canonical_stage.join(&artifact.filename);
            let dest = target_dir.join(&artifact.filename);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
            }
            fs::copy(&src, &dest).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
        }

        self.ingest_local_package(&target_dir, trust_tier)
    }
}

fn row_to_installed(
    manifest_json: &str,
    manifest_sha256: String,
    trust_tier_json: &str,
    package_dir: String,
    revoked: i64,
) -> Result<InstalledTrainer, TrainerManagerError> {
    let manifest: TrainerManifest = serde_json::from_str(manifest_json)
        .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
    let trust_tier: TrustTier = serde_json::from_str(trust_tier_json)
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
    Ok(InstalledTrainer {
        manifest,
        manifest_sha256,
        trust_tier,
        package_dir,
        revoked: revoked != 0,
    })
}

impl TrainerCatalog for SqliteTrainerCatalog {
    fn list_for_game(&self, game_id: &str) -> Result<Vec<InstalledTrainer>, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare(
                "SELECT manifest_json, manifest_sha256, trust_tier, package_dir, revoked
                 FROM installed_trainers WHERE game_id = ?1 ORDER BY trainer_id ASC",
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(params![game_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        rows.map(|r| {
            let (m_json, sha, tier_json, pkg_dir, revoked) =
                r.map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
            row_to_installed(&m_json, sha, &tier_json, pkg_dir, revoked)
        })
        .collect()
    }

    fn get_trainer(&self, trainer_id: &str) -> Result<InstalledTrainer, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("trainer db mutex poisoned".to_owned()))?;
        let row: Option<(String, String, String, String, i64)> = conn
            .query_row(
                "SELECT manifest_json, manifest_sha256, trust_tier, package_dir, revoked
                 FROM installed_trainers WHERE trainer_id = ?1",
                params![trainer_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let Some((m_json, sha, tier_json, pkg_dir, revoked)) = row else {
            return Err(TrainerManagerError::NotFound(trainer_id.to_owned()));
        };
        row_to_installed(&m_json, sha, &tier_json, pkg_dir, revoked)
    }
}

/// Checks artifact path containment, file size, and SHA-256 digest on disk.
pub fn verify_package_artifacts(
    canonical_package_root: &Path,
    manifest: &TrainerManifest,
) -> Result<(), TrainerManagerError> {
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.filename)
            .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;

        let joined: PathBuf = canonical_package_root.join(&artifact.filename);
        let canonical_file = fs::canonicalize(&joined).map_err(|e| {
            TrainerManagerError::IntegrityMismatch(
                artifact.filename.clone(),
                format!("missing or unreadable file: {e}"),
            )
        })?;

        if !canonical_file.starts_with(canonical_package_root) {
            return Err(TrainerManagerError::IntegrityMismatch(
                artifact.filename.clone(),
                "artifact escapes package directory".to_owned(),
            ));
        }

        let bytes = fs::read(&canonical_file).map_err(|e| {
            TrainerManagerError::IntegrityMismatch(artifact.filename.clone(), e.to_string())
        })?;

        if bytes.len() as u64 != artifact.size_bytes {
            return Err(TrainerManagerError::IntegrityMismatch(
                artifact.filename.clone(),
                format!(
                    "expected {} bytes, found {} bytes",
                    artifact.size_bytes,
                    bytes.len()
                ),
            ));
        }

        let actual_sha = sha256_hex(&bytes);
        if actual_sha != artifact.sha256 {
            return Err(TrainerManagerError::IntegrityMismatch(
                artifact.filename.clone(),
                format!("expected sha256 {}, got {actual_sha}", artifact.sha256),
            ));
        }
    }
    Ok(())
}

/// Re-verifies manifest digest and artifact SHA-256 checksums on disk, then loads
/// the trainer's declared `runtime.entrypoint` with path containment guarantees.
pub fn load_verified_entrypoint_str(
    trainer: &InstalledTrainer,
) -> Result<String, TrainerManagerError> {
    if trainer.revoked {
        return Err(TrainerManagerError::Revoked(trainer.manifest.id.clone()));
    }

    let canonical_root = fs::canonicalize(&trainer.package_dir).map_err(|e| {
        TrainerManagerError::Io(format!(
            "cannot resolve package_dir '{}': {e}",
            trainer.package_dir
        ))
    })?;

    let manifest_bytes = fs::read(canonical_root.join("trainer-manifest.json")).map_err(|e| {
        TrainerManagerError::IntegrityMismatch(
            "trainer-manifest.json".to_owned(),
            format!("unreadable manifest file: {e}"),
        )
    })?;
    let actual_manifest_sha = sha256_hex(&manifest_bytes);
    if actual_manifest_sha != trainer.manifest_sha256 {
        return Err(TrainerManagerError::IntegrityMismatch(
            "trainer-manifest.json".to_owned(),
            format!(
                "manifest SHA-256 changed after install (expected {}, got {actual_manifest_sha})",
                trainer.manifest_sha256
            ),
        ));
    }

    verify_package_artifacts(&canonical_root, &trainer.manifest)?;

    let entrypoint = &trainer.manifest.runtime.entrypoint;
    validate_relative_path(entrypoint)
        .map_err(|e| TrainerManagerError::InvalidManifest(e.to_string()))?;
    if !trainer
        .manifest
        .artifacts
        .iter()
        .any(|a| a.filename == *entrypoint)
    {
        return Err(TrainerManagerError::IntegrityMismatch(
            entrypoint.clone(),
            "runtime.entrypoint must be listed in manifest.artifacts with a verified SHA-256"
                .to_owned(),
        ));
    }

    let canonical_entry = fs::canonicalize(canonical_root.join(entrypoint)).map_err(|e| {
        TrainerManagerError::IntegrityMismatch(
            entrypoint.clone(),
            format!("missing or unreadable entrypoint: {e}"),
        )
    })?;
    if !canonical_entry.starts_with(&canonical_root) {
        return Err(TrainerManagerError::IntegrityMismatch(
            entrypoint.clone(),
            "entrypoint escapes package directory".to_owned(),
        ));
    }

    fs::read_to_string(&canonical_entry).map_err(|e| {
        TrainerManagerError::Io(format!("failed reading entrypoint '{entrypoint}': {e}"))
    })
}

/// Computes compatibility status for a trainer against a platform and optional store build/version.
pub fn compute_compatibility(
    trainer: &TrainerManifest,
    platform: TargetPlatform,
    detected_build_id: Option<&str>,
) -> CompatibilityMatch {
    if !trainer.runtime.supported_platforms.contains(&platform) {
        return CompatibilityMatch {
            trainer_id: trainer.id.clone(),
            game_id: trainer.target_game.game_id.clone(),
            status: CompatibilityStatus::Broken,
            notes: Some(format!("platform '{platform:?}' is not supported")),
        };
    }

    let matching_versions: Vec<_> = trainer
        .supported_versions
        .iter()
        .filter(|v| v.platform == platform)
        .collect();

    if matching_versions.is_empty() {
        return CompatibilityMatch {
            trainer_id: trainer.id.clone(),
            game_id: trainer.target_game.game_id.clone(),
            status: CompatibilityStatus::Untested,
            notes: Some("no supported version metadata declared for this platform".to_owned()),
        };
    }

    if let Some(build_id) = detected_build_id
        && let Some(exact) = matching_versions
            .iter()
            .find(|v| v.version_pattern == build_id || v.version_pattern == "*")
    {
        return CompatibilityMatch {
            trainer_id: trainer.id.clone(),
            game_id: trainer.target_game.game_id.clone(),
            status: exact.status,
            notes: Some(format!("matched build '{build_id}'")),
        };
    }

    let best = matching_versions[0];
    CompatibilityMatch {
        trainer_id: trainer.id.clone(),
        game_id: trainer.target_game.game_id.clone(),
        status: best.status,
        notes: Some(format!(
            "declared for game version '{}'",
            best.version_pattern
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_pkg_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekod_{prefix}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_valid_package(dir: &Path) -> TrainerManifest {
        let payload = br#"{"patches":[]}"#;
        let payload_sha = sha256_hex(payload);
        fs::write(dir.join("trainer.json"), payload).unwrap();

        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut manifest: TrainerManifest = serde_json::from_str(raw).unwrap();
        manifest.artifacts[0].sha256 = payload_sha;
        manifest.artifacts[0].size_bytes = payload.len() as u64;

        fs::write(
            dir.join("trainer-manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn ingests_valid_local_trainer_package_and_verifies_sha256() {
        let dir = temp_pkg_dir("valid_pkg");
        write_valid_package(&dir);

        let catalog = SqliteTrainerCatalog::open_in_memory().unwrap();
        let installed = catalog
            .ingest_local_package(&dir, TrustTier::CommunityVerified)
            .expect("ingestion succeeds");

        assert_eq!(installed.manifest.id, "dev.example.sample-singleplayer-rpg");
        assert_eq!(installed.trust_tier, TrustTier::CommunityVerified);
        assert_eq!(
            catalog
                .list_for_game("sample-singleplayer-rpg")
                .unwrap()
                .len(),
            1
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_tampered_artifact_hash() {
        let dir = temp_pkg_dir("tampered_pkg");
        write_valid_package(&dir);
        fs::write(dir.join("trainer.json"), br#"{"patches":["evil"]}"#).unwrap();

        let catalog = SqliteTrainerCatalog::open_in_memory().unwrap();
        let err = catalog
            .ingest_local_package(&dir, TrustTier::Unverified)
            .unwrap_err();
        assert!(matches!(err, TrainerManagerError::IntegrityMismatch(_, _)));

        let _ = fs::remove_dir_all(dir);
    }
}
