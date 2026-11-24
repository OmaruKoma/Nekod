//! Stdio JSON-RPC 2.0 MCP Server implementation backed by `nekod-core`, `nekod-validator`,
//! and `nekod-trainer-sdk`.
//!
//! Enforces strict read-only vs. mutating tool separation, explicit authorization flags for
//! workspace writes and process inspection, and singleplayer / anti-cheat safety refusals.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nekod_core::{NekodAppService, NekodCore};
use nekod_game_detection::current_target_platform;
use nekod_manifest::{
    AuthorInfo, CompatibilityMatrixEntry, CompatibilityStatus, DependencyKind,
    DependencyRedistribution, FeatureCategory, FeatureControlType, FeatureRiskLevel,
    ModCompatibility, ModKind, ModManifest, PackageFormat, PermissionDeclaration, PermissionScope,
    RegistryManifest, RegistryTrainerEntry, RegistryTrainerStatus, RuntimeDependencySpec,
    RuntimeKind, RuntimeRequirement, StorePlatformTag, SupportedGameVersion, TargetGameRef,
    TargetPlatform, TargetStoreIds, TrainerArtifact, TrainerFeature, TrainerManifest,
    TrainerPolicy, validate_relative_path,
};
use nekod_trainer_manager::{CompatibilityEvidence, NewCompatibilityRecord, compute_compatibility};
use nekod_trainer_runtime::{
    UnityBridgePayload, UnityFlavor, UnityLoaderKind, probe_nekod_ue4ss_bridge_readonly,
    probe_nekod_unity_bridge_readonly,
};
use nekod_trainer_sdk::{
    AddressLocator, DeclarativePatchBuilder, ScalarValueType, TrainerManifestBuilder,
    build_trainer_package, sha256_hex_bytes,
};
use nekod_validator::ct_importer::{CtImportOptions, convert_ct_xml_with_diagnostics};
use nekod_validator::{
    validate_declarative_patch_str, validate_game_manifest_str, validate_mod_manifest_str,
    validate_nmod_package_dir, validate_registry_manifest_str, validate_trainer_manifest_str,
    validate_trainer_package_dir,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::unity_metadata::{
    discover_unity_assemblies, inspect_type_in_assembly, list_types_in_assembly,
};
use crate::{McpToolGroup, McpToolSpec, PLANNED_MCP_TOOLS};

const PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_CT_ANALYSIS_BYTES: u64 = 16 * 1024 * 1024;
const DISALLOWED_PACK_EXTENSIONS: &[&str] = &[".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"];

#[derive(Debug, Error)]
pub enum McpError {
    #[error("authorization required: {0}")]
    AuthorizationRequired(String),

    #[error("security policy refusal: {0}")]
    PolicyRefusal(String),

    #[error("invalid tool parameters: {0}")]
    InvalidParams(String),

    #[error("unknown MCP tool '{0}'")]
    UnknownTool(String),

    #[error("execution error: {0}")]
    Execution(String),
}

#[derive(Debug, Clone, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Complete MCP JSON-RPC 2.0 developer server instance bound to an isolated `NekodAppService`.
pub struct NekodMcpServer {
    core: Arc<NekodAppService>,
    data_dir: Option<PathBuf>,
}

impl NekodMcpServer {
    pub fn new(core: Arc<NekodAppService>) -> Self {
        Self {
            core,
            data_dir: None,
        }
    }

    pub fn with_data_dir(core: Arc<NekodAppService>, data_dir: PathBuf) -> Self {
        Self {
            core,
            data_dir: Some(data_dir),
        }
    }

    pub fn core(&self) -> &Arc<NekodAppService> {
        &self.core
    }

    /// Runs the blocking newline-delimited JSON-RPC 2.0 MCP loop on `stdin`/`stdout`.
    pub fn run_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        self.serve_streams(BufReader::new(stdin.lock()), stdout.lock())
    }

    /// Processes newline-delimited JSON-RPC 2.0 messages across arbitrary reader/writer streams.
    pub fn serve_streams<R: BufRead, W: Write>(
        &self,
        mut reader: R,
        mut writer: W,
    ) -> std::io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_json_line(trimmed) {
                writeln!(writer, "{reply}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Parses and dispatches a single JSON-RPC 2.0 message line. Notifications return `None`.
    pub fn handle_json_line(&self, line: &str) -> Option<String> {
        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(err) => {
                let resp = JsonRpcResponse::err(Value::Null, -32700, format!("parse error: {err}"));
                return serde_json::to_string(&resp).ok();
            }
        };

        let Some(id) = req.id.clone() else {
            // JSON-RPC notifications (e.g. `notifications/initialized`) receive no response.
            return None;
        };

        if req.jsonrpc != "2.0" {
            let resp = JsonRpcResponse::err(id, -32600, "jsonrpc field must be '2.0'");
            return serde_json::to_string(&resp).ok();
        }

        let resp = match req.method.as_str() {
            "initialize" => JsonRpcResponse::ok(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "nekod-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "ping" => JsonRpcResponse::ok(id, json!({})),
            "tools/list" => JsonRpcResponse::ok(
                id,
                json!({
                    "tools": PLANNED_MCP_TOOLS.iter().map(tool_metadata_json).collect::<Vec<_>>()
                }),
            ),
            "tools/call" => self.handle_tools_call(id, &req.params),
            other => JsonRpcResponse::err(id, -32601, format!("method not found: {other}")),
        };

        serde_json::to_string(&resp).ok()
    }

    fn handle_tools_call(&self, id: Value, params: &Value) -> JsonRpcResponse {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return JsonRpcResponse::err(id, -32602, "tools/call requires string 'name'");
        };
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));

        match self.dispatch_tool(name, &args) {
            Ok(payload) => {
                let text =
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
                JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": payload,
                        "isError": false
                    }),
                )
            }
            Err(err) => {
                let msg = err.to_string();
                JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": msg }],
                        "isError": true
                    }),
                )
            }
        }
    }

    /// Executes one canonical MCP tool with policy, capability, and authorization checks.
    pub fn dispatch_tool(&self, name: &str, args: &Value) -> Result<Value, McpError> {
        let spec = PLANNED_MCP_TOOLS
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| McpError::UnknownTool(name.to_owned()))?;

        enforce_prohibited_pattern_guard(args)?;
        if spec.requires_explicit_authorization {
            let flag = match name {
                "inspect_authorized_game_process" | "probe_unity_bridge" | "probe_ue4ss_bridge" => {
                    "authorize_process_inspection"
                }
                _ => "authorize_write",
            };
            if args.get(flag).and_then(Value::as_bool) != Some(true) {
                return Err(McpError::AuthorizationRequired(format!(
                    "tool '{name}' requires explicit '{flag}: true' confirmation"
                )));
            }
        }

        match name {
            "list_installed_games" => self.tool_list_installed_games(),
            "get_game_info" => self.tool_get_game_info(args),
            "inspect_game_environment" => self.tool_inspect_game_environment(args),
            "recommend_runtime_adapter" => self.tool_recommend_runtime_adapter(args),
            "inspect_unity_assemblies" => self.tool_inspect_unity_assemblies(args),
            "create_trainer_manifest" => self.tool_create_trainer_manifest(args),
            "validate_trainer_manifest" => self.tool_validate_trainer_manifest(args),
            "inspect_trainer" => self.tool_inspect_trainer(args),
            "generate_trainer_template" => self.tool_generate_trainer_template(args),
            "forge_nmod_package" => self.tool_forge_nmod_package(args),
            "pack_nmod_archive" => self.tool_pack_nmod_archive(args),
            "analyze_ct_file" => self.tool_analyze_ct_file(args),
            "check_game_version" => self.tool_check_game_version(args),
            "validate_compatibility" => self.tool_validate_compatibility(args),
            "verify_forged_mod_package" => self.tool_verify_forged_mod_package(args),
            "inspect_runtime_diagnostics" => self.tool_inspect_runtime_diagnostics(),
            "validate_repository" => self.tool_validate_repository(args),
            "inspect_registry_entry" => self.tool_inspect_registry_entry(args),
            "prepare_trainer_release" => self.tool_prepare_trainer_release(args),
            "inspect_authorized_game_process" => self.tool_inspect_authorized_game_process(args),
            "probe_unity_bridge" => self.tool_probe_unity_bridge(args),
            "probe_ue4ss_bridge" => self.tool_probe_ue4ss_bridge(args),
            "inspect_trainer_state" => self.tool_inspect_trainer_state(args),
            "get_execution_logs" => self.tool_get_execution_logs(args),
            _ => Err(McpError::UnknownTool(name.to_owned())),
        }
    }

    fn tool_list_installed_games(&self) -> Result<Value, McpError> {
        let games = self
            .core
            .list_library_games()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        Ok(json!({ "count": games.len(), "games": games }))
    }

    fn tool_get_game_info(&self, args: &Value) -> Result<Value, McpError> {
        let game_id = req_str(args, "game_id")?;
        let manifests = self
            .core
            .list_game_manifests()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let manifest = manifests.into_iter().find(|g| g.id == game_id);

        let games = self
            .core
            .list_library_games()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let detected = games
            .into_iter()
            .find(|g| g.game.matched_game_id.as_deref() == Some(game_id));

        if manifest.is_none() && detected.is_none() {
            return Err(McpError::Execution(format!(
                "no registered GameManifest or detected install found for '{game_id}'"
            )));
        }

        Ok(json!({
            "game_id": game_id,
            "registered_manifest": manifest,
            "detected_install": detected
        }))
    }

    fn tool_inspect_game_environment(&self, args: &Value) -> Result<Value, McpError> {
        let platform = format!("{:?}", current_target_platform());
        if let Some(install_dir) = opt_str(args, "install_dir") {
            let game_id = opt_str(args, "game_id");
            let env_profile = self.core.inspect_installed_game_environment(install_dir);
            let manifests = self
                .core
                .list_game_manifests()
                .map_err(|e| McpError::Execution(e.to_string()))?;
            let compat = game_id.and_then(|gid| {
                manifests
                    .into_iter()
                    .find(|m| m.id == gid)
                    .map(|m| m.compatibility_policy)
            });
            return Ok(json!({
                "game_id": game_id,
                "host_platform": platform,
                "install_dir": install_dir,
                "install_dir_exists": env_profile.dir_exists,
                "executable_path": Value::Null,
                "executable_exists": false,
                "store_build_id": Value::Null,
                "compatibility_policy": compat,
                "engine_environment": env_profile
            }));
        }

        let game_id = req_str(args, "game_id")?;
        let games = self
            .core
            .list_library_games()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let detected = games
            .iter()
            .find(|g| g.game.matched_game_id.as_deref() == Some(game_id))
            .ok_or_else(|| {
                McpError::Execution(format!(
                    "game '{game_id}' not detected (pass 'install_dir' directly for unregistered games)"
                ))
            })?;

        let install_exists = Path::new(&detected.game.install_dir).is_dir();
        let exe_exists = detected
            .game
            .executable_path
            .as_ref()
            .is_some_and(|p| Path::new(p).is_file());

        let manifests = self
            .core
            .list_game_manifests()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let anti_cheat = manifests
            .iter()
            .find(|m| m.id == game_id)
            .map(|m| m.compatibility_policy.clone());

        let env_profile = self
            .core
            .inspect_installed_game_environment(&detected.game.install_dir);

        Ok(json!({
            "game_id": game_id,
            "host_platform": platform,
            "install_dir": detected.game.install_dir,
            "install_dir_exists": install_exists,
            "executable_path": detected.game.executable_path,
            "executable_exists": exe_exists,
            "store_build_id": detected.game.store_build_id,
            "compatibility_policy": anti_cheat,
            "engine_environment": env_profile
        }))
    }

    fn tool_recommend_runtime_adapter(&self, args: &Value) -> Result<Value, McpError> {
        let game_id = req_str(args, "game_id")?;
        if let Some(gm) = self
            .core
            .list_game_manifests()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|g| g.id == game_id)
            && gm.compatibility_policy.trainer_policy == TrainerPolicy::ProhibitedOnlineCompetitive
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing runtime adapter recommendation for prohibited competitive game '{game_id}'"
            )));
        }

        let install_dir = if let Some(dir) = args.get("install_dir").and_then(Value::as_str) {
            dir.to_owned()
        } else {
            let games = self
                .core
                .list_library_games()
                .map_err(|e| McpError::Execution(e.to_string()))?;
            games
                .into_iter()
                .find(|g| g.game.matched_game_id.as_deref() == Some(game_id))
                .map(|g| g.game.install_dir)
                .ok_or_else(|| {
                    McpError::Execution(format!(
                        "provide 'install_dir' or register/detect game '{game_id}' first"
                    ))
                })?
        };

        let env = self.core.inspect_installed_game_environment(&install_dir);
        let compat_records = self
            .core
            .query_compatibility_matrix(game_id, None, None)
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let primary_adapter = env
            .recommended_adapters
            .first()
            .cloned()
            .unwrap_or_else(|| "declarative_patch_v1".to_owned());

        Ok(json!({
            "game_id": game_id,
            "engine": env.engine,
            "installed_loaders": env.installed_loaders,
            "anti_cheat_files_present": env.anti_cheat_files_present,
            "recommended_adapters": env.recommended_adapters,
            "primary_adapter": primary_adapter,
            "plugin_bucket_strategy": "on_demand_sha256_bucket_plugin",
            "existing_compatibility_records": compat_records
        }))
    }

    fn tool_inspect_unity_assemblies(&self, args: &Value) -> Result<Value, McpError> {
        let mut available_assemblies = Vec::new();
        let dll_path = if let Some(raw_asm) = opt_str(args, "assembly_path") {
            let path = PathBuf::from(raw_asm);
            if !path.is_file() {
                return Err(McpError::InvalidParams(format!(
                    "assembly_path '{}' does not exist",
                    path.display()
                )));
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                available_assemblies.push(name.to_owned());
            }
            path
        } else {
            let install_dir = req_str(args, "install_dir")?;
            let env = self.core.inspect_installed_game_environment(install_dir);
            if !env.dir_exists {
                return Err(McpError::InvalidParams(format!(
                    "install_dir '{}' does not exist",
                    install_dir
                )));
            }
            if !env.anti_cheat_files_present.is_empty() {
                return Err(McpError::PolicyRefusal(format!(
                    "refusing Unity assembly inspection: anti-cheat markers present ({:?})",
                    env.anti_cheat_files_present
                )));
            }

            let dlls = discover_unity_assemblies(Path::new(install_dir));
            if dlls.is_empty() {
                return Err(McpError::InvalidParams(format!(
                    "no Unity Managed/*.dll or BepInEx/interop/*.dll assemblies found in '{install_dir}'"
                )));
            }
            available_assemblies = dlls
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
                .collect();

            if let Some(wanted) = opt_str(args, "assembly_name") {
                dlls.into_iter()
                    .find(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                    })
                    .ok_or_else(|| {
                        McpError::InvalidParams(format!(
                            "assembly '{wanted}' not found; available: {}",
                            available_assemblies.join(", ")
                        ))
                    })?
            } else {
                dlls[0].clone()
            }
        };

        if let Some(type_name) = opt_str(args, "type_name") {
            let detail =
                inspect_type_in_assembly(&dll_path, type_name).map_err(McpError::Execution)?;
            return serde_json::to_value(detail).map_err(|e| McpError::Execution(e.to_string()));
        }

        let filter = opt_str(args, "filter");
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(60) as usize;
        let include_compiler_generated = args
            .get("include_compiler_generated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let overview = list_types_in_assembly(
            &dll_path,
            available_assemblies,
            filter,
            limit,
            include_compiler_generated,
        )
        .map_err(McpError::Execution)?;
        serde_json::to_value(overview).map_err(|e| McpError::Execution(e.to_string()))
    }

    fn tool_forge_nmod_package(&self, args: &Value) -> Result<Value, McpError> {
        let output_dir = PathBuf::from(req_str(args, "output_dir")?);
        let mod_id = opt_str(args, "mod_id")
            .or_else(|| opt_str(args, "trainer_id"))
            .ok_or_else(|| McpError::InvalidParams("missing 'mod_id' (or 'trainer_id')".into()))?;
        let name = req_str(args, "name")?;
        let game_id = req_str(args, "game_id")?;
        let game_name = req_str(args, "game_name")?;
        let source_repo = req_str(args, "source_repository")?;
        let runtime_str = opt_str(args, "runtime_kind").unwrap_or("declarative_patch_v1");
        let mod_kind_str = opt_str(args, "mod_kind").unwrap_or("trainer");
        let module_name = opt_str(args, "module_name").unwrap_or("Game.exe");
        let author_name = opt_str(args, "author_name").unwrap_or("AI Mod Forge Contributor");
        let version_str = opt_str(args, "version").unwrap_or("0.1.0");
        let version = Version::parse(version_str).map_err(|e| {
            McpError::InvalidParams(format!("invalid SemVer version '{version_str}': {e}"))
        })?;
        let flavor_arg = opt_str(args, "flavor");
        let steam_app_id = args.get("steam_app_id").and_then(Value::as_u64);
        let include_legacy_v1 = args
            .get("include_legacy_v1_manifest")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if let Some(gm) = self
            .core
            .list_game_manifests()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|g| g.id == game_id)
            && gm.compatibility_policy.trainer_policy == TrainerPolicy::ProhibitedOnlineCompetitive
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing mod forge for prohibited competitive game '{game_id}'"
            )));
        }

        let runtime_kind: RuntimeKind =
            serde_json::from_value(Value::String(runtime_str.to_owned())).map_err(|e| {
                McpError::InvalidParams(format!("invalid runtime_kind '{runtime_str}': {e}"))
            })?;
        if matches!(
            runtime_kind,
            RuntimeKind::NativeProviderV1 | RuntimeKind::FridaGumV1
        ) {
            return Err(McpError::PolicyRefusal(format!(
                "AI Mod Forge refuses auto-generating '{runtime_str}' packages without manual engineering workflow"
            )));
        }
        let mod_kind: ModKind = serde_json::from_value(Value::String(mod_kind_str.to_owned()))
            .map_err(|e| {
                McpError::InvalidParams(format!("invalid mod_kind '{mod_kind_str}': {e}"))
            })?;

        let custom_features: Vec<TrainerFeature> = if let Some(raw_feats) = args.get("features") {
            serde_json::from_value(raw_feats.clone())
                .map_err(|e| McpError::InvalidParams(format!("invalid 'features' array: {e}")))?
        } else {
            vec![TrainerFeature {
                id: "forged-feature".to_owned(),
                name: "Forged Draft Toggle".to_owned(),
                description:
                    "Generated experimental draft feature (requires runtime rollback test)."
                        .to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: Some("F1".to_owned()),
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }]
        };

        let custom_entrypoint_rel = opt_str(args, "entrypoint");
        let (entrypoint_rel, payload_json) =
            if let Some(custom_payload) = args.get("entrypoint_payload") {
                let rel = custom_entrypoint_rel
                    .unwrap_or_else(|| default_entrypoint_path_for_kind(runtime_kind))
                    .to_owned();
                validate_relative_path(&rel).map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let formatted = serde_json::to_string_pretty(custom_payload)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                if runtime_kind == RuntimeKind::UnityBridgeV1 {
                    let parsed: UnityBridgePayload = serde_json::from_value(custom_payload.clone())
                        .map_err(|e| {
                            McpError::InvalidParams(format!("invalid unity_bridge_v1 payload: {e}"))
                        })?;
                    if parsed.flavor == UnityFlavor::Mono
                        && parsed.loader == UnityLoaderKind::BepinexIl2cpp
                    {
                        return Err(McpError::InvalidParams(
                            "Unity Mono flavor cannot use bepinex_il2cpp loader".to_owned(),
                        ));
                    }
                    if parsed.flavor == UnityFlavor::Il2cpp
                        && parsed.loader == UnityLoaderKind::BepinexMono
                    {
                        return Err(McpError::InvalidParams(
                            "Unity IL2CPP flavor cannot use bepinex_mono loader".to_owned(),
                        ));
                    }
                } else if runtime_kind == RuntimeKind::DeclarativePatchV1 {
                    validate_declarative_patch_str(&formatted)
                        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                }
                (rel, formatted)
            } else {
                build_forged_entrypoint_payload(runtime_kind, module_name, flavor_arg)?
            };

        fs::create_dir_all(&output_dir).map_err(|e| McpError::Execution(e.to_string()))?;
        let entry_path = output_dir.join(&entrypoint_rel);
        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent).map_err(|e| McpError::Execution(e.to_string()))?;
        }
        fs::write(&entry_path, payload_json.as_bytes())
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let entry_sha256 = sha256_hex_bytes(payload_json.as_bytes());
        let entry_size = payload_json.len() as u64;

        let mut wrote_game_manifest = false;
        if let Some(gm_val) = args.get("game_manifest") {
            let raw_gm = match gm_val {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?,
            };
            let validated_gm = validate_game_manifest_str(&raw_gm)
                .map_err(|e| McpError::InvalidParams(format!("invalid game_manifest: {e}")))?;
            if validated_gm.id != game_id {
                return Err(McpError::InvalidParams(format!(
                    "game_manifest.id '{}' must match game_id '{game_id}'",
                    validated_gm.id
                )));
            }
            fs::write(output_dir.join("game-manifest.json"), raw_gm.as_bytes())
                .map_err(|e| McpError::Execution(e.to_string()))?;
            wrote_game_manifest = true;
        }

        let description = opt_str(args, "description")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("Source-first .nmod package targeting {game_name} via {runtime_str}.")
            });

        let dependencies = build_default_dependencies_for_kind(runtime_kind, flavor_arg);
        let store_ids = steam_app_id.map(|aid| TargetStoreIds {
            steam_app_id: Some(aid),
            ..TargetStoreIds::default()
        });

        let mod_v2 = ModManifest {
            schema_version: Version::new(2, 0, 0),
            package_format: PackageFormat::NmodV1,
            mod_kind,
            id: mod_id.to_owned(),
            version,
            name: name.to_owned(),
            description,
            authors: vec![AuthorInfo {
                name: author_name.to_owned(),
                github: opt_str(args, "author_github").map(str::to_owned),
                url: opt_str(args, "author_url").map(str::to_owned),
            }],
            license: "MIT".to_owned(),
            source_repository: source_repo.to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: game_id.to_owned(),
                game_name: game_name.to_owned(),
                store_ids,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Experimental,
            }],
            runtime: RuntimeRequirement {
                kind: runtime_kind,
                api_version: Version::new(1, 0, 0),
                min_nekod_version: Version::parse("0.1.0-alpha.0").unwrap(),
                entrypoint: entrypoint_rel.clone(),
                supported_platforms: vec![TargetPlatform::WindowsX64],
            },
            dependencies,
            permissions: vec![
                PermissionDeclaration {
                    scope: PermissionScope::ProcessDiscover,
                    reason: "Identify target singleplayer game process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessReadMemory,
                    reason: "Inspect target game structures or signatures".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Apply reversible mod changes".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: custom_features,
            compatibility: ModCompatibility {
                singleplayer_only: true,
                requires_offline_mode: false,
                anti_cheat_incompatible: vec![],
                last_verified_at: None,
                compatibility_matrix: vec![CompatibilityMatrixEntry {
                    game_version: "*".to_owned(),
                    store_platform: StorePlatformTag::Any,
                    runtime_adapter: runtime_str.to_owned(),
                    runtime_version: None,
                    status: CompatibilityStatus::Experimental,
                    notes: Some(
                        "Unverified AI Mod Forge draft (requires runtime verification gate)."
                            .into(),
                    ),
                }],
            },
            artifacts: vec![TrainerArtifact {
                filename: entrypoint_rel.clone(),
                sha256: entry_sha256,
                size_bytes: entry_size,
                platform: Some(TargetPlatform::WindowsX64),
            }],
            extensions: None,
        };

        let mod_json = serde_json::to_string_pretty(&mod_v2)
            .map_err(|e| McpError::Execution(e.to_string()))?;
        validate_mod_manifest_str(&mod_json).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        fs::write(output_dir.join("mod-manifest.json"), mod_json.as_bytes())
            .map_err(|e| McpError::Execution(e.to_string()))?;

        if include_legacy_v1 {
            let trainer_v1 = mod_v2.to_trainer_manifest();
            let trainer_json = serde_json::to_string_pretty(&trainer_v1)
                .map_err(|e| McpError::Execution(e.to_string()))?;
            validate_trainer_manifest_str(&trainer_json)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            fs::write(
                output_dir.join("trainer-manifest.json"),
                trainer_json.as_bytes(),
            )
            .map_err(|e| McpError::Execution(e.to_string()))?;
        }

        let (validated_mod, mod_manifest_sha256) = validate_nmod_package_dir(&output_dir)
            .map_err(|e| McpError::InvalidParams(format!("forged .nmod failed check: {e}")))?;

        let nmod_archive_path = if let Some(pack_out) = opt_str(args, "pack_nmod_path") {
            let p = PathBuf::from(pack_out);
            pack_nmod_dir_to_archive(&output_dir, &p)?;
            Some(p)
        } else {
            None
        };

        Ok(json!({
            "mod_id": validated_mod.id,
            "version": validated_mod.version.to_string(),
            "output_dir": output_dir,
            "runtime_kind": runtime_str,
            "feature_count": validated_mod.features.len(),
            "mod_manifest_sha256": mod_manifest_sha256,
            "entrypoint_file": entrypoint_rel,
            "wrote_game_manifest": wrote_game_manifest,
            "wrote_legacy_v1_manifest": include_legacy_v1,
            "nmod_archive_path": nmod_archive_path,
            "declared_status": "experimental"
        }))
    }

    fn tool_pack_nmod_archive(&self, args: &Value) -> Result<Value, McpError> {
        let package_dir = PathBuf::from(req_str(args, "package_dir")?);
        if !package_dir.is_dir() {
            return Err(McpError::InvalidParams(format!(
                "package_dir '{}' is not a directory",
                package_dir.display()
            )));
        }

        if args
            .get("recompute_checksums")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            recompute_mod_manifest_checksums(&package_dir)?;
        }

        let (manifest, _) = validate_nmod_package_dir(&package_dir)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let install_to_local_nmods = args
            .get("install_to_local_nmods")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let safe_id: String = manifest
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let default_filename = format!("{safe_id}-{}.nmod", manifest.version);

        let output_path = if let Some(raw_out) = opt_str(args, "output_nmod_path") {
            PathBuf::from(raw_out)
        } else if install_to_local_nmods {
            let Some(data_dir) = &self.data_dir else {
                return Err(McpError::InvalidParams(
                    "install_to_local_nmods=true requires nekod-mcp --data-dir or explicit output_nmod_path".to_owned(),
                ));
            };
            data_dir.join("nmods").join(&default_filename)
        } else {
            package_dir.with_extension("nmod")
        };

        let (validated, archive_sha256, archive_size_bytes) =
            pack_nmod_dir_to_archive(&package_dir, &output_path)?;

        let mut local_nmods_copy = None;
        if install_to_local_nmods
            && opt_str(args, "output_nmod_path").is_some()
            && let Some(data_dir) = &self.data_dir
        {
            let dest = data_dir.join("nmods").join(&default_filename);
            if dest != output_path {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| McpError::Execution(e.to_string()))?;
                }
                fs::copy(&output_path, &dest).map_err(|e| McpError::Execution(e.to_string()))?;
                local_nmods_copy = Some(dest);
            }
        }

        Ok(json!({
            "packed": true,
            "mod_id": validated.id,
            "version": validated.version.to_string(),
            "feature_count": validated.features.len(),
            "archive_path": output_path,
            "local_nmods_copy": local_nmods_copy,
            "archive_sha256": archive_sha256,
            "archive_size_bytes": archive_size_bytes
        }))
    }

    fn tool_analyze_ct_file(&self, args: &Value) -> Result<Value, McpError> {
        let ct_path = PathBuf::from(req_str(args, "ct_path")?);
        let meta = fs::metadata(&ct_path).map_err(|e| {
            McpError::InvalidParams(format!("cannot access .CT '{}': {e}", ct_path.display()))
        })?;
        if !meta.is_file() {
            return Err(McpError::InvalidParams(format!(
                "'{}' is not a file",
                ct_path.display()
            )));
        }
        if meta.len() > MAX_CT_ANALYSIS_BYTES {
            return Err(McpError::InvalidParams(format!(
                ".CT file '{}' exceeds 16 MiB analysis limit",
                ct_path.display()
            )));
        }
        let raw_xml = fs::read_to_string(&ct_path)
            .map_err(|e| McpError::Execution(format!("failed reading .CT as UTF-8: {e}")))?;

        let game_id = opt_str(args, "game_id").unwrap_or("ct-analysis-target");
        let game_name = opt_str(args, "game_name").unwrap_or("Cheat Engine Target");
        let opts = CtImportOptions {
            trainer_id: format!("dev.ct.{game_id}"),
            trainer_name: format!("{game_name} (.CT Analysis)"),
            game_id: game_id.to_owned(),
            game_name: game_name.to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            default_module: opt_str(args, "default_module").map(str::to_owned),
        };

        let (_, summary) = convert_ct_xml_with_diagnostics(&raw_xml, &opts)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;

        let convertible_count = summary.converted_features.len();
        let skipped_count = summary.skipped_entries.len();
        let recommendation = if convertible_count > 0 && skipped_count == 0 {
            "All entries can convert directly to declarative_patch_v1."
        } else if convertible_count > 0 {
            "Partial static conversion succeeded; rewrite skipped Lua/alloc scripts using Unity Bridge or Declarative AOB patches."
        } else {
            "Zero entries are statically convertible (likely Lua-only or dynamic alloc scripts). If this is a Unity/UE game, inspect engine metadata and forge a managed bridge .nmod instead."
        };

        Ok(json!({
            "ct_path": ct_path,
            "convertible_count": convertible_count,
            "converted_features": summary.converted_features,
            "skipped_count": skipped_count,
            "skipped_entries": summary.skipped_entries,
            "modules": summary.modules,
            "recommendation": recommendation
        }))
    }

    fn tool_verify_forged_mod_package(&self, args: &Value) -> Result<Value, McpError> {
        let package_dir = PathBuf::from(req_str(args, "package_dir")?);
        let game_version = req_str(args, "game_version")?;
        let store_str = args
            .get("store_platform")
            .and_then(Value::as_str)
            .unwrap_or("steam");
        let store_platform: StorePlatformTag =
            serde_json::from_value(Value::String(store_str.to_owned())).map_err(|e| {
                McpError::InvalidParams(format!("invalid store_platform '{store_str}': {e}"))
            })?;
        let runtime_rollback_verified = args
            .get("runtime_rollback_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let (manifest, manifest_sha256) = validate_nmod_package_dir(&package_dir)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let runtime_adapter = serde_json::to_value(manifest.runtime.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{:?}", manifest.runtime.kind));

        let (status, evidence, notes) = if runtime_rollback_verified {
            (
                CompatibilityStatus::Verified,
                CompatibilityEvidence::AutomatedRuntimeTest,
                Some("Passed .nmod verification and automated runtime rollback test.".to_owned()),
            )
        } else {
            (
                CompatibilityStatus::Experimental,
                CompatibilityEvidence::AiDraftUnverified,
                Some(
                    "Static/artifact schema verified only; awaits runtime attach + LIFO rollback test (ADR-019)."
                        .to_owned(),
                ),
            )
        };

        let recorded = self
            .core
            .record_compatibility_result(&NewCompatibilityRecord {
                game_id: manifest.target_game.game_id.clone(),
                game_version: game_version.to_owned(),
                store_platform,
                runtime_adapter,
                runtime_version: None,
                mod_id: manifest.id.clone(),
                mod_version: manifest.version.to_string(),
                status,
                evidence,
                notes,
            })
            .map_err(|e| McpError::Execution(e.to_string()))?;

        Ok(json!({
            "valid": true,
            "mod_id": manifest.id,
            "manifest_sha256": manifest_sha256,
            "recorded_compatibility": recorded
        }))
    }

    fn tool_create_trainer_manifest(&self, args: &Value) -> Result<Value, McpError> {
        let output_dir = PathBuf::from(req_str(args, "output_dir")?);
        let trainer_id = req_str(args, "trainer_id")?;
        let name = req_str(args, "name")?;
        let game_id = req_str(args, "game_id")?;
        let game_name = req_str(args, "game_name")?;
        let source_repo = req_str(args, "source_repository")?;
        let module_name = args
            .get("module_name")
            .and_then(Value::as_str)
            .unwrap_or("Game.exe");
        let author_name = args
            .get("author_name")
            .and_then(Value::as_str)
            .unwrap_or("Community Contributor");
        let version_str = args
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("1.0.0");
        let version = Version::parse(version_str)
            .map_err(|e| McpError::InvalidParams(format!("invalid semver '{version_str}': {e}")))?;

        let entrypoint_filename = args
            .get("entrypoint")
            .and_then(Value::as_str)
            .unwrap_or("trainer.json");
        let entrypoint_path = output_dir.join(entrypoint_filename);
        let artifact_entry = if entrypoint_path.is_file() {
            let bytes = fs::read(&entrypoint_path)
                .map_err(|e| McpError::Execution(format!("failed to read entrypoint: {e}")))?;
            Some((sha256_hex_bytes(&bytes), bytes.len() as u64))
        } else {
            None
        };

        let mut builder =
            TrainerManifestBuilder::new(trainer_id, name, game_id, game_name, source_repo)
                .version(version)
                .author(author_name, None, None)
                .entrypoint(entrypoint_filename)
                .default_memory_permissions(vec![module_name.to_owned()], 64)
                .toggle_feature(
                    "sample-toggle",
                    "Sample Toggle",
                    "Author placeholder toggle feature (update before shipping).",
                    FeatureCategory::Player,
                    Some("F1".to_owned()),
                );

        let mut draft = builder.clone().build_draft();
        if let Some((sha, size)) = artifact_entry {
            draft.artifacts.push(nekod_manifest::TrainerArtifact {
                filename: entrypoint_filename.to_owned(),
                sha256: sha,
                size_bytes: size,
                platform: Some(nekod_manifest::TargetPlatform::WindowsX64),
            });
            builder =
                TrainerManifestBuilder::new(trainer_id, name, game_id, game_name, source_repo);
            let _ = builder;
        } else {
            return Err(McpError::InvalidParams(format!(
                "entrypoint '{}' does not exist in '{}'; use generate_trainer_template to scaffold both files together",
                entrypoint_filename,
                output_dir.display()
            )));
        }

        let json_str =
            serde_json::to_string_pretty(&draft).map_err(|e| McpError::Execution(e.to_string()))?;
        let validated = validate_trainer_manifest_str(&json_str)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        fs::create_dir_all(&output_dir).map_err(|e| McpError::Execution(e.to_string()))?;
        let out_path = output_dir.join("trainer-manifest.json");
        fs::write(&out_path, json_str.as_bytes())
            .map_err(|e| McpError::Execution(e.to_string()))?;

        Ok(json!({
            "written_path": out_path,
            "manifest_sha256": sha256_hex_bytes(json_str.as_bytes()),
            "trainer_id": validated.id,
            "version": validated.version.to_string(),
            "requires_human_review": true
        }))
    }

    fn tool_validate_trainer_manifest(&self, args: &Value) -> Result<Value, McpError> {
        if let Some(dir) = opt_str(args, "package_dir") {
            let pkg_path = Path::new(dir);
            let (manifest, digest, manifest_kind) = load_any_package_from_dir(pkg_path)?;
            return Ok(json!({
                "valid": true,
                "mode": "package_dir",
                "manifest_format": manifest_kind,
                "trainer_id": manifest.id,
                "mod_id": manifest.id,
                "version": manifest.version.to_string(),
                "manifest_sha256": digest
            }));
        }

        let raw = if let Some(inline) = opt_str(args, "manifest_json") {
            inline.to_owned()
        } else if let Some(path) = opt_str(args, "manifest_path") {
            fs::read_to_string(path).map_err(|e| McpError::Execution(e.to_string()))?
        } else {
            return Err(McpError::InvalidParams(
                "provide one of 'package_dir', 'manifest_path', or 'manifest_json'".to_owned(),
            ));
        };

        let val: Value =
            serde_json::from_str(&raw).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let is_v2 = val
            .get("schema_version")
            .and_then(Value::as_str)
            .is_some_and(|v| v.starts_with("2."));
        if is_v2 {
            let manifest = validate_mod_manifest_str(&raw)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            return Ok(json!({
                "valid": true,
                "mode": "manifest_only",
                "manifest_format": "mod_manifest_v2",
                "trainer_id": manifest.id,
                "mod_id": manifest.id,
                "version": manifest.version.to_string(),
                "manifest_sha256": sha256_hex_bytes(raw.as_bytes())
            }));
        }

        let manifest = validate_trainer_manifest_str(&raw)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        Ok(json!({
            "valid": true,
            "mode": "manifest_only",
            "manifest_format": "trainer_manifest_v1",
            "trainer_id": manifest.id,
            "version": manifest.version.to_string(),
            "manifest_sha256": sha256_hex_bytes(raw.as_bytes())
        }))
    }

    fn tool_inspect_trainer(&self, args: &Value) -> Result<Value, McpError> {
        if let Some(dir) = opt_str(args, "package_dir") {
            let pkg_path = Path::new(dir);
            if pkg_path.join("mod-manifest.json").is_file() {
                let (manifest, digest) = validate_nmod_package_dir(pkg_path)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                return Ok(json!({
                    "source": "package_dir",
                    "manifest_format": "mod_manifest_v2",
                    "manifest": manifest,
                    "manifest_sha256": digest
                }));
            }
            let (manifest, digest) = validate_trainer_package_dir(pkg_path)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            return Ok(json!({
                "source": "package_dir",
                "manifest_format": "trainer_manifest_v1",
                "manifest": manifest,
                "manifest_sha256": digest
            }));
        }

        let trainer_id = req_str(args, "trainer_id")?;
        let installed = self
            .core
            .list_all_installed_trainers()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|t| t.manifest.id == trainer_id)
            .ok_or_else(|| McpError::Execution(format!("trainer '{trainer_id}' not found")))?;

        Ok(json!({
            "source": "installed_catalog",
            "trainer": installed
        }))
    }

    fn tool_generate_trainer_template(&self, args: &Value) -> Result<Value, McpError> {
        let output_dir = PathBuf::from(req_str(args, "output_dir")?);
        let trainer_id = req_str(args, "trainer_id")?;
        let name = req_str(args, "name")?;
        let game_id = req_str(args, "game_id")?;
        let game_name = req_str(args, "game_name")?;
        let source_repo = req_str(args, "source_repository")?;
        let module_name = args
            .get("module_name")
            .and_then(Value::as_str)
            .unwrap_or("Game.exe");
        let aob_pattern = args
            .get("aob_pattern")
            .and_then(Value::as_str)
            .unwrap_or("48 8B 05 ?? ?? ?? ??");
        let author_name = args
            .get("author_name")
            .and_then(Value::as_str)
            .unwrap_or("Community Contributor");

        let draft = TrainerManifestBuilder::new(trainer_id, name, game_id, game_name, source_repo)
            .author(author_name, None, None)
            .default_memory_permissions(vec![module_name.to_owned()], 64)
            .toggle_feature(
                "god-mode",
                "God Mode",
                "Example reversible health patch (update pattern and bytes before release).",
                FeatureCategory::Player,
                Some("F1".to_owned()),
            )
            .integer_feature(
                "set-gold",
                "Gold Amount",
                "Example scalar write_value feature (update locator before release).",
                FeatureCategory::Economy,
                Some("F2".to_owned()),
                nekod_manifest::FeatureValueRange {
                    min: 0.0,
                    max: 999_999.0,
                    step: 100.0,
                    default: 1000.0,
                },
            )
            .build_draft();

        let payload = DeclarativePatchBuilder::new()
            .byte_patch(
                "god-mode",
                AddressLocator::AobPattern {
                    module: module_name.to_owned(),
                    pattern: aob_pattern.to_owned(),
                    match_index: 0,
                    offset: 0,
                },
                Some("90 90".to_owned()),
                "90 90",
            )
            .write_value(
                "set-gold",
                AddressLocator::PointerChain {
                    module: module_name.to_owned(),
                    base_offset: 0x1000,
                    offsets: vec![0x10],
                },
                ScalarValueType::I32,
                None,
                None,
            )
            .build()
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;

        let pkg = build_trainer_package(draft, payload)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let manifest_path = pkg
            .write_to_dir(&output_dir)
            .map_err(|e| McpError::Execution(e.to_string()))?;
        validate_trainer_package_dir(&output_dir).map_err(|e| {
            McpError::Execution(format!("generated template failed self-check: {e}"))
        })?;

        Ok(json!({
            "output_dir": output_dir,
            "manifest_path": manifest_path,
            "entrypoint_file": pkg.manifest.runtime.entrypoint,
            "manifest_sha256": pkg.manifest_sha256,
            "entrypoint_sha256": pkg.entrypoint_sha256,
            "requires_human_review": true
        }))
    }

    fn tool_check_game_version(&self, args: &Value) -> Result<Value, McpError> {
        let trainer = load_trainer_from_args(&self.core, args)?;
        let detected_build = args.get("detected_build").and_then(Value::as_str);
        let compat = compute_compatibility(&trainer, current_target_platform(), detected_build);
        Ok(json!({
            "trainer_id": trainer.id,
            "trainer_version": trainer.version.to_string(),
            "detected_build": detected_build,
            "compatibility": compat
        }))
    }

    fn tool_validate_compatibility(&self, args: &Value) -> Result<Value, McpError> {
        let trainer = load_trainer_from_args(&self.core, args)?;
        let game = if let Some(raw_game) = args.get("game_manifest_json").and_then(Value::as_str) {
            validate_game_manifest_str(raw_game)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?
        } else {
            let target_id = &trainer.target_game.game_id;
            self.core
                .list_game_manifests()
                .map_err(|e| McpError::Execution(e.to_string()))?
                .into_iter()
                .find(|g| &g.id == target_id)
                .ok_or_else(|| {
                    McpError::Execution(format!(
                        "no registered GameManifest for '{}' (pass game_manifest_json)",
                        target_id
                    ))
                })?
        };

        NekodCore::validate_game_trainer_compatibility(&game, &trainer)
            .map_err(|e| McpError::PolicyRefusal(e.to_string()))?;
        Ok(json!({
            "compatible": true,
            "game_id": game.id,
            "trainer_id": trainer.id,
            "singleplayer_only": trainer.compatibility.singleplayer_only,
            "requires_offline_mode": trainer.compatibility.requires_offline_mode
        }))
    }

    fn tool_inspect_runtime_diagnostics(&self) -> Result<Value, McpError> {
        let policy = self
            .core
            .get_security_policy()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let logs = self.core.get_diagnostic_logs();
        Ok(json!({
            "security_policy": policy,
            "recent_diagnostics_count": logs.len(),
            "recent_diagnostics": logs
        }))
    }

    fn tool_validate_repository(&self, args: &Value) -> Result<Value, McpError> {
        let repo_dir = PathBuf::from(req_str(args, "repository_dir")?);
        if !repo_dir.is_dir() {
            return Err(McpError::InvalidParams(format!(
                "repository directory '{}' does not exist",
                repo_dir.display()
            )));
        }

        let mut results = serde_json::Map::new();
        let reg_path = repo_dir.join("registry-manifest.json");
        if reg_path.is_file() {
            let raw =
                fs::read_to_string(&reg_path).map_err(|e| McpError::Execution(e.to_string()))?;
            let reg = validate_registry_manifest_str(&raw)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            results.insert(
                "registry_manifest".to_owned(),
                json!({
                    "valid": true,
                    "repository_id": reg.repository_id,
                    "entries": reg.trainers.len()
                }),
            );
        }

        if repo_dir.join("mod-manifest.json").is_file() {
            let (manifest, digest) = validate_nmod_package_dir(&repo_dir)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            results.insert(
                "root_nmod_package".to_owned(),
                json!({
                    "valid": true,
                    "mod_id": manifest.id,
                    "version": manifest.version.to_string(),
                    "manifest_sha256": digest
                }),
            );
        }

        let pkg_manifest = repo_dir.join("trainer-manifest.json");
        if pkg_manifest.is_file() {
            let (manifest, digest) = validate_trainer_package_dir(&repo_dir)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            results.insert(
                "root_trainer_package".to_owned(),
                json!({
                    "valid": true,
                    "trainer_id": manifest.id,
                    "manifest_sha256": digest
                }),
            );
        }

        if results.is_empty() {
            return Err(McpError::InvalidParams(format!(
                "no 'registry-manifest.json', 'mod-manifest.json', or 'trainer-manifest.json' found in '{}'",
                repo_dir.display()
            )));
        }

        Ok(Value::Object(results))
    }

    fn tool_inspect_registry_entry(&self, args: &Value) -> Result<Value, McpError> {
        let trainer_id = req_str(args, "trainer_id")?;
        if let Some(raw_reg) = args.get("registry_manifest_json").and_then(Value::as_str) {
            let reg: RegistryManifest = validate_registry_manifest_str(raw_reg)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            let entry = reg
                .trainers
                .into_iter()
                .find(|t| t.trainer_id == trainer_id)
                .ok_or_else(|| {
                    McpError::Execution(format!(
                        "trainer '{trainer_id}' not listed in provided registry manifest"
                    ))
                })?;
            return Ok(json!({ "repository_id": reg.repository_id, "entry": entry }));
        }

        if let Some(repo_id) = args.get("repository_id").and_then(Value::as_str) {
            let cached = self
                .core
                .registry_cache()
                .find_entry(repo_id, trainer_id)
                .map_err(|e| McpError::Execution(e.to_string()))?
                .ok_or_else(|| {
                    McpError::Execution(format!(
                        "trainer '{trainer_id}' in repo '{repo_id}' not in local registry cache"
                    ))
                })?;
            return Ok(json!({ "cached_entry": cached }));
        }

        let matches: Vec<_> = self
            .core
            .registry_cache()
            .list_all_indexed_trainers()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .filter(|r| r.entry.trainer_id == trainer_id)
            .collect();
        if matches.is_empty() {
            return Err(McpError::Execution(format!(
                "trainer '{trainer_id}' not in local registry cache"
            )));
        }
        Ok(json!({ "cached_entries": matches }))
    }

    fn tool_prepare_trainer_release(&self, args: &Value) -> Result<Value, McpError> {
        let package_dir = PathBuf::from(req_str(args, "package_dir")?);
        let manifest_url = req_str(args, "manifest_url")?;
        let tag = req_str(args, "tag")?;
        nekod_manifest::validate_https_url(manifest_url)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;

        let (manifest, manifest_sha256, manifest_format) = load_any_package_from_dir(&package_dir)?;

        let entry = RegistryTrainerEntry {
            trainer_id: manifest.id.clone(),
            game_id: manifest.target_game.game_id.clone(),
            latest_version: manifest.version.clone(),
            manifest_url: manifest_url.to_owned(),
            manifest_sha256: Some(manifest_sha256.clone()),
            release_tag: Some(tag.to_owned()),
            status: RegistryTrainerStatus::Active,
            compatibility_summary: None,
        };

        let summary_path = package_dir.join("release-entry-summary.json");
        let summary_json =
            serde_json::to_string_pretty(&entry).map_err(|e| McpError::Execution(e.to_string()))?;
        fs::write(&summary_path, summary_json.as_bytes())
            .map_err(|e| McpError::Execution(e.to_string()))?;

        Ok(json!({
            "validated": true,
            "manifest_format": manifest_format,
            "trainer_id": manifest.id,
            "mod_id": manifest.id,
            "version": manifest.version.to_string(),
            "manifest_sha256": manifest_sha256,
            "written_summary_path": summary_path,
            "registry_entry": entry,
            "notice": "Human review and signed git release upload are required before submitting to Nekod-Community."
        }))
    }

    fn tool_probe_unity_bridge(&self, args: &Value) -> Result<Value, McpError> {
        if let Some(game_id) = opt_str(args, "game_id")
            && let Some(gm) = self
                .core
                .list_game_manifests()
                .map_err(|e| McpError::Execution(e.to_string()))?
                .into_iter()
                .find(|g| g.id == game_id)
            && gm.compatibility_policy.trainer_policy == TrainerPolicy::ProhibitedOnlineCompetitive
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing live bridge probe for prohibited competitive game '{game_id}'"
            )));
        }

        let pid_u64 = args
            .get("pid")
            .and_then(Value::as_u64)
            .ok_or_else(|| McpError::InvalidParams("missing positive integer 'pid'".to_owned()))?;
        let pid = u32::try_from(pid_u64)
            .map_err(|_| McpError::InvalidParams(format!("pid '{pid_u64}' out of u32 range")))?;
        if pid == 0 {
            return Err(McpError::InvalidParams(
                "pid must be a positive process id".to_owned(),
            ));
        }

        let op = opt_str(args, "op").unwrap_or("ping");
        let discovery_dir = opt_str(args, "discovery_dir");

        let mut merged_params = match args.get("params") {
            Some(Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        };
        for key in [
            "filter",
            "class_name",
            "field_name",
            "game_object_name",
            "include_inactive",
            "include_static",
            "include_methods",
            "max_depth",
            "limit",
        ] {
            if !merged_params.contains_key(key)
                && let Some(val) = args.get(key)
            {
                merged_params.insert(key.to_owned(), val.clone());
            }
        }

        let data =
            probe_nekod_unity_bridge_readonly(pid, discovery_dir, op, Value::Object(merged_params))
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("refuses operation") {
                        McpError::PolicyRefusal(msg)
                    } else {
                        McpError::Execution(msg)
                    }
                })?;

        Ok(json!({
            "pid": pid,
            "op": op,
            "read_only": true,
            "data": data
        }))
    }

    fn tool_probe_ue4ss_bridge(&self, args: &Value) -> Result<Value, McpError> {
        if let Some(game_id) = opt_str(args, "game_id")
            && let Some(gm) = self
                .core
                .list_game_manifests()
                .map_err(|e| McpError::Execution(e.to_string()))?
                .into_iter()
                .find(|g| g.id == game_id)
            && gm.compatibility_policy.trainer_policy == TrainerPolicy::ProhibitedOnlineCompetitive
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing live UE4SS bridge probe for prohibited competitive game '{game_id}'"
            )));
        }

        let pid = match args.get("pid").filter(|v| !v.is_null()) {
            Some(v) => {
                let pid_u64 = v.as_u64().ok_or_else(|| {
                    McpError::InvalidParams("optional 'pid' must be a positive integer".to_owned())
                })?;
                let parsed = u32::try_from(pid_u64).map_err(|_| {
                    McpError::InvalidParams(format!("pid '{pid_u64}' out of u32 range"))
                })?;
                if parsed == 0 {
                    return Err(McpError::InvalidParams(
                        "pid must be a positive process id".to_owned(),
                    ));
                }
                Some(parsed)
            }
            None => None,
        };

        let op = opt_str(args, "op").unwrap_or("ping");
        let discovery_dir = opt_str(args, "discovery_dir");

        let mut merged_params = match args.get("params") {
            Some(Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        };
        for key in [
            "class_name",
            "object_class",
            "property_name",
            "filter",
            "limit",
            "instance_index",
            "live_only",
            "unique_only",
        ] {
            if !merged_params.contains_key(key)
                && let Some(val) = args.get(key)
            {
                merged_params.insert(key.to_owned(), val.clone());
            }
        }

        let data =
            probe_nekod_ue4ss_bridge_readonly(pid, discovery_dir, op, Value::Object(merged_params))
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("refuses operation") {
                        McpError::PolicyRefusal(msg)
                    } else {
                        McpError::Execution(msg)
                    }
                })?;

        Ok(json!({
            "pid": pid,
            "op": op,
            "read_only": true,
            "data": data
        }))
    }

    fn tool_inspect_authorized_game_process(&self, args: &Value) -> Result<Value, McpError> {
        let game_id = req_str(args, "game_id")?;
        if let Some(gm) = self
            .core
            .list_game_manifests()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|g| g.id == game_id)
            && gm.compatibility_policy.trainer_policy
                == nekod_manifest::TrainerPolicy::ProhibitedOnlineCompetitive
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing process inspection for prohibited competitive game '{game_id}'"
            )));
        }

        let processes = self
            .core
            .discover_running_processes_for_game(game_id)
            .map_err(|e| McpError::Execution(e.to_string()))?;
        Ok(json!({
            "game_id": game_id,
            "read_only": true,
            "matching_processes": processes
        }))
    }

    fn tool_inspect_trainer_state(&self, args: &Value) -> Result<Value, McpError> {
        let trainer_id = req_str(args, "trainer_id")?;
        let installed = self
            .core
            .list_all_installed_trainers()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|t| t.manifest.id == trainer_id)
            .ok_or_else(|| McpError::Execution(format!("trainer '{trainer_id}' not installed")))?;
        Ok(json!({
            "trainer_id": installed.manifest.id,
            "version": installed.manifest.version.to_string(),
            "trust_tier": installed.trust_tier,
            "manifest_sha256": installed.manifest_sha256,
            "revoked": installed.revoked,
            "package_dir": installed.package_dir
        }))
    }

    fn tool_get_execution_logs(&self, args: &Value) -> Result<Value, McpError> {
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(250) as usize;
        let logs = self.core.get_diagnostic_logs();
        let start = logs.len().saturating_sub(limit);
        let slice = &logs[start..];
        Ok(json!({
            "total": logs.len(),
            "returned": slice.len(),
            "logs": slice
        }))
    }
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, McpError> {
    opt_str(args, key).ok_or_else(|| {
        McpError::InvalidParams(format!("missing required non-empty string '{key}'"))
    })
}

