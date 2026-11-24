//! Zero-injection external IPC adapter runtime (`adapter_ipc_v1`).
//!
//! Supports loopback-only external emulator/tool control protocols (beginning with
//! `retroarch_udp_v1`) without opening process memory write handles or injecting DLLs.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nekod_manifest::{PermissionScope, RuntimeKind, is_valid_feature_id};
use nekod_runtime_api::{
    FeatureCommand, FeatureValue, ProcessIdentity, RuntimeDiagnostics, RuntimeError,
    RuntimeProvider, RuntimeSessionId, SessionLifecycleState, StartSessionRequest,
};
use serde::{Deserialize, Serialize};

use crate::session::validate_session_preflight_for_kind;

const ALLOWED_COMMAND_VERBS: &[&str] = &[
    "PAUSE_TOGGLE",
    "RESET",
    "FRAMEADVANCE",
    "READ_CORE_MEMORY",
    "WRITE_CORE_MEMORY",
    "SET_PARAM",
    "CHEAT_TOGGLE",
    "CHEAT_SET",
];

/// Supported external IPC transport protocols under `adapter_ipc_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterProtocol {
    RetroarchUdpV1,
}

/// Declarative payload schema for `adapter_ipc_v1` entrypoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterIpcPayload {
    pub schema_version: String,
    pub protocol: AdapterProtocol,
    pub host: String,
    pub port: u16,
    pub features: Vec<AdapterFeatureBinding>,
}

/// Feature-to-command binding inside an `AdapterIpcPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterFeatureBinding {
    pub feature_id: String,
    #[serde(default)]
    pub enable_command: Option<String>,
    #[serde(default)]
    pub disable_command: Option<String>,
    #[serde(default)]
    pub command_template: Option<String>,
}

struct ActiveAdapterSession {
    target_addr: SocketAddr,
    target_process: ProcessIdentity,
    payload: AdapterIpcPayload,
    /// LIFO stack of `(feature_id, disable_command)` for deterministic session teardown rollback.
    rollback_stack: Vec<(String, String)>,
    state: SessionLifecycleState,
    rollback_failures: usize,
    recent_events: Vec<String>,
}

/// Controlled zero-injection runtime for loopback external emulator/tool IPC protocols.
pub struct AdapterIpcRuntime {
    next_id: AtomicU64,
    sessions: Mutex<BTreeMap<String, ActiveAdapterSession>>,
    stopped_diagnostics: Mutex<BTreeMap<String, RuntimeDiagnostics>>,
}

impl Default for AdapterIpcRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AdapterIpcRuntime {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(BTreeMap::new()),
            stopped_diagnostics: Mutex::new(BTreeMap::new()),
        }
    }

    fn validate_payload(payload: &AdapterIpcPayload) -> Result<SocketAddr, RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported adapter_ipc_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.port == 0 {
            return Err(RuntimeError::InvalidPayload(
                "adapter_ipc_v1 port must be non-zero".to_owned(),
            ));
        }

        // Strict SSRF protection: forbid any non-loopback network host or IP.
        let ip: IpAddr = match payload.host.as_str() {
            "localhost" => "127.0.0.1".parse().unwrap(),
            other => other.parse().map_err(|_| {
                RuntimeError::InvalidPayload(format!(
                    "adapter_ipc_v1 requires loopback IP or 'localhost', got '{other}'"
                ))
            })?,
        };
        if !ip.is_loopback() {
            return Err(RuntimeError::InvalidPayload(format!(
                "adapter_ipc_v1 refuses non-loopback endpoint '{ip}'"
            )));
        }

        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "adapter_ipc_v1 payload must define at least one feature".to_owned(),
            ));
        }
        for feat in &payload.features {
            if !is_valid_feature_id(&feat.feature_id) {
                return Err(RuntimeError::InvalidPayload(format!(
                    "invalid feature_id '{}'",
                    feat.feature_id
                )));
            }
            if let Some(cmd) = &feat.enable_command {
                validate_command_string(cmd)?;
            }
            if let Some(cmd) = &feat.disable_command {
                validate_command_string(cmd)?;
            }
            if let Some(tpl) = &feat.command_template {
                validate_command_string(tpl)?;
            }
        }

        Ok(SocketAddr::new(ip, payload.port))
    }

    fn send_udp_command(addr: SocketAddr, cmd: &str) -> Result<(), RuntimeError> {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .map_err(|e| RuntimeError::ExecutionFailed(format!("UDP bind failed: {e}")))?;
        socket
            .set_write_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| {
                RuntimeError::ExecutionFailed(format!("UDP timeout config failed: {e}"))
            })?;
        socket.send_to(cmd.as_bytes(), addr).map_err(|e| {
            RuntimeError::ExecutionFailed(format!("UDP send to {addr} failed: {e}"))
        })?;
        Ok(())
    }
}

