//! Capability-scoped `wasm_sandbox_v1` host mediation boundary & fuel-limited bridge
//! (ADR-018 / U9).
//!
//! Sandboxed guest handlers never hold raw OS process handles, sockets, or filesystem access.
//! All memory reads and byte patches pass through `SandboxHostBridge`, which checks
//! `GrantedPermissions`, `PermissionConstraints`, fuel budgets, process identity, and records
//! patches into `PatchLedger` for 5-step LIFO rollback.

use std::collections::BTreeMap;
use std::sync::Mutex;

use nekod_manifest::{PermissionScope, RuntimeKind, TrustTier};
use nekod_runtime_api::{
    FeatureCommand, GrantedPermissions, ProcessIdentity, RuntimeDiagnostics, RuntimeError,
    RuntimeProvider, RuntimeSessionId, SessionLifecycleState, StartSessionRequest,
};

use crate::{PatchLedger, ProcessAccessMode, ProcessHandle, ProcessMemoryBackend};

/// Execution resource limits enforced on every sandboxed trainer feature invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxBudget {
    pub max_host_calls: u32,
    pub max_read_bytes_per_call: usize,
}

impl Default for SandboxBudget {
    fn default() -> Self {
        Self {
            max_host_calls: 64,
            max_read_bytes_per_call: 4096,
        }
    }
}

/// Mediated host interface exposed to `wasm_sandbox_v1` guest trainers.
pub struct SandboxHostBridge {
    handle: Box<dyn ProcessHandle>,
    identity: ProcessIdentity,
    grants: GrantedPermissions,
    budget: SandboxBudget,
    host_calls_used: u32,
    ledger: PatchLedger,
    events: Vec<String>,
}

impl SandboxHostBridge {
    /// Constructs a new capability-checked host bridge bound to one target process handle.
    pub fn new(
        handle: Box<dyn ProcessHandle>,
        identity: ProcessIdentity,
        grants: GrantedPermissions,
        budget: SandboxBudget,
    ) -> Self {
        Self {
            handle,
            identity,
            grants,
            budget,
            host_calls_used: 0,
            ledger: PatchLedger::default(),
            events: Vec::new(),
        }
    }

    fn consume_call_budget(&mut self) -> Result<(), RuntimeError> {
        if self.host_calls_used >= self.budget.max_host_calls {
            return Err(RuntimeError::PermissionConstraintViolation(format!(
                "sandbox host call budget exhausted (limit={})",
                self.budget.max_host_calls
            )));
        }
        self.host_calls_used += 1;
        Ok(())
    }

    fn enforce_module_scope(
        &self,
        scope: PermissionScope,
        module_name: &str,
    ) -> Result<(), RuntimeError> {
        if !self.grants.allows(scope) {
            return Err(RuntimeError::PermissionDenied(scope));
        }
        if let Some(constraints) = self.grants.constraints_for(scope)
            && let Some(allowed) = &constraints.allowed_modules
            && !allowed.iter().any(|m| m.eq_ignore_ascii_case(module_name))
        {
            return Err(RuntimeError::PermissionConstraintViolation(format!(
                "sandbox attempted access to module '{module_name}' outside allowed_modules"
            )));
        }
        Ok(())
    }

    /// Reads bounded bytes from a permitted module offset inside the verified target process.
    pub fn host_read_module_memory(
        &mut self,
        module_name: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.consume_call_budget()?;
        self.enforce_module_scope(PermissionScope::ProcessReadMemory, module_name)?;
        if len == 0 || len > self.budget.max_read_bytes_per_call {
            return Err(RuntimeError::PermissionConstraintViolation(format!(
                "sandbox read length {len} exceeds per-call limit {}",
                self.budget.max_read_bytes_per_call
            )));
        }
        self.handle.verify_identity(&self.identity)?;
        let modules = self.handle.list_modules()?;
        let module = modules
            .into_iter()
            .find(|m| m.name.eq_ignore_ascii_case(module_name))
            .ok_or_else(|| {
                RuntimeError::AddressResolutionFailed(format!(
                    "module '{module_name}' not loaded in PID {}",
                    self.identity.pid
                ))
            })?;
        let addr = module.base_address.checked_add(offset).ok_or_else(|| {
            RuntimeError::AddressResolutionFailed("module offset overflow".to_owned())
        })?;
        self.handle.read_memory(addr, len)
    }

