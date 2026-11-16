//! Multi-dimensional Compatibility Database (`Game × Version × Store × Runtime Adapter × Mod`).
//!
//! Enforces ADR-019 provenance integrity: unverified AI-generated drafts cannot
//! self-assert `CompatibilityStatus::Verified` until passing automated runtime or
//! human-verified validation.

use std::path::Path;
use std::sync::Mutex;

use nekod_manifest::{CompatibilityStatus, StorePlatformTag};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::TrainerManagerError;

/// Provenance of a persisted compatibility test/verification record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityEvidence {
    AutomatedRuntimeTest,
    HumanVerified,
    CommunityReport,
    AiDraftUnverified,
}

impl CompatibilityEvidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::AutomatedRuntimeTest => "automated_runtime_test",
            Self::HumanVerified => "human_verified",
            Self::CommunityReport => "community_report",
            Self::AiDraftUnverified => "ai_draft_unverified",
        }
    }

    fn from_str(raw: &str) -> Result<Self, TrainerManagerError> {
        match raw {
            "automated_runtime_test" => Ok(Self::AutomatedRuntimeTest),
            "human_verified" => Ok(Self::HumanVerified),
            "community_report" => Ok(Self::CommunityReport),
            "ai_draft_unverified" => Ok(Self::AiDraftUnverified),
            other => Err(TrainerManagerError::Storage(format!(
                "unknown compatibility evidence '{other}'"
            ))),
        }
    }
}

/// Persisted compatibility observation across the five-dimensional platform matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityRecord {
    pub record_id: i64,
    pub game_id: String,
    pub game_version: String,
    pub store_platform: StorePlatformTag,
    pub runtime_adapter: String,
    pub runtime_version: Option<String>,
    pub mod_id: String,
    pub mod_version: String,
    pub status: CompatibilityStatus,
    pub evidence: CompatibilityEvidence,
    pub notes: Option<String>,
    pub recorded_at: i64,
}

/// Input payload for recording a compatibility tuple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCompatibilityRecord {
    pub game_id: String,
    pub game_version: String,
    pub store_platform: StorePlatformTag,
    pub runtime_adapter: String,
    pub runtime_version: Option<String>,
    pub mod_id: String,
    pub mod_version: String,
    pub status: CompatibilityStatus,
    pub evidence: CompatibilityEvidence,
    pub notes: Option<String>,
}

/// SQLite-backed Compatibility Matrix store.
pub struct CompatibilityDatabase {
    conn: Mutex<Connection>,
}

impl CompatibilityDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TrainerManagerError> {
        let conn =
            Connection::open(path).map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, TrainerManagerError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("compat db mutex poisoned".to_owned()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS compatibility_matrix (
                record_id INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id TEXT NOT NULL,
                game_version TEXT NOT NULL,
                store_platform TEXT NOT NULL,
                runtime_adapter TEXT NOT NULL,
                runtime_version TEXT,
                mod_id TEXT NOT NULL,
                mod_version TEXT NOT NULL,
                status TEXT NOT NULL,
                evidence TEXT NOT NULL,
                notes TEXT,
                recorded_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_compat_lookup
                ON compatibility_matrix(game_id, mod_id, game_version);
            ",
        )
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Persists a compatibility test entry while rejecting unverified AI self-promotion
    /// to `CompatibilityStatus::Verified` (ADR-019 anti-hallucination gate).
    pub fn record_entry(
        &self,
        entry: &NewCompatibilityRecord,
    ) -> Result<CompatibilityRecord, TrainerManagerError> {
        if entry.evidence == CompatibilityEvidence::AiDraftUnverified
            && entry.status == CompatibilityStatus::Verified
        {
            return Err(TrainerManagerError::InvalidManifest(
                "AI-generated drafts cannot assert status=verified before automated runtime or human verification (ADR-019)".to_owned(),
            ));
        }

        let store_str = serialize_enum(&entry.store_platform)?;
        let status_str = serialize_enum(&entry.status)?;
        let evidence_str = entry.evidence.as_str();

        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("compat db mutex poisoned".to_owned()))?;
        conn.execute(
            "
            INSERT INTO compatibility_matrix (
                game_id, game_version, store_platform, runtime_adapter, runtime_version,
                mod_id, mod_version, status, evidence, notes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ",
            params![
                entry.game_id,
                entry.game_version,
                store_str,
                entry.runtime_adapter,
                entry.runtime_version,
                entry.mod_id,
                entry.mod_version,
                status_str,
                evidence_str,
                entry.notes
            ],
        )
        .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let row_id = conn.last_insert_rowid();
        let mut stmt = conn
            .prepare(
                "
                SELECT record_id, game_id, game_version, store_platform, runtime_adapter,
                       runtime_version, mod_id, mod_version, status, evidence, notes, recorded_at
                FROM compatibility_matrix WHERE record_id = ?1
                ",
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
        stmt.query_row(params![row_id], map_compat_row)
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))
    }

