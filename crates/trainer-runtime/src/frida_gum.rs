//! Optional isolated Frida Gum Dynamic Instrumentation runtime adapter (`frida_gum_v1`).
//!
//! Disabled by default (`FridaGumRuntime::new()`): requires explicit runtime opt-in
//! (`with_isolated_agent(true)`), `RuntimeNativeModule` + `ProcessWriteMemory` permissions,
//! static JS hook safety checks, and talks strictly to an isolated local helper worker
//! over authenticated loopback JSON-line IPC with deterministic `Interceptor.revert` rollback.

use std::collections::BTreeMap;
use std::sync::Mutex;

use nekod_manifest::{PermissionScope, RuntimeKind};
use nekod_runtime_api::{
    FeatureCommand, FeatureValue, ProcessIdentity, RuntimeAdapterClass, RuntimeAdapterMetadata,
    RuntimeDiagnostics, RuntimeError, RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId,
    SessionLifecycleState, StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::bridge_ipc::{
    BridgeRpcRequest, BridgeSessionStore, ManagedBridgeSession, send_bridge_rpc,
    validate_loopback_bridge,
};
use crate::session::validate_session_preflight_for_kind;

const FORBIDDEN_FRIDA_JS_MARKERS: &[&str] = &[
    "child_process",
    "createprocess",
    "winexec",
    "shellexecute",
    "require(",
    "eval(",
    "new function",
    "fetch(",
    "xmlhttprequest",
    "websocket",
    "http://",
    "https://",
    "ws://",
    "wss://",
    "file://",
];

/// Entrypoint JSON payload for `frida_gum_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FridaGumPayload {
    pub schema_version: String,
    pub expected_gum_version: String,
    pub host: String,
    pub port: u16,
    pub auth_token: String,
    pub hooks: Vec<FridaHookBinding>,
}

/// Declarative Frida Gum interceptor binding mapped from a UI `FeatureCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FridaHookBinding {
    InterceptorAttach {
        feature_id: String,
        module_name: String,
        symbol_or_offset: String,
        #[serde(default)]
        js_on_enter: Option<String>,
        #[serde(default)]
        js_on_leave: Option<String>,
    },
    InterceptorReplace {
        feature_id: String,
        module_name: String,
        symbol_or_offset: String,
        js_handler: String,
    },
}

impl FridaHookBinding {
    fn feature_id(&self) -> &str {
        match self {
            Self::InterceptorAttach { feature_id, .. }
            | Self::InterceptorReplace { feature_id, .. } => feature_id,
        }
    }

    fn module_name(&self) -> &str {
        match self {
            Self::InterceptorAttach { module_name, .. }
            | Self::InterceptorReplace { module_name, .. } => module_name,
        }
    }

    fn symbol_or_offset(&self) -> &str {
        match self {
            Self::InterceptorAttach {
                symbol_or_offset, ..
            }
            | Self::InterceptorReplace {
                symbol_or_offset, ..
            } => symbol_or_offset,
        }
    }
}

/// Optional isolated Dynamic Instrumentation Runtime adapter (`frida_gum_v1`).
pub struct FridaGumRuntime {
    isolated_agent_enabled: bool,
    store: BridgeSessionStore,
    bindings: Mutex<BTreeMap<String, FridaGumPayload>>,
}

impl Default for FridaGumRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl FridaGumRuntime {
    /// Creates `FridaGumRuntime` in its default-disabled posture (fails closed on attach).
    pub fn new() -> Self {
        Self::with_isolated_agent(false)
    }

    /// Creates `FridaGumRuntime` with explicit control over whether isolated dynamic hooking
    /// sessions are permitted by host configuration.
    pub fn with_isolated_agent(isolated_agent_enabled: bool) -> Self {
        Self {
            isolated_agent_enabled,
            store: BridgeSessionStore::new(),
            bindings: Mutex::new(BTreeMap::new()),
        }
    }

