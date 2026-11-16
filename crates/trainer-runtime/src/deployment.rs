//! Disk-level clean staging and rollback ledger (`DeploymentLedger`) for managed bridge
//! and framework runtime artifacts deployed inside game install directories.
//!
//! Mirrors `PatchLedger` safety invariants on disk: strict relative-path containment,
//! SHA-256 verification before staging, original-file backup snapshots, persistent
//! crash-recovery metadata, and conflict-safe LIFO removal/restoration.

use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::{is_valid_sha256_hex, validate_relative_path};
use nekod_validator::sha256_hex_bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors emitted by `DeploymentLedger`.
#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("invalid game directory '{0}': {1}")]
    InvalidGameDir(String, String),

    #[error("unsafe deployment relative path '{0}': {1}")]
    InvalidRelativePath(String, String),

    #[error("artifact SHA-256 mismatch for '{path}': expected {expected}, actual {actual}")]
    IntegrityMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("I/O error during deployment operation: {0}")]
    Io(#[from] std::io::Error),

    #[error("ledger serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Lifecycle state of a single managed file deployment record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentState {
    Deployed,
    Restored,
    Conflict,
}

/// Persisted record of one file staged into a game installation folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRecord {
    pub record_id: u64,
    pub game_id: String,
    pub package_id: String,
    pub relative_path: String,
    pub deployed_sha256: String,
    pub backup_filename: Option<String>,
    pub original_sha256: Option<String>,
    pub state: DeploymentState,
}

/// Summary produced when cleanly removing/restoring managed files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UndeployReport {
    pub removed_count: usize,
    pub restored_originals: usize,
    pub already_absent: usize,
    pub conflicts: usize,
    pub conflict_paths: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedLedger {
    next_id: u64,
    records: Vec<DeploymentRecord>,
}

/// Persistent clean deployment & rollback ledger scoped to a single game install root.
pub struct DeploymentLedger {
    game_dir: PathBuf,
    backup_dir: PathBuf,
    ledger_path: PathBuf,
    next_id: u64,
    records: Vec<DeploymentRecord>,
}

impl DeploymentLedger {
    /// Opens or initializes a persistent `DeploymentLedger` for `game_dir`.
    pub fn open(
        game_dir: impl AsRef<Path>,
        backup_dir: impl AsRef<Path>,
        ledger_path: impl AsRef<Path>,
    ) -> Result<Self, DeploymentError> {
        let raw_game = game_dir.as_ref();
        let canon_game = fs::canonicalize(raw_game).map_err(|e| {
            DeploymentError::InvalidGameDir(raw_game.display().to_string(), e.to_string())
        })?;
        if !canon_game.is_dir() {
            return Err(DeploymentError::InvalidGameDir(
                canon_game.display().to_string(),
                "path is not a directory".to_owned(),
            ));
        }

        let backup_dir = backup_dir.as_ref().to_path_buf();
        fs::create_dir_all(&backup_dir)?;

        let ledger_path = ledger_path.as_ref().to_path_buf();
        if let Some(parent) = ledger_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let persisted = if ledger_path.is_file() {
            let raw = fs::read_to_string(&ledger_path)?;
            serde_json::from_str::<PersistedLedger>(&raw)?
        } else {
            PersistedLedger {
                next_id: 1,
                records: Vec::new(),
            }
        };

        Ok(Self {
            game_dir: canon_game,
            backup_dir,
            ledger_path,
            next_id: persisted.next_id.max(1),
            records: persisted.records,
        })
    }

    /// Returns all currently tracked records (active, restored, or conflicted).
    pub fn records(&self) -> &[DeploymentRecord] {
        &self.records
    }

