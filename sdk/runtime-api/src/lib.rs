//! L4 Runtime API boundary separating Application Core (L2) from Trainer Runtime (L3).
//!
//! This crate intentionally contains no UI, networking, or OS-specific process handles.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nekod_manifest::{
    BinaryArch, PermissionConstraints, PermissionScope, RuntimeDependencySpec, RuntimeKind,
    TargetPlatform, TrainerManifest, TrustTier,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Opaque runtime session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeSessionId(pub String);

impl std::fmt::Display for RuntimeSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Immutable permission grant snapshot issued by `permission-manager` for a single session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantedPermissions {
    pub trainer_id: String,
    pub trainer_version: String,
    pub manifest_sha256: String,
    pub trust_tier: TrustTier,
    pub granted_scopes: BTreeSet<PermissionScope>,
    /// Optional per-scope bounds (e.g. `max_write_bytes`, `allowed_modules`) enforced by L3.
    #[serde(default)]
    pub scope_constraints: BTreeMap<PermissionScope, PermissionConstraints>,
}

impl GrantedPermissions {
    /// Checks whether a given capability scope was explicitly granted for this session.
    pub fn allows(&self, scope: PermissionScope) -> bool {
        self.granted_scopes.contains(&scope)
    }

    /// Returns constraints associated with a granted scope, if any.
    pub fn constraints_for(&self, scope: PermissionScope) -> Option<&PermissionConstraints> {
        if self.allows(scope) {
            self.scope_constraints.get(&scope)
        } else {
            None
        }
    }
}

/// Verified identity of a running target game process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub process_name: String,
    pub executable_path: String,
    pub platform: TargetPlatform,
    pub architecture: BinaryArch,
    pub executable_sha256: Option<String>,
    pub detected_version: Option<String>,
    /// Windows `FILETIME` process creation timestamp in 100ns intervals (ADR-012)
    /// used to detect PID reuse across attach, patch, freeze, and rollback.
    #[serde(default)]
    pub process_creation_time_100ns: Option<u64>,
}

/// Request sent from `nekod-core` to start a controlled trainer session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartSessionRequest {
    pub trainer: TrainerManifest,
    pub target_process: ProcessIdentity,
    pub granted_permissions: GrantedPermissions,
    /// Optional raw `declarative_patch_v1` JSON read from the trainer's verified entrypoint.
    #[serde(default)]
    pub declarative_payload_json: Option<String>,
    /// Package-relative artifact paths mapped to canonical SHA-256-verified on-disk file paths.
    #[serde(default)]
    pub verified_package_assets: BTreeMap<String, String>,
}

/// Player-controlled feature value crossing the L2 <-> L3 boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FeatureValue {
    Toggle(bool),
    Action,
    Integer(i64),
    Float(f64),
}

/// Command to apply, update, or roll back a single trainer feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureCommand {
    pub feature_id: String,
    pub value: FeatureValue,
}

/// Lifecycle status of a single `PatchRecord` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchActivationState {
    Active,
    RolledBack,
    RollbackConflict,
}

/// Recorded memory patch enabling deterministic LIFO rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchRecord {
    pub patch_id: u64,
    pub feature_id: String,
    pub target_pid: u32,
    #[serde(default)]
    pub process_creation_time_100ns: Option<u64>,
    pub address: u64,
    pub original_bytes: Vec<u8>,
    pub patched_bytes: Vec<u8>,
    pub activation_state: PatchActivationState,
    pub applied_at_epoch_ms: u64,
}

/// Lifecycle state of an active trainer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Initializing,
    Attached,
    Suspended,
    Stopped,
    Faulted,
}

/// Snapshot of runtime diagnostics and rollback ledger summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub session_id: RuntimeSessionId,
    pub runtime_kind: RuntimeKind,
    pub state: SessionLifecycleState,
    pub active_patches: usize,
    pub rollback_failures: usize,
    #[serde(default)]
    pub rollback_conflicts: usize,
    #[serde(default)]
    pub active_freeze_features: usize,
    #[serde(default)]
    pub target_pid: Option<u32>,
    #[serde(default)]
    pub target_process_name: Option<String>,
    pub recent_events: Vec<String>,
}

/// Errors returned across the L2 <-> L3 runtime boundary.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("permission denied: missing required scope '{0:?}'")]
    PermissionDenied(PermissionScope),

    #[error("permission constraint violation: {0}")]
    PermissionConstraintViolation(String),

    #[error("grant binding mismatch: {0}")]
    GrantMismatch(String),

    #[error("untrusted runtime kind '{0:?}' is disabled by default for trust tier '{1:?}'")]
    UntrustedRuntimeBlocked(RuntimeKind, TrustTier),

    #[error("anti-cheat conflict detected: {0}")]
    AntiCheatDetected(String),

    #[error("process identity mismatch: {0}")]
    ProcessIdentityMismatch(String),

    #[error(
        "target process requires higher privileges (ERROR_ACCESS_DENIED); Nekod does not auto-elevate"
    )]
    ProcessElevatedAccessDenied,

    #[error("unsupported platform '{0:?}' for trainer runtime")]
    UnsupportedPlatform(TargetPlatform),

    #[error("target process is not running or exited (pid {0})")]
    ProcessUnavailable(u32),

    #[error("unknown trainer feature '{0}'")]
    UnknownFeature(String),

    #[error("invalid declarative patch payload: {0}")]
    InvalidPayload(String),

    #[error("address resolution failed: {0}")]
    AddressResolutionFailed(String),

    #[error("signature verification mismatch: {0}")]
    SignatureMismatch(String),

    #[error("session '{0}' not found")]
    SessionNotFound(RuntimeSessionId),

    #[error("freeze loop error: {0}")]
    FreezeError(String),

    #[error("patch rollback failed for feature '{feature_id}': {reason}")]
    RollbackFailed { feature_id: String, reason: String },

    #[error("runtime execution error: {0}")]
    ExecutionFailed(String),

    #[error("runtime operation not implemented in current milestone: {0}")]
    NotImplemented(&'static str),
}

