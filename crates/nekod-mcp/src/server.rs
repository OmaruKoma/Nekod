//! Stdio JSON-RPC 2.0 MCP Server implementation backed by `nekod-core`, `nekod-validator`,
//! and `nekod-trainer-sdk`.
//!
//! Enforces strict read-only vs. mutating tool separation, explicit authorization flags for
//! workspace writes and process inspection, and singleplayer / anti-cheat safety refusals.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nekod_core::{NekodAppService, NekodCore};
use nekod_game_detection::current_target_platform;
use nekod_manifest::{
    FeatureCategory, RegistryManifest, RegistryTrainerEntry, RegistryTrainerStatus, TrainerManifest,
};
use nekod_trainer_manager::compute_compatibility;
use nekod_trainer_sdk::{
    AddressLocator, DeclarativePatchBuilder, ScalarValueType, TrainerManifestBuilder,
    build_trainer_package, sha256_hex_bytes,
};
use nekod_validator::{
    validate_game_manifest_str, validate_registry_manifest_str, validate_trainer_manifest_str,
    validate_trainer_package_dir,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{McpToolGroup, McpToolSpec, PLANNED_MCP_TOOLS};

const PROTOCOL_VERSION: &str = "2024-11-05";

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
}

impl NekodMcpServer {
    pub fn new(core: Arc<NekodAppService>) -> Self {
        Self { core }
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
                "inspect_authorized_game_process" => "authorize_process_inspection",
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
            "create_trainer_manifest" => self.tool_create_trainer_manifest(args),
            "validate_trainer_manifest" => self.tool_validate_trainer_manifest(args),
            "inspect_trainer" => self.tool_inspect_trainer(args),
            "generate_trainer_template" => self.tool_generate_trainer_template(args),
            "check_game_version" => self.tool_check_game_version(args),
            "validate_compatibility" => self.tool_validate_compatibility(args),
            "inspect_runtime_diagnostics" => self.tool_inspect_runtime_diagnostics(),
            "validate_repository" => self.tool_validate_repository(args),
            "inspect_registry_entry" => self.tool_inspect_registry_entry(args),
            "prepare_trainer_release" => self.tool_prepare_trainer_release(args),
            "inspect_authorized_game_process" => self.tool_inspect_authorized_game_process(args),
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
        let game_id = req_str(args, "game_id")?;
        let platform = format!("{:?}", current_target_platform());
        let games = self
            .core
            .list_library_games()
            .map_err(|e| McpError::Execution(e.to_string()))?;
        let detected = games
            .iter()
            .find(|g| g.game.matched_game_id.as_deref() == Some(game_id))
            .ok_or_else(|| McpError::Execution(format!("game '{game_id}' not detected")))?;

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
        if let Some(dir) = args.get("package_dir").and_then(Value::as_str) {
            let (manifest, digest) = validate_trainer_package_dir(dir)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            return Ok(json!({
                "valid": true,
                "mode": "package_dir",
                "trainer_id": manifest.id,
                "version": manifest.version.to_string(),
                "manifest_sha256": digest
            }));
        }

        let raw = if let Some(inline) = args.get("manifest_json").and_then(Value::as_str) {
            inline.to_owned()
        } else if let Some(path) = args.get("manifest_path").and_then(Value::as_str) {
            fs::read_to_string(path).map_err(|e| McpError::Execution(e.to_string()))?
        } else {
            return Err(McpError::InvalidParams(
                "provide one of 'package_dir', 'manifest_path', or 'manifest_json'".to_owned(),
            ));
        };

        let manifest = validate_trainer_manifest_str(&raw)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        Ok(json!({
            "valid": true,
            "mode": "manifest_only",
            "trainer_id": manifest.id,
            "version": manifest.version.to_string(),
            "manifest_sha256": sha256_hex_bytes(raw.as_bytes())
        }))
    }

    fn tool_inspect_trainer(&self, args: &Value) -> Result<Value, McpError> {
        if let Some(dir) = args.get("package_dir").and_then(Value::as_str) {
            let (manifest, digest) = validate_trainer_package_dir(dir)
                .map_err(|e| McpError::InvalidParams(e.to_string()))?;
            return Ok(json!({
                "source": "package_dir",
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
                "no 'registry-manifest.json' or 'trainer-manifest.json' found in '{}'",
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

        let (manifest, manifest_sha256) = validate_trainer_package_dir(&package_dir)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;

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
            "trainer_id": manifest.id,
            "version": manifest.version.to_string(),
            "manifest_sha256": manifest_sha256,
            "written_summary_path": summary_path,
            "registry_entry": entry,
            "notice": "Human review and signed git release upload are required before submitting to Nekod-Community."
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

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            McpError::InvalidParams(format!("missing required non-empty string '{key}'"))
        })
}

fn load_trainer_from_args(
    core: &NekodAppService,
    args: &Value,
) -> Result<TrainerManifest, McpError> {
    if let Some(raw) = args.get("trainer_manifest_json").and_then(Value::as_str) {
        return validate_trainer_manifest_str(raw)
            .map_err(|e| McpError::InvalidParams(e.to_string()));
    }
    if let Some(id) = args.get("trainer_id").and_then(Value::as_str) {
        return core
            .list_all_installed_trainers()
            .map_err(|e| McpError::Execution(e.to_string()))?
            .into_iter()
            .find(|t| t.manifest.id == id)
            .map(|t| t.manifest)
            .ok_or_else(|| McpError::Execution(format!("trainer '{id}' not installed")));
    }
    Err(McpError::InvalidParams(
        "provide 'trainer_manifest_json' or 'trainer_id'".to_owned(),
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
            "Inspect host platform, install path, and anti-cheat policy for a detected game (read-only)."
        }
        "create_trainer_manifest" => {
            "Generate and validate trainer-manifest.json for an existing entrypoint (requires authorize_write: true)."
        }
        "validate_trainer_manifest" => {
            "Validate a trainer package directory or trainer-manifest.json against JSON Schema v1 & security rules (read-only)."
        }
        "inspect_trainer" => {
            "Inspect an installed trainer or local package directory including permissions and features (read-only)."
        }
        "generate_trainer_template" => {
            "Scaffold a complete declarative_patch_v1 trainer directory with SHA-256 artifacts (requires authorize_write: true)."
        }
        "check_game_version" => {
            "Evaluate trainer supported_versions compatibility against a target game build (read-only)."
        }
        "validate_compatibility" => {
            "Verify GameManifest & TrainerManifest singleplayer/offline/anti-cheat policy compatibility (read-only)."
        }
        "inspect_runtime_diagnostics" => {
            "Read Nekod core security policy state and recent diagnostics (read-only)."
        }
        "validate_repository" => {
            "Validate a community registry-manifest.json and/or trainer package directory (read-only)."
        }
        "inspect_registry_entry" => {
            "Inspect a trainer entry in a RegistryManifest JSON or cached source index (read-only)."
        }
        "prepare_trainer_release" => {
            "Validate a trainer package and emit a SHA-256-pinned RegistryTrainerEntry summary (requires authorize_write: true)."
        }
        "inspect_authorized_game_process" => {
            "Discover matching running processes for a non-prohibited singleplayer game (requires authorize_process_inspection: true)."
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
        "get_game_info" | "inspect_game_environment" => json!({
            "type": "object",
            "required": ["game_id"],
            "properties": { "game_id": { "type": "string" } }
        }),
        "inspect_authorized_game_process" => json!({
            "type": "object",
            "required": ["game_id", "authorize_process_inspection"],
            "properties": {
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
