//! L3 Controlled Trainer Runtime (`declarative_patch_v1`).
//!
//! Enforces `GrantedPermissions` and `PermissionConstraints`, verifies target process
//! identity via `(pid, creation_time_100ns, executable_path)`, executes bounded
//! declarative patch operations (`byte_patch`, `write_value`), and performs safe
//! 5-step LIFO rollback via `PatchLedger`.

pub mod adapter_ipc;
pub mod aob;
pub mod backend;
mod bridge_ipc;
pub mod ce_subset;
pub mod deployment;
pub mod frida_gum;
pub mod ledger;
pub mod linux;
pub mod managed_framework;
pub mod router;
pub mod sandbox;
mod session;
pub mod ue4ss_bridge;
pub mod unity_bridge;
#[cfg(target_os = "windows")]
pub mod win32;

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use nekod_manifest::RuntimeKind;
use nekod_runtime_api::{
    FeatureCommand, ProcessIdentity, RuntimeAdapterClass, RuntimeAdapterMetadata,
    RuntimeDiagnostics, RuntimeError, RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId,
    StartSessionRequest,
};

pub use adapter_ipc::{
    AdapterFeatureBinding, AdapterIpcPayload, AdapterIpcRuntime, AdapterProtocol,
};
pub use backend::{
    MemoryRegionInfo, ModuleInfo, ProcessAccessMode, ProcessHandle, ProcessMemoryBackend,
};
pub use ce_subset::{CeScriptRuntime, CeSubsetFeature, CeSubsetPayload};
pub use deployment::{
    DeploymentError, DeploymentLedger, DeploymentRecord, DeploymentState, UndeployReport,
};
pub use frida_gum::{FridaGumPayload, FridaGumRuntime, FridaHookBinding};
pub use ledger::{PatchLedger, RollbackReport};
pub use managed_framework::{
    FrameworkIpcConfig, ManagedFrameworkFeature, ManagedFrameworkKind, ManagedFrameworkPayload,
    ManagedFrameworkRuntime, ManagedScriptAsset,
};
pub use router::CompositeRuntimeRouter;
pub use sandbox::{SandboxBudget, SandboxHostBridge, SandboxedGuestAdapter, SandboxedWasmRuntime};
pub use session::{check_running_anti_cheat, validate_session_preflight};
pub use ue4ss_bridge::{Ue4ssBridgePayload, Ue4ssBridgeRuntime, Ue4ssFeatureBinding};
pub use unity_bridge::{
    NekodUnityDiscovery, UnityBridgePayload, UnityBridgeProtocol, UnityBridgeRuntime,
    UnityFeatureBinding, UnityFlavor, UnityLoaderKind, resolve_nekod_unity_discovery,
    resolve_unity_explorer_discovery_port,
};

use session::{
    ManagedSession, SessionMap, apply_feature_command, insert_session_id, start_managed_session,
    stop_managed_session,
};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
struct UnsupportedOsBackend;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl ProcessMemoryBackend for UnsupportedOsBackend {
    fn enumerate_processes(&self) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Err(RuntimeError::NotImplemented(
            "process enumeration requires Windows x64 or Linux",
        ))
    }

    fn open_process(
        &self,
        _pid: u32,
        _mode: ProcessAccessMode,
    ) -> Result<Box<dyn ProcessHandle>, RuntimeError> {
        Err(RuntimeError::NotImplemented(
            "process memory attachment requires Windows x64 or Linux",
        ))
    }
}

/// Controlled runtime engine for `declarative_patch_v1` trainers.
pub struct DeclarativeRuntime {
    backend: Arc<dyn ProcessMemoryBackend>,
    active_sessions: Mutex<SessionMap>,
    stopped_diagnostics: Mutex<BTreeMap<String, RuntimeDiagnostics>>,
}

impl std::fmt::Debug for DeclarativeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let active_count = self.active_sessions.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("DeclarativeRuntime")
            .field("active_sessions", &active_count)
            .finish()
    }
}

impl Default for DeclarativeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl DeclarativeRuntime {
    /// Creates a `DeclarativeRuntime` wired to the host OS backend
    /// (`Win32ProcessBackend` on Windows, `LinuxProcBackend` on Linux/SteamOS).
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        let backend: Arc<dyn ProcessMemoryBackend> = Arc::new(win32::Win32ProcessBackend::new());
        #[cfg(target_os = "linux")]
        let backend: Arc<dyn ProcessMemoryBackend> =
            Arc::new(linux::backend::LinuxProcBackend::new());
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let backend: Arc<dyn ProcessMemoryBackend> = Arc::new(UnsupportedOsBackend);