    /// Applies a reversible byte patch inside an allowed module, recording it in `PatchLedger`.
    pub fn host_apply_byte_patch(
        &mut self,
        feature_id: &str,
        module_name: &str,
        offset: u64,
        expected_original: Option<&[u8]>,
        patch: &[u8],
    ) -> Result<u64, RuntimeError> {
        self.consume_call_budget()?;
        self.enforce_module_scope(PermissionScope::ProcessWriteMemory, module_name)?;
        if patch.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "sandbox patch_bytes cannot be empty".to_owned(),
            ));
        }
        if let Some(constraints) = self
            .grants
            .constraints_for(PermissionScope::ProcessWriteMemory)
            && let Some(max_bytes) = constraints.max_write_bytes
            && patch.len() as u64 > max_bytes
        {
            return Err(RuntimeError::PermissionConstraintViolation(format!(
                "sandbox write of {} bytes exceeds max_write_bytes ({max_bytes})",
                patch.len()
            )));
        }

        self.handle.verify_identity(&self.identity)?;
        let modules = self.handle.list_modules()?;
        let module = modules
            .into_iter()
            .find(|m| m.name.eq_ignore_ascii_case(module_name))
            .ok_or_else(|| {
                RuntimeError::AddressResolutionFailed(format!(
                    "module '{module_name}' not loaded in PID {}",
                    self.identity.pid
                ))
            })?;
        let addr = module.base_address.checked_add(offset).ok_or_else(|| {
            RuntimeError::AddressResolutionFailed("module offset overflow".to_owned())
        })?;

        let region = self.handle.query_region(addr)?;
        if !region.is_committed || !region.is_writable || !region.contains_range(addr, patch.len())
        {
            return Err(RuntimeError::AddressResolutionFailed(format!(
                "target address 0x{addr:x} is not writable committed memory"
            )));
        }

        let original = self.handle.read_memory(addr, patch.len())?;
        if let Some(expected) = expected_original
            && original.as_slice() != expected
        {
            return Err(RuntimeError::SignatureMismatch(format!(
                "sandbox feature '{feature_id}': bytes at 0x{addr:x} do not match expected_original"
            )));
        }

        self.handle.write_memory(addr, patch)?;
        let patch_id =
            self.ledger
                .record_patch(feature_id, &self.identity, addr, original, patch.to_vec());
        self.events.push(format!(
            "sandbox applied patch #{patch_id} for '{feature_id}' at 0x{addr:x}"
        ));
        Ok(patch_id)
    }

    /// Executes 5-step verified LIFO rollback across all active sandbox patches via `PatchLedger`.
    pub fn rollback_all(&mut self) -> Result<usize, RuntimeError> {
        let report = self.ledger.rollback_all(&*self.handle, &self.identity);
        if !report.conflicts.is_empty() || !report.failures.is_empty() {
            let mut reasons = report.conflicts;
            reasons.extend(report.failures);
            return Err(RuntimeError::RollbackFailed {
                feature_id: "sandbox-session".to_owned(),
                reason: reasons.join("; "),
            });
        }
        Ok(report.restored)
    }

    pub fn active_patch_count(&self) -> usize {
        self.ledger.len()
    }

    pub fn rollback_conflicts(&self) -> usize {
        self.ledger.rollback_conflicts()
    }

    pub fn events(&self) -> &[String] {
        &self.events
    }
}

/// Guest handler callback executed under `SandboxHostBridge` mediation (e.g. from a future
/// Wasmtime guest instance or deterministic Rust-hosted sandbox script).
pub trait SandboxedGuestAdapter: Send + Sync {
    fn invoke_feature(
        &self,
        bridge: &mut SandboxHostBridge,
        command: &FeatureCommand,
    ) -> Result<(), RuntimeError>;
}

/// Capability-enforced `wasm_sandbox_v1` `RuntimeProvider` coordinating guest adapters via `SandboxHostBridge`.
pub struct SandboxedWasmRuntime {
    backend: std::sync::Arc<dyn ProcessMemoryBackend>,
    adapter: std::sync::Arc<dyn SandboxedGuestAdapter>,
    budget: SandboxBudget,
    allow_unverified: bool,
    sessions: Mutex<BTreeMap<String, SandboxHostBridge>>,
    next_id: Mutex<u64>,
}