    /// Stages a verified file into `game_dir/relative_path`, snapshotting any pre-existing file.
    pub fn stage_file(
        &mut self,
        game_id: &str,
        package_id: &str,
        relative_path: &str,
        bytes: &[u8],
        expected_sha256: &str,
    ) -> Result<&DeploymentRecord, DeploymentError> {
        validate_relative_path(relative_path).map_err(|e| {
            DeploymentError::InvalidRelativePath(relative_path.to_owned(), e.to_string())
        })?;

        let expected_lower = expected_sha256.to_ascii_lowercase();
        if !is_valid_sha256_hex(&expected_lower) {
            return Err(DeploymentError::IntegrityMismatch {
                path: relative_path.to_owned(),
                expected: expected_sha256.to_owned(),
                actual: "invalid-sha256-format".to_owned(),
            });
        }

        let actual_sha = sha256_hex_bytes(bytes);
        if actual_sha != expected_lower {
            return Err(DeploymentError::IntegrityMismatch {
                path: relative_path.to_owned(),
                expected: expected_lower,
                actual: actual_sha,
            });
        }

        let target_path = self.resolve_contained_path(relative_path)?;
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let record_id = self.next_id;
        self.next_id += 1;

        let (backup_filename, original_sha256) = if target_path.exists() {
            let existing = fs::read(&target_path)?;
            let orig_sha = sha256_hex_bytes(&existing);
            let backup_name = format!("backup_{record_id}_{orig_sha}.bin");
            fs::write(self.backup_dir.join(&backup_name), &existing)?;
            (Some(backup_name), Some(orig_sha))
        } else {
            (None, None)
        };

        fs::write(&target_path, bytes)?;
        self.records.push(DeploymentRecord {
            record_id,
            game_id: game_id.to_owned(),
            package_id: package_id.to_owned(),
            relative_path: relative_path.to_owned(),
            deployed_sha256: actual_sha,
            backup_filename,
            original_sha256,
            state: DeploymentState::Deployed,
        });
        self.persist()?;

        Ok(self.records.last().expect("just pushed"))
    }

    /// Cleanly removes all deployed files for `package_id` in LIFO order without overwriting
    /// files externally modified after staging.
    pub fn undeploy_package(
        &mut self,
        package_id: &str,
    ) -> Result<UndeployReport, DeploymentError> {
        self.undeploy_filtered(|r| r.package_id == package_id)
    }

    /// Cleanly removes all active managed deployments in LIFO order.
    pub fn undeploy_all(&mut self) -> Result<UndeployReport, DeploymentError> {
        self.undeploy_filtered(|_| true)
    }

    fn undeploy_filtered(
        &mut self,
        predicate: impl Fn(&DeploymentRecord) -> bool,
    ) -> Result<UndeployReport, DeploymentError> {
        let mut report = UndeployReport::default();

        for idx in (0..self.records.len()).rev() {
            if self.records[idx].state != DeploymentState::Deployed
                || !predicate(&self.records[idx])
            {
                continue;
            }

            let rel = self.records[idx].relative_path.clone();
            let deployed_sha = self.records[idx].deployed_sha256.clone();
            let backup_file = self.records[idx].backup_filename.clone();
            let orig_sha = self.records[idx].original_sha256.clone();

            let target_path = self.resolve_contained_path(&rel)?;
            if !target_path.exists() {
                self.records[idx].state = DeploymentState::Restored;
                report.already_absent += 1;
                continue;
            }

            let current = fs::read(&target_path)?;
            let current_sha = sha256_hex_bytes(&current);
            if current_sha != deployed_sha {
                self.records[idx].state = DeploymentState::Conflict;
                report.conflicts += 1;
                report.conflict_paths.push(rel);
                continue;
            }

            if let (Some(backup_name), Some(expected_orig)) = (backup_file, orig_sha) {
                let backup_path = self.backup_dir.join(&backup_name);
                let orig_bytes = fs::read(&backup_path)?;
                let actual_orig = sha256_hex_bytes(&orig_bytes);
                if actual_orig != expected_orig {
                    return Err(DeploymentError::IntegrityMismatch {
                        path: backup_path.display().to_string(),
                        expected: expected_orig,
                        actual: actual_orig,
                    });
                }
                fs::write(&target_path, &orig_bytes)?;
                let _ = fs::remove_file(&backup_path);
                report.restored_originals += 1;
            } else {
                fs::remove_file(&target_path)?;
            }

            self.records[idx].state = DeploymentState::Restored;
            report.removed_count += 1;
        }

        self.persist()?;
        Ok(report)
    }