    fn validate_js_snippet(snippet: &str) -> Result<(), RuntimeError> {
        let lower = snippet.to_ascii_lowercase();
        for forbidden in FORBIDDEN_FRIDA_JS_MARKERS {
            if lower.contains(forbidden) {
                return Err(RuntimeError::InvalidPayload(format!(
                    "frida_gum_v1 rejected unsafe JS primitive '{forbidden}'"
                )));
            }
        }
        Ok(())
    }

    fn validate_payload(payload: &FridaGumPayload) -> Result<std::net::SocketAddr, RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported frida_gum_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.expected_gum_version.trim().is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "frida_gum_v1 requires explicit expected_gum_version".to_owned(),
            ));
        }
        if payload.hooks.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "frida_gum_v1 payload must declare at least one hook".to_owned(),
            ));
        }

        for hook in &payload.hooks {
            if hook.feature_id().trim().is_empty() {
                return Err(RuntimeError::InvalidPayload(
                    "frida_gum_v1 hook feature_id must be non-empty".to_owned(),
                ));
            }
            let module = hook.module_name().trim();
            if module.is_empty()
                || module.contains("..")
                || module.contains('/')
                || module.contains('\\')
            {
                return Err(RuntimeError::InvalidPayload(format!(
                    "invalid frida_gum_v1 module_name '{module}'"
                )));
            }
            if hook.symbol_or_offset().trim().is_empty() {
                return Err(RuntimeError::InvalidPayload(format!(
                    "frida_gum_v1 hook '{}' missing symbol_or_offset",
                    hook.feature_id()
                )));
            }
            match hook {
                FridaHookBinding::InterceptorAttach {
                    js_on_enter,
                    js_on_leave,
                    ..
                } => {
                    if js_on_enter.is_none() && js_on_leave.is_none() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "interceptor_attach '{}' must provide js_on_enter or js_on_leave",
                            hook.feature_id()
                        )));
                    }
                    if let Some(code) = js_on_enter {
                        Self::validate_js_snippet(code)?;
                    }
                    if let Some(code) = js_on_leave {
                        Self::validate_js_snippet(code)?;
                    }
                }
                FridaHookBinding::InterceptorReplace { js_handler, .. } => {
                    if js_handler.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "interceptor_replace '{}' requires non-empty js_handler",
                            hook.feature_id()
                        )));
                    }
                    Self::validate_js_snippet(js_handler)?;
                }
            }
        }

        validate_loopback_bridge(&payload.host, payload.port, &payload.auth_token)
    }
}