fn load_any_package_from_dir(
    dir: &Path,
) -> Result<(TrainerManifest, String, &'static str), McpError> {
    if dir.join("mod-manifest.json").is_file() {
        let (mod_m, digest) =
            validate_nmod_package_dir(dir).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        return Ok((mod_m.to_trainer_manifest(), digest, "mod_manifest_v2"));
    }
    let (trainer_m, digest) =
        validate_trainer_package_dir(dir).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    Ok((trainer_m, digest, "trainer_manifest_v1"))
}

fn recompute_mod_manifest_checksums(pkg_dir: &Path) -> Result<(), McpError> {
    let manifest_path = pkg_dir.join("mod-manifest.json");
    let raw = fs::read_to_string(&manifest_path)
        .map_err(|e| McpError::InvalidParams(format!("missing mod-manifest.json: {e}")))?;
    let mut mod_m: ModManifest =
        serde_json::from_str(&raw).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let canon_root = fs::canonicalize(pkg_dir).map_err(|e| McpError::Execution(e.to_string()))?;

    for artifact in &mut mod_m.artifacts {
        validate_relative_path(&artifact.filename)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let canon_file = fs::canonicalize(canon_root.join(&artifact.filename)).map_err(|e| {
            McpError::InvalidParams(format!("missing artifact '{}': {e}", artifact.filename))
        })?;
        if !canon_file.starts_with(&canon_root) {
            return Err(McpError::PolicyRefusal(format!(
                "artifact '{}' escapes package root",
                artifact.filename
            )));
        }
        let bytes = fs::read(&canon_file).map_err(|e| McpError::Execution(e.to_string()))?;
        artifact.sha256 = sha256_hex_bytes(&bytes);
        artifact.size_bytes = bytes.len() as u64;
    }

    let updated =
        serde_json::to_string_pretty(&mod_m).map_err(|e| McpError::Execution(e.to_string()))?;
    validate_mod_manifest_str(&updated).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    fs::write(&manifest_path, updated.as_bytes()).map_err(|e| McpError::Execution(e.to_string()))
}

