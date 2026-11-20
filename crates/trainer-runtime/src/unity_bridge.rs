//! Unity Reflection Bridge runtime adapter (`unity_bridge_v1`).
//!
//! Communicates with an authenticated managed loader bridge (BepInEx / MelonLoader)
//! over loopback JSON-line IPC, strictly separating `Mono` vs. `IL2CPP` runtime flavors
//! and restoring modified class fields in LIFO order on session teardown.

use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::{PermissionScope, RuntimeDependencySpec, RuntimeKind};
use nekod_runtime_api::{
    DependencyCheckResult, DependencyStatus, FeatureCommand, FeatureValue, ProcessIdentity,
    RuntimeAdapterClass, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId, SessionLifecycleState,
    StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bridge_ipc::{
    BridgeRpcRequest, BridgeSessionStore, ManagedBridgeSession, send_bridge_rpc,
    send_unity_explorer_http_rpc, validate_loopback_bridge, validate_loopback_host_port,
};
use crate::session::validate_session_preflight_for_kind;

/// Wire protocol spoken between Nekod and an in-game Unity companion plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnityBridgeProtocol {
    /// Nekod default authenticated newline-delimited JSON RPC.
    #[default]
    NekodJsonLineV1,
    /// Native compatibility with `OmaruKoma/UnityExplorerMCP` (`MCPBridge.dll`) HTTP POST `/`.
    UnityExplorerMcpHttpV1,
}

/// Explicit Unity scripting backend flavor (Mono and IL2CPP are never assumed identical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityFlavor {
    Mono,
    Il2cpp,
}

impl UnityFlavor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Il2cpp => "il2cpp",
        }
    }
}

/// Supported Unity loader family hosting the controlled bridge plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityLoaderKind {
    BepinexMono,
    BepinexIl2cpp,
    Melonloader,
}

/// Entrypoint JSON payload for `unity_bridge_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityBridgePayload {
    pub schema_version: String,
    #[serde(default)]
    pub protocol: UnityBridgeProtocol,
    pub flavor: UnityFlavor,
    pub loader: UnityLoaderKind,
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub auto_discover_by_pid: bool,
    #[serde(default)]
    pub discovery_dir: Option<String>,
    #[serde(default)]
    pub auth_token: String,
    pub features: Vec<UnityFeatureBinding>,
}

/// Declarative Unity class/field/method binding mapped from a UI `FeatureCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnityFeatureBinding {
    ToggleField {
        feature_id: String,
        class_name: String,
        field_name: String,
        enabled_value: Value,
        disabled_value: Value,
    },
    SetField {
        feature_id: String,
        class_name: String,
        field_name: String,
    },
    InvokeMethod {
        feature_id: String,
        class_name: String,
        method_name: String,
    },
}

impl UnityFeatureBinding {
    fn feature_id(&self) -> &str {
        match self {
            Self::ToggleField { feature_id, .. }
            | Self::SetField { feature_id, .. }
            | Self::InvokeMethod { feature_id, .. } => feature_id,
        }
    }
}

/// Controlled Unity Reflection Bridge adapter (`unity_bridge_v1`).
pub struct UnityBridgeRuntime {
    store: BridgeSessionStore,
    bindings: std::sync::Mutex<std::collections::BTreeMap<String, UnityBridgePayload>>,
}