/// Architectural classification of an L3 Runtime Adapter (ADR-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterClass {
    DeclarativeCe,
    WasmSandbox,
    DynamicInstrumentation,
    UnityBridge,
    UnrealBridge,
    GameSpecificBridge,
}

/// Execution lifecycle model distinguishing session-based vs. disk-deployed adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLifecycleMode {
    MemorySession,
    HookSession,
    ManagedLoaderDeployment,
    ExternalProtocolIpc,
}

/// Descriptive capabilities & lifecycle contract exposed uniformly by every `RuntimeProvider`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdapterMetadata {
    pub adapter_id: String,
    pub adapter_class: RuntimeAdapterClass,
    pub runtime_kind: RuntimeKind,
    pub lifecycle_mode: RuntimeLifecycleMode,
    pub manages_disk_files: bool,
    pub requires_process_attach: bool,
}

/// Preflight resolution outcome for a declared [`RuntimeDependencySpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DependencyStatus {
    Satisfied { resolved_location: String },
    Missing { suggested_action: String },
    LicenseBlocked { reason: String },
}

/// Per-dependency inspection summary returned before attaching an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyCheckResult {
    pub dependency_id: String,
    pub optional: bool,
    pub outcome: DependencyStatus,
}

/// Stable trait implemented by L3 runtime adapters (`crates/trainer-runtime`).
pub trait RuntimeProvider: Send + Sync {
    /// Returns the runtime kind supported by this provider.
    fn kind(&self) -> RuntimeKind;

    /// Returns unified adapter lifecycle & capability metadata for L2/L6 callers.
    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: format!("{:?}", self.kind()),
            adapter_class: RuntimeAdapterClass::DeclarativeCe,
            runtime_kind: self.kind(),
            lifecycle_mode: RuntimeLifecycleMode::MemorySession,
            manages_disk_files: false,
            requires_process_attach: true,
        }
    }

    /// Evaluates declared external loader/protocol dependencies against `game_install_dir`.
    fn check_dependencies(
        &self,
        dependencies: &[RuntimeDependencySpec],
        _game_install_dir: Option<&Path>,
    ) -> Vec<DependencyCheckResult> {
        dependencies
            .iter()
            .map(|dep| DependencyCheckResult {
                dependency_id: dep.dependency_id.clone(),
                optional: dep.optional,
                outcome: DependencyStatus::Missing {
                    suggested_action: format!(
                        "Dependency '{}' ({}) requires manual verification or upstream fetch",
                        dep.dependency_id, dep.version_req
                    ),
                },
            })
            .collect()
    }

    /// Discovers running processes matching one of `expected_process_names`.
    /// Requires no privilege elevation; skips inaccessible protected processes.
    fn discover_matching_processes(
        &self,
        _expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Err(RuntimeError::NotImplemented("process discovery"))
    }

    /// Verifies permissions & process identity and starts a trainer session.
    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError>;

    /// Applies or updates a feature toggle/value within an active session.
    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError>;

    /// Rolls back all active patches and terminates the trainer session.
    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError>;

    /// Reads current session diagnostics without mutating game memory.
    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_permissions_enforces_explicit_scopes() {
        let mut scopes = BTreeSet::new();
        scopes.insert(PermissionScope::ProcessDiscover);
        scopes.insert(PermissionScope::ProcessReadMemory);

        let mut constraints = BTreeMap::new();
        constraints.insert(
            PermissionScope::ProcessWriteMemory,
            PermissionConstraints {
                max_write_bytes: Some(32),
                allowed_modules: None,
                path_scope: None,
            },
        );

        let grants = GrantedPermissions {
            trainer_id: "dev.example.sample".to_owned(),
            trainer_version: "1.0.0".to_owned(),
            manifest_sha256: "a".repeat(64),
            trust_tier: TrustTier::CommunityVerified,
            granted_scopes: scopes,
            scope_constraints: constraints,
        };

        assert!(grants.allows(PermissionScope::ProcessReadMemory));
        assert!(!grants.allows(PermissionScope::ProcessWriteMemory));
        assert!(
            grants
                .constraints_for(PermissionScope::ProcessWriteMemory)
                .is_none()
        );
        assert!(!grants.allows(PermissionScope::RuntimeNativeModule));
    }
}
