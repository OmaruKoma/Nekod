//! Unreal Engine UE4SS Reflection Bridge runtime adapter (`ue4ss_bridge_v1`).
//!
//! Communicates with an authenticated UE4SS bridge plugin over loopback JSON-line IPC
//! to invoke `CheatManager` commands, mutate `UObject` properties, and trigger `UFunction`s
//! with explicit version guardrails and LIFO rollback on session stop.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use nekod_manifest::{PermissionScope, RuntimeDependencySpec, RuntimeKind};
use nekod_runtime_api::{
    DependencyCheckResult, DependencyStatus, FeatureCommand, FeatureValue, ProcessIdentity,
    RuntimeAdapterClass, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId, SessionLifecycleState,
    StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::bridge_ipc::{
    BridgeRpcRequest, BridgeSessionStore, ManagedBridgeSession, send_bridge_rpc,
    validate_loopback_bridge,
};
use crate::session::validate_session_preflight_for_kind;

/// Entrypoint JSON payload for `ue4ss_bridge_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssBridgePayload {
    pub schema_version: String,
    pub engine_version: String,
    pub host: String,
    pub port: u16,
    pub auth_token: String,
    pub features: Vec<Ue4ssFeatureBinding>,
}

/// Declarative Unreal reflection action mapped from UI `FeatureCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ue4ssFeatureBinding {
    CheatManagerCommand {
        feature_id: String,
        enable_command: String,
        #[serde(default)]
        disable_command: Option<String>,
    },
    ToggleUProperty {
        feature_id: String,
        object_class: String,
        property_name: String,
        enabled_value: serde_json::Value,
        disabled_value: serde_json::Value,
    },
    CallUFunction {
        feature_id: String,
        object_class: String,
        function_name: String,
    },
}

impl Ue4ssFeatureBinding {
    fn feature_id(&self) -> &str {
        match self {
            Self::CheatManagerCommand { feature_id, .. }
            | Self::ToggleUProperty { feature_id, .. }
            | Self::CallUFunction { feature_id, .. } => feature_id,
        }
    }
}

/// Controlled Unreal Engine `UE4SS` Reflection Bridge adapter (`ue4ss_bridge_v1`).
pub struct Ue4ssBridgeRuntime {
    store: BridgeSessionStore,
    bindings: Mutex<BTreeMap<String, Ue4ssBridgePayload>>,
}

impl Default for Ue4ssBridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Ue4ssBridgeRuntime {
    pub fn new() -> Self {
        Self {
            store: BridgeSessionStore::new(),
            bindings: Mutex::new(BTreeMap::new()),
        }
    }

    fn validate_payload(payload: &Ue4ssBridgePayload) -> Result<(), RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported ue4ss_bridge_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.engine_version.trim().is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "ue4ss_bridge_v1 requires explicit target engine_version (e.g. '5.3')".to_owned(),
            ));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "ue4ss_bridge_v1 payload must define at least one feature".to_owned(),
            ));
        }
        Ok(())
    }
}

