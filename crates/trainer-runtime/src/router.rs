//! Multi-runtime `RuntimeProvider` router dispatching trainer sessions by `RuntimeKind`
//! without altering L1 UI or L2 Core APIs (ADR-018 / U9).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nekod_manifest::RuntimeKind;
use nekod_runtime_api::{
    FeatureCommand, ProcessIdentity, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeProvider, RuntimeSessionId, StartSessionRequest,
};

use crate::{
    AdapterIpcRuntime, CeScriptRuntime, DeclarativeRuntime, FridaGumRuntime,
    ManagedFrameworkRuntime, Ue4ssBridgeRuntime, UnityBridgeRuntime,
};

/// Dispatches trainer runtime sessions across registered `RuntimeProvider` engines
/// keyed by `TrainerManifest.runtime.kind`.
pub struct CompositeRuntimeRouter {
    providers: Vec<(RuntimeKind, Arc<dyn RuntimeProvider>)>,
    session_routes: Mutex<BTreeMap<String, RuntimeKind>>,
}

impl CompositeRuntimeRouter {
    /// Creates a router pre-loaded with `DeclarativeRuntime` for `declarative_patch_v1`.
    pub fn with_declarative_default() -> Self {
        Self::with_declarative(Arc::new(DeclarativeRuntime::new()))
    }

    /// Creates a router pre-loaded with built-in providers (`declarative_patch_v1`,
    /// `ce_subset_v1`, `unity_bridge_v1`, `ue4ss_bridge_v1`, `managed_framework_v1`,
    /// zero-injection `adapter_ipc_v1`, and optional isolated `frida_gum_v1` in its
    /// default-disabled state).
    pub fn with_default_providers() -> Self {
        let mut router = Self::with_declarative_default();
        router.register_provider(Arc::new(CeScriptRuntime::new()));
        router.register_provider(Arc::new(UnityBridgeRuntime::new()));
        router.register_provider(Arc::new(Ue4ssBridgeRuntime::new()));
        router.register_provider(Arc::new(ManagedFrameworkRuntime::new()));
        router.register_provider(Arc::new(AdapterIpcRuntime::new()));
        router.register_provider(Arc::new(FridaGumRuntime::new()));
        router
    }

    /// Creates a router backed by an explicit `declarative_patch_v1` provider.
    pub fn with_declarative(declarative: Arc<dyn RuntimeProvider>) -> Self {
        let kind = declarative.kind();
        Self {
            providers: vec![(kind, declarative)],
            session_routes: Mutex::new(BTreeMap::new()),
        }
    }

    /// Registers an additional `RuntimeProvider` (e.g. `SandboxedWasmRuntime`).
    pub fn register_provider(&mut self, provider: Arc<dyn RuntimeProvider>) {
        let kind = provider.kind();
        if let Some((_, slot)) = self.providers.iter_mut().find(|(k, _)| *k == kind) {
            *slot = provider;
        } else {
            self.providers.push((kind, provider));
        }
    }

    /// Returns adapter lifecycle & capability metadata for a registered `RuntimeKind`.
    pub fn adapter_metadata_for(
        &self,
        kind: RuntimeKind,
    ) -> Result<RuntimeAdapterMetadata, RuntimeError> {
        Ok(self.provider_for(kind)?.adapter_metadata())
    }

    fn provider_for(&self, kind: RuntimeKind) -> Result<&Arc<dyn RuntimeProvider>, RuntimeError> {
        self.providers
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, p)| p)
            .ok_or(match kind {
                RuntimeKind::CeSubsetV1 => {
                    RuntimeError::NotImplemented("ce_subset_v1 provider not registered")
                }
                RuntimeKind::UnityBridgeV1 => {
                    RuntimeError::NotImplemented("unity_bridge_v1 provider not registered")
                }
                RuntimeKind::Ue4ssBridgeV1 => {
                    RuntimeError::NotImplemented("ue4ss_bridge_v1 provider not registered")
                }
                RuntimeKind::FridaGumV1 => {
                    RuntimeError::NotImplemented("frida_gum_v1 provider not registered")
                }
                RuntimeKind::ManagedFrameworkV1 => {
                    RuntimeError::NotImplemented("managed_framework_v1 provider not registered")
                }
                RuntimeKind::WasmSandboxV1 => {
                    RuntimeError::NotImplemented("wasm_sandbox_v1 provider not registered")
                }
                RuntimeKind::AdapterIpcV1 => {
                    RuntimeError::NotImplemented("adapter_ipc_v1 provider not registered")
                }
                RuntimeKind::NativeProviderV1 => {
                    RuntimeError::NotImplemented("native_provider_v1 provider not enabled")
                }
                RuntimeKind::DeclarativePatchV1 => {
                    RuntimeError::NotImplemented("declarative_patch_v1 provider not registered")
                }
            })
    }
}