    /// Queries compatibility entries for `game_id` (optionally narrowed by `mod_id` and `game_version`).
    pub fn query_entries(
        &self,
        game_id: &str,
        mod_id: Option<&str>,
        game_version: Option<&str>,
    ) -> Result<Vec<CompatibilityRecord>, TrainerManagerError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| TrainerManagerError::Storage("compat db mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare(
                "
                SELECT record_id, game_id, game_version, store_platform, runtime_adapter,
                       runtime_version, mod_id, mod_version, status, evidence, notes, recorded_at
                FROM compatibility_matrix
                WHERE game_id = ?1
                  AND (?2 IS NULL OR mod_id = ?2)
                  AND (?3 IS NULL OR game_version = ?3 OR game_version = '*')
                ORDER BY record_id DESC
                ",
            )
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(params![game_id, mod_id, game_version], map_compat_row)
            .map_err(|e| TrainerManagerError::Storage(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| TrainerManagerError::Storage(e.to_string()))?);
        }
        Ok(out)
    }
}

fn serialize_enum<T: Serialize>(value: &T) -> Result<String, TrainerManagerError> {
    let json =
        serde_json::to_value(value).map_err(|e| TrainerManagerError::Storage(e.to_string()))?;
    json.as_str().map(ToOwned::to_owned).ok_or_else(|| {
        TrainerManagerError::Storage("expected string enum serialization".to_owned())
    })
}

fn map_compat_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompatibilityRecord> {
    let store_raw: String = row.get(3)?;
    let status_raw: String = row.get(8)?;
    let evidence_raw: String = row.get(9)?;

    let store_platform: StorePlatformTag =
        serde_json::from_value(serde_json::Value::String(store_raw))
            .unwrap_or(StorePlatformTag::Any);
    let status: CompatibilityStatus = serde_json::from_value(serde_json::Value::String(status_raw))
        .unwrap_or(CompatibilityStatus::Untested);
    let evidence = CompatibilityEvidence::from_str(&evidence_raw)
        .unwrap_or(CompatibilityEvidence::CommunityReport);

    Ok(CompatibilityRecord {
        record_id: row.get(0)?,
        game_id: row.get(1)?,
        game_version: row.get(2)?,
        store_platform,
        runtime_adapter: row.get(4)?,
        runtime_version: row.get(5)?,
        mod_id: row.get(6)?,
        mod_version: row.get(7)?,
        status,
        evidence,
        notes: row.get(10)?,
        recorded_at: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unverified_ai_verified_claim_and_queries_matrix() {
        let db = CompatibilityDatabase::open_in_memory().unwrap();
        let bad = NewCompatibilityRecord {
            game_id: "hollow-knight".to_owned(),
            game_version: "1.5.78.11833".to_owned(),
            store_platform: StorePlatformTag::Gog,
            runtime_adapter: "unity_bridge_v1".to_owned(),
            runtime_version: Some("5.4.22".to_owned()),
            mod_id: "org.nekod.hk-qol".to_owned(),
            mod_version: "1.0.0".to_owned(),
            status: CompatibilityStatus::Verified,
            evidence: CompatibilityEvidence::AiDraftUnverified,
            notes: Some("AI hallucinated verified claim".to_owned()),
        };
        assert!(db.record_entry(&bad).is_err());

        let mut verified = bad;
        verified.evidence = CompatibilityEvidence::AutomatedRuntimeTest;
        let saved = db.record_entry(&verified).unwrap();
        assert_eq!(saved.status, CompatibilityStatus::Verified);

        let queried = db
            .query_entries(
                "hollow-knight",
                Some("org.nekod.hk-qol"),
                Some("1.5.78.11833"),
            )
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].store_platform, StorePlatformTag::Gog);
    }
}