fn pack_nmod_dir_to_archive(
    pkg_dir: &Path,
    archive_path: &Path,
) -> Result<(ModManifest, String, u64), McpError> {
    let (manifest, _) =
        validate_nmod_package_dir(pkg_dir).map_err(|e| McpError::InvalidParams(e.to_string()))?;

    let mut rel_files = BTreeSet::new();
    rel_files.insert("mod-manifest.json".to_owned());
    let gm_path = pkg_dir.join("game-manifest.json");
    if gm_path.is_file() {
        let gm_raw =
            fs::read_to_string(&gm_path).map_err(|e| McpError::Execution(e.to_string()))?;
        validate_game_manifest_str(&gm_raw)
            .map_err(|e| McpError::InvalidParams(format!("invalid game-manifest.json: {e}")))?;
        rel_files.insert("game-manifest.json".to_owned());
    }

    for artifact in &manifest.artifacts {
        let lower = artifact.filename.to_ascii_lowercase();
        if DISALLOWED_PACK_EXTENSIONS
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            return Err(McpError::PolicyRefusal(format!(
                "refusing to pack disallowed executable '{}' into source-first .nmod archive",
                artifact.filename
            )));
        }
        rel_files.insert(artifact.filename.clone());
    }

    if let Some(parent) = archive_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| McpError::Execution(e.to_string()))?;
    }

    {
        let file =
            fs::File::create(archive_path).map_err(|e| McpError::Execution(e.to_string()))?;
        let mut zw = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for rel in &rel_files {
            let bytes =
                fs::read(pkg_dir.join(rel)).map_err(|e| McpError::Execution(e.to_string()))?;
            zw.start_file(rel, opts)
                .map_err(|e| McpError::Execution(e.to_string()))?;
            zw.write_all(&bytes)
                .map_err(|e| McpError::Execution(e.to_string()))?;
        }
        zw.finish()
            .map_err(|e| McpError::Execution(e.to_string()))?;
    }

    let archive_bytes = fs::read(archive_path).map_err(|e| McpError::Execution(e.to_string()))?;
    Ok((
        manifest,
        sha256_hex_bytes(&archive_bytes),
        archive_bytes.len() as u64,
    ))
}

