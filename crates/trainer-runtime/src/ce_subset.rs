//! Constrained Cheat Engine `.CT` script subset runtime (`ce_subset_v1`).
//!
//! Parses declarative `.CT` Auto-Assembler byte patches and module+offset/pointer records,
//! refuses unsafe scripting constructs (`{$lua}`, `alloc`, `createthread`, `loadlibrary`),
//! and delegates execution and LIFO rollback strictly through `DeclarativeRuntime` & `PatchLedger`.

use std::sync::Arc;

use nekod_manifest::{PermissionScope, RuntimeKind};
use nekod_runtime_api::{
    FeatureCommand, ProcessIdentity, RuntimeAdapterClass, RuntimeAdapterMetadata,
    RuntimeDiagnostics, RuntimeError, RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId,
    StartSessionRequest,
};
use nekod_trainer_sdk::{
    AddressLocator, DeclarativeFeatureEntry, DeclarativeOperation, DeclarativePatchPayload,
    ScalarValueType,
};
use serde::{Deserialize, Serialize};

use crate::DeclarativeRuntime;
use crate::backend::ProcessMemoryBackend;
use crate::session::validate_session_preflight_for_kind;

/// Payload format for `ce_subset_v1` entrypoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeSubsetPayload {
    pub schema_version: String,
    pub features: Vec<CeSubsetFeature>,
}

/// A single CE feature entry backed either by a restricted Auto-Assembler script or
/// a typed address expression (`"Module.exe"+HEX` + optional bottom-up pointer offsets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CeSubsetFeature {
    AutoAssemblerPatch {
        feature_id: String,
        #[serde(default)]
        default_module: Option<String>,
        script: String,
    },
    AddressRecord {
        feature_id: String,
        address_expr: String,
        #[serde(default)]
        offsets: Vec<i64>,
        value_type: ScalarValueType,
        #[serde(default)]
        freeze_interval_ms: Option<u32>,
    },
}

/// Runtime Adapter executing constrained CE `.CT` subsets safely on top of `DeclarativeRuntime`.
pub struct CeScriptRuntime {
    inner: DeclarativeRuntime,
}

impl Default for CeScriptRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CeScriptRuntime {
    pub fn new() -> Self {
        Self {
            inner: DeclarativeRuntime::new(),
        }
    }

    pub fn with_backend(backend: Arc<dyn ProcessMemoryBackend>) -> Self {
        Self {
            inner: DeclarativeRuntime::with_backend(backend),
        }
    }

    /// Compiles a `ce_subset_v1` payload into a `DeclarativePatchPayload` while failing
    /// closed on any Lua/allocation/shellcode directive.
    pub fn compile_to_declarative(
        payload: &CeSubsetPayload,
    ) -> Result<DeclarativePatchPayload, RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported ce_subset_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "ce_subset_v1 payload must define at least one feature".to_owned(),
            ));
        }

        let mut entries = Vec::with_capacity(payload.features.len());
        for feat in &payload.features {
            match feat {
                CeSubsetFeature::AutoAssemblerPatch {
                    feature_id,
                    default_module,
                    script,
                } => {
                    let op = compile_restricted_aa(script, default_module.as_deref())?;
                    entries.push(DeclarativeFeatureEntry {
                        feature_id: feature_id.clone(),
                        operations: vec![op],
                    });
                }
                CeSubsetFeature::AddressRecord {
                    feature_id,
                    address_expr,
                    offsets,
                    value_type,
                    freeze_interval_ms,
                } => {
                    let (module, base_offset) = parse_module_offset(address_expr)?;
                    let locator = if offsets.is_empty() {
                        AddressLocator::ModuleOffset {
                            module,
                            offset: base_offset,
                        }
                    } else {
                        AddressLocator::PointerChain {
                            module,
                            base_offset,
                            offsets: offsets.iter().rev().copied().collect(),
                        }
                    };
                    entries.push(DeclarativeFeatureEntry {
                        feature_id: feature_id.clone(),
                        operations: vec![DeclarativeOperation::WriteValue {
                            locator,
                            value_type: *value_type,
                            value: None,
                            freeze_interval_ms: *freeze_interval_ms,
                        }],
                    });
                }
            }
        }

        let compiled = DeclarativePatchPayload {
            schema_version: "1.0.0"
                .parse()
                .map_err(|e| RuntimeError::InvalidPayload(format!("invalid version: {e}")))?,
            features: entries,
        };
        compiled
            .validate_semantic()
            .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;
        Ok(compiled)
    }
}