impl SandboxedWasmRuntime {
    pub fn new(
        backend: std::sync::Arc<dyn ProcessMemoryBackend>,
        adapter: std::sync::Arc<dyn SandboxedGuestAdapter>,
        budget: SandboxBudget,
        allow_unverified: bool,
    ) -> Self {
        Self {
            backend,
            adapter,
            budget,
            allow_unverified,
            sessions: Mutex::new(BTreeMap::new()),
            next_id: Mutex::new(1),
        }
    }
}

impl RuntimeProvider for SandboxedWasmRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::WasmSandboxV1
    }

    fn adapter_metadata(&self) -> nekod_runtime_api::RuntimeAdapterMetadata {
        nekod_runtime_api::RuntimeAdapterMetadata {
            adapter_id: "wasm_sandbox_v1".to_owned(),
            adapter_class: nekod_runtime_api::RuntimeAdapterClass::WasmSandbox,
            runtime_kind: RuntimeKind::WasmSandboxV1,
            lifecycle_mode: nekod_runtime_api::RuntimeLifecycleMode::MemorySession,
            manages_disk_files: false,
            requires_process_attach: true,
        }
    }

    fn discover_matching_processes(
        &self,
        expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        let all = self.backend.enumerate_processes()?;
        Ok(all
            .into_iter()
            .filter(|p| {
                expected_process_names
                    .iter()
                    .any(|n| p.process_name.eq_ignore_ascii_case(n))
            })
            .collect())
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        if req.trainer.runtime.kind != RuntimeKind::WasmSandboxV1 {
            return Err(RuntimeError::InvalidPayload(
                "SandboxedWasmRuntime requires runtime.kind = wasm_sandbox_v1".to_owned(),
            ));
        }
        if !self.allow_unverified && req.granted_permissions.trust_tier == TrustTier::Unverified {
            return Err(RuntimeError::UntrustedRuntimeBlocked(
                RuntimeKind::WasmSandboxV1,
                TrustTier::Unverified,
            ));
        }
        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessDiscover)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessDiscover,
            ));
        }

        let mode = if req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            ProcessAccessMode::ReadWrite
        } else {
            ProcessAccessMode::ReadOnly
        };
        let handle = self.backend.open_process(req.target_process.pid, mode)?;
        handle.verify_identity(&req.target_process)?;

        let bridge = SandboxHostBridge::new(
            handle,
            req.target_process,
            req.granted_permissions,
            self.budget,
        );

        let mut seq = self.next_id.lock().unwrap();
        let id = RuntimeSessionId(format!("wasm-{}", *seq));
        *seq += 1;

        self.sessions.lock().unwrap().insert(id.0.clone(), bridge);
        Ok(id)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let mut guard = self.sessions.lock().unwrap();
        let bridge = guard
            .get_mut(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        self.adapter.invoke_feature(bridge, &command)
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let mut bridge = self
            .sessions
            .lock()
            .unwrap()
            .remove(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        let rollback_res = bridge.rollback_all();
        let active_patches = bridge.active_patch_count();
        let conflicts = bridge.rollback_conflicts();
        let diag = RuntimeDiagnostics {
            session_id: session_id.clone(),
            runtime_kind: RuntimeKind::WasmSandboxV1,
            state: if rollback_res.is_ok() {
                SessionLifecycleState::Stopped
            } else {
                SessionLifecycleState::Faulted
            },
            active_patches,
            rollback_failures: usize::from(rollback_res.is_err()),
            rollback_conflicts: conflicts,
            active_freeze_features: 0,
            target_pid: Some(bridge.identity.pid),
            target_process_name: Some(bridge.identity.process_name.clone()),
            recent_events: bridge.events().to_vec(),
        };
        rollback_res?;
        Ok(diag)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let guard = self.sessions.lock().unwrap();
        let bridge = guard
            .get(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        Ok(RuntimeDiagnostics {
            session_id: session_id.clone(),
            runtime_kind: RuntimeKind::WasmSandboxV1,
            state: SessionLifecycleState::Attached,
            active_patches: bridge.active_patch_count(),
            rollback_failures: 0,
            rollback_conflicts: bridge.rollback_conflicts(),
            active_freeze_features: 0,
            target_pid: Some(bridge.identity.pid),
            target_process_name: Some(bridge.identity.process_name.clone()),
            recent_events: bridge.events().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use nekod_manifest::{PermissionConstraints, TrainerManifest};
    use nekod_runtime_api::FeatureValue;

    use super::*;
    use crate::CompositeRuntimeRouter;
    use crate::backend::mock::MockMemoryBackend;

    struct DemoWasmGuest;

    impl SandboxedGuestAdapter for DemoWasmGuest {
        fn invoke_feature(
            &self,
            bridge: &mut SandboxHostBridge,
            command: &FeatureCommand,
        ) -> Result<(), RuntimeError> {
            match command.feature_id.as_str() {
                "god-mode" => {
                    let bytes = bridge.host_read_module_memory("SampleRPG.exe", 0x10, 2)?;
                    assert_eq!(bytes, vec![0x74, 0x05]);
                    bridge.host_apply_byte_patch(
                        "god-mode",
                        "SampleRPG.exe",
                        0x10,
                        Some(&[0x74, 0x05]),
                        &[0x90, 0x90],
                    )?;
                    Ok(())
                }
                "escape-module" => {
                    bridge.host_apply_byte_patch(
                        "escape-module",
                        "kernel32.dll",
                        0,
                        None,
                        &[0x90],
                    )?;
                    Ok(())
                }
                other => Err(RuntimeError::UnknownFeature(other.to_owned())),
            }
        }
    }

    fn wasm_manifest() -> TrainerManifest {
        let mut manifest: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        manifest.runtime.kind = RuntimeKind::WasmSandboxV1;
        manifest.runtime.entrypoint = "trainer.wasm".to_owned();
        manifest
    }

    #[test]
    fn router_dispatches_wasm_sandbox_enforces_module_scope_and_rolls_back() {
        let mut bytes = vec![0u8; 64];
        bytes[0x10] = 0x74;
        bytes[0x11] = 0x05;

        let backend = Arc::new(MockMemoryBackend::new());
        let img = MockMemoryBackend::sample_windows_process(9001, "SampleRPG.exe", bytes);
        let identity = img.identity.clone();
        backend.insert_process(img);

        let wasm = Arc::new(SandboxedWasmRuntime::new(
            backend.clone(),
            Arc::new(DemoWasmGuest),
            SandboxBudget {
                max_host_calls: 8,
                max_read_bytes_per_call: 64,
            },
            false,
        ));
        let mut router = CompositeRuntimeRouter::with_declarative_default();
        router.register_provider(wasm);

        let mut scopes = BTreeSet::new();
        scopes.insert(PermissionScope::ProcessDiscover);
        scopes.insert(PermissionScope::ProcessReadMemory);
        scopes.insert(PermissionScope::ProcessWriteMemory);
        let mut constraints = BTreeMap::new();
        constraints.insert(
            PermissionScope::ProcessWriteMemory,
            PermissionConstraints {
                max_write_bytes: Some(8),
                allowed_modules: Some(vec!["SampleRPG.exe".to_owned()]),
                path_scope: None,
            },
        );
        constraints.insert(
            PermissionScope::ProcessReadMemory,
            PermissionConstraints {
                max_write_bytes: None,
                allowed_modules: Some(vec!["SampleRPG.exe".to_owned()]),
                path_scope: None,
            },
        );

        let req = StartSessionRequest {
            trainer: wasm_manifest(),
            target_process: identity,
            granted_permissions: GrantedPermissions {
                trainer_id: "dev.example.sample-singleplayer-rpg".to_owned(),
                trainer_version: "1.0.0".to_owned(),
                manifest_sha256: "a".repeat(64),
                trust_tier: TrustTier::CommunityVerified,
                granted_scopes: scopes,
                scope_constraints: constraints,
            },
            declarative_payload_json: None,
        };

        let sid = router.start_session(req).unwrap();

        // Undeclared module write fails closed inside SandboxHostBridge.
        let escape_err = router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "escape-module".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap_err();
        assert!(matches!(
            escape_err,
            RuntimeError::PermissionConstraintViolation(_)
        ));

        // Permitted patch succeeds and modifies bytes cleanly.
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "god-mode".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let patched_view = backend
            .with_process_mut(9001, |p| p.pages[&0x1400_00000].bytes[0x10..0x12].to_vec())
            .unwrap();
        assert_eq!(patched_view, vec![0x90, 0x90]);

        // Stopping session rolls back all sandbox patches via PatchLedger.
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.active_patches, 0);
        let restored = backend
            .with_process_mut(9001, |p| p.pages[&0x1400_00000].bytes[0x10..0x12].to_vec())
            .unwrap();
        assert_eq!(restored, vec![0x74, 0x05]);
    }
}