impl RuntimeProvider for CompositeRuntimeRouter {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::DeclarativePatchV1
    }

    fn discover_matching_processes(
        &self,
        expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        let (_, provider) = self.providers.first().ok_or(RuntimeError::NotImplemented(
            "no runtime providers registered",
        ))?;
        provider.discover_matching_processes(expected_process_names)
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        let kind = req.trainer.runtime.kind;
        let provider = self.provider_for(kind)?;
        let session_id = provider.start_session(req)?;
        self.session_routes
            .lock()
            .unwrap()
            .insert(session_id.0.clone(), kind);
        Ok(session_id)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let kind = *self
            .session_routes
            .lock()
            .unwrap()
            .get(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        self.provider_for(kind)?.apply_feature(session_id, command)
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let kind = self
            .session_routes
            .lock()
            .unwrap()
            .remove(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        self.provider_for(kind)?.stop_session(session_id)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let kind = *self
            .session_routes
            .lock()
            .unwrap()
            .get(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        self.provider_for(kind)?.diagnostics(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekod_runtime_api::{RuntimeAdapterClass, RuntimeLifecycleMode};

    #[test]
    fn router_exposes_distinct_adapter_metadata_per_runtime_kind() {
        let router = CompositeRuntimeRouter::with_default_providers();

        let decl = router
            .adapter_metadata_for(RuntimeKind::DeclarativePatchV1)
            .unwrap();
        assert_eq!(decl.adapter_class, RuntimeAdapterClass::DeclarativeCe);
        assert_eq!(decl.lifecycle_mode, RuntimeLifecycleMode::MemorySession);
        assert!(decl.requires_process_attach);

        let adapter = router
            .adapter_metadata_for(RuntimeKind::AdapterIpcV1)
            .unwrap();
        assert_eq!(
            adapter.adapter_class,
            RuntimeAdapterClass::GameSpecificBridge
        );
        assert_eq!(
            adapter.lifecycle_mode,
            RuntimeLifecycleMode::ExternalProtocolIpc
        );
        assert!(!adapter.requires_process_attach);

        let ce = router
            .adapter_metadata_for(RuntimeKind::CeSubsetV1)
            .unwrap();
        assert_eq!(ce.adapter_class, RuntimeAdapterClass::DeclarativeCe);
        assert_eq!(ce.lifecycle_mode, RuntimeLifecycleMode::MemorySession);
        assert!(ce.requires_process_attach);

        let unity = router
            .adapter_metadata_for(RuntimeKind::UnityBridgeV1)
            .unwrap();
        assert_eq!(unity.adapter_class, RuntimeAdapterClass::UnityBridge);
        assert_eq!(
            unity.lifecycle_mode,
            RuntimeLifecycleMode::ManagedLoaderDeployment
        );
        assert!(unity.manages_disk_files);

        let ue4ss = router
            .adapter_metadata_for(RuntimeKind::Ue4ssBridgeV1)
            .unwrap();
        assert_eq!(ue4ss.adapter_class, RuntimeAdapterClass::UnrealBridge);
        assert_eq!(
            ue4ss.lifecycle_mode,
            RuntimeLifecycleMode::ManagedLoaderDeployment
        );
        assert!(ue4ss.manages_disk_files);

        let frida = router
            .adapter_metadata_for(RuntimeKind::FridaGumV1)
            .unwrap();
        assert_eq!(
            frida.adapter_class,
            RuntimeAdapterClass::DynamicInstrumentation
        );
        assert_eq!(frida.lifecycle_mode, RuntimeLifecycleMode::HookSession);
        assert!(frida.requires_process_attach);

        let framework = router
            .adapter_metadata_for(RuntimeKind::ManagedFrameworkV1)
            .unwrap();
        assert_eq!(
            framework.adapter_class,
            RuntimeAdapterClass::GameSpecificBridge
        );
        assert_eq!(
            framework.lifecycle_mode,
            RuntimeLifecycleMode::ManagedLoaderDeployment
        );
        assert!(framework.manages_disk_files);
    }
}