impl RuntimeProvider for CeScriptRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::CeSubsetV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "ce_subset_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::DeclarativeCe,
            runtime_kind: RuntimeKind::CeSubsetV1,
            lifecycle_mode: RuntimeLifecycleMode::MemorySession,
            manages_disk_files: false,
            requires_process_attach: true,
        }
    }

    fn discover_matching_processes(
        &self,
        expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        self.inner
            .discover_matching_processes(expected_process_names)
    }

    fn start_session(
        &self,
        mut req: StartSessionRequest,
    ) -> Result<RuntimeSessionId, RuntimeError> {
        validate_session_preflight_for_kind(&req, RuntimeKind::CeSubsetV1)?;

        if req
            .granted_permissions
            .allows(PermissionScope::RuntimeNativeModule)
        {
            return Err(RuntimeError::InvalidPayload(
                "ce_subset_v1 forbids runtime.native_module escalation".to_owned(),
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload("missing ce_subset_v1 JSON payload".to_owned())
        })?;
        let ce_payload: CeSubsetPayload = serde_json::from_str(raw)
            .map_err(|e| RuntimeError::InvalidPayload(format!("invalid ce_subset_v1 JSON: {e}")))?;
        let compiled = Self::compile_to_declarative(&ce_payload)?;
        let compiled_json = serde_json::to_string(&compiled)
            .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;

        // Re-target inner DeclarativeRuntime cleanly after ce_subset_v1 preflight passed.
        req.trainer.runtime.kind = RuntimeKind::DeclarativePatchV1;
        req.declarative_payload_json = Some(compiled_json);
        self.inner.start_session(req)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        self.inner.apply_feature(session_id, command)
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let mut diag = self.inner.stop_session(session_id)?;
        diag.runtime_kind = RuntimeKind::CeSubsetV1;
        Ok(diag)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let mut diag = self.inner.diagnostics(session_id)?;
        diag.runtime_kind = RuntimeKind::CeSubsetV1;
        Ok(diag)
    }
}

fn parse_module_offset(raw: &str) -> Result<(String, u64), RuntimeError> {
    let Some((left, right)) = raw.split_once('+') else {
        return Err(RuntimeError::InvalidPayload(format!(
            "unanchored CE address '{raw}'; expected '\"Module.exe\"+HEX'"
        )));
    };
    let module = left.trim().trim_matches('"').trim().to_owned();
    if module.is_empty() {
        return Err(RuntimeError::InvalidPayload(format!(
            "missing module name in CE address '{raw}'"
        )));
    }
    let hex = right
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let offset = u64::from_str_radix(hex, 16)
        .map_err(|_| RuntimeError::InvalidPayload(format!("invalid hex offset '{right}'")))?;
    Ok((module, offset))
}

fn compile_restricted_aa(
    script: &str,
    default_module: Option<&str>,
) -> Result<DeclarativeOperation, RuntimeError> {
    let lower = script.to_ascii_lowercase();
    for forbidden in [
        "{$lua}",
        "alloc(",
        "createthread(",
        "loadlibrary",
        "fullaccess(",
    ] {
        if lower.contains(forbidden) {
            return Err(RuntimeError::InvalidPayload(format!(
                "ce_subset_v1 refuses unsafe Auto-Assembler directive '{forbidden}'"
            )));
        }
    }

    let upper = script.to_ascii_uppercase();
    let en_idx = upper
        .find("[ENABLE]")
        .ok_or_else(|| RuntimeError::InvalidPayload("missing [ENABLE] section".to_owned()))?;
    let dis_idx = upper
        .find("[DISABLE]")
        .ok_or_else(|| RuntimeError::InvalidPayload("missing [DISABLE] section".to_owned()))?;
    if dis_idx <= en_idx {
        return Err(RuntimeError::InvalidPayload(
            "[DISABLE] must follow [ENABLE]".to_owned(),
        ));
    }

    let enable_sec = &script[en_idx + "[ENABLE]".len()..dis_idx];
    let disable_sec = &script[dis_idx + "[DISABLE]".len()..];

    let (symbol, module, pattern) = parse_aobscan(enable_sec, default_module)?;
    let (en_off, patch_bytes) = extract_db(enable_sec, &symbol)?;
    let (dis_off, orig_bytes) = extract_db(disable_sec, &symbol)?;
    if en_off != dis_off {
        return Err(RuntimeError::InvalidPayload(
            "[ENABLE] and [DISABLE] patch offsets differ".to_owned(),
        ));
    }

    Ok(DeclarativeOperation::BytePatch {
        locator: AddressLocator::AobPattern {
            module,
            pattern,
            match_index: 0,
            offset: en_off,
        },
        expected_original_bytes: Some(orig_bytes),
        patch_bytes,
    })
}