fn load_trainer_from_args(
    core: &NekodAppService,
    args: &Value,
) -> Result<TrainerManifest, McpError> {
    if let Some(dir) = opt_str(args, "package_dir") {
        let (m, _, _) = load_any_package_from_dir(Path::new(dir))?;
        return Ok(m);
    }
    if let Some(raw) = opt_str(args, "mod_manifest_json") {
        let mod_m =
            validate_mod_manifest_str(raw).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        return Ok(mod_m.to_trainer_manifest());
    }
    if let Some(raw) = opt_str(args, "trainer_manifest_json") {
        let val: Value =
            serde_json::from_str(raw).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        if val
            .get("schema_version")
            .and_then(Value::as_str)
            .is_some_and(|v| v.starts_with("2."))
        {
            let mod_m = validate_mod_manifest_str(raw)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            return Ok(mod_m.to_trainer_manifest());
        }
        return validate_trainer_manifest_str(raw)
            .map_err(|e| McpError::InvalidParams(e.to_string()));
    }
    if let Some(id) = opt_str(args, "trainer_id") {
        return core
            .list_all_installed_trainers()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|t| t.manifest.id == id)
            .map(|t| t.manifest)
            .ok_or_else(|| McpError::Execution(format!("trainer '{id}' not installed")));
    }
    Err(McpError::InvalidParams(
        "provide 'package_dir', 'mod_manifest_json', 'trainer_manifest_json', or 'trainer_id'"
            .to_owned(),
    ))
}

