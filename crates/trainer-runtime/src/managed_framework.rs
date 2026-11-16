//! Managed specialized game-framework bridge runtime (`managed_framework_v1`).
//!
//! Supports domain-specific mod framework ecosystems (`REFramework` for RE Engine,
//! `ScriptHookV` / `ScriptHookRDR2` for Rockstar titles) cleanly via either:
//! 1. Verified file staging inside strict framework subdirectories via `DeploymentLedger`, or
//! 2. Authenticated loopback JSON-line companion bridge IPC (`bridge_ipc.rs`),
//!    without forcing unrelated engine protocols into a single generic hook abstraction.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use nekod_manifest::{PermissionScope, RuntimeDependencySpec, RuntimeKind, is_valid_sha256_hex};
use nekod_runtime_api::{
    DependencyCheckResult, DependencyStatus, FeatureCommand, FeatureValue, ProcessIdentity,
    RuntimeAdapterClass, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId, SessionLifecycleState,
    StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bridge_ipc::{BridgeRpcRequest, send_bridge_rpc, validate_loopback_bridge};
use crate::deployment::DeploymentLedger;
use crate::session::validate_session_preflight_for_kind;

const ALLOWED_FRAMEWORK_SCRIPT_EXTENSIONS: &[&str] = &[".lua", ".cs", ".json", ".ini", ".cfg"];

/// Specific game mod framework managed under `managed_framework_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedFrameworkKind {
    Reframework,
    ScripthookV,
    ScripthookRdr2,
}

impl ManagedFrameworkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reframework => "reframework",
            Self::ScripthookV => "scripthook_v",
            Self::ScripthookRdr2 => "scripthook_rdr2",
        }
    }

    fn allowed_subdirectories(self) -> &'static [&'static str] {
        match self {
            Self::Reframework => &["reframework/autorun/", "reframework/plugins/"],
            Self::ScripthookV | Self::ScripthookRdr2 => &["scripts/"],
        }
    }
}

/// Optional loopback companion IPC endpoint exposed by a framework script/plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkIpcConfig {
    pub host: String,
    pub port: u16,
    pub auth_token: String,
}

/// SHA-256-verified text/script asset staged via `DeploymentLedger`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedScriptAsset {
    pub relative_path: String,
    pub sha256: String,
    pub content: String,
}

/// Declarative feature binding inside `managed_framework_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedFrameworkFeature {
    StageScript {
        feature_id: String,
        script_relative_path: String,
    },
    IpcToggle {
        feature_id: String,
        enable_command: String,
        disable_command: String,
        #[serde(default)]
        params: Option<Value>,
    },
    IpcAction {
        feature_id: String,
        command: String,
        #[serde(default)]
        params: Option<Value>,
    },
}

impl ManagedFrameworkFeature {
    fn feature_id(&self) -> &str {
        match self {
            Self::StageScript { feature_id, .. }
            | Self::IpcToggle { feature_id, .. }
            | Self::IpcAction { feature_id, .. } => feature_id,
        }
    }
}

/// Entrypoint JSON payload for `managed_framework_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFrameworkPayload {
    pub schema_version: String,
    pub framework: ManagedFrameworkKind,
    #[serde(default)]
    pub game_install_dir: Option<String>,
    #[serde(default)]
    pub backup_dir: Option<String>,
    #[serde(default)]
    pub ledger_path: Option<String>,
    #[serde(default)]
    pub ipc: Option<FrameworkIpcConfig>,
    #[serde(default)]
    pub staged_scripts: Vec<ManagedScriptAsset>,
    pub features: Vec<ManagedFrameworkFeature>,
}

struct ManagedFrameworkSession {
    package_id: String,
    game_id: String,
    payload: ManagedFrameworkPayload,
    endpoint: Option<std::net::SocketAddr>,
    target_process: ProcessIdentity,
    active_staged_features: Vec<String>,
    ipc_rollback_stack: Vec<(String, BridgeRpcRequest)>,
    state: SessionLifecycleState,
    rollback_failures: usize,
    rollback_conflicts: usize,
    recent_events: Vec<String>,
}