        Self::with_backend(backend)
    }

    /// Creates a `DeclarativeRuntime` backed by an explicit `ProcessMemoryBackend` (real or mock).
    pub fn with_backend(backend: Arc<dyn ProcessMemoryBackend>) -> Self {
        Self {
            backend,
            active_sessions: Mutex::new(BTreeMap::new()),
            stopped_diagnostics: Mutex::new(BTreeMap::new()),
        }
    }

    /// Performs mandatory preflight security checks before any OS process handle is opened.
    pub fn validate_session_preflight(req: &StartSessionRequest) -> Result<(), RuntimeError> {
        session::validate_session_preflight(req)
    }
}

impl Drop for ManagedSession {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.freeze_thread.take() {
            let _ = handle.join();
        }
    }
}

impl RuntimeProvider for DeclarativeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::DeclarativePatchV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "declarative_patch_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::DeclarativeCe,
            runtime_kind: RuntimeKind::DeclarativePatchV1,
            lifecycle_mode: RuntimeLifecycleMode::MemorySession,
            manages_disk_files: false,
            requires_process_attach: true,
        }
    }

    fn discover_matching_processes(
        &self,
        expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        let mut procs = self.backend.enumerate_processes()?;
        if !expected_process_names.is_empty() {
            procs.retain(|p| {
                expected_process_names
                    .iter()
                    .any(|expected| p.process_name.eq_ignore_ascii_case(expected))
            });
        }
        procs.sort_by(|a, b| {
            a.process_name
                .to_ascii_lowercase()
                .cmp(&b.process_name.to_ascii_lowercase())
                .then_with(|| a.pid.cmp(&b.pid))
        });
        Ok(procs)
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        let session_id = insert_session_id();
        let managed = start_managed_session(&*self.backend, session_id.clone(), req)?;
        self.active_sessions
            .lock()
            .unwrap()
            .insert(session_id.0.clone(), managed);
        Ok(session_id)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let mut sessions = self.active_sessions.lock().unwrap();
        let managed = sessions
            .get_mut(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        apply_feature_command(managed, command)
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let removed = self.active_sessions.lock().unwrap().remove(&session_id.0);
        if let Some(mut managed) = removed {
            let diag = stop_managed_session(&mut managed);
            self.stopped_diagnostics
                .lock()
                .unwrap()
                .insert(session_id.0.clone(), diag.clone());
            return Ok(diag);
        }

        if let Some(diag) = self
            .stopped_diagnostics
            .lock()
            .unwrap()
            .get(&session_id.0)
            .cloned()
        {
            return Ok(diag);
        }

        Err(RuntimeError::SessionNotFound(session_id.clone()))
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        if let Some(managed) = self.active_sessions.lock().unwrap().get(&session_id.0) {
            return Ok(managed.inner.lock().unwrap().diagnostics());
        }
        if let Some(diag) = self
            .stopped_diagnostics
            .lock()
            .unwrap()
            .get(&session_id.0)
            .cloned()
        {
            return Ok(diag);
        }
        Err(RuntimeError::SessionNotFound(session_id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::thread;
    use std::time::Duration;

    use nekod_manifest::{
        BinaryArch, PermissionConstraints, PermissionScope, TargetPlatform, TrainerManifest,
        TrustTier,
    };
    use nekod_runtime_api::{
        FeatureValue, GrantedPermissions, PatchActivationState, PatchRecord, ProcessIdentity,
        SessionLifecycleState,
    };

    use super::*;
    use backend::mock::MockMemoryBackend;

    fn sample_trainer_manifest() -> TrainerManifest {
        serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap()
    }

    fn sample_payload_json() -> String {
        include_str!("../../../schemas/examples/declarative-patch.example.json").to_owned()
    }

    fn base_process() -> ProcessIdentity {
        ProcessIdentity {
            pid: 4242,
            process_name: "SampleRPG.exe".to_owned(),
            executable_path: r"C:\Games\SampleRPG.exe".to_owned(),
            platform: TargetPlatform::WindowsX64,
            architecture: BinaryArch::X86_64,
            executable_sha256: None,
            detected_version: Some("1.0.0".to_owned()),
            process_creation_time_100ns: Some(133_500_000_000_000_000),
        }
    }

    fn base_permissions(
        manifest: &TrainerManifest,
        scopes: Vec<PermissionScope>,
    ) -> GrantedPermissions {
        GrantedPermissions {
            trainer_id: manifest.id.clone(),
            trainer_version: manifest.version.to_string(),
            manifest_sha256: "a".repeat(64),
            trust_tier: TrustTier::CommunityVerified,
            granted_scopes: scopes.into_iter().collect(),
            scope_constraints: BTreeMap::new(),
        }
    }

    fn base_request() -> StartSessionRequest {
        let trainer = sample_trainer_manifest();
        let granted_permissions = base_permissions(
            &trainer,
            vec![
                PermissionScope::ProcessDiscover,
                PermissionScope::ProcessReadMemory,
                PermissionScope::ProcessWriteMemory,
            ],
        );

        StartSessionRequest {
            trainer,
            target_process: base_process(),
            granted_permissions,
            declarative_payload_json: Some(sample_payload_json()),
        }
    }

    fn build_mock_backend() -> Arc<MockMemoryBackend> {
        let backend = Arc::new(MockMemoryBackend::new());
        // Prepare SampleRPG.exe at base 0x1400_00000:
        // - offset 0x0100: AOB pattern `29 73 10 8B 43 10` (infinite-health)
        // - offset 0x1234 (4660): 8-byte LE pointer to player struct = 0x1400_02000
        // - at 0x1400_02000 + 16: pointer to inventory = 0x1400_02400
        // - at 0x1400_02400 + 32: i32 gold = 100
        let mut mem = vec![0u8; 0x3000];
        let sig = [0x29, 0x73, 0x10, 0x8B, 0x43, 0x10];
        mem[0x0100..0x0106].copy_from_slice(&sig);

        let base: u64 = 0x1400_00000;
        let ptr1: u64 = base + 0x2000;
        let ptr2: u64 = base + 0x2400;
        mem[0x1234..0x123C].copy_from_slice(&ptr1.to_le_bytes());
        mem[0x2010..0x2018].copy_from_slice(&ptr2.to_le_bytes());
        mem[0x2420..0x2424].copy_from_slice(&100i32.to_le_bytes());

        backend.insert_process(MockMemoryBackend::sample_windows_process(
            4242,
            "SampleRPG.exe",
            mem,
        ));
        backend
    }

    #[test]
    fn blocks_untrusted_native_runtime_before_session_start() {
        let mut req = base_request();
        req.trainer.runtime.kind = RuntimeKind::NativeProviderV1;
        req.trainer
            .permissions
            .push(nekod_manifest::PermissionDeclaration {
                scope: PermissionScope::RuntimeNativeModule,
                reason: "Native module test".to_owned(),
                required: true,
                constraints: None,
            });
        req.granted_permissions.trust_tier = TrustTier::Unverified;
        req.granted_permissions
            .granted_scopes
            .insert(PermissionScope::RuntimeNativeModule);

        let err = DeclarativeRuntime::validate_session_preflight(&req).unwrap_err();
        assert!(matches!(err, RuntimeError::UntrustedRuntimeBlocked(_, _)));
    }

    #[test]
    fn denies_session_without_process_discover_scope() {
        let mut req = base_request();
        req.granted_permissions
            .granted_scopes
            .remove(&PermissionScope::ProcessDiscover);

        let err = DeclarativeRuntime::validate_session_preflight(&req).unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::PermissionDenied(PermissionScope::ProcessDiscover)
        ));
    }

    #[test]
    fn patch_ledger_returns_feature_patches_in_lifo_order() {
        let mut ledger = PatchLedger::default();
        let target = base_process();
        ledger.push(PatchRecord {
            patch_id: 1,
            feature_id: "god-mode".to_owned(),
            target_pid: target.pid,
            process_creation_time_100ns: target.process_creation_time_100ns,
            address: 0x1000,
            original_bytes: vec![0x74, 0x05],
            patched_bytes: vec![0x90, 0x90],
            activation_state: PatchActivationState::Active,
            applied_at_epoch_ms: 1,
        });
        ledger.push(PatchRecord {
            patch_id: 2,
            feature_id: "gold".to_owned(),
            target_pid: target.pid,
            process_creation_time_100ns: target.process_creation_time_100ns,
            address: 0x2000,
            original_bytes: vec![0x01],
            patched_bytes: vec![0x63],
            activation_state: PatchActivationState::Active,
            applied_at_epoch_ms: 2,
        });
        ledger.push(PatchRecord {
            patch_id: 3,
            feature_id: "god-mode".to_owned(),
            target_pid: target.pid,
            process_creation_time_100ns: target.process_creation_time_100ns,
            address: 0x1010,
            original_bytes: vec![0x29],
            patched_bytes: vec![0x90],
            activation_state: PatchActivationState::Active,
            applied_at_epoch_ms: 3,
        });

        let rollback = ledger.take_feature_rollback_order("god-mode");
        let ids: Vec<u64> = rollback.into_iter().map(|r| r.patch_id).collect();
        assert_eq!(ids, vec![3, 1]);
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn executes_aob_patch_pointer_chain_and_rolls_back_cleanly() {
        let backend = build_mock_backend();
        let runtime = DeclarativeRuntime::with_backend(backend.clone());
        let sid = runtime.start_session(base_request()).unwrap();

        // Enable AOB byte_patch (`infinite-health`)
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let h = backend
            .open_process(4242, ProcessAccessMode::ReadOnly)
            .unwrap();
        assert_eq!(
            h.read_memory(0x1400_00100, 6).unwrap(),
            vec![0x90, 0x90, 0x90, 0x8B, 0x43, 0x10]
        );

        // Toggle off -> safe 5-step rollback restores `29 73 10`
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(false),
                },
            )
            .unwrap();
        assert_eq!(
            h.read_memory(0x1400_00100, 6).unwrap(),
            vec![0x29, 0x73, 0x10, 0x8B, 0x43, 0x10]
        );

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
        assert_eq!(diag.active_patches, 0);
        assert_eq!(diag.rollback_conflicts, 0);
        assert_eq!(diag.rollback_failures, 0);
    }

    #[test]
    fn enforces_max_write_bytes_and_allowed_modules_constraints() {
        let backend = build_mock_backend();
        let runtime = DeclarativeRuntime::with_backend(backend);

        let mut req = base_request();
        req.granted_permissions.scope_constraints.insert(
            PermissionScope::ProcessWriteMemory,
            PermissionConstraints {
                max_write_bytes: Some(2), // infinite-health writes 3 bytes!
                allowed_modules: Some(vec!["SampleRPG.exe".to_owned()]),
                path_scope: None,
            },
        );

        let sid = runtime.start_session(req).unwrap();
        let err = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::PermissionConstraintViolation(_)
        ));
        let _ = runtime.stop_session(&sid).unwrap();
    }

    #[test]
    fn detects_rollback_conflict_without_silently_overwriting_modified_bytes() {
        let backend = build_mock_backend();
        let runtime = DeclarativeRuntime::with_backend(backend.clone());
        let sid = runtime.start_session(base_request()).unwrap();

        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        // External actor modifies the patched bytes before trainer disables the feature.
        let rw = backend
            .open_process(4242, ProcessAccessMode::ReadWrite)
            .unwrap();
        rw.write_memory(0x1400_00100, &[0xCC, 0xCC, 0xCC]).unwrap();

        let err = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(false),
                },
            )
            .unwrap_err();
        assert!(matches!(err, RuntimeError::RollbackFailed { .. }));

        // Memory MUST remain `[0xCC, 0xCC, 0xCC]` (not blindly overwritten!)
        assert_eq!(
            rw.read_memory(0x1400_00100, 3).unwrap(),
            vec![0xCC, 0xCC, 0xCC]
        );

        let diag = runtime.diagnostics(&sid).unwrap();
        assert_eq!(diag.rollback_conflicts, 1);
        let _ = runtime.stop_session(&sid).unwrap();
    }

    #[test]
    fn detects_pid_reuse_and_aborts_write() {
        let backend = build_mock_backend();
        let runtime = DeclarativeRuntime::with_backend(backend.clone());
        let sid = runtime.start_session(base_request()).unwrap();

        // Simulate OS terminating pid 4242 and reusing pid 4242 for another process.
        backend.with_process_mut(4242, |img| {
            img.identity.process_creation_time_100ns = Some(999_999_999_999_999_999);
            img.identity.executable_path = r"C:\Windows\System32\notepad.exe".to_owned();
        });

        let err = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap_err();
        assert!(matches!(err, RuntimeError::ProcessIdentityMismatch(_)));

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
    }

    #[test]
    fn rejects_elevated_process_without_auto_elevation() {
        let backend = build_mock_backend();
        backend.with_process_mut(4242, |img| img.requires_elevation = true);

        let runtime = DeclarativeRuntime::with_backend(backend);
        let err = runtime.start_session(base_request()).unwrap_err();
        assert_eq!(err, RuntimeError::ProcessElevatedAccessDenied);
    }

    #[test]
    fn freezes_scalar_value_and_restores_original_on_stop() {
        let backend = build_mock_backend();
        let runtime = DeclarativeRuntime::with_backend(backend.clone());
        let sid = runtime.start_session(base_request()).unwrap();

        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "set-gold".to_owned(),
                    value: FeatureValue::Integer(9999),
                },
            )
            .unwrap();

        let rw = backend
            .open_process(4242, ProcessAccessMode::ReadWrite)
            .unwrap();
        // Reset gold to 1; freeze thread should overwrite it back to 9999 within ~150ms.
        rw.write_memory(0x1400_02420, &1i32.to_le_bytes()).unwrap();
        thread::sleep(Duration::from_millis(180));
        let raw_after_freeze = rw.read_memory(0x1400_02420, 4).unwrap();
        assert_eq!(
            i32::from_le_bytes(raw_after_freeze.try_into().unwrap()),
            9999
        );

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.active_patches, 0);
        assert_eq!(diag.rollback_conflicts, 0);
        // Stop session restores original gold (`100`) cleanly.
        let restored = rw.read_memory(0x1400_02420, 4).unwrap();
        assert_eq!(i32::from_le_bytes(restored.try_into().unwrap()), 100);
    }
}
