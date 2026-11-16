//! Unity Reflection Bridge runtime adapter (`unity_bridge_v1`).
//!
//! Communicates with an authenticated managed loader bridge (BepInEx / MelonLoader)
//! over loopback JSON-line IPC, strictly separating `Mono` vs. `IL2CPP` runtime flavors
//! and restoring modified class fields in LIFO order on session teardown.

use std::path::Path;

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
    validate_loopback_bridge,
};
use crate::session::validate_session_preflight_for_kind;

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
    pub flavor: UnityFlavor,
    pub loader: UnityLoaderKind,
    pub host: String,
    pub port: u16,
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
                let outcome = match game_install_dir {
                    Some(dir)
                        if dep.dependency_id.contains("bepinex")
                            && dir.join("BepInEx").join("core").is_dir() =>
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
                        if dep.dependency_id.contains("melonloader")
                            && dir.join("MelonLoader").is_dir() =>
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
        let endpoint = validate_loopback_bridge(&payload.host, payload.port, &payload.auth_token)?;

        // Perform authenticated handshake verifying reported Unity flavor matches manifest declaration.
        let ping_resp = send_bridge_rpc(
            endpoint,
            &BridgeRpcRequest {
                token: payload.auth_token.clone(),
                op: "ping".to_owned(),
                params: json!({ "expected_flavor": payload.flavor.as_str() }),
            },
        )?;
        let reported_flavor = ping_resp
            .data
            .get("flavor")
            .and_then(Value::as_str)
            .unwrap_or("");
        if reported_flavor != payload.flavor.as_str() {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "unity bridge flavor mismatch: expected '{}', bridge reported '{reported_flavor}'",
                payload.flavor.as_str()
            )));
        }

        let session = ManagedBridgeSession {
            endpoint,
            auth_token: payload.auth_token.clone(),
            target_process: req.target_process,
            runtime_kind: RuntimeKind::UnityBridgeV1,
            state: SessionLifecycleState::Attached,
            rollback_stack: Vec::new(),
            rollback_failures: 0,
            recent_events: vec![format!(
                "Authenticated Unity {} bridge on {endpoint}",
                payload.flavor.as_str()
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
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "set_field".to_owned(),
                            params: json!({
                                "class_name": class_name,
                                "field_name": field_name,
                                "value": target_val
                            }),
                        },
                    )?;
                    if enable {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: session.auth_token.clone(),
                                op: "set_field".to_owned(),
                                params: json!({
                                    "class_name": class_name,
                                    "field_name": field_name,
                                    "value": disabled_value
                                }),
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
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "set_field".to_owned(),
                            params: json!({
                                "class_name": class_name,
                                "field_name": field_name,
                                "value": val
                            }),
                        },
                    )?;
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
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "set_field".to_owned(),
                            params: json!({
                                "class_name": class_name,
                                "field_name": field_name,
                                "value": val
                            }),
                        },
                    )?;
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
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "invoke_method".to_owned(),
                            params: json!({
                                "class_name": class_name,
                                "method_name": method_name
                            }),
                        },
                    )?;
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
}