/// Blocks disallowed anti-cheat evasion, unauthorized memory-write, or non-singleplayer requests.
fn enforce_prohibited_pattern_guard(args: &Value) -> Result<(), McpError> {
    if args.get("singleplayer_only").and_then(Value::as_bool) == Some(false) {
        return Err(McpError::PolicyRefusal(
            "singleplayer_only=false is prohibited by Nekod policy".to_owned(),
        ));
    }
    for forbidden_flag in [
        "bypass_anti_cheat",
        "anti_cheat_bypass",
        "stealth_inject",
        "kernel_evasion",
        "grant_permissions",
        "write_process_memory",
    ] {
        if args.get(forbidden_flag).is_some() {
            return Err(McpError::PolicyRefusal(format!(
                "MCP parameter '{forbidden_flag}' is prohibited by Nekod security policy"
            )));
        }
    }
    Ok(())
}

fn tool_metadata_json(spec: &McpToolSpec) -> Value {
    let description = match spec.name {
        "list_installed_games" => {
            "List installed games discovered across Steam and configured folders (read-only)."
        }
        "get_game_info" => {
            "Get registered GameManifest and local install metadata for a canonical game_id (read-only)."
        }
        "inspect_game_environment" => {
            "Inspect host platform, engine, installed loaders, and anti-cheat policy for a registered game_id or local install_dir (read-only)."
        }
        "recommend_runtime_adapter" => {
            "Inspect game engine/loaders and compatibility DB to recommend optimal Runtime Adapter & Plugin Bucket strategy (read-only)."
        }
        "inspect_unity_assemblies" => {
            "Pure-Rust read-only ECMA-335 metadata inspector for Unity Managed/*.dll or BepInEx/interop/*.dll assemblies (list filtered types or inspect class inheritance, fields, properties, and parameterless methods)."
        }
        "create_trainer_manifest" => {
            "Generate and validate trainer-manifest.json for an existing entrypoint (requires authorize_write: true)."
        }
        "validate_trainer_manifest" => {
            "Validate a .nmod package directory, trainer package directory, mod-manifest.json (v2), or trainer-manifest.json (v1) against JSON Schema & security rules (read-only)."
        }
        "inspect_trainer" => {
            "Inspect an installed mod/trainer or local .nmod package directory including permissions and features (read-only)."
        }
        "generate_trainer_template" => {
            "Scaffold a complete declarative_patch_v1 trainer directory with SHA-256 artifacts (requires authorize_write: true)."
        }
        "forge_nmod_package" => {
            "Forge a schema-validated .nmod v2 source-first package supporting custom features, validated runtime entrypoint_payload (Unity Mono/IL2CPP, Declarative Patch, UE4SS, etc.), optional game-manifest.json, and optional .nmod packing (requires authorize_write: true)."
        }
        "pack_nmod_archive" => {
            "Validate (with optional SHA-256 recomputation) and pack a .nmod package directory into a portable .nmod ZIP archive, optionally staging directly into local data_dir/nmods/ (requires authorize_write: true)."
        }
        "analyze_ct_file" => {
            "Statically analyze a Cheat Engine .CT XML table, reporting convertible declarative entries vs safely rejected Lua/alloc scripts (read-only)."
        }
        "check_game_version" => {
            "Evaluate mod/trainer supported_versions compatibility against a target game build (read-only)."
        }
        "validate_compatibility" => {
            "Verify GameManifest & ModManifest/TrainerManifest singleplayer/offline/anti-cheat policy compatibility (read-only)."
        }
        "verify_forged_mod_package" => {
            "Verify .nmod manifest & SHA-256 artifacts and record Experimental or AutomatedRuntimeTest compatibility under ADR-019 gate (requires authorize_write: true)."
        }
        "inspect_runtime_diagnostics" => {
            "Read Nekod core security policy state and recent diagnostics (read-only)."
        }
        "validate_repository" => {
            "Validate a community registry-manifest.json, root .nmod package, or trainer package directory (read-only)."
        }
        "inspect_registry_entry" => {
            "Inspect a trainer entry in a RegistryManifest JSON or cached source index (read-only)."
        }
        "prepare_trainer_release" => {
            "Validate a .nmod or trainer package and emit a SHA-256-pinned RegistryTrainerEntry summary (requires authorize_write: true)."
        }
        "inspect_authorized_game_process" => {
            "Discover matching running processes for a non-prohibited singleplayer game (requires authorize_process_inspection: true)."
        }
        "probe_unity_bridge" => {
            "Send a strictly read-only RPC (ping, list_types, list_members, find_instances, dump_hierarchy, get_field) to a live NekodUnityBridge loopback instance discovered by PID (requires authorize_process_inspection: true)."
        }
        "probe_ue4ss_bridge" => {
            "Send a strictly read-only RPC (ping, find_objects, dump_ui_texts, get_uproperty, list_properties, list_functions) to a live NekodUE4SSBridge instance via File-IPC or TCP loopback (requires authorize_process_inspection: true)."
        }
        "inspect_trainer_state" => {
            "Inspect local catalog state and revocation status for an installed trainer (read-only)."
        }
        "get_execution_logs" => {
            "Retrieve bounded recent diagnostic & security audit log events (read-only)."
        }
        _ => "Nekod developer tool.",
    };

    let input_schema = match spec.group {
        McpToolGroup::GameDiscovery
        | McpToolGroup::TrainerDevelopment
        | McpToolGroup::Compatibility
        | McpToolGroup::Repository
        | McpToolGroup::RuntimeInspection => schema_for_tool(spec.name),
    };

    json!({
        "name": spec.name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": spec.read_only,
            "destructiveHint": !spec.read_only,
            "idempotentHint": spec.read_only,
            "openWorldHint": false
        },
        "x_nekod": {
            "group": spec.group,
            "read_only": spec.read_only,
            "requires_explicit_authorization": spec.requires_explicit_authorization
        }
    })
}

