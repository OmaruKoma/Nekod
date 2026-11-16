//! L2 Permission Manager implementing Nekod's default-deny capability policy
//! and SQLite grant persistence bound to `(trainer_id, version, manifest_sha256)`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use nekod_manifest::{
    ManifestError, PermissionDeclaration, PermissionScope, RuntimeKind, TrainerManifest, TrustTier,
    is_valid_sha256_hex,
};
use nekod_runtime_api::GrantedPermissions;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Global security policy settings configured by the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecurityPolicyConfig {
    /// Disabled by default: third-party native modules are treated as untrusted.
    pub allow_third_party_native_modules: bool,
    /// Disabled by default: requires explicit user opt-in for unverified local/repo trainers.
    pub allow_unverified_sources: bool,
}

/// Result of evaluating a trainer manifest against security policy and user grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationOutcome {
    /// All required permissions are approved and bound to this manifest digest.
    Approved(GrantedPermissions),
    /// User confirmation is required for one or more elevated scopes before execution.
    RequiresConfirmation {
        missing_confirmations: Vec<PermissionDeclaration>,
    },
}

/// Errors produced when evaluating or granting trainer permissions.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PermissionError {
    #[error("manifest validation failed: {0}")]
    InvalidManifest(#[from] ManifestError),

    #[error("execution blocked by policy: {0}")]
    PolicyBlocked(String),

    #[error("grant storage error: {0}")]
    Storage(String),
}

/// Persistent SQLite grant store binding approvals strictly to `(trainer_id, version, manifest_sha256)`.
pub struct SqliteGrantStore {
    conn: Mutex<Connection>,
}