impl Default for UnityBridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl UnityBridgeRuntime {
    pub fn new() -> Self {
        Self {
            store: BridgeSessionStore::new(),
            bindings: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    fn validate_payload(payload: &UnityBridgePayload) -> Result<(), RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported unity_bridge_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        // Enforce strict pairing between Unity runtime flavor and BepInEx loader target.
        if payload.flavor == UnityFlavor::Mono && payload.loader == UnityLoaderKind::BepinexIl2cpp {
            return Err(RuntimeError::InvalidPayload(
                "Unity Mono flavor cannot use bepinex_il2cpp loader".to_owned(),
            ));
        }
        if payload.flavor == UnityFlavor::Il2cpp && payload.loader == UnityLoaderKind::BepinexMono {
            return Err(RuntimeError::InvalidPayload(
                "Unity IL2CPP flavor cannot use bepinex_mono loader".to_owned(),
            ));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "unity_bridge_v1 payload must declare at least one feature".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Resolves a live `UnityExplorerMCP` loopback port from `<dir>/bridge-<pid>.json` written by
/// `MCPBridge.dll` (`OmaruKoma/UnityExplorerMCP`).
pub fn resolve_unity_explorer_discovery_port(
    target_pid: u32,
    custom_dir: Option<&str>,
) -> Option<u16> {
    let dir: PathBuf = match custom_dir {
        Some(custom) => PathBuf::from(custom),
        None => {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                PathBuf::from(local_app_data)
                    .join("UnityExplorerMCP")
                    .join("bridges")
            } else {
                std::env::temp_dir()
                    .join("UnityExplorerMCP")
                    .join("bridges")
            }
        }
    };
    let file = dir.join(format!("bridge-{target_pid}.json"));
    let raw = fs::read_to_string(file).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let port_u64 = val.get("port").or_else(|| val.get("Port"))?.as_u64()?;
    u16::try_from(port_u64).ok().filter(|p| *p != 0)
}

/// Discovery record written by standalone `NekodUnityBridge` (`plugins/unity-nekod-bridge/`)
/// under `%LOCALAPPDATA%\Nekod\bridges\unity-<pid>.json` (zero UnityExplorer dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NekodUnityDiscovery {
    pub port: u16,
    pub auth_token: Option<String>,
    pub flavor: Option<String>,
}

/// Resolves port and per-launch auth token from `<dir>/unity-<pid>.json` written by
/// standalone `NekodUnityBridge.IL2CPP.dll` / `NekodUnityBridge.Mono.dll`.
pub fn resolve_nekod_unity_discovery(
    target_pid: u32,
    custom_dir: Option<&str>,
) -> Option<NekodUnityDiscovery> {
    let dir: PathBuf = match custom_dir {
        Some(custom) => PathBuf::from(custom),
        None => {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                PathBuf::from(local_app_data).join("Nekod").join("bridges")
            } else {
                std::env::temp_dir().join("Nekod").join("bridges")
            }
        }
    };
    let file = dir.join(format!("unity-{target_pid}.json"));
    let raw = fs::read_to_string(file).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let port_u64 = val.get("port")?.as_u64()?;
    let port = u16::try_from(port_u64).ok().filter(|p| *p != 0)?;
    let auth_token = val
        .get("auth_token")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let flavor = val.get("flavor").and_then(Value::as_str).map(str::to_owned);
    Some(NekodUnityDiscovery {
        port,
        auth_token,
        flavor,
    })
}

fn find_plugin_dll_matching(plugins_dir: &Path, needle: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(plugins_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            let lower = name.to_ascii_lowercase();
            if lower.contains(needle) && lower.ends_with(".dll") {
                return Some(path);
            }
        } else if path.is_dir()
            && let Some(found) = fs::read_dir(&path).ok().and_then(|sub| {
                sub.flatten().map(|e| e.path()).find(|p| {
                    p.is_file()
                        && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            let lower = n.to_ascii_lowercase();
                            lower.contains(needle) && lower.ends_with(".dll")
                        })
                })
            })
        {
            return Some(found);
        }
    }
    None
}

fn send_unity_rpc(
    use_http: bool,
    endpoint: std::net::SocketAddr,
    token: &str,
    op: &str,
    params: Value,
) -> Result<Value, RuntimeError> {
    if use_http {
        send_unity_explorer_http_rpc(endpoint, op, &params)
    } else {
        send_bridge_rpc(
            endpoint,
            &BridgeRpcRequest {
                token: token.to_owned(),
                op: op.to_owned(),
                params,
            },
        )
        .map(|resp| resp.data)
    }
}