fn schema_for_tool(name: &str) -> Value {
    match name {
        "list_installed_games" | "inspect_runtime_diagnostics" => {
            json!({ "type": "object", "properties": {}, "additionalProperties": false })
        }
        "get_game_info" => json!({
            "type": "object",
            "required": ["game_id"],
            "properties": { "game_id": { "type": "string" } }
        }),
        "inspect_game_environment" => json!({
            "type": "object",
            "properties": {
                "game_id": { "type": "string" },
                "install_dir": { "type": "string" }
            }
        }),
        "recommend_runtime_adapter" => json!({
            "type": "object",
            "required": ["game_id"],
            "properties": {
                "game_id": { "type": "string" },
                "install_dir": { "type": "string" }
            }
        }),
        "inspect_unity_assemblies" => json!({
            "type": "object",
            "properties": {
                "install_dir": { "type": "string" },
                "assembly_path": { "type": "string" },
                "assembly_name": { "type": "string" },
                "filter": { "type": "string" },
                "type_name": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                "include_compiler_generated": { "type": "boolean" }
            }
        }),
        "forge_nmod_package" => json!({
            "type": "object",
            "required": ["authorize_write", "output_dir", "mod_id", "name", "game_id", "game_name", "source_repository"],
            "properties": {
                "authorize_write": { "type": "boolean" },
                "output_dir": { "type": "string" },
                "mod_id": { "type": "string" },
                "name": { "type": "string" },
                "version": { "type": "string" },
                "description": { "type": "string" },
                "game_id": { "type": "string" },
                "game_name": { "type": "string" },
                "steam_app_id": { "type": "integer", "minimum": 1 },
                "source_repository": { "type": "string" },
                "runtime_kind": { "type": "string" },
                "mod_kind": { "type": "string" },
                "flavor": { "type": "string", "enum": ["mono", "il2cpp"] },
                "module_name": { "type": "string" },
                "entrypoint": { "type": "string" },
                "features": { "type": "array" },
                "entrypoint_payload": { "type": "object" },
                "game_manifest": { "type": ["object", "string"] },
                "include_legacy_v1_manifest": { "type": "boolean" },
                "pack_nmod_path": { "type": "string" }
            }
        }),
        "pack_nmod_archive" => json!({
            "type": "object",
            "required": ["authorize_write", "package_dir"],
            "properties": {
                "authorize_write": { "type": "boolean" },
                "package_dir": { "type": "string" },
                "output_nmod_path": { "type": "string" },
                "install_to_local_nmods": { "type": "boolean" },
                "recompute_checksums": { "type": "boolean" }
            }
        }),
        "analyze_ct_file" => json!({
            "type": "object",
            "required": ["ct_path"],
            "properties": {
                "ct_path": { "type": "string" },
                "game_id": { "type": "string" },
                "game_name": { "type": "string" },
                "default_module": { "type": "string" }
            }
        }),
        "verify_forged_mod_package" => json!({
            "type": "object",
            "required": ["authorize_write", "package_dir", "game_version"],
            "properties": {
                "authorize_write": { "type": "boolean" },
                "package_dir": { "type": "string" },
                "game_version": { "type": "string" },
                "store_platform": { "type": "string" },
                "runtime_rollback_verified": { "type": "boolean" }
            }
        }),
        "inspect_authorized_game_process" => json!({
            "type": "object",
            "required": ["game_id", "authorize_process_inspection"],
            "properties": {
                "game_id": { "type": "string" },
                "authorize_process_inspection": { "type": "boolean" }
            }
        }),
        "probe_unity_bridge" => json!({
            "type": "object",
            "required": ["pid", "op", "authorize_process_inspection"],
            "properties": {
                "pid": { "type": "integer", "minimum": 1 },
                "op": {
                    "type": "string",
                    "enum": ["ping", "list_types", "list_members", "find_instances", "dump_hierarchy", "get_field"]
                },
                "params": { "type": "object" },
                "filter": { "type": "string" },
                "class_name": { "type": "string" },
                "field_name": { "type": "string" },
                "game_object_name": { "type": "string" },
                "include_inactive": { "type": "boolean" },
                "include_static": { "type": "boolean" },
                "include_methods": { "type": "boolean" },
                "max_depth": { "type": "integer", "minimum": 1, "maximum": 16 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 4000 },
                "discovery_dir": { "type": "string" },
                "game_id": { "type": "string" },
                "authorize_process_inspection": { "type": "boolean" }
            }
        }),
        "probe_ue4ss_bridge" => json!({
            "type": "object",
            "required": ["op", "authorize_process_inspection"],
            "properties": {
                "pid": { "type": "integer", "minimum": 1 },
                "op": {
                    "type": "string",
                    "enum": ["ping", "find_objects", "dump_ui_texts", "get_uproperty", "list_properties", "list_functions"]
                },
                "params": { "type": "object" },
                "class_name": { "type": "string" },
                "object_class": { "type": "string" },
                "property_name": { "type": "string" },
                "filter": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500 },
                "instance_index": { "type": "integer", "minimum": 1 },
                "live_only": { "type": "boolean" },
                "unique_only": { "type": "boolean" },
                "discovery_dir": { "type": "string" },
                "game_id": { "type": "string" },
                "authorize_process_inspection": { "type": "boolean" }
            }
        }),
        "create_trainer_manifest" | "generate_trainer_template" => json!({
            "type": "object",
            "required": ["authorize_write", "output_dir", "trainer_id", "name", "game_id", "game_name", "source_repository"],
            "properties": {
                "authorize_write": { "type": "boolean" },
                "output_dir": { "type": "string" },
                "trainer_id": { "type": "string" },
                "name": { "type": "string" },
                "game_id": { "type": "string" },
                "game_name": { "type": "string" },
                "source_repository": { "type": "string" },
                "module_name": { "type": "string" },
                "author_name": { "type": "string" },
                "aob_pattern": { "type": "string" }
            }
        }),
        "prepare_trainer_release" => json!({
            "type": "object",
            "required": ["authorize_write", "package_dir", "manifest_url", "tag"],
            "properties": {
                "authorize_write": { "type": "boolean" },
                "package_dir": { "type": "string" },
                "manifest_url": { "type": "string" },
                "archive_url": { "type": "string" },
                "tag": { "type": "string" }
            }
        }),
        _ => json!({ "type": "object", "properties": {} }),
    }
}