    fn resolve_contained_path(&self, relative_path: &str) -> Result<PathBuf, DeploymentError> {
        let joined = self.game_dir.join(relative_path);
        // Walk up to the deepest existing ancestor to ensure no symlink escapes `game_dir`.
        let mut cursor = joined.as_path();
        while !cursor.exists() {
            match cursor.parent() {
                Some(parent) => cursor = parent,
                None => {
                    return Err(DeploymentError::InvalidRelativePath(
                        relative_path.to_owned(),
                        "could not resolve parent directory".to_owned(),
                    ));
                }
            }
        }
        let canon_ancestor = fs::canonicalize(cursor)?;
        if !canon_ancestor.starts_with(&self.game_dir) {
            return Err(DeploymentError::InvalidRelativePath(
                relative_path.to_owned(),
                "resolved path escapes game installation root".to_owned(),
            ));
        }
        Ok(joined)
    }

    fn persist(&self) -> Result<(), DeploymentError> {
        let data = serde_json::to_string_pretty(&PersistedLedger {
            next_id: self.next_id,
            records: self.records.clone(),
        })?;
        let tmp_path = self.ledger_path.with_extension("json.tmp");
        fs::write(&tmp_path, data)?;
        fs::rename(&tmp_path, &self.ledger_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nekod_deploy_{name}_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn stages_backs_up_persists_across_reopen_and_restores_cleanly() {
        let root = temp_root("ok");
        let game_dir = root.join("Game");
        let backup_dir = root.join("backups");
        let ledger_path = root.join("deployment_ledger.json");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("winhttp.dll"), b"ORIGINAL_WINHTTP").unwrap();

        let bridge_bytes = b"NEKOD_UNITY_BRIDGE_DLL";
        let bridge_sha = sha256_hex_bytes(bridge_bytes);
        let cfg_bytes = b"{ \"bridge\": true }";
        let cfg_sha = sha256_hex_bytes(cfg_bytes);

        {
            let mut ledger = DeploymentLedger::open(&game_dir, &backup_dir, &ledger_path).unwrap();
            ledger
                .stage_file(
                    "sample-unity-game",
                    "org.nekod.unity-bridge",
                    "winhttp.dll",
                    bridge_bytes,
                    &bridge_sha,
                )
                .unwrap();
            ledger
                .stage_file(
                    "sample-unity-game",
                    "org.nekod.unity-bridge",
                    "BepInEx/config/nekod.json",
                    cfg_bytes,
                    &cfg_sha,
                )
                .unwrap();
            assert_eq!(
                fs::read(game_dir.join("winhttp.dll")).unwrap(),
                bridge_bytes
            );
            assert!(game_dir.join("BepInEx/config/nekod.json").is_file());
        }

        // Simulate crash/restart and recover cleanly from persisted ledger file.
        let mut reopened = DeploymentLedger::open(&game_dir, &backup_dir, &ledger_path).unwrap();
        let report = reopened.undeploy_package("org.nekod.unity-bridge").unwrap();
        assert_eq!(report.removed_count, 2);
        assert_eq!(report.restored_originals, 1);
        assert_eq!(report.conflicts, 0);
        assert_eq!(
            fs::read(game_dir.join("winhttp.dll")).unwrap(),
            b"ORIGINAL_WINHTTP"
        );
        assert!(!game_dir.join("BepInEx/config/nekod.json").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_traversal_hash_mismatch_and_detects_external_overwrite_conflict() {
        let root = temp_root("conflict");
        let game_dir = root.join("Game");
        let backup_dir = root.join("backups");
        let ledger_path = root.join("ledger.json");
        fs::create_dir_all(&game_dir).unwrap();

        let payload = b"PROXY_DLL_V1";
        let sha = sha256_hex_bytes(payload);

        let mut ledger = DeploymentLedger::open(&game_dir, &backup_dir, &ledger_path).unwrap();
        assert!(
            ledger
                .stage_file("g", "pkg", "../escape.dll", payload, &sha)
                .is_err()
        );
        assert!(
            ledger
                .stage_file("g", "pkg", "dinput8.dll", payload, &"0".repeat(64))
                .is_err()
        );

        ledger
            .stage_file("g", "pkg", "dinput8.dll", payload, &sha)
            .unwrap();
        // Simulate Steam game update overwriting dinput8.dll externally.
        fs::write(game_dir.join("dinput8.dll"), b"STEAM_UPDATED_FILE").unwrap();

        let report = ledger.undeploy_all().unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.removed_count, 0);
        assert_eq!(report.conflict_paths, vec!["dinput8.dll"]);
        assert_eq!(
            fs::read(game_dir.join("dinput8.dll")).unwrap(),
            b"STEAM_UPDATED_FILE"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