fn parse_aobscan(
    section: &str,
    default_module: Option<&str>,
) -> Result<(String, String, String), RuntimeError> {
    for raw in section.lines() {
        let line = raw.split("//").next().unwrap_or(raw).trim();
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("aobscanmodule(")
            && let Some(inner) = line
                .get("aobscanmodule(".len()..)
                .and_then(|s| s.strip_suffix(')'))
        {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() == 3 {
                return Ok((
                    parts[0].to_owned(),
                    parts[1].trim_matches('"').to_owned(),
                    normalize_aob(parts[2]),
                ));
            }
        }
        if lower.starts_with("aobscan(")
            && let Some(inner) = line
                .get("aobscan(".len()..)
                .and_then(|s| s.strip_suffix(')'))
        {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() == 2 {
                let module = default_module.ok_or_else(|| {
                    RuntimeError::InvalidPayload("aobscan() requires default_module".to_owned())
                })?;
                return Ok((
                    parts[0].to_owned(),
                    module.to_owned(),
                    normalize_aob(parts[1]),
                ));
            }
        }
    }
    Err(RuntimeError::InvalidPayload(
        "ce_subset_v1 requires aobscanmodule(SYM, Module, Pattern)".to_owned(),
    ))
}

fn normalize_aob(raw: &str) -> String {
    raw.split_whitespace()
        .map(|tok| {
            if tok == "?" || tok == "??" || tok == "*" {
                "??".to_owned()
            } else {
                tok.to_ascii_uppercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_db(section: &str, symbol: &str) -> Result<(i64, String), RuntimeError> {
    let mut offset: Option<i64> = None;
    let mut bytes = Vec::new();
    for raw in section.lines() {
        let line = raw.split("//").next().unwrap_or(raw).trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("aobscanmodule(")
            || lower.starts_with("aobscan(")
            || lower.starts_with("registersymbol(")
            || lower.starts_with("unregistersymbol(")
        {
            continue;
        }
        if let Some(label) = line.strip_suffix(':') {
            let l = label.trim();
            if l == symbol {
                offset = Some(0);
            } else if let Some(rest) = l.strip_prefix(symbol) {
                let hex = rest
                    .trim()
                    .trim_start_matches('+')
                    .trim_start_matches("0x")
                    .trim_start_matches("0X");
                let val = i64::from_str_radix(hex, 16).map_err(|_| {
                    RuntimeError::InvalidPayload(format!("invalid label offset '{l}'"))
                })?;
                offset = Some(val);
            }
            continue;
        }
        if offset.is_some()
            && lower.starts_with("db")
            && let Some(rest) = line.get(2..)
        {
            for tok in rest.split(|c: char| c.is_whitespace() || c == ',') {
                let tok = tok.trim();
                if !tok.is_empty() {
                    bytes.push(tok.to_ascii_uppercase());
                }
            }
            continue;
        }
        if offset.is_some() {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported AA instruction '{line}' inside ce_subset_v1"
            )));
        }
    }

    let off = offset
        .ok_or_else(|| RuntimeError::InvalidPayload(format!("missing patch label '{symbol}:'")))?;
    if bytes.is_empty() {
        return Err(RuntimeError::InvalidPayload(
            "missing `db` bytes inside ce_subset_v1".to_owned(),
        ));
    }
    Ok((off, bytes.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionDeclaration, SupportedGameVersion, TargetGameRef,
        TargetPlatform, TrainerArtifact, TrainerCompatibility, TrainerFeature, TrainerManifest,
        TrustTier,
    };
    use nekod_runtime_api::{FeatureValue, GrantedPermissions};

    use crate::CompositeRuntimeRouter;
    use crate::backend::mock::MockMemoryBackend;

    fn sample_ce_request(payload_json: String) -> StartSessionRequest {
        let digest = "a".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.ce-subset".to_owned(),
            name: "CE Subset Test".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Constrained CE .CT runtime test".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-singleplayer-rpg".to_owned(),
                game_name: "Sample Singleplayer RPG".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::CeSubsetV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "ce_subset.json".to_owned(),
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
                    reason: "Find running game process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessReadMemory,
                    reason: "Scan AOB pattern".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Apply reversible CE byte patch".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: vec![TrainerFeature {
                id: "infinite-health".to_owned(),
                name: "Infinite Health".to_owned(),
                description: "NOP health subtraction".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "ce_subset.json".to_owned(),
                sha256: digest.clone(),
                size_bytes: payload_json.len() as u64,
                platform: None,
            }],
            extensions: None,
        };

        let mut granted_scopes = BTreeSet::new();
        granted_scopes.insert(PermissionScope::ProcessDiscover);
        granted_scopes.insert(PermissionScope::ProcessReadMemory);
        granted_scopes.insert(PermissionScope::ProcessWriteMemory);

        StartSessionRequest {
            trainer: manifest.clone(),
            target_process: ProcessIdentity {
                pid: 4100,
                process_name: "SampleRPG.exe".to_owned(),
                executable_path: "C:\\Games\\SampleRPG.exe".to_owned(),
                platform: TargetPlatform::WindowsX64,
                architecture: BinaryArch::X86_64,
                executable_sha256: None,
                detected_version: None,
                process_creation_time_100ns: Some(111_111),
            },
            granted_permissions: GrantedPermissions {
                trainer_id: manifest.id,
                trainer_version: "1.0.0".to_owned(),
                manifest_sha256: digest,
                trust_tier: TrustTier::CommunityVerified,
                granted_scopes,
                scope_constraints: BTreeMap::new(),
            },
            declarative_payload_json: Some(payload_json),
        }
    }

    #[test]
    fn ce_subset_executes_aa_byte_patch_and_rejects_lua_injection() {
        let mut exe_bytes = vec![0u8; 4096];
        exe_bytes[0x200..0x206].copy_from_slice(&[0x29, 0x73, 0x10, 0x8B, 0x43, 0x10]);

        let backend = Arc::new(MockMemoryBackend::new());
        let proc_img = MockMemoryBackend::sample_windows_process(4100, "SampleRPG.exe", exe_bytes);
        let mut proc_img = proc_img;
        proc_img.identity.process_creation_time_100ns = Some(111_111);
        backend.insert_process(proc_img);

        let mut router = CompositeRuntimeRouter::with_declarative(Arc::new(
            DeclarativeRuntime::with_backend(backend.clone()),
        ));
        router.register_provider(Arc::new(CeScriptRuntime::with_backend(backend.clone())));

        let valid_payload = serde_json::json!({
            "schema_version": "1.0.0",
            "features": [{
                "kind": "auto_assembler_patch",
                "feature_id": "infinite-health",
                "script": "[ENABLE]\naobscanmodule(HP, SampleRPG.exe, 29 73 10 8B 43 10)\nHP:\n  db 90 90 90\n[DISABLE]\nHP:\n  db 29 73 10\n"
            }]
        })
        .to_string();

        let sid = router
            .start_session(sample_ce_request(valid_payload))
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let patched = backend
            .with_process_mut(4100, |p| {
                p.pages[&0x1400_00000].bytes[0x200..0x203].to_vec()
            })
            .unwrap();
        assert_eq!(patched, vec![0x90, 0x90, 0x90]);

        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::CeSubsetV1);
        assert_eq!(diag.rollback_conflicts, 0);

        let restored = backend
            .with_process_mut(4100, |p| {
                p.pages[&0x1400_00000].bytes[0x200..0x203].to_vec()
            })
            .unwrap();
        assert_eq!(restored, vec![0x29, 0x73, 0x10]);

        // Unsafe Lua / alloc script must fail closed before opening any process handle.
        let unsafe_payload = serde_json::json!({
            "schema_version": "1.0.0",
            "features": [{
                "kind": "auto_assembler_patch",
                "feature_id": "infinite-health",
                "script": "{$lua}\n[ENABLE]\nalloc(mem, 64)\n[DISABLE]\n"
            }]
        })
        .to_string();
        assert!(matches!(
            router.start_session(sample_ce_request(unsafe_payload)),
            Err(RuntimeError::InvalidPayload(_))
        ));
    }
}