impl SqliteGrantStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PermissionError> {
        let conn = Connection::open(path).map_err(|e| PermissionError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, PermissionError> {
        let conn =
            Connection::open_in_memory().map_err(|e| PermissionError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), PermissionError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| PermissionError::Storage("grant db mutex poisoned".to_owned()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS permission_grants (
                trainer_id TEXT NOT NULL,
                trainer_version TEXT NOT NULL,
                manifest_sha256 TEXT NOT NULL,
                confirmed_scopes_json TEXT NOT NULL,
                granted_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (trainer_id, trainer_version, manifest_sha256)
            );
            ",
        )
        .map_err(|e| PermissionError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Loads previously confirmed scopes only when `(trainer_id, version, manifest_sha256)` all match.
    pub fn load_confirmed_scopes(
        &self,
        trainer_id: &str,
        trainer_version: &str,
        manifest_sha256: &str,
    ) -> Result<BTreeSet<PermissionScope>, PermissionError> {
        if !is_valid_sha256_hex(manifest_sha256) {
            return Err(PermissionError::InvalidManifest(
                ManifestError::InvalidSha256(manifest_sha256.to_owned()),
            ));
        }

        let conn = self
            .conn
            .lock()
            .map_err(|_| PermissionError::Storage("grant db mutex poisoned".to_owned()))?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT confirmed_scopes_json FROM permission_grants
                 WHERE trainer_id = ?1 AND trainer_version = ?2 AND manifest_sha256 = ?3",
                params![trainer_id, trainer_version, manifest_sha256],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| PermissionError::Storage(e.to_string()))?;

        match raw {
            Some(json) => {
                serde_json::from_str(&json).map_err(|e| PermissionError::Storage(e.to_string()))
            }
            None => Ok(BTreeSet::new()),
        }
    }

    /// Persists confirmed scopes bound to `(trainer_id, version, manifest_sha256)`.
    pub fn save_confirmed_scopes(
        &self,
        trainer_id: &str,
        trainer_version: &str,
        manifest_sha256: &str,
        scopes: &BTreeSet<PermissionScope>,
    ) -> Result<(), PermissionError> {
        if !is_valid_sha256_hex(manifest_sha256) {
            return Err(PermissionError::InvalidManifest(
                ManifestError::InvalidSha256(manifest_sha256.to_owned()),
            ));
        }
        let json =
            serde_json::to_string(scopes).map_err(|e| PermissionError::Storage(e.to_string()))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| PermissionError::Storage("grant db mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO permission_grants (trainer_id, trainer_version, manifest_sha256, confirmed_scopes_json, granted_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(trainer_id, trainer_version, manifest_sha256) DO UPDATE SET
                confirmed_scopes_json = excluded.confirmed_scopes_json,
                granted_at = unixepoch()",
            params![trainer_id, trainer_version, manifest_sha256, json],
        )
        .map_err(|e| PermissionError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Revokes all stored grants for a trainer (e.g., on user revocation or security block).
    pub fn revoke_trainer_grants(&self, trainer_id: &str) -> Result<(), PermissionError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| PermissionError::Storage("grant db mutex poisoned".to_owned()))?;
        conn.execute(
            "DELETE FROM permission_grants WHERE trainer_id = ?1",
            params![trainer_id],
        )
        .map_err(|e| PermissionError::Storage(e.to_string()))?;
        Ok(())
    }
}

/// Stateless policy evaluator enforcing default-deny rules.
#[derive(Debug, Default)]
pub struct PermissionEvaluator;

impl PermissionEvaluator {
    /// Evaluates whether a trainer may execute under `policy` given `user_confirmed_scopes`.
    pub fn evaluate(
        manifest: &TrainerManifest,
        manifest_sha256: &str,
        trust_tier: TrustTier,
        policy: &SecurityPolicyConfig,
        user_confirmed_scopes: &BTreeSet<PermissionScope>,
    ) -> Result<EvaluationOutcome, PermissionError> {
        if !is_valid_sha256_hex(manifest_sha256) {
            return Err(PermissionError::InvalidManifest(
                ManifestError::InvalidSha256(manifest_sha256.to_owned()),
            ));
        }

        manifest.validate_semantic()?;

        if trust_tier == TrustTier::Unverified && !policy.allow_unverified_sources {
            return Err(PermissionError::PolicyBlocked(
                "unverified trainer sources are disabled by default policy".to_owned(),
            ));
        }

        let uses_native = manifest.runtime.kind == RuntimeKind::NativeProviderV1
            || manifest.has_permission(PermissionScope::RuntimeNativeModule);

        if uses_native
            && trust_tier != TrustTier::Official
            && !policy.allow_third_party_native_modules
        {
            return Err(PermissionError::PolicyBlocked(format!(
                "native runtime module blocked by default for trust tier '{trust_tier:?}'"
            )));
        }

        let mut missing = Vec::new();
        let mut granted = BTreeSet::new();
        let mut scope_constraints = BTreeMap::new();

        for decl in &manifest.permissions {
            if decl.scope.requires_explicit_confirmation()
                && !user_confirmed_scopes.contains(&decl.scope)
            {
                if decl.required {
                    missing.push(decl.clone());
                }
                continue;
            }
            granted.insert(decl.scope);
            if let Some(constraints) = &decl.constraints {
                scope_constraints.insert(decl.scope, constraints.clone());
            }
        }

        if !missing.is_empty() {
            return Ok(EvaluationOutcome::RequiresConfirmation {
                missing_confirmations: missing,
            });
        }

        Ok(EvaluationOutcome::Approved(GrantedPermissions {
            trainer_id: manifest.id.clone(),
            trainer_version: manifest.version.to_string(),
            manifest_sha256: manifest_sha256.to_owned(),
            trust_tier,
            granted_scopes: granted,
            scope_constraints,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trainer() -> TrainerManifest {
        let raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        serde_json::from_str(raw).expect("valid example trainer")
    }

    #[test]
    fn rejects_invalid_manifest_digest_for_grant_binding() {
        let trainer = sample_trainer();
        let res = PermissionEvaluator::evaluate(
            &trainer,
            "",
            TrustTier::Official,
            &SecurityPolicyConfig::default(),
            &BTreeSet::new(),
        );
        assert!(matches!(
            res,
            Err(PermissionError::InvalidManifest(
                ManifestError::InvalidSha256(_)
            ))
        ));
    }

    #[test]
    fn blocks_unverified_sources_by_default() {
        let trainer = sample_trainer();
        let res = PermissionEvaluator::evaluate(
            &trainer,
            &"a".repeat(64),
            TrustTier::Unverified,
            &SecurityPolicyConfig::default(),
            &BTreeSet::new(),
        );
        assert!(matches!(res, Err(PermissionError::PolicyBlocked(_))));
    }

    #[test]
    fn requires_explicit_confirmation_for_memory_write() {
        let trainer = sample_trainer();
        let res = PermissionEvaluator::evaluate(
            &trainer,
            &"a".repeat(64),
            TrustTier::CommunityVerified,
            &SecurityPolicyConfig::default(),
            &BTreeSet::new(),
        )
        .expect("evaluation succeeds with confirmation prompt");

        assert!(matches!(
            res,
            EvaluationOutcome::RequiresConfirmation { .. }
        ));
    }

    #[test]
    fn approves_after_explicit_confirmation_and_preserves_constraints() {
        let trainer = sample_trainer();
        let mut confirmed = BTreeSet::new();
        confirmed.insert(PermissionScope::ProcessReadMemory);
        confirmed.insert(PermissionScope::ProcessWriteMemory);

        let res = PermissionEvaluator::evaluate(
            &trainer,
            &"a".repeat(64),
            TrustTier::CommunityVerified,
            &SecurityPolicyConfig::default(),
            &confirmed,
        )
        .expect("evaluation succeeds");

        match res {
            EvaluationOutcome::Approved(grants) => {
                assert!(grants.allows(PermissionScope::ProcessDiscover));
                assert!(grants.allows(PermissionScope::ProcessWriteMemory));
                let write_bounds = grants
                    .constraints_for(PermissionScope::ProcessWriteMemory)
                    .expect("constraints preserved");
                assert_eq!(write_bounds.max_write_bytes, Some(64));
            }
            _ => panic!("expected approval"),
        }
    }

    #[test]
    fn blocks_third_party_native_modules_by_default() {
        let mut trainer = sample_trainer();
        trainer.runtime.kind = RuntimeKind::NativeProviderV1;
        trainer.permissions.push(PermissionDeclaration {
            scope: PermissionScope::RuntimeNativeModule,
            reason: "Load native helper DLL".to_owned(),
            required: true,
            constraints: None,
        });

        let mut confirmed = BTreeSet::new();
        confirmed.insert(PermissionScope::ProcessReadMemory);
        confirmed.insert(PermissionScope::ProcessWriteMemory);
        confirmed.insert(PermissionScope::RuntimeNativeModule);

        let res = PermissionEvaluator::evaluate(
            &trainer,
            &"a".repeat(64),
            TrustTier::CommunityVerified,
            &SecurityPolicyConfig::default(),
            &confirmed,
        );
        assert!(matches!(res, Err(PermissionError::PolicyBlocked(_))));
    }

    #[test]
    fn sqlite_grant_store_invalidates_on_version_or_hash_change() {
        let store = SqliteGrantStore::open_in_memory().unwrap();
        let mut scopes = BTreeSet::new();
        scopes.insert(PermissionScope::ProcessReadMemory);
        scopes.insert(PermissionScope::ProcessWriteMemory);

        let hash1 = "a".repeat(64);
        let hash2 = "b".repeat(64);

        store
            .save_confirmed_scopes("dev.example.sample", "1.0.0", &hash1, &scopes)
            .unwrap();

        assert_eq!(
            store
                .load_confirmed_scopes("dev.example.sample", "1.0.0", &hash1)
                .unwrap(),
            scopes
        );
        // Modified manifest hash requires fresh authorization.
        assert!(
            store
                .load_confirmed_scopes("dev.example.sample", "1.0.0", &hash2)
                .unwrap()
                .is_empty()
        );
        // New version requires fresh authorization.
        assert!(
            store
                .load_confirmed_scopes("dev.example.sample", "1.0.1", &hash1)
                .unwrap()
                .is_empty()
        );

        store.revoke_trainer_grants("dev.example.sample").unwrap();
        assert!(
            store
                .load_confirmed_scopes("dev.example.sample", "1.0.0", &hash1)
                .unwrap()
                .is_empty()
        );
    }
}
