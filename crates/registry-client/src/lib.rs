//! L2 Registry Client for `Nekod-Community` and independent GitHub repositories.
//!
//! Provides SQLite repository/index caching, conditional HTTP (`ETag` / `Last-Modified`),
//! authoritative `TrustTier` clamping, revocation propagation, and fail-closed trainer
//! release staging (ADR-014, ADR-015, ADR-016).

pub mod transport;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nekod_manifest::{
    RegistryManifest, RegistryTrainerEntry, RegistryTrainerStatus, TrustTier,
    is_valid_namespaced_id, validate_https_url, validate_relative_path,
};
use nekod_validator::{
    sha256_hex_bytes, validate_registry_manifest_str, validate_trainer_manifest_str,
    validate_trainer_package_dir,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

pub use transport::{
    HttpFetchResponse, HttpTransport, MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES, ReqwestHttpTransport,
};

/// Configured registry source tracked by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySource {
    pub repository_id: String,
    pub index_url: String,
    /// Authoritative trust tier assigned by client root configuration, never self-asserted.
    pub authoritative_trust_tier: TrustTier,
    pub enabled: bool,
}

/// Persisted registry source metadata paired with last sync diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySourceState {
    pub source: RegistrySource,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_synced_at: Option<i64>,
    pub last_status: String,
    pub last_error: Option<String>,
}

/// Cached trainer index record paired with authoritative source trust tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedTrainerRecord {
    pub repository_id: String,
    pub effective_trust_tier: TrustTier,
    pub entry: RegistryTrainerEntry,
}

/// Summary returned after syncing enabled registry feeds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySyncReport {
    pub sources_checked: usize,
    pub updated: usize,
    pub not_modified: usize,
    pub failed: usize,
    pub revoked_trainer_ids: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryError {
    #[error("invalid registry or download URL: {0}")]
    InvalidUrl(String),

    #[error("registry source or asset '{0}' not found upstream (HTTP 404/410)")]
    UpstreamNotFound(String),

    #[error("rate limited fetching '{target}' (retry_after={retry_after_secs:?}s)")]
    RateLimited {
        target: String,
        retry_after_secs: Option<u64>,
    },

    #[error("download '{target}' exceeded maximum size limit ({limit_bytes} bytes)")]
    PayloadTooLarge { target: String, limit_bytes: usize },

    #[error("registry source '{0}' unreachable: {1}")]
    Transport(String, String),

    #[error("invalid registry manifest: {0}")]
    InvalidManifest(String),

    #[error("trainer release integrity error: {0}")]
    IntegrityMismatch(String),

    #[error("trainer '{0}' has been revoked by its repository index")]
    Revoked(String),

    #[error("registry storage error: {0}")]
    Storage(String),
}

/// Trait for fetching and validating remote `RegistryManifest` indexes.
pub trait RegistrySyncClient: Send + Sync {
    fn fetch_index(&self, source: &RegistrySource) -> Result<RegistryManifest, RegistryError>;
}

/// Clamps self-declared trust tier to the authoritative tier assigned to `source`.
pub fn resolve_effective_trust_tier(source_authority: TrustTier, declared: TrustTier) -> TrustTier {
    source_authority.min(declared)
}

/// SQLite-backed local registry index cache used for offline discovery and conditional sync.
pub struct SqliteRegistryCache {
    conn: Mutex<Connection>,
}