fn default_entrypoint_path_for_kind(kind: RuntimeKind) -> &'static str {
    match kind {
        RuntimeKind::DeclarativePatchV1 => "rules/main.json",
        RuntimeKind::CeSubsetV1 => "rules/ce_subset.json",
        RuntimeKind::UnityBridgeV1 => "scripts/unity_bridge.json",
        RuntimeKind::Ue4ssBridgeV1 => "scripts/ue4ss_bridge.json",
        RuntimeKind::ManagedFrameworkV1 => "scripts/framework.json",
        _ => "rules/entrypoint.json",
    }
}

fn build_default_dependencies_for_kind(
    kind: RuntimeKind,
    flavor: Option<&str>,
) -> Vec<RuntimeDependencySpec> {
    if kind != RuntimeKind::UnityBridgeV1 {
        return Vec::new();
    }
    let is_mono = flavor.is_some_and(|f| f.eq_ignore_ascii_case("mono"));
    vec![
        RuntimeDependencySpec {
            dependency_id: if is_mono {
                "bepinex-mono".to_owned()
            } else {
                "bepinex-il2cpp".to_owned()
            },
            kind: DependencyKind::ModLoader,
            version_req: if is_mono {
                ">=5.4.0".to_owned()
            } else {
                ">=6.0.0-alpha.0".to_owned()
            },
            optional: false,
            redistribution: DependencyRedistribution::LocalDetectOnly,
            upstream: None,
        },
        RuntimeDependencySpec {
            dependency_id: "nekod-unity-bridge".to_owned(),
            kind: DependencyKind::RuntimeBridge,
            version_req: ">=0.1.0".to_owned(),
            optional: false,
            redistribution: DependencyRedistribution::LocalDetectOnly,
            upstream: None,
        },
    ]
}