impl RuntimeProvider for FridaGumRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::FridaGumV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "frida_gum_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::DynamicInstrumentation,
            runtime_kind: RuntimeKind::FridaGumV1,
            lifecycle_mode: RuntimeLifecycleMode::HookSession,
            manages_disk_files: false,
            requires_process_attach: true,
        }
    }

    fn discover_matching_processes(
        &self,
        _expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Ok(Vec::new())
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        if !self.isolated_agent_enabled {
            return Err(RuntimeError::UntrustedRuntimeBlocked(
                RuntimeKind::FridaGumV1,
                req.granted_permissions.trust_tier,
            ));
        }

        validate_session_preflight_for_kind(&req, RuntimeKind::FridaGumV1)?;

        if !req
            .granted_permissions
            .allows(PermissionScope::RuntimeNativeModule)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::RuntimeNativeModule,
            ));
        }
        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload(
                "frida_gum_v1 requires entrypoint JSON payload in StartSessionRequest".to_owned(),
            )
        })?;
        let payload: FridaGumPayload = serde_json::from_str(raw)
            .map_err(|e| RuntimeError::InvalidPayload(format!("invalid frida_gum_v1 JSON: {e}")))?;
        let endpoint = Self::validate_payload(&payload)?;

        if let Some(constraints) = req
            .granted_permissions
            .constraints_for(PermissionScope::ProcessWriteMemory)
            && let Some(allowed_modules) = &constraints.allowed_modules
        {
            for hook in &payload.hooks {
                if !allowed_modules
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(hook.module_name()))
                {
                    return Err(RuntimeError::PermissionConstraintViolation(format!(
                        "module '{}' is not in granted allowed_modules for frida_gum_v1",
                        hook.module_name()
                    )));
                }
            }
        }

        // Verify isolated worker is alive, matches target PID, and matches Frida Gum version.
        let ping = send_bridge_rpc(
            endpoint,
            &BridgeRpcRequest {
                token: payload.auth_token.clone(),
                op: "ping".to_owned(),
                params: json!({
                    "target_pid": req.target_process.pid,
                    "expected_gum_version": payload.expected_gum_version,
                }),
            },
        )?;
        let reported_pid = ping.data.get("pid").and_then(|v| v.as_u64());
        if reported_pid != Some(u64::from(req.target_process.pid)) {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "isolated frida worker reported pid {:?}, expected {}",
                reported_pid, req.target_process.pid
            )));
        }
        let reported_ver = ping
            .data
            .get("gum_version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if reported_ver != payload.expected_gum_version {
            return Err(RuntimeError::SignatureMismatch(format!(
                "Frida Gum version mismatch: expected '{}', worker reported '{reported_ver}'",
                payload.expected_gum_version
            )));
        }

        let sid = self.store.insert(
            "frida",
            ManagedBridgeSession {
                endpoint,
                auth_token: payload.auth_token.clone(),
                target_process: req.target_process.clone(),
                runtime_kind: RuntimeKind::FridaGumV1,
                state: SessionLifecycleState::Attached,
                rollback_stack: Vec::new(),
                rollback_failures: 0,
                recent_events: vec![format!(
                    "Attached isolated Frida Gum v{} session for pid {}",
                    payload.expected_gum_version, req.target_process.pid
                )],
            },
        );
        self.bindings.lock().unwrap().insert(sid.0.clone(), payload);
        Ok(sid)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let hook = {
            let guard = self.bindings.lock().unwrap();
            let payload = guard
                .get(&session_id.0)
                .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
            payload
                .hooks
                .iter()
                .find(|h| h.feature_id() == command.feature_id)
                .cloned()
                .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?
        };

        self.store.with_session_mut(session_id, |session| {
            let token = session.auth_token.clone();
            match command.value {
                FeatureValue::Toggle(false) => {
                    let revert_req = BridgeRpcRequest {
                        token,
                        op: "revert_hook".to_owned(),
                        params: json!({
                            "feature_id": command.feature_id,
                            "module_name": hook.module_name(),
                            "symbol_or_offset": hook.symbol_or_offset(),
                        }),
                    };
                    send_bridge_rpc(session.endpoint, &revert_req)?;
                    if let Some(pos) = session
                        .rollback_stack
                        .iter()
                        .rposition(|(fid, _)| fid == &command.feature_id)
                    {
                        session.rollback_stack.remove(pos);
                    }
                    session
                        .recent_events
                        .push(format!("Reverted Frida hook '{}'", command.feature_id));
                    Ok(())
                }
                FeatureValue::Toggle(true) | FeatureValue::Action => {
                    let (install_params, op_name) = match &hook {
                        FridaHookBinding::InterceptorAttach {
                            module_name,
                            symbol_or_offset,
                            js_on_enter,
                            js_on_leave,
                            ..
                        } => (
                            json!({
                                "feature_id": command.feature_id,
                                "module_name": module_name,
                                "symbol_or_offset": symbol_or_offset,
                                "js_on_enter": js_on_enter,
                                "js_on_leave": js_on_leave,
                            }),
                            "install_attach_hook",
                        ),
                        FridaHookBinding::InterceptorReplace {
                            module_name,
                            symbol_or_offset,
                            js_handler,
                            ..
                        } => (
                            json!({
                                "feature_id": command.feature_id,
                                "module_name": module_name,
                                "symbol_or_offset": symbol_or_offset,
                                "js_handler": js_handler,
                            }),
                            "install_replace_hook",
                        ),
                    };

                    send_bridge_rpc(
                        session.endpoint,
                        &BridgeRpcRequest {
                            token: token.clone(),
                            op: op_name.to_owned(),
                            params: install_params,
                        },
                    )?;

                    session.rollback_stack.push((
                        command.feature_id.clone(),
                        BridgeRpcRequest {
                            token,
                            op: "revert_hook".to_owned(),
                            params: json!({
                                "feature_id": command.feature_id,
                                "module_name": hook.module_name(),
                                "symbol_or_offset": hook.symbol_or_offset(),
                            }),
                        },
                    ));
                    session
                        .recent_events
                        .push(format!("Installed Frida hook '{}'", command.feature_id));
                    Ok(())
                }
                _ => Err(RuntimeError::InvalidPayload(format!(
                    "frida_gum_v1 feature '{}' requires Toggle or Action",
                    command.feature_id
                ))),
            }
        })
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let diag = self.store.stop_and_rollback(session_id)?;
        self.bindings.lock().unwrap().remove(&session_id.0);
        Ok(diag)
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionConstraints, PermissionDeclaration, PermissionScope,
        RuntimeRequirement, SupportedGameVersion, TargetGameRef, TargetPlatform, TrainerArtifact,
        TrainerCompatibility, TrainerFeature, TrainerManifest, TrustTier,
    };
    use nekod_runtime_api::GrantedPermissions;

    use super::*;

    fn sample_frida_manifest(
        token: &str,
        port: u16,
        js_on_enter: &str,
    ) -> (TrainerManifest, String) {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "expected_gum_version": "16.5.9",
            "host": "127.0.0.1",
            "port": port,
            "auth_token": token,
            "hooks": [{
                "kind": "interceptor_attach",
                "feature_id": "god-mode",
                "module_name": "Game.exe",
                "symbol_or_offset": "0x142A0",
                "js_on_enter": js_on_enter
            }]
        })
        .to_string();

        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.frida-test".to_owned(),
            name: "Frida Test".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Frida dynamic instrumentation test".to_owned(),
            license: "MIT".to_owned(),
            authors: vec![AuthorInfo {
                name: "Test".to_owned(),
                github: None,
                url: None,
            }],
            homepage: None,
            source_repository: "https://github.com/example/frida-test".to_owned(),
            target_game: TargetGameRef {
                game_id: "sample-game".to_owned(),
                game_name: "Sample Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: RuntimeRequirement {
                kind: RuntimeKind::FridaGumV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "hooks/main.json".to_owned(),
                supported_platforms: vec![TargetPlatform::WindowsX64],
            },
            permissions: vec![
                PermissionDeclaration {
                    scope: PermissionScope::ProcessDiscover,
                    reason: "Identify process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Hook function".to_owned(),
                    required: true,
                    constraints: Some(PermissionConstraints {
                        max_write_bytes: None,
                        allowed_modules: Some(vec!["Game.exe".to_owned()]),
                        path_scope: None,
                    }),
                },
                PermissionDeclaration {
                    scope: PermissionScope::RuntimeNativeModule,
                    reason: "Dynamic Gum interceptor".to_owned(),
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
            features: vec![TrainerFeature {
                id: "god-mode".to_owned(),
                name: "God Mode".to_owned(),
                description: "No damage".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::High,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "hooks/main.json".to_owned(),
                sha256: "a".repeat(64),
                size_bytes: payload.len() as u64,
                platform: None,
            }],
            extensions: None,
        };
        (manifest, payload)
    }

    fn grants_for(manifest: &TrainerManifest) -> GrantedPermissions {
        let mut scopes = BTreeSet::new();
        scopes.insert(PermissionScope::ProcessDiscover);
        scopes.insert(PermissionScope::ProcessWriteMemory);
        scopes.insert(PermissionScope::RuntimeNativeModule);
        let mut constraints = BTreeMap::new();
        constraints.insert(
            PermissionScope::ProcessWriteMemory,
            PermissionConstraints {
                max_write_bytes: None,
                allowed_modules: Some(vec!["Game.exe".to_owned()]),
                path_scope: None,
            },
        );
        GrantedPermissions {
            trainer_id: manifest.id.clone(),
            trainer_version: manifest.version.to_string(),
            manifest_sha256: "b".repeat(64),
            trust_tier: TrustTier::CommunityVerified,
            granted_scopes: scopes,
            scope_constraints: constraints,
        }
    }

    fn process_identity(pid: u32) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            process_name: "Game.exe".to_owned(),
            executable_path: "C:\\Games\\Sample\\Game.exe".to_owned(),
            platform: TargetPlatform::WindowsX64,
            architecture: BinaryArch::X86_64,
            executable_sha256: None,
            detected_version: None,
            process_creation_time_100ns: Some(12345),
        }
    }

    #[test]
    fn frida_gum_is_disabled_by_default_and_blocks_unsafe_js_and_rolls_back_when_opted_in() {
        let token = "nekod_frida_isolated_token_123456";
        let (manifest_ok, payload_ok) =
            sample_frida_manifest(token, 1, "this.keep = args[0].toInt32();");

        // 1. Default-disabled posture fails closed.
        let disabled_runtime = FridaGumRuntime::new();
        let blocked = disabled_runtime.start_session(StartSessionRequest {
            trainer: manifest_ok.clone(),
            target_process: process_identity(4242),
            granted_permissions: grants_for(&manifest_ok),
            declarative_payload_json: Some(payload_ok.clone()),
        });
        assert!(matches!(
            blocked,
            Err(RuntimeError::UntrustedRuntimeBlocked(
                RuntimeKind::FridaGumV1,
                TrustTier::CommunityVerified
            ))
        ));

        // 2. Opted-in isolated worker executes ping -> install -> LIFO revert.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let recorded = Arc::new(Mutex::new(Vec::<String>::new()));
        let rec_clone = Arc::clone(&recorded);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let v: serde_json::Value = serde_json::from_str(&line).unwrap();
                let op = v.get("op").and_then(|o| o.as_str()).unwrap().to_owned();
                rec_clone.lock().unwrap().push(op.clone());
                let reply = match op.as_str() {
                    "ping" => "{\"ok\":true,\"data\":{\"pid\":4242,\"gum_version\":\"16.5.9\"}}",
                    _ => "{\"ok\":true,\"data\":{}}",
                };
                writeln!(stream, "{reply}").unwrap();
            }
        });

        let enabled_runtime = FridaGumRuntime::with_isolated_agent(true);

        // Unsafe JS (`child_process`) is rejected statically before connecting.
        let (bad_manifest, bad_payload) = sample_frida_manifest(
            token,
            port,
            "const cp = require('child_process'); cp.exec('calc');",
        );
        let unsafe_res = enabled_runtime.start_session(StartSessionRequest {
            trainer: bad_manifest.clone(),
            target_process: process_identity(4242),
            granted_permissions: grants_for(&bad_manifest),
            declarative_payload_json: Some(bad_payload),
        });
        assert!(matches!(unsafe_res, Err(RuntimeError::InvalidPayload(_))));

        // Clean attach, hook install, and LIFO revert on stop.
        let (good_manifest, good_payload) =
            sample_frida_manifest(token, port, "this.hp = args[0].toInt32();");
        let sid = enabled_runtime
            .start_session(StartSessionRequest {
                trainer: good_manifest.clone(),
                target_process: process_identity(4242),
                granted_permissions: grants_for(&good_manifest),
                declarative_payload_json: Some(good_payload),
            })
            .unwrap();

        enabled_runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "god-mode".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let diag = enabled_runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::FridaGumV1);
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
        assert_eq!(diag.active_patches, 0);
        assert_eq!(diag.rollback_failures, 0);

        server.join().unwrap();
        assert_eq!(
            *recorded.lock().unwrap(),
            vec!["ping", "install_attach_hook", "revert_hook"]
        );
    }
}