impl SqliteRegistryCache {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let conn = Connection::open(path).map_err(|e| RegistryError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, RegistryError> {
        let conn =
            Connection::open_in_memory().map_err(|e| RegistryError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS registry_sources (
                repository_id TEXT PRIMARY KEY,
                index_url TEXT NOT NULL,
                authoritative_trust_tier TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                etag TEXT,
                last_modified TEXT,
                last_synced_at INTEGER,
                last_status TEXT NOT NULL DEFAULT 'never_synced',
                last_error TEXT
            );

            CREATE TABLE IF NOT EXISTS registry_manifests (
                repository_id TEXT PRIMARY KEY,
                manifest_json TEXT NOT NULL,
                effective_trust_tier TEXT NOT NULL,
                cached_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            ",
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Registers or updates a configured registry feed (`validate_https_url` enforced).
    pub fn upsert_source(&self, source: &RegistrySource) -> Result<(), RegistryError> {
        if !is_valid_namespaced_id(&source.repository_id) {
            return Err(RegistryError::InvalidManifest(format!(
                "invalid repository_id '{}'",
                source.repository_id
            )));
        }
        validate_https_url(&source.index_url)
            .map_err(|e| RegistryError::InvalidUrl(e.to_string()))?;

        let tier_json = serde_json::to_string(&source.authoritative_trust_tier)
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO registry_sources (repository_id, index_url, authoritative_trust_tier, enabled)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repository_id) DO UPDATE SET
                index_url = excluded.index_url,
                authoritative_trust_tier = excluded.authoritative_trust_tier,
                enabled = excluded.enabled",
            params![
                source.repository_id,
                source.index_url,
                tier_json,
                i64::from(source.enabled)
            ],
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Removes a configured registry source and purges its cached manifest.
    pub fn remove_source(&self, repository_id: &str) -> Result<(), RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        conn.execute(
            "DELETE FROM registry_sources WHERE repository_id = ?1",
            params![repository_id],
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;
        conn.execute(
            "DELETE FROM registry_manifests WHERE repository_id = ?1",
            params![repository_id],
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Lists all configured registry sources and their sync health.
    pub fn list_sources(&self) -> Result<Vec<RegistrySourceState>, RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare(
                "SELECT repository_id, index_url, authoritative_trust_tier, enabled, etag, last_modified, last_synced_at, last_status, last_error
                 FROM registry_sources ORDER BY repository_id ASC",
            )
            .map_err(|e| RegistryError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let mut list = Vec::new();
        for row in rows {
            let (
                repository_id,
                index_url,
                tier_json,
                enabled_i64,
                etag,
                last_modified,
                last_synced_at,
                last_status,
                last_error,
            ) = row.map_err(|e| RegistryError::Storage(e.to_string()))?;
            let authoritative_trust_tier: TrustTier = serde_json::from_str(&tier_json)
                .map_err(|e| RegistryError::Storage(e.to_string()))?;
            list.push(RegistrySourceState {
                source: RegistrySource {
                    repository_id,
                    index_url,
                    authoritative_trust_tier,
                    enabled: enabled_i64 != 0,
                },
                etag,
                last_modified,
                last_synced_at,
                last_status,
                last_error,
            });
        }
        Ok(list)
    }

    /// Validates and caches a `RegistryManifest` JSON document under `source`'s authoritative trust tier.
    pub fn ingest_manifest_str(
        &self,
        source: &RegistrySource,
        raw_json: &str,
    ) -> Result<RegistryManifest, RegistryError> {
        let mut manifest = validate_registry_manifest_str(raw_json)
            .map_err(|e| RegistryError::InvalidManifest(e.to_string()))?;

        if manifest.repository_id != source.repository_id {
            return Err(RegistryError::InvalidManifest(format!(
                "manifest repository_id '{}' does not match configured source '{}'",
                manifest.repository_id, source.repository_id
            )));
        }

        let effective_tier =
            resolve_effective_trust_tier(source.authoritative_trust_tier, manifest.trust_tier);
        manifest.trust_tier = effective_tier;

        let canonical =
            serde_json::to_string(&manifest).map_err(|e| RegistryError::Storage(e.to_string()))?;
        let tier_json = serde_json::to_string(&effective_tier)
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO registry_manifests (repository_id, manifest_json, effective_trust_tier, cached_at)
             VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT(repository_id) DO UPDATE SET
                manifest_json = excluded.manifest_json,
                effective_trust_tier = excluded.effective_trust_tier,
                cached_at = unixepoch()",
            params![manifest.repository_id, canonical, tier_json],
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;

        Ok(manifest)
    }

    /// Syncs a single registry feed with conditional HTTP (`ETag` / `Last-Modified`),
    /// preserving cached manifests if upstream returns `304`, `404`/`410`, or rate limits.
    pub fn sync_source(
        &self,
        source: &RegistrySource,
        transport: &dyn HttpTransport,
    ) -> Result<bool, RegistryError> {
        self.upsert_source(source)?;
        let (prev_etag, prev_lm) = self.load_cache_headers(&source.repository_id)?;

        match transport.fetch(
            &source.index_url,
            prev_etag.as_deref(),
            prev_lm.as_deref(),
            MAX_MANIFEST_BYTES,
        ) {
            Ok(HttpFetchResponse::NotModified) => {
                self.record_sync_status(
                    &source.repository_id,
                    "not_modified",
                    None,
                    prev_etag,
                    prev_lm,
                )?;
                info!(repo = %source.repository_id, "Registry feed unchanged (304 Not Modified)");
                Ok(false)
            }
            Ok(HttpFetchResponse::Ok {
                body,
                etag,
                last_modified,
            }) => {
                let raw_str = std::str::from_utf8(&body).map_err(|e| {
                    RegistryError::InvalidManifest(format!("manifest response is not UTF-8: {e}"))
                })?;
                if let Err(err) = self.ingest_manifest_str(source, raw_str) {
                    let msg = err.to_string();
                    self.record_sync_status(
                        &source.repository_id,
                        "invalid_manifest",
                        Some(&msg),
                        prev_etag,
                        prev_lm,
                    )?;
                    return Err(err);
                }
                self.record_sync_status(&source.repository_id, "ok", None, etag, last_modified)?;
                info!(repo = %source.repository_id, "Synced updated registry manifest");
                Ok(true)
            }
            Err(err) => {
                let status_label = match &err {
                    RegistryError::UpstreamNotFound(_) => "not_found",
                    RegistryError::RateLimited { .. } => "rate_limited",
                    _ => "error",
                };
                let msg = err.to_string();
                self.record_sync_status(
                    &source.repository_id,
                    status_label,
                    Some(&msg),
                    prev_etag,
                    prev_lm,
                )?;
                warn!(repo = %source.repository_id, error = %msg, "Registry sync failed; preserving cache");
                Err(err)
            }
        }
    }

    /// Syncs every enabled registry source independently, isolating individual failures.
    pub fn sync_all_enabled(
        &self,
        transport: &dyn HttpTransport,
    ) -> Result<RegistrySyncReport, RegistryError> {
        let sources = self.list_sources()?;
        let mut report = RegistrySyncReport::default();

        for st in sources {
            if !st.source.enabled {
                continue;
            }
            report.sources_checked = report.sources_checked.saturating_add(1);
            match self.sync_source(&st.source, transport) {
                Ok(true) => report.updated = report.updated.saturating_add(1),
                Ok(false) => report.not_modified = report.not_modified.saturating_add(1),
                Err(err) => {
                    report.failed = report.failed.saturating_add(1);
                    report
                        .errors
                        .push(format!("{}: {err}", st.source.repository_id));
                }
            }
        }

        report.revoked_trainer_ids = self.list_revoked_trainer_ids()?;
        Ok(report)
    }

    fn load_cache_headers(
        &self,
        repository_id: &str,
    ) -> Result<(Option<String>, Option<String>), RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        let row = conn
            .query_row(
                "SELECT etag, last_modified FROM registry_sources WHERE repository_id = ?1",
                params![repository_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| RegistryError::Storage(e.to_string()))?;
        Ok(row.unwrap_or((None, None)))
    }

    fn record_sync_status(
        &self,
        repository_id: &str,
        status: &str,
        error: Option<&str>,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> Result<(), RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        conn.execute(
            "UPDATE registry_sources
             SET last_status = ?2, last_error = ?3, etag = ?4, last_modified = ?5, last_synced_at = unixepoch()
             WHERE repository_id = ?1",
            params![repository_id, status, error, etag, last_modified],
        )
        .map_err(|e| RegistryError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Queries cached non-revoked trainer index records for a given canonical `game_id`.
    pub fn find_trainers_for_game(
        &self,
        game_id: &str,
    ) -> Result<Vec<IndexedTrainerRecord>, RegistryError> {
        Ok(self
            .list_all_indexed_trainers()?
            .into_iter()
            .filter(|rec| rec.entry.game_id == game_id)
            .collect())
    }

    /// Lists all cached non-revoked trainers across enabled sources.
    pub fn list_all_indexed_trainers(&self) -> Result<Vec<IndexedTrainerRecord>, RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare(
                "SELECT m.manifest_json, m.effective_trust_tier
                 FROM registry_manifests m
                 LEFT JOIN registry_sources s ON s.repository_id = m.repository_id
                 WHERE COALESCE(s.enabled, 1) = 1",
            )
            .map_err(|e| RegistryError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let (m_json, tier_json) = row.map_err(|e| RegistryError::Storage(e.to_string()))?;
            let manifest: RegistryManifest = serde_json::from_str(&m_json)
                .map_err(|e| RegistryError::InvalidManifest(e.to_string()))?;
            let effective_trust_tier: TrustTier = serde_json::from_str(&tier_json)
                .map_err(|e| RegistryError::Storage(e.to_string()))?;

            for entry in manifest.trainers {
                if entry.status != RegistryTrainerStatus::Revoked {
                    results.push(IndexedTrainerRecord {
                        repository_id: manifest.repository_id.clone(),
                        effective_trust_tier,
                        entry,
                    });
                }
            }
        }
        Ok(results)
    }

    /// Collects all unique `trainer_id` entries marked `revoked` in any cached manifest.
    pub fn list_revoked_trainer_ids(&self) -> Result<Vec<String>, RegistryError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| RegistryError::Storage("registry db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare("SELECT manifest_json FROM registry_manifests")
            .map_err(|e| RegistryError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let mut revoked = BTreeSet::new();
        for row in rows {
            let m_json = row.map_err(|e| RegistryError::Storage(e.to_string()))?;
            let manifest: RegistryManifest = serde_json::from_str(&m_json)
                .map_err(|e| RegistryError::InvalidManifest(e.to_string()))?;
            for entry in manifest.trainers {
                if entry.status == RegistryTrainerStatus::Revoked {
                    revoked.insert(entry.trainer_id);
                }
            }
        }
        Ok(revoked.into_iter().collect())
    }

    /// Looks up a cached non-revoked trainer entry by `(repository_id, trainer_id)`.
    pub fn find_entry(
        &self,
        repository_id: &str,
        trainer_id: &str,
    ) -> Result<Option<IndexedTrainerRecord>, RegistryError> {
        Ok(self
            .list_all_indexed_trainers()?
            .into_iter()
            .find(|r| r.repository_id == repository_id && r.entry.trainer_id == trainer_id))
    }
}

/// Downloads a trainer manifest and all declared artifacts into a verified temporary staging
/// folder inside `staging_root`, rejecting tampered digests, tag mutations, revocations,
/// or path traversal before returning the staged package directory path.
pub fn stage_verified_trainer_package(
    record: &IndexedTrainerRecord,
    staging_root: &Path,
    transport: &dyn HttpTransport,
) -> Result<PathBuf, RegistryError> {
    if record.entry.status == RegistryTrainerStatus::Revoked {
        return Err(RegistryError::Revoked(record.entry.trainer_id.clone()));
    }

    if matches!(
        record.effective_trust_tier,
        TrustTier::Official | TrustTier::CommunityVerified
    ) && record.entry.manifest_sha256.is_none()
    {
        return Err(RegistryError::IntegrityMismatch(format!(
            "verified repository entry '{}' must pin manifest_sha256",
            record.entry.trainer_id
        )));
    }

    let manifest_url = &record.entry.manifest_url;
    validate_https_url(manifest_url).map_err(|e| RegistryError::InvalidUrl(e.to_string()))?;
    let base_url = release_base_url(manifest_url)?;

    let manifest_bytes = match transport.fetch(manifest_url, None, None, MAX_MANIFEST_BYTES)? {
        HttpFetchResponse::Ok { body, .. } => body,
        HttpFetchResponse::NotModified => {
            return Err(RegistryError::Transport(
                manifest_url.clone(),
                "unexpected 304 Not Modified without validator".to_owned(),
            ));
        }
    };

    let actual_manifest_sha = sha256_hex_bytes(&manifest_bytes);
    if let Some(expected_sha) = &record.entry.manifest_sha256
        && actual_manifest_sha != *expected_sha
    {
        return Err(RegistryError::IntegrityMismatch(format!(
            "downloaded manifest SHA-256 ({actual_manifest_sha}) mismatches pinned registry digest ({expected_sha}); possible tag mutation"
        )));
    }

    let manifest_str = std::str::from_utf8(&manifest_bytes).map_err(|e| {
        RegistryError::InvalidManifest(format!("trainer manifest is not UTF-8: {e}"))
    })?;
    let manifest = validate_trainer_manifest_str(manifest_str)
        .map_err(|e| RegistryError::InvalidManifest(e.to_string()))?;

    if manifest.id != record.entry.trainer_id
        || manifest.target_game.game_id != record.entry.game_id
        || manifest.version != record.entry.latest_version
    {
        return Err(RegistryError::IntegrityMismatch(format!(
            "downloaded manifest ({}@{} for '{}') does not match registry entry ({}@{} for '{}')",
            manifest.id,
            manifest.version,
            manifest.target_game.game_id,
            record.entry.trainer_id,
            record.entry.latest_version,
            record.entry.game_id
        )));
    }

    fs::create_dir_all(staging_root).map_err(|e| RegistryError::Storage(e.to_string()))?;
    let temp_dir = staging_root.join(format!(
        "stage_{}_{}",
        sanitize_segment(&manifest.id),
        &actual_manifest_sha[..12]
    ));
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir).map_err(|e| RegistryError::Storage(e.to_string()))?;

    let result = (|| -> Result<(), RegistryError> {
        fs::write(temp_dir.join("trainer-manifest.json"), &manifest_bytes)
            .map_err(|e| RegistryError::Storage(e.to_string()))?;

        let canonical_stage =
            fs::canonicalize(&temp_dir).map_err(|e| RegistryError::Storage(e.to_string()))?;

        for artifact in &manifest.artifacts {
            validate_relative_path(&artifact.filename)
                .map_err(|e| RegistryError::InvalidManifest(e.to_string()))?;

            let artifact_url = format!("{base_url}{}", artifact.filename);
            validate_https_url(&artifact_url)
                .map_err(|e| RegistryError::InvalidUrl(e.to_string()))?;

            let body = match transport.fetch(&artifact_url, None, None, MAX_ARTIFACT_BYTES)? {
                HttpFetchResponse::Ok { body, .. } => body,
                HttpFetchResponse::NotModified => {
                    return Err(RegistryError::Transport(
                        artifact_url,
                        "unexpected 304 on uncached artifact download".to_owned(),
                    ));
                }
            };

            if body.len() as u64 != artifact.size_bytes {
                return Err(RegistryError::IntegrityMismatch(format!(
                    "artifact '{}' expected {} bytes, downloaded {} bytes",
                    artifact.filename,
                    artifact.size_bytes,
                    body.len()
                )));
            }

            let actual_sha = sha256_hex_bytes(&body);
            if actual_sha != artifact.sha256 {
                return Err(RegistryError::IntegrityMismatch(format!(
                    "artifact '{}' SHA-256 mismatch (expected {}, downloaded {actual_sha})",
                    artifact.filename, artifact.sha256
                )));
            }

            let dest = canonical_stage.join(&artifact.filename);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| RegistryError::Storage(e.to_string()))?;
            }
            fs::write(&dest, &body).map_err(|e| RegistryError::Storage(e.to_string()))?;
        }

        validate_trainer_package_dir(&temp_dir)
            .map_err(|e| RegistryError::IntegrityMismatch(e.to_string()))?;

        Ok(())
    })();

    if let Err(err) = result {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(err);
    }

    Ok(temp_dir)
}

fn release_base_url(manifest_url: &str) -> Result<String, RegistryError> {
    let idx = manifest_url.rfind('/').ok_or_else(|| {
        RegistryError::InvalidUrl("manifest_url missing release folder separator".to_owned())
    })?;
    Ok(manifest_url[..=idx].to_owned())
}

fn sanitize_segment(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use nekod_manifest::TrainerManifest;
    use transport::mock::{MockHttpReply, MockHttpTransport};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekod_registry_{prefix}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build_valid_release_assets() -> (String, Vec<u8>, Vec<u8>) {
        let payload_bytes =
            include_bytes!("../../../schemas/examples/declarative-patch.example.json").to_vec();
        let mut trainer: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        trainer.artifacts[0].sha256 = sha256_hex_bytes(&payload_bytes);
        trainer.artifacts[0].size_bytes = payload_bytes.len() as u64;
        let manifest_bytes = serde_json::to_vec_pretty(&trainer).unwrap();
        let manifest_sha = sha256_hex_bytes(&manifest_bytes);
        (manifest_sha, manifest_bytes, payload_bytes)
    }

    #[test]
    fn clamps_self_promoted_official_tier_to_source_authority() {
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json")
            .replace("\"community_verified\"", "\"official\"");
        let source = RegistrySource {
            repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
            index_url: "https://github.com/example-contributor/sample-rpg-trainer".to_owned(),
            authoritative_trust_tier: TrustTier::Unverified,
            enabled: true,
        };

        let cache = SqliteRegistryCache::open_in_memory().unwrap();
        let ingested = cache.ingest_manifest_str(&source, &raw).unwrap();
        assert_eq!(ingested.trust_tier, TrustTier::Unverified);

        let hits = cache
            .find_trainers_for_game("sample-singleplayer-rpg")
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].effective_trust_tier, TrustTier::Unverified);
    }

    #[test]
    fn excludes_revoked_trainers_from_discovery() {
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json")
            .replace("\"active\"", "\"revoked\"");
        let source = RegistrySource {
            repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
            index_url: "https://github.com/example-contributor/sample-rpg-trainer".to_owned(),
            authoritative_trust_tier: TrustTier::CommunityVerified,
            enabled: true,
        };

        let cache = SqliteRegistryCache::open_in_memory().unwrap();
        cache.ingest_manifest_str(&source, &raw).unwrap();
        let hits = cache
            .find_trainers_for_game("sample-singleplayer-rpg")
            .unwrap();
        assert!(hits.is_empty());
        assert_eq!(
            cache.list_revoked_trainer_ids().unwrap(),
            vec!["dev.example.sample-singleplayer-rpg".to_owned()]
        );
    }

    #[test]
    fn sync_uses_etag_304_and_preserves_cache_on_upstream_deletion() {
        let index_url = "https://raw.githubusercontent.com/example-contributor/sample-rpg-trainer/main/registry.json";
        let raw = include_str!("../../../schemas/examples/registry-manifest.example.json");
        let source = RegistrySource {
            repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
            index_url: index_url.to_owned(),
            authoritative_trust_tier: TrustTier::CommunityVerified,
            enabled: true,
        };

        let transport = MockHttpTransport::new();
        transport.enqueue(
            index_url,
            MockHttpReply::Ok {
                body: raw.as_bytes().to_vec(),
                etag: Some("\"v1-etag\"".to_owned()),
                last_modified: None,
            },
        );
        transport.enqueue(index_url, MockHttpReply::NotModified);
        transport.enqueue(
            index_url,
            MockHttpReply::Error(RegistryError::UpstreamNotFound(index_url.to_owned())),
        );

        let cache = SqliteRegistryCache::open_in_memory().unwrap();
        cache.upsert_source(&source).unwrap();

        // 1st sync -> 200 OK
        let r1 = cache.sync_all_enabled(&transport).unwrap();
        assert_eq!(r1.updated, 1);
        assert_eq!(r1.not_modified, 0);

        // 2nd sync -> sends If-None-Match: "v1-etag" and receives 304 Not Modified
        let r2 = cache.sync_all_enabled(&transport).unwrap();
        assert_eq!(r2.not_modified, 1);
        assert_eq!(
            transport.recorded_requests()[1],
            (index_url.to_owned(), Some("\"v1-etag\"".to_owned()))
        );

        // 3rd sync -> repo deleted upstream (404), cached trainers stay readable!
        let r3 = cache.sync_all_enabled(&transport).unwrap();
        assert_eq!(r3.failed, 1);
        assert_eq!(
            cache
                .find_trainers_for_game("sample-singleplayer-rpg")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn staging_downloads_verified_release_and_rejects_tag_mutation_or_tampered_artifact() {
        let (manifest_sha, manifest_bytes, payload_bytes) = build_valid_release_assets();
        let manifest_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer-manifest.json";
        let payload_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer.json";

        let raw_reg = include_str!("../../../schemas/examples/registry-manifest.example.json")
            .replace(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                &manifest_sha,
            );
        let source = RegistrySource {
            repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
            index_url: "https://raw.githubusercontent.com/example/reg.json".to_owned(),
            authoritative_trust_tier: TrustTier::CommunityVerified,
            enabled: true,
        };
        let cache = SqliteRegistryCache::open_in_memory().unwrap();
        cache.ingest_manifest_str(&source, &raw_reg).unwrap();
        let mut record = cache
            .find_entry(
                "com.github.example-contributor.sample-trainers",
                "dev.example.sample-singleplayer-rpg",
            )
            .unwrap()
            .unwrap();

        let transport = MockHttpTransport::new();
        transport.set_ok(manifest_url, manifest_bytes.clone(), None);
        transport.set_ok(payload_url, payload_bytes, None);

        let stage_root = temp_dir("stage_ok");
        let staged_dir = stage_verified_trainer_package(&record, &stage_root, &transport).unwrap();
        assert!(staged_dir.join("trainer.json").is_file());
        let _ = fs::remove_dir_all(&stage_root);

        // Mutated release artifact fails SHA-256 verification and leaves no half-staged dir
        let bad_transport = MockHttpTransport::new();
        bad_transport.set_ok(manifest_url, manifest_bytes, None);
        bad_transport.set_ok(payload_url, b"{\"tampered\":true}".to_vec(), None);
        let stage_bad = temp_dir("stage_bad");
        let err = stage_verified_trainer_package(&record, &stage_bad, &bad_transport).unwrap_err();
        assert!(matches!(err, RegistryError::IntegrityMismatch(_)));
        let _ = fs::remove_dir_all(&stage_bad);

        // Tag mutation (changed manifest bytes vs pinned manifest_sha256) fails closed
        record.entry.manifest_sha256 = Some("c".repeat(64));
        let err_mut = stage_verified_trainer_package(&record, &stage_root, &transport).unwrap_err();
        assert!(matches!(err_mut, RegistryError::IntegrityMismatch(_)));
    }
}