fn build_forged_entrypoint_payload(
    kind: RuntimeKind,
    module_name: &str,
    flavor: Option<&str>,
) -> Result<(String, String), McpError> {
    let rel = default_entrypoint_path_for_kind(kind).to_owned();
    match kind {
        RuntimeKind::DeclarativePatchV1 => {
            let payload = DeclarativePatchBuilder::new()
                .byte_patch(
                    "forged-feature",
                    AddressLocator::AobPattern {
                        module: module_name.to_owned(),
                        pattern: "48 8B 05 ?? ?? ?? ??".to_owned(),
                        match_index: 0,
                        offset: 0,
                    },
                    Some("90 90".to_owned()),
                    "90 90",
                )
                .build()
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            let json = serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::Execution(e.to_string()))?;
            Ok((rel, json))
        }
        RuntimeKind::CeSubsetV1 => Ok((
            rel,
            serde_json::to_string_pretty(&json!({
                "schema_version": "1.0.0",
                "features": [{
                    "kind": "auto_assembler_patch",
                    "feature_id": "forged-feature",
                    "default_module": module_name,
                    "script": format!("[ENABLE]\naobscanmodule(MOD,{module_name},48 8B 05 ?? ?? ?? ??)\nMOD:\ndb 90 90\n[DISABLE]\nMOD:\ndb 90 90")
                }]
            }))
            .unwrap(),
        )),
        RuntimeKind::UnityBridgeV1 => {
            let is_mono = flavor.is_some_and(|f| f.eq_ignore_ascii_case("mono"));
            let (fl, ld) = if is_mono {
                ("mono", "bepinex_mono")
            } else {
                ("il2cpp", "bepinex_il2cpp")
            };
            Ok((
                rel,
                serde_json::to_string_pretty(&json!({
                    "schema_version": "1.0.0",
                    "protocol": "nekod_json_line_v1",
                    "flavor": fl,
                    "loader": ld,
                    "host": "127.0.0.1",
                    "port": 19540,
                    "auto_discover_by_pid": true,
                    "features": [{
                        "kind": "toggle_field",
                        "feature_id": "forged-feature",
                        "class_name": "UnityEngine.Time",
                        "field_name": "timeScale",
                        "enabled_value": 0.0,
                        "disabled_value": 1.0
                    }]
                }))
                .unwrap(),
            ))
        }
        RuntimeKind::Ue4ssBridgeV1 => Ok((
            rel,
            serde_json::to_string_pretty(&json!({
                "schema_version": "1.0.0",
                "engine_version": "5.3",
                "host": "127.0.0.1",
                "port": 19541,
                "auth_token": "replace_with_session_token_16",
                "features": [{
                    "kind": "cheat_manager_command",
                    "feature_id": "forged-feature",
                    "enable_command": "God",
                    "disable_command": "God"
                }]
            }))
            .unwrap(),
        )),
        RuntimeKind::ManagedFrameworkV1 => Ok((
            rel,
            serde_json::to_string_pretty(&json!({
                "schema_version": "1.0.0",
                "framework": "reframework",
                "ipc": {
                    "host": "127.0.0.1",
                    "port": 19542,
                    "auth_token": "replace_with_session_token_16"
                },
                "staged_scripts": [],
                "features": [{
                    "kind": "ipc_toggle",
                    "feature_id": "forged-feature",
                    "enable_command": "enable_mod",
                    "disable_command": "disable_mod"
                }]
            }))
            .unwrap(),
        )),
        _ => Ok((rel, json!({ "schema_version": "1.0.0" }).to_string())),
    }
}