fn validate_command_string(cmd: &str) -> Result<(), RuntimeError> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || trimmed.contains(['\r', '\n', '\0']) {
        return Err(RuntimeError::InvalidPayload(
            "adapter command must be a single non-empty ASCII line <= 256 bytes".to_owned(),
        ));
    }
    let verb = trimmed.split_whitespace().next().unwrap_or("");
    if !ALLOWED_COMMAND_VERBS.contains(&verb) {
        return Err(RuntimeError::InvalidPayload(format!(
            "unsupported adapter command verb '{verb}'"
        )));
    }
    Ok(())
}

impl RuntimeProvider for AdapterIpcRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::AdapterIpcV1
    }

    fn adapter_metadata(&self) -> nekod_runtime_api::RuntimeAdapterMetadata {
        nekod_runtime_api::RuntimeAdapterMetadata {
            adapter_id: "adapter_ipc_v1".to_owned(),
            adapter_class: nekod_runtime_api::RuntimeAdapterClass::GameSpecificBridge,
            runtime_kind: RuntimeKind::AdapterIpcV1,
            lifecycle_mode: nekod_runtime_api::RuntimeLifecycleMode::ExternalProtocolIpc,
            manages_disk_files: false,
            requires_process_attach: false,
        }
    }

    fn discover_matching_processes(
        &self,
        _expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Ok(Vec::new())
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        validate_session_preflight_for_kind(&req, RuntimeKind::AdapterIpcV1)?;

        if req
            .granted_permissions
            .allows(PermissionScope::RuntimeNativeModule)
        {
            return Err(RuntimeError::InvalidPayload(
                "adapter_ipc_v1 does not accept runtime.native_module permission".to_owned(),
            ));
        }

        let raw_payload = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload(
                "missing adapter_ipc_v1 JSON entrypoint payload".to_owned(),
            )
        })?;
        let payload: AdapterIpcPayload = serde_json::from_str(raw_payload).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid adapter_ipc_v1 JSON: {e}"))
        })?;
        let target_addr = Self::validate_payload(&payload)?;

        let id_num = self.next_id.fetch_add(1, Ordering::Relaxed);
        let session_id = RuntimeSessionId(format!("adapter-{id_num}"));

        let session = ActiveAdapterSession {
            target_addr,
            target_process: req.target_process,
            payload,
            rollback_stack: Vec::new(),
            state: SessionLifecycleState::Attached,
            rollback_failures: 0,
            recent_events: vec![format!("Bound adapter_ipc_v1 endpoint {target_addr}")],
        };

        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.0.clone(), session);
        Ok(session_id)
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

        let binding = session
            .payload
            .features
            .iter()
            .find(|f| f.feature_id == command.feature_id)
            .ok_or_else(|| {
                RuntimeError::InvalidPayload(format!(
                    "feature '{}' not defined in adapter_ipc_v1 payload",
                    command.feature_id
                ))
            })?
            .clone();

        match command.value {
            FeatureValue::Toggle(true) => {
                let cmd = binding.enable_command.as_deref().ok_or_else(|| {
                    RuntimeError::InvalidPayload(format!(
                        "feature '{}' missing enable_command",
                        command.feature_id
                    ))
                })?;
                Self::send_udp_command(session.target_addr, cmd)?;
                if let Some(disable_cmd) = binding.disable_command {
                    session
                        .rollback_stack
                        .push((command.feature_id.clone(), disable_cmd));
                }
                session
                    .recent_events
                    .push(format!("Enabled '{}' -> {}", command.feature_id, cmd));
            }
            FeatureValue::Toggle(false) => {
                let cmd = binding
                    .disable_command
                    .as_deref()
                    .or(binding.enable_command.as_deref())
                    .ok_or_else(|| {
                        RuntimeError::InvalidPayload(format!(
                            "feature '{}' missing disable_command",
                            command.feature_id
                        ))
                    })?;
                Self::send_udp_command(session.target_addr, cmd)?;
                session
                    .rollback_stack
                    .retain(|(fid, _)| fid != &command.feature_id);
                session
                    .recent_events
                    .push(format!("Disabled '{}' -> {}", command.feature_id, cmd));
            }
            FeatureValue::Action => {
                let cmd = binding.enable_command.as_deref().ok_or_else(|| {
                    RuntimeError::InvalidPayload(format!(
                        "action '{}' missing enable_command",
                        command.feature_id
                    ))
                })?;
                Self::send_udp_command(session.target_addr, cmd)?;
                session.recent_events.push(format!(
                    "Triggered action '{}' -> {}",
                    command.feature_id, cmd
                ));
            }
            FeatureValue::Integer(val) => {
                let tpl = binding.command_template.as_deref().ok_or_else(|| {
                    RuntimeError::InvalidPayload(format!(
                        "integer feature '{}' missing command_template",
                        command.feature_id
                    ))
                })?;
                let rendered = tpl.replace("{value}", &val.to_string());
                validate_command_string(&rendered)?;
                Self::send_udp_command(session.target_addr, &rendered)?;
                session.recent_events.push(format!(
                    "Sent integer '{}' -> {}",
                    command.feature_id, rendered
                ));
            }
            FeatureValue::Float(val) => {
                if !val.is_finite() {
                    return Err(RuntimeError::InvalidPayload(
                        "non-finite float parameter rejected".to_owned(),
                    ));
                }
                let tpl = binding.command_template.as_deref().ok_or_else(|| {
                    RuntimeError::InvalidPayload(format!(
                        "float feature '{}' missing command_template",
                        command.feature_id
                    ))
                })?;
                let rendered = tpl.replace("{value}", &format!("{val:.4}"));
                validate_command_string(&rendered)?;
                Self::send_udp_command(session.target_addr, &rendered)?;
                session.recent_events.push(format!(
                    "Sent float '{}' -> {}",
                    command.feature_id, rendered
                ));
            }
        }

        Ok(())
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

        // LIFO rollback of active reversible adapter toggles.
        while let Some((fid, disable_cmd)) = session.rollback_stack.pop() {
            if let Err(err) = Self::send_udp_command(session.target_addr, &disable_cmd) {
                session.rollback_failures += 1;
                session
                    .recent_events
                    .push(format!("Rollback failed for '{fid}': {err}"));
            } else {
                session
                    .recent_events
                    .push(format!("Rolled back '{fid}' -> {disable_cmd}"));
            }
        }
        session.state = SessionLifecycleState::Stopped;

        let diag = RuntimeDiagnostics {
            session_id: session_id.clone(),
            runtime_kind: RuntimeKind::AdapterIpcV1,
            state: session.state,
            active_patches: 0,
            rollback_failures: session.rollback_failures,
            rollback_conflicts: 0,
            active_freeze_features: 0,
            target_pid: Some(session.target_process.pid),
            target_process_name: Some(session.target_process.process_name),
            recent_events: session.recent_events,
        };
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
            return Ok(RuntimeDiagnostics {
                session_id: session_id.clone(),
                runtime_kind: RuntimeKind::AdapterIpcV1,
                state: session.state,
                active_patches: session.rollback_stack.len(),
                rollback_failures: session.rollback_failures,
                rollback_conflicts: 0,
                active_freeze_features: 0,
                target_pid: Some(session.target_process.pid),
                target_process_name: Some(session.target_process.process_name.clone()),
                recent_events: session.recent_events.clone(),
            });
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
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionDeclaration, SupportedGameVersion, TargetGameRef,
        TargetPlatform, TrainerArtifact, TrainerCompatibility, TrainerFeature, TrainerManifest,
        TrustTier,
    };
    use nekod_runtime_api::GrantedPermissions;

    use crate::CompositeRuntimeRouter;

    fn sample_adapter_request(host: &str, port: u16) -> StartSessionRequest {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "protocol": "retroarch_udp_v1",
            "host": host,
            "port": port,
            "features": [
                {
                    "feature_id": "pause-toggle",
                    "enable_command": "CHEAT_TOGGLE 0 1",
                    "disable_command": "CHEAT_TOGGLE 0 0"
                },
                {
                    "feature_id": "set-lives",
                    "command_template": "CHEAT_SET 1 {value}"
                }
            ]
        })
        .to_string();

        let digest = "a".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.retroarch-snes".to_owned(),
            name: "RetroArch SNES Adapter".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Loopback UDP adapter test trainer".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/nekod-org/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-retroarch-game".to_owned(),
                game_name: "Sample RetroArch Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::AdapterIpcV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "adapter.json".to_owned(),
                supported_platforms: vec![TargetPlatform::WindowsX64, TargetPlatform::LinuxX64],
            },
            compatibility: TrainerCompatibility {
                singleplayer_only: true,
                requires_offline_mode: false,
                anti_cheat_incompatible: vec![],
                last_verified_at: None,
            },
            permissions: vec![PermissionDeclaration {
                scope: PermissionScope::ProcessDiscover,
                reason: "Verify RetroArch instance is running locally".to_owned(),
                required: true,
                constraints: None,
            }],
            features: vec![
                TrainerFeature {
                    id: "pause-toggle".to_owned(),
                    name: "Infinite Lives".to_owned(),
                    description: "Toggle cheat index 0".to_owned(),
                    category: FeatureCategory::Player,
                    control_type: FeatureControlType::Toggle,
                    default_hotkey: None,
                    mutates_memory: false,
                    supports_rollback: true,
                    risk_level: FeatureRiskLevel::Low,
                    value_range: None,
                },
                TrainerFeature {
                    id: "set-lives".to_owned(),
                    name: "Set Lives".to_owned(),
                    description: "Set cheat parameter 1".to_owned(),
                    category: FeatureCategory::Player,
                    control_type: FeatureControlType::Integer,
                    default_hotkey: None,
                    mutates_memory: false,
                    supports_rollback: false,
                    risk_level: FeatureRiskLevel::Low,
                    value_range: None,
                },
            ],
            artifacts: vec![TrainerArtifact {
                filename: "adapter.json".to_owned(),
                sha256: digest.clone(),
                size_bytes: payload.len() as u64,
                platform: None,
            }],
            extensions: None,
        };

        let mut granted_scopes = BTreeSet::new();
        granted_scopes.insert(PermissionScope::ProcessDiscover);

        StartSessionRequest {
            trainer: manifest.clone(),
            target_process: ProcessIdentity {
                pid: 4321,
                process_name: "retroarch.exe".to_owned(),
                executable_path: "C:\\RetroArch\\retroarch.exe".to_owned(),
                platform: TargetPlatform::WindowsX64,
                architecture: BinaryArch::X86_64,
                executable_sha256: None,
                detected_version: None,
                process_creation_time_100ns: None,
            },
            granted_permissions: GrantedPermissions {
                trainer_id: manifest.id,
                trainer_version: "1.0.0".to_owned(),
                manifest_sha256: digest,
                trust_tier: TrustTier::CommunityVerified,
                granted_scopes,
                scope_constraints: BTreeMap::new(),
            },
            declarative_payload_json: Some(payload),
            verified_package_assets: BTreeMap::new(),
        }
    }

    #[test]
    fn adapter_ipc_dispatches_loopback_udp_and_rolls_back_on_stop() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let port = server.local_addr().unwrap().port();

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router
            .start_session(sample_adapter_request("127.0.0.1", port))
            .unwrap();

        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "pause-toggle".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "set-lives".to_owned(),
                    value: FeatureValue::Integer(99),
                },
            )
            .unwrap();

        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::AdapterIpcV1);
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
        assert_eq!(diag.rollback_failures, 0);

        let mut received = Vec::new();
        let mut buf = [0u8; 256];
        for _ in 0..3 {
            let (n, _) = server.recv_from(&mut buf).unwrap();
            received.push(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
        assert_eq!(
            received,
            vec!["CHEAT_TOGGLE 0 1", "CHEAT_SET 1 99", "CHEAT_TOGGLE 0 0"]
        );
    }

    #[test]
    fn rejects_non_loopback_host_ssrf_and_command_injection() {
        let runtime = Arc::new(AdapterIpcRuntime::new());
        let err = runtime
            .start_session(sample_adapter_request("8.8.8.8", 55355))
            .unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidPayload(_)));

        let mut bad_cmd_req = sample_adapter_request("127.0.0.1", 55355);
        bad_cmd_req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "protocol": "retroarch_udp_v1",
                "host": "127.0.0.1",
                "port": 55355,
                "features": [{
                    "feature_id": "pause-toggle",
                    "enable_command": "EXEC_ARBITRARY_SHELL"
                }]
            })
            .to_string(),
        );
        assert!(matches!(
            runtime.start_session(bad_cmd_req),
            Err(RuntimeError::InvalidPayload(_))
        ));
    }
}