impl RuntimeProvider for Ue4ssBridgeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Ue4ssBridgeV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "ue4ss_bridge_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::UnrealBridge,
            runtime_kind: RuntimeKind::Ue4ssBridgeV1,
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
                let satisfied = game_install_dir.and_then(|dir| {
                    if dep.dependency_id.contains("ue4ss")
                        && (dir.join("ue4ss").join("UE4SS.dll").is_file()
                            || dir.join("UE4SS.dll").is_file())
                    {
                        Some(dir.display().to_string())
                    } else {
                        None
                    }
                });
                DependencyCheckResult {
                    dependency_id: dep.dependency_id.clone(),
                    optional: dep.optional,
                    outcome: match satisfied {
                        Some(path) => DependencyStatus::Satisfied {
                            resolved_location: path,
                        },
                        None => DependencyStatus::Missing {
                            suggested_action: format!(
                                "Stage or install UE4SS '{}' ({}) via RuntimeDependencyManager",
                                dep.dependency_id, dep.version_req
                            ),
                        },
                    },
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
        validate_session_preflight_for_kind(&req, RuntimeKind::Ue4ssBridgeV1)?;

        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload("missing ue4ss_bridge_v1 JSON entrypoint".to_owned())
        })?;
        let payload: Ue4ssBridgePayload = serde_json::from_str(raw).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid ue4ss_bridge_v1 JSON: {e}"))
        })?;
        Self::validate_payload(&payload)?;
        let endpoint = validate_loopback_bridge(&payload.host, payload.port, &payload.auth_token)?;

        let ping = send_bridge_rpc(
            endpoint,
            &BridgeRpcRequest {
                token: payload.auth_token.clone(),
                op: "ping".to_owned(),
                params: json!({ "expected_engine_version": payload.engine_version }),
            },
        )?;
        let reported_ver = ping
            .data
            .get("engine_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if reported_ver != payload.engine_version {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "UE4SS engine version mismatch: expected '{}', bridge reported '{reported_ver}'",
                payload.engine_version
            )));
        }

        let session = ManagedBridgeSession {
            endpoint,
            auth_token: payload.auth_token.clone(),
            target_process: req.target_process,
            runtime_kind: RuntimeKind::Ue4ssBridgeV1,
            state: SessionLifecycleState::Attached,
            rollback_stack: Vec::new(),
            rollback_failures: 0,
            recent_events: vec![format!(
                "Authenticated UE4SS {} bridge on {endpoint}",
                payload.engine_version
            )],
        };

        let sid = self.store.insert("ue4ss", session);
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
                    Ue4ssFeatureBinding::CheatManagerCommand {
                        enable_command,
                        disable_command,
                        ..
                    },
                    FeatureValue::Toggle(enable),
                ) => {
                    let cmd = if enable {
                        enable_command
                    } else {
                        disable_command.clone().unwrap_or(enable_command)
                    };
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "cheat_manager_exec".to_owned(),
                            params: json!({ "command": cmd }),
                        },
                    )?;
                    if enable && let Some(dis) = disable_command {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: session.auth_token.clone(),
                                op: "cheat_manager_exec".to_owned(),
                                params: json!({ "command": dis }),
                            },
                        ));
                    } else if !enable {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                    }
                    session
                        .recent_events
                        .push(format!("UE4SS CheatManager '{cmd}' (enable={enable})"));
                }
                (
                    Ue4ssFeatureBinding::ToggleUProperty {
                        object_class,
                        property_name,
                        enabled_value,
                        disabled_value,
                        ..
                    },
                    FeatureValue::Toggle(enable),
                ) => {
                    let val = if enable {
                        enabled_value
                    } else {
                        disabled_value.clone()
                    };
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "set_uproperty".to_owned(),
                            params: json!({
                                "object_class": object_class,
                                "property_name": property_name,
                                "value": val
                            }),
                        },
                    )?;
                    if enable {
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: session.auth_token.clone(),
                                op: "set_uproperty".to_owned(),
                                params: json!({
                                    "object_class": object_class,
                                    "property_name": property_name,
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
                        "UE4SS UProperty '{object_class}.{property_name}' updated"
                    ));
                }
                (
                    Ue4ssFeatureBinding::CallUFunction {
                        object_class,
                        function_name,
                        ..
                    },
                    FeatureValue::Action,
                ) => {
                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: session.auth_token.clone(),
                            op: "call_ufunction".to_owned(),
                            params: json!({
                                "object_class": object_class,
                                "function_name": function_name
                            }),
                        },
                    )?;
                    session
                        .recent_events
                        .push(format!("UE4SS called '{object_class}::{function_name}'"));
                }
                _ => {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "control type mismatch for UE4SS feature '{}'",
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
    use std::collections::BTreeSet;
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

    fn ue4ss_request(engine_ver: &str, port: u16) -> StartSessionRequest {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "engine_version": engine_ver,
            "host": "127.0.0.1",
            "port": port,
            "auth_token": "ue4ss_secret_token_9876543210",
            "features": [{
                "kind": "cheat_manager_command",
                "feature_id": "god-mode",
                "enable_command": "God",
                "disable_command": "God"
            }]
        })
        .to_string();

        let digest = "b".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.ue4ss-bridge".to_owned(),
            name: "UE4SS Bridge Mod".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "UE4SS reflection bridge test".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-unreal-game".to_owned(),
                game_name: "Sample Unreal Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::Ue4ssBridgeV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "ue4ss_bridge.json".to_owned(),
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
                    reason: "Discover UE process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Invoke CheatManager and mutate UProperties via UE4SS".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: vec![TrainerFeature {
                id: "god-mode".to_owned(),
                name: "God Mode".to_owned(),
                description: "Toggle CheatManager God command".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "ue4ss_bridge.json".to_owned(),
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
                pid: 6200,
                process_name: "SampleUE5-Win64-Shipping.exe".to_owned(),
                executable_path: "C:\\Games\\SampleUE5.exe".to_owned(),
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
    fn ue4ss_bridge_verifies_engine_version_executes_cheat_manager_and_rolls_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let ops_clone = ops.clone();

        let thread_handle = thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                ops_clone.lock().unwrap().push(op.clone());
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"engine_version\":\"5.3\"}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{}}}}").unwrap();
                }
            }
        });

        let router = CompositeRuntimeRouter::with_default_providers();

        // Version mismatch fails before creating session.
        assert!(matches!(
            router.start_session(ue4ss_request("4.27", port)),
            Err(RuntimeError::ProcessIdentityMismatch(_))
        ));

        let sid = router.start_session(ue4ss_request("5.3", port)).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "god-mode".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::Ue4ssBridgeV1);
        assert_eq!(diag.rollback_failures, 0);

        thread_handle.join().unwrap();
        let recorded = ops.lock().unwrap();
        assert_eq!(
            recorded.as_slice(),
            &["ping", "ping", "cheat_manager_exec", "cheat_manager_exec"]
        );
    }
}