/// Controlled specialized-framework adapter (`managed_framework_v1`: REFramework & ScriptHook).
pub struct ManagedFrameworkRuntime {
    next_id: AtomicU64,
    sessions: Mutex<BTreeMap<String, ManagedFrameworkSession>>,
    stopped_diagnostics: Mutex<BTreeMap<String, RuntimeDiagnostics>>,
}

impl Default for ManagedFrameworkRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ManagedFrameworkRuntime {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(BTreeMap::new()),
            stopped_diagnostics: Mutex::new(BTreeMap::new()),
        }
    }

    fn validate_script_path(
        framework: ManagedFrameworkKind,
        rel_path: &str,
    ) -> Result<(), RuntimeError> {
        let normalized = rel_path.replace('\\', "/");
        if normalized.contains("..") || normalized.starts_with('/') || normalized.contains(':') {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsafe managed script path '{rel_path}'"
            )));
        }
        let lower = normalized.to_ascii_lowercase();
        if !framework
            .allowed_subdirectories()
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        {
            return Err(RuntimeError::InvalidPayload(format!(
                "script '{rel_path}' outside allowed {} directories ({:?})",
                framework.as_str(),
                framework.allowed_subdirectories()
            )));
        }
        if !ALLOWED_FRAMEWORK_SCRIPT_EXTENSIONS
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            return Err(RuntimeError::InvalidPayload(format!(
                "managed script '{rel_path}' uses disallowed extension"
            )));
        }
        Ok(())
    }

    fn validate_payload(
        payload: &ManagedFrameworkPayload,
    ) -> Result<Option<std::net::SocketAddr>, RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported managed_framework_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "managed_framework_v1 requires at least one feature".to_owned(),
            ));
        }
        for asset in &payload.staged_scripts {
            Self::validate_script_path(payload.framework, &asset.relative_path)?;
            if !is_valid_sha256_hex(&asset.sha256) {
                return Err(RuntimeError::InvalidPayload(format!(
                    "managed script '{}' has invalid sha256",
                    asset.relative_path
                )));
            }
        }
        for feat in &payload.features {
            match feat {
                ManagedFrameworkFeature::StageScript {
                    script_relative_path,
                    ..
                } => {
                    if !payload
                        .staged_scripts
                        .iter()
                        .any(|s| s.relative_path == *script_relative_path)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "stage_script references undeclared asset '{script_relative_path}'"
                        )));
                    }
                    if payload.game_install_dir.is_none()
                        || payload.backup_dir.is_none()
                        || payload.ledger_path.is_none()
                    {
                        return Err(RuntimeError::InvalidPayload(
                            "stage_script features require game_install_dir, backup_dir, and ledger_path"
                                .to_owned(),
                        ));
                    }
                }
                ManagedFrameworkFeature::IpcToggle { .. }
                | ManagedFrameworkFeature::IpcAction { .. } => {
                    if payload.ipc.is_none() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "feature '{}' requires ipc configuration",
                            feat.feature_id()
                        )));
                    }
                }
            }
        }

        let endpoint = match &payload.ipc {
            Some(ipc) => Some(validate_loopback_bridge(
                &ipc.host,
                ipc.port,
                &ipc.auth_token,
            )?),
            None => None,
        };
        Ok(endpoint)
    }

    fn build_diagnostics(
        session_id: &RuntimeSessionId,
        session: &ManagedFrameworkSession,
    ) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            session_id: session_id.clone(),
            runtime_kind: RuntimeKind::ManagedFrameworkV1,
            state: session.state,
            active_patches: session.active_staged_features.len() + session.ipc_rollback_stack.len(),
            rollback_failures: session.rollback_failures,
            rollback_conflicts: session.rollback_conflicts,
            active_freeze_features: 0,
            target_pid: Some(session.target_process.pid),
            target_process_name: Some(session.target_process.process_name.clone()),
            recent_events: session.recent_events.clone(),
        }
    }
}