impl RuntimeProvider for UnityBridgeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::UnityBridgeV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "unity_bridge_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::UnityBridge,
            runtime_kind: RuntimeKind::UnityBridgeV1,
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
                let outcome = match game_install_dir {
                    Some(dir) if low.contains("nekod-unity-bridge") => {
                        if let Some(dll) = find_plugin_dll_matching(
                            &dir.join("BepInEx").join("plugins"),
                            "nekodunitybridge",
                        ) {
                            DependencyStatus::Satisfied {
                                resolved_location: dll.display().to_string(),
                            }
                        } else {
                            DependencyStatus::Missing {
                                suggested_action: format!(
                                    "Stage standalone '{}' ({}) from Plugin Bucket into BepInEx/plugins",
                                    dep.dependency_id, dep.version_req
                                ),
                            }
                        }
                    }
                    Some(dir)
                        if (low.contains("unity-explorer-mcp") || low.contains("mcpbridge")) =>
                    {
                        if let Some(dll) = find_plugin_dll_matching(
                            &dir.join("BepInEx").join("plugins"),
                            "mcpbridge",
                        ) {
                            DependencyStatus::Satisfied {
                                resolved_location: dll.display().to_string(),
                            }
                        } else {
                            DependencyStatus::Missing {
                                suggested_action: format!(
                                    "Stage '{}' ({}) from Plugin Bucket via RuntimeDependencyManager",
                                    dep.dependency_id, dep.version_req
                                ),
                            }
                        }
                    }
                    Some(dir)
                        if low.contains("bepinex") && dir.join("BepInEx").join("core").is_dir() =>
                    {
                        DependencyStatus::Satisfied {
                            resolved_location: dir
                                .join("BepInEx")
                                .join("core")
                                .display()
                                .to_string(),
                        }
                    }
                    Some(dir)
                        if low.contains("melonloader") && dir.join("MelonLoader").is_dir() =>
                    {
                        DependencyStatus::Satisfied {
                            resolved_location: dir.join("MelonLoader").display().to_string(),
                        }
                    }
                    _ => DependencyStatus::Missing {
                        suggested_action: format!(
                            "Stage or install '{}' ({}) via RuntimeDependencyManager",
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
        validate_session_preflight_for_kind(&req, RuntimeKind::UnityBridgeV1)?;

        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload("missing unity_bridge_v1 JSON entrypoint".to_owned())
        })?;
        let payload: UnityBridgePayload = serde_json::from_str(raw).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid unity_bridge_v1 JSON: {e}"))
        })?;
        Self::validate_payload(&payload)?;

        let use_http = payload.protocol == UnityBridgeProtocol::UnityExplorerMcpHttpV1;
        let (resolved_port, effective_token) = if payload.auto_discover_by_pid {
            if use_http {
                let discovered_port = resolve_unity_explorer_discovery_port(
                    req.target_process.pid,
                    payload.discovery_dir.as_deref(),
                )
                .unwrap_or(payload.port);
                (discovered_port, payload.auth_token.clone())
            } else if let Some(disc) = resolve_nekod_unity_discovery(
                req.target_process.pid,
                payload.discovery_dir.as_deref(),
            ) {
                let tok = if payload.auth_token.trim().is_empty() {
                    disc.auth_token.unwrap_or_default()
                } else {
                    payload.auth_token.clone()
                };
                (disc.port, tok)
            } else {
                (payload.port, payload.auth_token.clone())
            }
        } else {
            (payload.port, payload.auth_token.clone())
        };

        let endpoint = if use_http {
            validate_loopback_host_port(&payload.host, resolved_port)?
        } else {
            validate_loopback_bridge(&payload.host, resolved_port, &effective_token)?
        };

        // Perform handshake verifying reported Unity flavor (and optional PID) matches manifest.
        let ping_data = send_unity_rpc(
            use_http,
            endpoint,
            &effective_token,
            "ping",
            if use_http {
                json!({})
            } else {
                json!({ "expected_flavor": payload.flavor.as_str() })
            },
        )?;

        let reported_flavor = ping_data
            .get("flavor")
            .or_else(|| ping_data.get("Backend"))
            .or_else(|| ping_data.get("backend"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !reported_flavor.eq_ignore_ascii_case(payload.flavor.as_str()) {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "unity bridge flavor mismatch: expected '{}', bridge reported '{reported_flavor}'",
                payload.flavor.as_str()
            )));
        }

        if let Some(reported_pid) = ping_data
            .get("Pid")
            .or_else(|| ping_data.get("pid"))
            .and_then(Value::as_u64)
            && reported_pid != 0
            && reported_pid != u64::from(req.target_process.pid)
        {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "unity bridge PID mismatch: target pid {}, bridge reported {reported_pid}",
                req.target_process.pid
            )));
        }

        let session = ManagedBridgeSession {
            endpoint,
            auth_token: effective_token,
            target_process: req.target_process,
            runtime_kind: RuntimeKind::UnityBridgeV1,
            use_unity_explorer_http: use_http,
            state: SessionLifecycleState::Attached,
            rollback_stack: Vec::new(),
            rollback_failures: 0,
            recent_events: vec![format!(
                "Authenticated Unity {} ({:?}) bridge on {endpoint}",
                payload.flavor.as_str(),
                payload.protocol
            )],
        };

        let sid = self.store.insert("unity", session);
        self.bindings.lock().unwrap().insert(sid.0.clone(), payload);
        Ok(sid)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let binding = {
            let guard = self.bindings.lock().unwrap();
            let payload = guard
                .get(&session_id.0)
                .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
            payload
                .features
                .iter()
                .find(|f| f.feature_id() == command.feature_id)
                .cloned()
                .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?
        };

        self.store.with_session_mut(session_id, |session| {
            let use_http = session.use_unity_explorer_http;
            match (binding, command.value) {
                (
                    UnityFeatureBinding::ToggleField {
                        class_name,
                        field_name,
                        enabled_value,
                        disabled_value,
                        ..
                    },
                    FeatureValue::Toggle(enable),
                ) => {
                    let target_val = if enable {
                        enabled_value
                    } else {
                        disabled_value.clone()
                    };
                    let (op, params, undo_params) = if use_http {
                        (
                            "set_static",
                            json!({ "type": class_name, "member": field_name, "value": target_val }),
                            json!({ "type": class_name, "member": field_name, "value": disabled_value }),
                        )
                    } else {
                        (
                            "set_field",
                            json!({ "class_name": class_name, "field_name": field_name, "value": target_val }),
                            json!({ "class_name": class_name, "field_name": field_name, "value": disabled_value }),
                        )
                    };
                    send_unity_rpc(use_http, session.endpoint, &session.auth_token, op, params)?;
                    if enable {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: session.auth_token.clone(),
                                op: op.to_owned(),
                                params: undo_params,
                            },
                        ));
                    } else {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                    }
                    session.recent_events.push(format!(
                        "Unity field '{class_name}.{field_name}' set (enable={enable})"
                    ));
                }
                (
                    UnityFeatureBinding::SetField {
                        class_name,
                        field_name,
                        ..
                    },
                    FeatureValue::Integer(val),
                ) => {
                    let (op, params) = if use_http {
                        (
                            "set_static",
                            json!({ "type": class_name, "member": field_name, "value": val }),
                        )
                    } else {
                        (
                            "set_field",
                            json!({ "class_name": class_name, "field_name": field_name, "value": val }),
                        )
                    };
                    send_unity_rpc(use_http, session.endpoint, &session.auth_token, op, params)?;
                    if use_http {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: String::new(),
                                op: "undo_write".to_owned(),
                                params: json!({ "steps": 1 }),
                            },
                        ));
                    }
                    session
                        .recent_events
                        .push(format!("Unity field '{class_name}.{field_name}' = {val}"));
                }
                (
                    UnityFeatureBinding::SetField {
                        class_name,
                        field_name,
                        ..
                    },
                    FeatureValue::Float(val),
                ) => {
                    let (op, params) = if use_http {
                        (
                            "set_static",
                            json!({ "type": class_name, "member": field_name, "value": val }),
                        )
                    } else {
                        (
                            "set_field",
                            json!({ "class_name": class_name, "field_name": field_name, "value": val }),
                        )
                    };
                    send_unity_rpc(use_http, session.endpoint, &session.auth_token, op, params)?;
                    if use_http {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: String::new(),
                                op: "undo_write".to_owned(),
                                params: json!({ "steps": 1 }),
                            },
                        ));
                    }
                    session
                        .recent_events
                        .push(format!("Unity field '{class_name}.{field_name}' = {val}"));
                }
                (
                    UnityFeatureBinding::InvokeMethod {
                        class_name,
                        method_name,
                        ..
                    },
                    FeatureValue::Action,
                ) => {
                    let (op, params) = if use_http {
                        (
                            "invoke_static",
                            json!({ "type": class_name, "method": method_name, "args": [] }),
                        )
                    } else {
                        (
                            "invoke_method",
                            json!({ "class_name": class_name, "method_name": method_name }),
                        )
                    };
                    send_unity_rpc(use_http, session.endpoint, &session.auth_token, op, params)?;
                    session.recent_events.push(format!(
                        "Invoked Unity method '{class_name}.{method_name}()'"
                    ));
                }
                _ => {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "control type mismatch for Unity feature '{}'",
                        command.feature_id
                    )));
                }
            }
            Ok(())
        })
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        self.bindings.lock().unwrap().remove(&session_id.0);
        self.store.stop_and_rollback(session_id)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        self.store.diagnostics(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionDeclaration, SupportedGameVersion, TargetGameRef,
        TargetPlatform, TrainerArtifact, TrainerCompatibility, TrainerFeature, TrainerManifest,
        TrustTier,
    };
    use nekod_runtime_api::GrantedPermissions;

    use crate::CompositeRuntimeRouter;

    fn unity_request(
        flavor: UnityFlavor,
        loader: UnityLoaderKind,
        port: u16,
    ) -> StartSessionRequest {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": flavor,
            "loader": loader,
            "host": "127.0.0.1",
            "port": port,
            "auth_token": "session_token_0123456789abcdef",
            "features": [{
                "kind": "toggle_field",
                "feature_id": "infinite-stamina",
                "class_name": "PlayerController",
                "field_name": "drainRate",
                "enabled_value": 0.0,
                "disabled_value": 1.0
            }]
        })
        .to_string();

        let digest = "a".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.unity-bridge".to_owned(),
            name: "Unity Bridge Mod".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Unity Mono reflection bridge test".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-unity-game".to_owned(),
                game_name: "Sample Unity Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::UnityBridgeV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "unity_bridge.json".to_owned(),
                supported_platforms: vec![TargetPlatform::WindowsX64],
            },
            compatibility: TrainerCompatibility {
                singleplayer_only: true,
                requires_offline_mode: false,
                anti_cheat_incompatible: vec![],
                last_verified_at: None,
            },
            permissions: vec![
                PermissionDeclaration {
                    scope: PermissionScope::ProcessDiscover,
                    reason: "Discover Unity process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Mutate Unity fields via managed reflection bridge".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: vec![TrainerFeature {
                id: "infinite-stamina".to_owned(),
                name: "Infinite Stamina".to_owned(),
                description: "Set drainRate to 0".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "unity_bridge.json".to_owned(),
                sha256: digest.clone(),
                size_bytes: payload.len() as u64,
                platform: None,
            }],
            extensions: None,
        };

        let mut granted_scopes = BTreeSet::new();
        granted_scopes.insert(PermissionScope::ProcessDiscover);
        granted_scopes.insert(PermissionScope::ProcessWriteMemory);

        StartSessionRequest {
            trainer: manifest.clone(),
            target_process: ProcessIdentity {
                pid: 5200,
                process_name: "UnityGame.exe".to_owned(),
                executable_path: "C:\\Games\\UnityGame.exe".to_owned(),
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
        }
    }

    #[test]
    fn unity_bridge_authenticates_enforces_mono_vs_il2cpp_and_rolls_back_field() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let observed_ops = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let ops_clone = observed_ops.clone();

        let server_thread = thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                if req["token"] != "session_token_0123456789abcdef" {
                    writeln!(stream, "{{\"ok\":false,\"error\":\"unauthorized\"}}").unwrap();
                    continue;
                }
                let op = req["op"].as_str().unwrap().to_owned();
                ops_clone
                    .lock()
                    .unwrap()
                    .push((op.clone(), req["params"].clone()));
                if op == "ping" {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\"}}}}").unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{}}}}").unwrap();
                }
            }
        });

        let router = CompositeRuntimeRouter::with_default_providers();

        // 1. Incompatible Mono/IL2CPP pairing in manifest must fail before connecting.
        assert!(matches!(
            router.start_session(unity_request(
                UnityFlavor::Mono,
                UnityLoaderKind::BepinexIl2cpp,
                port
            )),
            Err(RuntimeError::InvalidPayload(_))
        ));

        // 2. IL2CPP request rejected when running bridge reports Mono flavor.
        assert!(matches!(
            router.start_session(unity_request(
                UnityFlavor::Il2cpp,
                UnityLoaderKind::BepinexIl2cpp,
                port
            )),
            Err(RuntimeError::ProcessIdentityMismatch(_))
        ));

        // 3. Matching Mono session mutates field and restores original value on stop.
        let sid = router
            .start_session(unity_request(
                UnityFlavor::Mono,
                UnityLoaderKind::BepinexMono,
                port,
            ))
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.runtime_kind, RuntimeKind::UnityBridgeV1);

        server_thread.join().unwrap();
        let log = observed_ops.lock().unwrap();
        assert_eq!(log.len(), 4);
        assert_eq!(log[2].0, "set_field");
        assert_eq!(log[2].1["value"], json!(0.0));
        assert_eq!(log[3].0, "set_field");
        assert_eq!(log[3].1["value"], json!(1.0));
    }

    #[test]
    fn unity_explorer_mcp_http_discovers_pid_port_verifies_il2cpp_and_rolls_back() {
        use std::io::Read;
        use std::time::{SystemTime, UNIX_EPOCH};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_ue_mcp_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();
        fs::write(
            discovery_dir.join("bridge-10456.json"),
            serde_json::json!({
                "pid": 10456,
                "port": port,
                "backend": "IL2CPP",
                "loader": "BepInEx",
                "unity_version": "2022.3.51f1"
            })
            .to_string(),
        )
        .unwrap();

        let observed = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let observed_clone = Arc::clone(&observed);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(6) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_len = 0usize;
                let mut line = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end_matches(['\r', '\n']);
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = trimmed.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        content_len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_len];
                reader.read_exact(&mut body).unwrap();
                let req: Value = serde_json::from_slice(&body).unwrap();
                let method = req["method"].as_str().unwrap().to_owned();
                observed_clone
                    .lock()
                    .unwrap()
                    .push((method.clone(), req["params"].clone()));

                let resp_json = match method.as_str() {
                    "ping" => serde_json::json!({
                        "Success": true,
                        "Data": {
                            "Status": "ok",
                            "Backend": "IL2CPP",
                            "Loader": "BepInEx",
                            "Pid": 10456,
                            "Port": port,
                            "UnityVersion": "2022.3.51f1"
                        }
                    })
                    .to_string(),
                    _ => serde_json::json!({ "Success": true, "Data": {} }).to_string(),
                };
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_json.len(),
                    resp_json
                );
                stream.write_all(http_resp.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });

        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "protocol": "unity_explorer_mcp_http_v1",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 0,
            "auto_discover_by_pid": true,
            "discovery_dir": discovery_dir.display().to_string(),
            "features": [
                {
                    "kind": "toggle_field",
                    "feature_id": "infinite-stamina",
                    "class_name": "Game.PlayerStats",
                    "field_name": "s_staminaCostMultiplier",
                    "enabled_value": 0.0,
                    "disabled_value": 1.0
                },
                {
                    "kind": "set_field",
                    "feature_id": "time-scale",
                    "class_name": "UnityEngine.Time",
                    "field_name": "timeScale"
                },
                {
                    "kind": "invoke_method",
                    "feature_id": "heal-player",
                    "class_name": "Game.PlayerStats",
                    "method_name": "RestoreAll"
                }
            ]
        })
        .to_string();

        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, 0);
        req.target_process.pid = 10456;
        req.trainer.features = vec![
            TrainerFeature {
                id: "infinite-stamina".to_owned(),
                name: "Infinite Stamina".to_owned(),
                description: "No stamina drain".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
            TrainerFeature {
                id: "time-scale".to_owned(),
                name: "Time Scale".to_owned(),
                description: "Adjust game speed".to_owned(),
                category: FeatureCategory::Time,
                control_type: FeatureControlType::Float,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
            TrainerFeature {
                id: "heal-player".to_owned(),
                name: "Heal Player".to_owned(),
                description: "Trigger static restore".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Action,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: false,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
        ];
        req.declarative_payload_json = Some(payload);

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router.start_session(req).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "heal-player".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "time-scale".to_owned(),
                    value: FeatureValue::Float(2.0),
                },
            )
            .unwrap();

        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.active_patches, 0);

        server.join().unwrap();
        let _ = fs::remove_dir_all(discovery_dir);

        let ops = observed.lock().unwrap();
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[0].0, "ping");
        assert_eq!(ops[1].0, "set_static");
        assert_eq!(ops[1].1["value"], json!(0.0));
        assert_eq!(ops[2].0, "invoke_static");
        assert_eq!(ops[2].1["method"], "RestoreAll");
        assert_eq!(ops[3].0, "set_static");
        assert_eq!(ops[3].1["value"], json!(2.0));
        // LIFO rollback: UndoStore 1 step for time-scale, then restore infinite-stamina = 1.0
        assert_eq!(ops[4].0, "undo_write");
        assert_eq!(ops[5].0, "set_static");
        assert_eq!(ops[5].1["value"], json!(1.0));
    }

    #[test]
    fn nekod_standalone_unity_bridge_discovers_pid_port_and_token_without_unity_explorer() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_standalone_unity_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();
        fs::write(
            discovery_dir.join("unity-22040.json"),
            serde_json::json!({
                "pid": 22040,
                "port": port,
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "auth_token": token,
                "unity_version": "2022.3.51f1",
                "bridge_version": "0.1.0"
            })
            .to_string(),
        )
        .unwrap();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["token"], token);
                let op = req["op"].as_str().unwrap();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":22040,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
        });

        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "protocol": "nekod_json_line_v1",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 0,
            "auto_discover_by_pid": true,
            "discovery_dir": discovery_dir.display().to_string(),
            "features": [{
                "kind": "toggle_field",
                "feature_id": "infinite-stamina",
                "class_name": "UnityEngine.Time",
                "field_name": "timeScale",
                "enabled_value": 0.0,
                "disabled_value": 1.0
            }]
        })
        .to_string();

        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, 0);
        req.target_process.pid = 22040;
        req.declarative_payload_json = Some(payload);

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router.start_session(req).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.active_patches, 0);

        server.join().unwrap();
        let _ = fs::remove_dir_all(discovery_dir);
    }
}