impl RuntimeProvider for ManagedFrameworkRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ManagedFrameworkV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "managed_framework_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::GameSpecificBridge,
            runtime_kind: RuntimeKind::ManagedFrameworkV1,
            lifecycle_mode: RuntimeLifecycleMode::ManagedLoaderDeployment,
            manages_disk_files: true,
            requires_process_attach: false,
        }
    }

    fn check_dependencies(
        &self,
        dependencies: &[RuntimeDependencySpec],
        game_install_dir: Option<&Path>,
    ) -> Vec<DependencyCheckResult> {
        dependencies
            .iter()
            .map(|dep| {
                let low = dep.dependency_id.to_ascii_lowercase();
                let satisfied = game_install_dir.and_then(|dir| {
                    if low.contains("reframework")
                        && dir.join("reframework").is_dir()
                        && dir.join("dinput8.dll").is_file()
                    {
                        Some(dir.join("dinput8.dll").display().to_string())
                    } else if (low.contains("scripthookv") || low == "scripthook-v")
                        && dir.join("ScriptHookV.dll").is_file()
                    {
                        Some(dir.join("ScriptHookV.dll").display().to_string())
                    } else if (low.contains("scripthookrdr2") || low == "scripthook-rdr2")
                        && dir.join("ScriptHookRDR2.dll").is_file()
                    {
                        Some(dir.join("ScriptHookRDR2.dll").display().to_string())
                    } else {
                        None
                    }
                });

                let outcome = match satisfied {
                    Some(resolved_location) => DependencyStatus::Satisfied { resolved_location },
                    None => DependencyStatus::Missing {
                        suggested_action: format!(
                            "Install '{}' ({}) into game directory via Dependency Manager",
                            dep.dependency_id, dep.version_req
                        ),
                    },
                };
                DependencyCheckResult {
                    dependency_id: dep.dependency_id.clone(),
                    optional: dep.optional,
                    outcome,
                }
            })
            .collect()
    }

    fn discover_matching_processes(
        &self,
        _expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Ok(Vec::new())
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        validate_session_preflight_for_kind(&req, RuntimeKind::ManagedFrameworkV1)?;

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload(
                "managed_framework_v1 requires entrypoint JSON payload in StartSessionRequest"
                    .to_owned(),
            )
        })?;
        let payload: ManagedFrameworkPayload = serde_json::from_str(raw).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid managed_framework_v1 JSON: {e}"))
        })?;
        let endpoint = Self::validate_payload(&payload)?;

        if !payload.staged_scripts.is_empty()
            && !req
                .granted_permissions
                .allows(PermissionScope::FilesystemWriteSaveDir)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::FilesystemWriteSaveDir,
            ));
        }

        if payload.ipc.is_some()
            && !req
                .granted_permissions
                .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        if let (Some(addr), Some(ipc)) = (endpoint, payload.ipc.as_ref()) {
            let ping = send_bridge_rpc(
                addr,
                &BridgeRpcRequest {
                    token: ipc.auth_token.clone(),
                    op: "ping".to_owned(),
                    params: json!({ "expected_framework": payload.framework.as_str() }),
                },
            )?;
            let reported = ping
                .data
                .get("framework")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if reported != payload.framework.as_str() {
                return Err(RuntimeError::SignatureMismatch(format!(
                    "managed framework mismatch: expected '{}', bridge reported '{reported}'",
                    payload.framework.as_str()
                )));
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sid = RuntimeSessionId(format!("framework-{id}"));
        let session = ManagedFrameworkSession {
            package_id: req.trainer.id.clone(),
            game_id: req.trainer.target_game.game_id.clone(),
            payload,
            endpoint,
            target_process: req.target_process.clone(),
            active_staged_features: Vec::new(),
            ipc_rollback_stack: Vec::new(),
            state: SessionLifecycleState::Attached,
            rollback_failures: 0,
            rollback_conflicts: 0,
            recent_events: vec![format!(
                "Attached managed_framework_v1 session for {}",
                req.target_process.process_name
            )],
        };
        self.sessions.lock().unwrap().insert(sid.0.clone(), session);
        Ok(sid)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let mut guard = self.sessions.lock().unwrap();
        let session = guard
            .get_mut(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;

        let feature = session
            .payload
            .features
            .iter()
            .find(|f| f.feature_id() == command.feature_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?;

        match feature {
            ManagedFrameworkFeature::StageScript {
                script_relative_path,
                ..
            } => {
                let asset = session
                    .payload
                    .staged_scripts
                    .iter()
                    .find(|a| a.relative_path == script_relative_path)
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::InvalidPayload(format!(
                            "missing staged asset '{script_relative_path}'"
                        ))
                    })?;

                let game_dir = session.payload.game_install_dir.as_deref().ok_or_else(|| {
                    RuntimeError::InvalidPayload("missing game_install_dir".into())
                })?;
                let backup_dir = session
                    .payload
                    .backup_dir
                    .as_deref()
                    .ok_or_else(|| RuntimeError::InvalidPayload("missing backup_dir".into()))?;
                let ledger_path =
                    session.payload.ledger_path.as_deref().ok_or_else(|| {
                        RuntimeError::InvalidPayload("missing ledger_path".into())
                    })?;

                let mut ledger = DeploymentLedger::open(game_dir, backup_dir, ledger_path)
                    .map_err(|e| {
                        RuntimeError::ExecutionFailed(format!("open DeploymentLedger failed: {e}"))
                    })?;

                match command.value {
                    FeatureValue::Toggle(true) => {
                        ledger
                            .stage_file(
                                &session.game_id,
                                &session.package_id,
                                &asset.relative_path,
                                asset.content.as_bytes(),
                                &asset.sha256,
                            )
                            .map_err(|e| {
                                RuntimeError::ExecutionFailed(format!(
                                    "stage managed script '{}' failed: {e}",
                                    asset.relative_path
                                ))
                            })?;
                        if !session.active_staged_features.contains(&command.feature_id) {
                            session
                                .active_staged_features
                                .push(command.feature_id.clone());
                        }
                        session.recent_events.push(format!(
                            "Staged verified framework script '{}'",
                            asset.relative_path
                        ));
                        Ok(())
                    }
                    FeatureValue::Toggle(false) => {
                        let report = ledger.undeploy_package(&session.package_id).map_err(|e| {
                            RuntimeError::ExecutionFailed(format!(
                                "undeploy managed script failed: {e}"
                            ))
                        })?;
                        session.rollback_conflicts += report.conflicts;
                        session
                            .active_staged_features
                            .retain(|f| f != &command.feature_id);
                        session.recent_events.push(format!(
                            "Undeployed framework script for '{}' (conflicts={})",
                            command.feature_id, report.conflicts
                        ));
                        Ok(())
                    }
                    _ => Err(RuntimeError::InvalidPayload(format!(
                        "stage_script '{}' requires Toggle(bool)",
                        command.feature_id
                    ))),
                }
            }
            ManagedFrameworkFeature::IpcToggle {
                enable_command,
                disable_command,
                params,
                ..
            } => {
                let endpoint = session.endpoint.ok_or_else(|| {
                    RuntimeError::InvalidPayload("missing IPC endpoint".to_owned())
                })?;
                let token = session
                    .payload
                    .ipc
                    .as_ref()
                    .map(|i| i.auth_token.clone())
                    .unwrap_or_default();
                let payload_params = params.unwrap_or_else(|| json!({}));

                match command.value {
                    FeatureValue::Toggle(true) => {
                        send_bridge_rpc(
                            endpoint,
                            &BridgeRpcRequest {
                                token: token.clone(),
                                op: enable_command.clone(),
                                params: payload_params.clone(),
                            },
                        )?;
                        session.ipc_rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token,
                                op: disable_command,
                                params: payload_params,
                            },
                        ));
                        session.recent_events.push(format!(
                            "Executed framework command '{}' for '{}'",
                            enable_command, command.feature_id
                        ));
                        Ok(())
                    }
                    FeatureValue::Toggle(false) => {
                        send_bridge_rpc(
                            endpoint,
                            &BridgeRpcRequest {
                                token,
                                op: disable_command.clone(),
                                params: payload_params,
                            },
                        )?;
                        if let Some(pos) = session
                            .ipc_rollback_stack
                            .iter()
                            .rposition(|(fid, _)| fid == &command.feature_id)
                        {
                            session.ipc_rollback_stack.remove(pos);
                        }
                        session.recent_events.push(format!(
                            "Disabled framework command '{}' for '{}'",
                            disable_command, command.feature_id
                        ));
                        Ok(())
                    }
                    _ => Err(RuntimeError::InvalidPayload(format!(
                        "ipc_toggle '{}' requires Toggle(bool)",
                        command.feature_id
                    ))),
                }
            }
            ManagedFrameworkFeature::IpcAction {
                command: rpc_op,
                params,
                ..
            } => {
                let endpoint = session.endpoint.ok_or_else(|| {
                    RuntimeError::InvalidPayload("missing IPC endpoint".to_owned())
                })?;
                let token = session
                    .payload
                    .ipc
                    .as_ref()
                    .map(|i| i.auth_token.clone())
                    .unwrap_or_default();
                match command.value {
                    FeatureValue::Action | FeatureValue::Toggle(true) => {
                        send_bridge_rpc(
                            endpoint,
                            &BridgeRpcRequest {
                                token,
                                op: rpc_op.clone(),
                                params: params.unwrap_or_else(|| json!({})),
                            },
                        )?;
                        session.recent_events.push(format!(
                            "Triggered framework action '{}' for '{}'",
                            rpc_op, command.feature_id
                        ));
                        Ok(())
                    }
                    _ => Err(RuntimeError::InvalidPayload(format!(
                        "ipc_action '{}' requires Action",
                        command.feature_id
                    ))),
                }
            }
        }
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let mut session = self
            .sessions
            .lock()
            .unwrap()
            .remove(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;

        if let Some(endpoint) = session.endpoint {
            while let Some((fid, undo_req)) = session.ipc_rollback_stack.pop() {
                if let Err(err) = send_bridge_rpc(endpoint, &undo_req) {
                    session.rollback_failures += 1;
                    session
                        .recent_events
                        .push(format!("Framework IPC rollback failed for '{fid}': {err}"));
                } else {
                    session
                        .recent_events
                        .push(format!("Rolled back framework IPC feature '{fid}'"));
                }
            }
        }

        if !session.active_staged_features.is_empty()
            && let (Some(game_dir), Some(backup_dir), Some(ledger_path)) = (
                session.payload.game_install_dir.as_deref(),
                session.payload.backup_dir.as_deref(),
                session.payload.ledger_path.as_deref(),
            )
        {
            match DeploymentLedger::open(game_dir, backup_dir, ledger_path)
                .and_then(|mut l| l.undeploy_package(&session.package_id))
            {
                Ok(report) => {
                    session.rollback_conflicts += report.conflicts;
                    session.active_staged_features.clear();
                    session.recent_events.push(format!(
                        "Cleaned up managed scripts (removed={}, restored={}, conflicts={})",
                        report.removed_count, report.restored_originals, report.conflicts
                    ));
                }
                Err(err) => {
                    session.rollback_failures += 1;
                    session
                        .recent_events
                        .push(format!("DeploymentLedger cleanup failed: {err}"));
                }
            }
        }

        session.state = SessionLifecycleState::Stopped;
        let diag = Self::build_diagnostics(session_id, &session);
        self.stopped_diagnostics
            .lock()
            .unwrap()
            .insert(session_id.0.clone(), diag.clone());
        Ok(diag)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        if let Some(session) = self.sessions.lock().unwrap().get(&session_id.0) {
            return Ok(Self::build_diagnostics(session_id, session));
        }
        self.stopped_diagnostics
            .lock()
            .unwrap()
            .get(&session_id.0)
            .cloned()
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionDeclaration, PermissionScope, RuntimeRequirement,
        SupportedGameVersion, TargetGameRef, TargetPlatform, TrainerArtifact, TrainerCompatibility,
        TrainerFeature, TrainerManifest, TrustTier,
    };
    use nekod_runtime_api::GrantedPermissions;
    use nekod_validator::sha256_hex_bytes;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("nekod_fw_{name}_{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn reframework_stages_lua_script_and_executes_loopback_ipc_with_clean_rollback() {
        let root = temp_dir("reframework");
        let game_dir = root.join("game");
        let backup_dir = root.join("backups");
        let ledger_path = root.join("ledger.json");
        fs::create_dir_all(&game_dir).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let ops_clone = Arc::clone(&ops);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let v: Value = serde_json::from_str(&line).unwrap();
                let op = v.get("op").and_then(|o| o.as_str()).unwrap().to_owned();
                ops_clone.lock().unwrap().push(op.clone());
                let reply = match op.as_str() {
                    "ping" => "{\"ok\":true,\"data\":{\"framework\":\"reframework\"}}",
                    _ => "{\"ok\":true,\"data\":{}}",
                };
                writeln!(stream, "{reply}").unwrap();
            }
        });

        let script_body = "re.on_draw_ui(function() end)\n";
        let script_sha = sha256_hex_bytes(script_body.as_bytes());
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "framework": "reframework",
            "game_install_dir": game_dir.display().to_string(),
            "backup_dir": backup_dir.display().to_string(),
            "ledger_path": ledger_path.display().to_string(),
            "ipc": {
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "reframework_loopback_secret_1234"
            },
            "staged_scripts": [{
                "relative_path": "reframework/autorun/nekod_fov.lua",
                "sha256": script_sha,
                "content": script_body
            }],
            "features": [
                {
                    "kind": "stage_script",
                    "feature_id": "fov-lua",
                    "script_relative_path": "reframework/autorun/nekod_fov.lua"
                },
                {
                    "kind": "ipc_toggle",
                    "feature_id": "free-camera",
                    "enable_command": "enable_freecam",
                    "disable_command": "disable_freecam"
                }
            ]
        })
        .to_string();

        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.re4-bridge".to_owned(),
            name: "RE4 Framework Bridge".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Managed REFramework QoL bridge".to_owned(),
            license: "MIT".to_owned(),
            authors: vec![AuthorInfo {
                name: "Test".to_owned(),
                github: None,
                url: None,
            }],
            homepage: None,
            source_repository: "https://github.com/example/re4-bridge".to_owned(),
            target_game: TargetGameRef {
                game_id: "resident-evil-4".to_owned(),
                game_name: "Resident Evil 4".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: RuntimeRequirement {
                kind: RuntimeKind::ManagedFrameworkV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "mods/reframework.json".to_owned(),
                supported_platforms: vec![TargetPlatform::WindowsX64],
            },
            permissions: vec![
                PermissionDeclaration {
                    scope: PermissionScope::ProcessDiscover,
                    reason: "Detect game process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Invoke REFramework companion bridge".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::FilesystemWriteSaveDir,
                    reason: "Stage Lua scripts into reframework/autorun".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            compatibility: TrainerCompatibility {
                singleplayer_only: true,
                requires_offline_mode: false,
                anti_cheat_incompatible: vec![],
                last_verified_at: None,
            },
            features: vec![
                TrainerFeature {
                    id: "fov-lua".to_owned(),
                    name: "FOV Autorun Lua".to_owned(),
                    description: "Stages FOV script".to_owned(),
                    category: FeatureCategory::Camera,
                    control_type: FeatureControlType::Toggle,
                    default_hotkey: None,
                    mutates_memory: false,
                    supports_rollback: true,
                    risk_level: FeatureRiskLevel::Low,
                    value_range: None,
                },
                TrainerFeature {
                    id: "free-camera".to_owned(),
                    name: "Free Camera".to_owned(),
                    description: "Toggles freecam".to_owned(),
                    category: FeatureCategory::Camera,
                    control_type: FeatureControlType::Toggle,
                    default_hotkey: None,
                    mutates_memory: true,
                    supports_rollback: true,
                    risk_level: FeatureRiskLevel::Low,
                    value_range: None,
                },
            ],
            artifacts: vec![TrainerArtifact {
                filename: "mods/reframework.json".to_owned(),
                sha256: "a".repeat(64),
                size_bytes: payload.len() as u64,
                platform: None,
            }],
            extensions: None,
        };

        let mut scopes = BTreeSet::new();
        scopes.insert(PermissionScope::ProcessDiscover);
        scopes.insert(PermissionScope::ProcessWriteMemory);
        scopes.insert(PermissionScope::FilesystemWriteSaveDir);
        let grants = GrantedPermissions {
            trainer_id: manifest.id.clone(),
            trainer_version: manifest.version.to_string(),
            manifest_sha256: "c".repeat(64),
            trust_tier: TrustTier::CommunityVerified,
            granted_scopes: scopes,
            scope_constraints: BTreeMap::new(),
        };

        let runtime = ManagedFrameworkRuntime::new();
        let sid = runtime
            .start_session(StartSessionRequest {
                trainer: manifest,
                target_process: ProcessIdentity {
                    pid: 5150,
                    process_name: "re4.exe".to_owned(),
                    executable_path: game_dir.join("re4.exe").display().to_string(),
                    platform: TargetPlatform::WindowsX64,
                    architecture: BinaryArch::X86_64,
                    executable_sha256: None,
                    detected_version: None,
                    process_creation_time_100ns: None,
                },
                granted_permissions: grants,
                declarative_payload_json: Some(payload),
            })
            .unwrap();

        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "fov-lua".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let staged_path = game_dir
            .join("reframework")
            .join("autorun")
            .join("nekod_fov.lua");
        assert!(staged_path.is_file());

        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "free-camera".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::ManagedFrameworkV1);
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
        assert_eq!(diag.active_patches, 0);
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.rollback_conflicts, 0);
        assert!(!staged_path.exists());

        server.join().unwrap();
        assert_eq!(
            *ops.lock().unwrap(),
            vec!["ping", "enable_freecam", "disable_freecam"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_disallowed_script_extension_for_scripthook() {
        let bad = ManagedFrameworkPayload {
            schema_version: "1.0.0".to_owned(),
            framework: ManagedFrameworkKind::ScripthookV,
            game_install_dir: Some("C:\\Games\\GTAV".to_owned()),
            backup_dir: Some("C:\\backups".to_owned()),
            ledger_path: Some("C:\\ledger.json".to_owned()),
            ipc: None,
            staged_scripts: vec![ManagedScriptAsset {
                relative_path: "scripts/evil.exe".to_owned(),
                sha256: "a".repeat(64),
                content: "MZ".to_owned(),
            }],
            features: vec![ManagedFrameworkFeature::StageScript {
                feature_id: "bad".to_owned(),
                script_relative_path: "scripts/evil.exe".to_owned(),
            }],
        };
        assert!(matches!(
            ManagedFrameworkRuntime::validate_payload(&bad),
            Err(RuntimeError::InvalidPayload(_))
        ));
    }
}
