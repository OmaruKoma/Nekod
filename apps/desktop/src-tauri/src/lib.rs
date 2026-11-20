//! L1 Tauri 2 desktop shell delegating all business & security operations to `nekod-core`.

mod artwork;
mod bridges;
mod hotkeys;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use artwork::{GameArtwork, resolve_game_artwork};
use bridges::{
    CompanionBridgeCatalogEntry, download_or_load_cached_bepinex_zip,
    extract_bepinex_zip_to_ledger, inspect_bepinex_loader_readiness,
    list_catalog as list_bridge_catalog, resolve_bridge_bytes_for_engine,
};
use hotkeys::{GlobalHotkeyBinding, GlobalHotkeyService};
use nekod_core::{
    CommunityTrainerView, DiagnosticEvent, LibraryGameView, NekodAppService, TrainerDetailView,
    TrainerPermissionState, TrainerUpdateInfo,
};
use nekod_game_detection::{GameEngineKind, GameEnvironmentProfile};
use nekod_manifest::{PermissionScope, TrustTier};
use nekod_permission_manager::SecurityPolicyConfig;
use nekod_profile_manager::{CustomGamePathRecord, GameProfile};
use nekod_registry_client::{RegistrySourceState, RegistrySyncReport};
use nekod_runtime_api::{FeatureCommand, ProcessIdentity, RuntimeDiagnostics, RuntimeSessionId};
use nekod_trainer_manager::{CompatibilityRecord, sha256_hex};
use nekod_trainer_runtime::{CompositeRuntimeRouter, DeploymentLedger, DeploymentState};
use serde::Serialize;
use serde_json::json;
use tauri::{Emitter, Manager, State};

pub struct AppState {
    core: NekodAppService,
    data_dir: PathBuf,
    hotkeys: GlobalHotkeyService,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeDeploymentStatus {
    pub deployed: bool,
    pub engine: GameEngineKind,
    pub active_files: Vec<String>,
    pub suggested_source_file: Option<String>,
    pub suggested_target_relative_path: Option<String>,
    pub matched_bridge: Option<CompanionBridgeCatalogEntry>,
    pub loader_installed: bool,
    pub loader_managed_by_nekod: bool,
    pub loader_display_name: Option<String>,
    pub loader_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuickSetupResult {
    pub game_id: String,
    pub trainer_id: String,
    pub engine: GameEngineKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeUndeploySummary {
    pub removed_count: usize,
    pub restored_originals: usize,
    pub conflicts: usize,
    pub conflict_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchGameResult {
    pub method: String,
    pub target: String,
}

fn map_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[tauri::command]
fn rescan_games(state: State<'_, AppState>) -> Result<Vec<LibraryGameView>, String> {
    state.core.rescan_games().map_err(map_err)
}

#[tauri::command]
fn list_library_games(state: State<'_, AppState>) -> Result<Vec<LibraryGameView>, String> {
    state.core.list_library_games().map_err(map_err)
}

#[tauri::command]
fn list_trainers_for_game(
    state: State<'_, AppState>,
    game_id: String,
    detected_build_id: Option<String>,
) -> Result<Vec<TrainerDetailView>, String> {
    state
        .core
        .list_trainers_for_game(&game_id, detected_build_id.as_deref())
        .map_err(map_err)
}

#[tauri::command]
fn grant_trainer_permissions(
    state: State<'_, AppState>,
    trainer_id: String,
    confirmed_scopes: Vec<PermissionScope>,
) -> Result<TrainerPermissionState, String> {
    let scope_set: BTreeSet<PermissionScope> = confirmed_scopes.into_iter().collect();
    state
        .core
        .grant_trainer_permissions(&trainer_id, scope_set)
        .map_err(map_err)
}

#[tauri::command]
fn revoke_trainer_permissions(
    state: State<'_, AppState>,
    trainer_id: String,
) -> Result<(), String> {
    state
        .core
        .revoke_trainer_permissions(&trainer_id)
        .map_err(map_err)
}

#[tauri::command]
fn ingest_local_trainer(
    state: State<'_, AppState>,
    package_dir: String,
    trust_tier: TrustTier,
) -> Result<(), String> {
    state
        .core
        .ingest_local_trainer(package_dir, trust_tier)
        .map(|_| ())
        .map_err(map_err)
}

#[tauri::command]
fn register_game_manifest(state: State<'_, AppState>, raw_json: String) -> Result<String, String> {
    state
        .core
        .register_game_manifest(&raw_json)
        .map(|m| m.id)
        .map_err(map_err)
}

#[tauri::command]
fn add_custom_game_path(
    state: State<'_, AppState>,
    path: String,
    label: Option<String>,
) -> Result<Vec<CustomGamePathRecord>, String> {
    state
        .core
        .add_custom_game_path(&path, label.as_deref())
        .map_err(map_err)
}

#[tauri::command]
fn remove_custom_game_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<CustomGamePathRecord>, String> {
    state.core.remove_custom_game_path(&path).map_err(map_err)
}

#[tauri::command]
fn list_custom_game_paths(state: State<'_, AppState>) -> Result<Vec<CustomGamePathRecord>, String> {
    state.core.list_custom_game_paths().map_err(map_err)
}

#[tauri::command]
fn get_security_policy(state: State<'_, AppState>) -> Result<SecurityPolicyConfig, String> {
    state.core.get_security_policy().map_err(map_err)
}

#[tauri::command]
fn update_security_policy(
    state: State<'_, AppState>,
    policy: SecurityPolicyConfig,
) -> Result<(), String> {
    state.core.update_security_policy(policy).map_err(map_err)
}

#[tauri::command]
fn load_profile(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
) -> Result<Option<GameProfile>, String> {
    state
        .core
        .load_profile(&game_id, &trainer_id)
        .map_err(map_err)
}

#[tauri::command]
fn save_profile(state: State<'_, AppState>, profile: GameProfile) -> Result<(), String> {
    state.core.save_profile(&profile).map_err(map_err)
}

#[tauri::command]
fn export_profile_json(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
) -> Result<String, String> {
    state
        .core
        .export_profile_json(&game_id, &trainer_id)
        .map_err(map_err)
}

#[tauri::command]
fn import_profile_json(
    state: State<'_, AppState>,
    raw_json: String,
) -> Result<GameProfile, String> {
    state.core.import_profile_json(&raw_json).map_err(map_err)
}

#[tauri::command]
fn get_diagnostic_logs(state: State<'_, AppState>) -> Vec<DiagnosticEvent> {
    state.core.get_diagnostic_logs()
}

#[tauri::command]
fn discover_running_processes(
    state: State<'_, AppState>,
    game_id: Option<String>,
    install_dir: Option<String>,
) -> Result<Vec<ProcessIdentity>, String> {
    if let Some(gid) = game_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let procs = state
            .core
            .discover_running_processes_for_game(gid)
            .map_err(map_err)?;
        if !procs.is_empty() {
            return Ok(procs);
        }
    }

    if let Some(dir) = install_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return state
            .core
            .discover_running_processes_in_dir(dir)
            .map_err(map_err);
    }

    Ok(Vec::new())
}

#[tauri::command]
fn start_trainer_session(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
    target_process: ProcessIdentity,
) -> Result<RuntimeSessionId, String> {
    state
        .core
        .start_trainer_session(&game_id, &trainer_id, target_process)
        .map_err(map_err)
}

#[tauri::command]
fn apply_trainer_feature(
    state: State<'_, AppState>,
    session_id: String,
    command: FeatureCommand,
) -> Result<(), String> {
    state
        .core
        .apply_trainer_feature(&RuntimeSessionId(session_id), command)
        .map_err(map_err)
}

#[tauri::command]
fn stop_trainer_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<RuntimeDiagnostics, String> {
    state
        .core
        .stop_trainer_session(&RuntimeSessionId(session_id))
        .map_err(map_err)
}

#[tauri::command]
fn get_session_diagnostics(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<RuntimeDiagnostics, String> {
    state
        .core
        .get_session_diagnostics(&RuntimeSessionId(session_id))
        .map_err(map_err)
}

#[tauri::command]
fn list_registry_sources(state: State<'_, AppState>) -> Result<Vec<RegistrySourceState>, String> {
    state.core.list_registry_sources().map_err(map_err)
}

#[tauri::command]
fn add_third_party_registry_source(
    state: State<'_, AppState>,
    repository_id: String,
    index_url: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .core
        .add_third_party_registry_source(&repository_id, &index_url, enabled)
        .map_err(map_err)
}

#[tauri::command]
fn remove_registry_source(state: State<'_, AppState>, repository_id: String) -> Result<(), String> {
    state
        .core
        .remove_registry_source(&repository_id)
        .map_err(map_err)
}

#[tauri::command]
fn sync_registry_sources(state: State<'_, AppState>) -> Result<RegistrySyncReport, String> {
    state.core.sync_registry_sources().map_err(map_err)
}

#[tauri::command]
fn list_community_trainers_for_game(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<CommunityTrainerView>, String> {
    state
        .core
        .list_community_trainers_for_game(&game_id)
        .map_err(map_err)
}

#[tauri::command]
fn check_trainer_updates(state: State<'_, AppState>) -> Result<Vec<TrainerUpdateInfo>, String> {
    state.core.check_trainer_updates().map_err(map_err)
}

#[tauri::command]
fn install_or_update_trainer_from_registry(
    state: State<'_, AppState>,
    repository_id: String,
    trainer_id: String,
) -> Result<(), String> {
    state
        .core
        .install_or_update_trainer_from_registry(&repository_id, &trainer_id)
        .map(|_| ())
        .map_err(map_err)
}

#[tauri::command]
fn inspect_game_environment(
    state: State<'_, AppState>,
    install_dir: String,
) -> GameEnvironmentProfile {
    state.core.inspect_installed_game_environment(install_dir)
}

#[tauri::command]
fn query_compatibility_matrix(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: Option<String>,
    game_version: Option<String>,
) -> Result<Vec<CompatibilityRecord>, String> {
    state
        .core
        .query_compatibility_matrix(&game_id, mod_id.as_deref(), game_version.as_deref())
        .map_err(map_err)
}

fn bridge_ledger_paths(data_dir: &Path, game_key: &str) -> (PathBuf, PathBuf) {
    let safe_key: String = game_key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let root = data_dir.join("deployments").join(safe_key);
    (root.join("backups"), root.join("ledger.json"))
}

fn suggest_companion_bridge(engine: GameEngineKind) -> (Option<String>, Option<String>) {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..");
    let (rel_src, target_rel) = match engine {
        GameEngineKind::UnityIl2cpp => (
            "plugins/unity-nekod-bridge/dist/il2cpp/NekodUnityBridge.IL2CPP.dll",
            "BepInEx/plugins/NekodUnityBridge.IL2CPP.dll",
        ),
        GameEngineKind::UnityMono => (
            "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
            "BepInEx/plugins/NekodUnityBridge.Mono.dll",
        ),
        GameEngineKind::ReEngine => (
            "plugins/reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua",
            "reframework/autorun/nekod_bridge.lua",
        ),
        GameEngineKind::UnrealEngine => (
            "plugins/ue4ss-nekod-bridge/Scripts/main.lua",
            "ue4ss/Mods/NekodBridge/Scripts/main.lua",
        ),
        _ => return (None, None),
    };
    let candidate = workspace_root.join(rel_src);
    let source = if candidate.is_file() {
        Some(candidate.display().to_string())
    } else {
        Some(rel_src.to_owned())
    };
    (source, Some(target_rel.to_owned()))
}

fn validate_bridge_target_path(rel_path: &str) -> Result<(), String> {
    let normalized = rel_path.replace('\\', "/");
    if normalized.contains("..") || normalized.starts_with('/') || normalized.contains(':') {
        return Err(format!("unsafe bridge target path '{rel_path}'"));
    }
    let lower = normalized.to_ascii_lowercase();
    let allowed_prefix = [
        "bepinex/plugins/",
        "reframework/autorun/",
        "ue4ss/mods/",
        "scripts/",
    ]
    .iter()
    .any(|p| lower.starts_with(p));
    if !allowed_prefix {
        return Err(format!(
            "target path '{rel_path}' must reside inside BepInEx/plugins/, reframework/autorun/, ue4ss/Mods/, or scripts/"
        ));
    }
    if [".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return Err(format!(
            "disallowed executable extension in bridge target '{rel_path}'"
        ));
    }
    Ok(())
}

#[tauri::command]
fn get_bridge_deployment_status(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeDeploymentStatus, String> {
    let env = state.core.inspect_installed_game_environment(&install_dir);
    let (suggested_source_file, suggested_target_relative_path) =
        suggest_companion_bridge(env.engine);
    let matched_bridge = list_bridge_catalog(&state.data_dir)
        .into_iter()
        .find(|b| b.engine == env.engine);
    let (loader_installed, loader_display_name, loader_warning) = if env.dir_exists {
        inspect_bepinex_loader_readiness(Path::new(&install_dir), env.engine)
    } else {
        (false, None, None)
    };
    let mut active_files = Vec::new();
    let mut loader_managed_by_nekod = false;
    if env.dir_exists {
        let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
        if ledger_path.is_file()
            && let Ok(ledger) = DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path)
        {
            for rec in ledger.records() {
                if rec.state == DeploymentState::Deployed {
                    if rec.package_id == "nekod.companion.bridge" {
                        active_files.push(rec.relative_path.clone());
                    } else if rec.package_id == "nekod.loader.bepinex" {
                        loader_managed_by_nekod = true;
                    }
                }
            }
        }
    }
    Ok(BridgeDeploymentStatus {
        deployed: !active_files.is_empty(),
        engine: env.engine,
        active_files,
        suggested_source_file,
        suggested_target_relative_path,
        matched_bridge,
        loader_installed,
        loader_managed_by_nekod,
        loader_display_name,
        loader_warning,
    })
}

#[tauri::command]
fn install_bepinex_loader(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeDeploymentStatus, String> {
    let env = state.core.inspect_installed_game_environment(&install_dir);
    if !env.dir_exists {
        return Err(format!("game install directory '{install_dir}' not found"));
    }
    if !env.anti_cheat_files_present.is_empty() {
        return Err(format!(
            "refusing BepInEx loader installation: anti-cheat markers present ({:?})",
            env.anti_cheat_files_present
        ));
    }

    let zip_bytes = download_or_load_cached_bepinex_zip(&state.data_dir, env.engine)?;
    let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
    let mut ledger =
        DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
    extract_bepinex_zip_to_ledger(&zip_bytes, &game_key, &mut ledger)?;

    get_bridge_deployment_status(state, game_key, install_dir)
}

#[tauri::command]
fn undeploy_bepinex_loader(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeUndeploySummary, String> {
    let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
    let mut ledger =
        DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
    let report = ledger
        .undeploy_package("nekod.loader.bepinex")
        .map_err(map_err)?;
    Ok(BridgeUndeploySummary {
        removed_count: report.removed_count,
        restored_originals: report.restored_originals,
        conflicts: report.conflicts,
        conflict_paths: report.conflict_paths,
    })
}

fn slugify_kebab(title: &str, store_app_id: Option<&str>) -> String {
    let mut out = String::new();
    let mut prev_hyphen = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen && !out.is_empty() {
            out.push('-');
            prev_hyphen = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    while out.len() > 44 {
        out.pop();
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() >= 2 {
        return out;
    }
    if let Some(app_id) = store_app_id
        && !app_id.is_empty()
        && app_id.chars().all(|c| c.is_ascii_digit())
    {
        return format!("unity-app-{app_id}");
    }
    "unity-local-game".to_owned()
}

#[tauri::command]
fn quick_setup_unity_starter(
    state: State<'_, AppState>,
    title: String,
    store_source: String,
    store_app_id: Option<String>,
    store_build_id: Option<String>,
    install_dir: String,
) -> Result<QuickSetupResult, String> {
    let env = state.core.inspect_installed_game_environment(&install_dir);
    if !env.dir_exists {
        return Err(format!("game install directory '{install_dir}' not found"));
    }
    if !env.anti_cheat_files_present.is_empty() {
        return Err(format!(
            "refusing Unity starter setup: anti-cheat markers present ({:?})",
            env.anti_cheat_files_present
        ));
    }

    let (flavor, loader) = match env.engine {
        GameEngineKind::UnityIl2cpp => ("il2cpp", "bepinex_il2cpp"),
        GameEngineKind::UnityMono => ("mono", "bepinex_mono"),
        other => {
            return Err(format!(
                "universal Unity starter only supports Unity Mono / IL2CPP games (detected '{other:?}')"
            ));
        }
    };

    let candidate_exes = NekodAppService::collect_candidate_exes_in_dir(&install_dir);
    let primary_exe = candidate_exes
        .into_iter()
        .next()
        .ok_or_else(|| format!("no primary game executable (.exe) found in '{install_dir}'"))?;

    let existing_manifests = state
        .core
        .list_registered_game_manifests()
        .map_err(map_err)?;
    let steam_app_u64 = if store_source.eq_ignore_ascii_case("steam") {
        store_app_id.as_deref().and_then(|s| s.parse::<u64>().ok())
    } else {
        None
    };

    let existing_match = existing_manifests.iter().find(|gm| {
        if let Some(aid) = steam_app_u64
            && gm.store_ids.steam_app_id == Some(aid)
        {
            return true;
        }
        gm.executables
            .iter()
            .any(|ex| ex.process_name.eq_ignore_ascii_case(&primary_exe))
    });

    let game_id = if let Some(gm) = existing_match {
        gm.id.clone()
    } else {
        let base_slug = slugify_kebab(&title, store_app_id.as_deref());
        let candidate_id = if existing_manifests.iter().any(|gm| gm.id == base_slug) {
            if let Some(aid) = steam_app_u64 {
                format!("{base_slug}-{aid}")
            } else {
                let hash_suffix = &sha256_hex(install_dir.as_bytes())[..8];
                format!("{base_slug}-{hash_suffix}")
            }
        } else {
            base_slug
        };

        let mut store_ids = serde_json::Map::new();
        if let Some(aid) = steam_app_u64 {
            store_ids.insert("steam_app_id".to_owned(), json!(aid));
        }

        let known_versions = if let Some(bid) = store_build_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            vec![json!({
                "version_id": "1.0.0",
                "display_version": format!("Steam Build {bid}"),
                "platform": "windows-x64",
                "store_build_id": bid
            })]
        } else {
            Vec::new()
        };

        let game_manifest = json!({
            "schema_version": "1.0.0",
            "id": candidate_id,
            "name": if title.trim().is_empty() { candidate_id.clone() } else { title.trim().to_owned() },
            "platforms": ["windows-x64"],
            "store_ids": store_ids,
            "executables": [{
                "platform": "windows-x64",
                "relative_path": primary_exe,
                "process_name": primary_exe,
                "architecture": "x86_64"
            }],
            "known_versions": known_versions,
            "compatibility_policy": {
                "multiplayer_mode": "singleplayer_only",
                "anti_cheat_detected": [],
                "trainer_policy": "allowed_singleplayer"
            }
        });
        let raw_gm = serde_json::to_string_pretty(&game_manifest).map_err(map_err)?;
        state
            .core
            .register_game_manifest(&raw_gm)
            .map_err(map_err)?
            .id
    };

    let trainer_id = format!("org.nekod.unity-starter.{game_id}");
    let pkg_dir = state.data_dir.join("generated-starters").join(&game_id);
    let scripts_dir = pkg_dir.join("scripts");
    fs::create_dir_all(&scripts_dir).map_err(map_err)?;

    let bridge_payload = json!({
        "schema_version": "1.0.0",
        "protocol": "nekod_json_line_v1",
        "flavor": flavor,
        "loader": loader,
        "host": "127.0.0.1",
        "port": 19540,
        "auto_discover_by_pid": true,
        "features": [
            {
                "kind": "toggle_field",
                "feature_id": "time-pause",
                "class_name": "UnityEngine.Time",
                "field_name": "timeScale",
                "enabled_value": 0.0,
                "disabled_value": 1.0
            },
            {
                "kind": "set_field",
                "feature_id": "game-speed",
                "class_name": "UnityEngine.Time",
                "field_name": "timeScale"
            },
            {
                "kind": "invoke_method",
                "feature_id": "unload-unused-assets",
                "class_name": "UnityEngine.Resources",
                "method_name": "UnloadUnusedAssets"
            }
        ]
    });
    let payload_bytes = serde_json::to_vec_pretty(&bridge_payload).map_err(map_err)?;
    fs::write(scripts_dir.join("unity_bridge.json"), &payload_bytes).map_err(map_err)?;

    let payload_sha = sha256_hex(&payload_bytes);
    let payload_len = payload_bytes.len() as u64;

    let mut target_game = serde_json::Map::new();
    target_game.insert("game_id".to_owned(), json!(game_id));
    target_game.insert(
        "game_name".to_owned(),
        json!(if title.trim().is_empty() {
            game_id.as_str()
        } else {
            title.trim()
        }),
    );
    if let Some(aid) = steam_app_u64 {
        target_game.insert("store_ids".to_owned(), json!({ "steam_app_id": aid }));
    }

    let trainer_manifest = json!({
        "schema_version": "1.0.0",
        "id": trainer_id,
        "version": "1.0.0",
        "name": "Nekod Universal Unity Control Pack",
        "description": "Zero-UnityExplorer control pack powered by NekodUnityBridge (Time Scale pause/multiplier and main-thread asset cleanup).",
        "authors": [{
            "name": "Nekod Contributors",
            "github": "OmaruKoma",
            "url": "https://github.com/OmaruKoma/Nekod"
        }],
        "license": "MIT",
        "source_repository": "https://github.com/OmaruKoma/Nekod",
        "target_game": target_game,
        "supported_versions": [{
            "platform": "windows-x64",
            "version_pattern": "*",
            "status": "verified"
        }],
        "runtime": {
            "kind": "unity_bridge_v1",
            "api_version": "1.0.0",
            "min_nekod_version": "0.1.0-alpha.0",
            "entrypoint": "scripts/unity_bridge.json",
            "supported_platforms": ["windows-x64"]
        },
        "permissions": [
            {
                "scope": "process.discover",
                "reason": "Match target Unity game PID with %LOCALAPPDATA%\\Nekod\\bridges\\unity-<pid>.json.",
                "required": true
            },
            {
                "scope": "process.write_memory",
                "reason": "Modify Unity fields/properties and invoke methods through NekodUnityBridge loopback RPC.",
                "required": true
            }
        ],
        "features": [
            {
                "id": "time-pause",
                "name": "Pause Game Time",
                "description": "Sets UnityEngine.Time.timeScale to 0.0 and restores 1.0 cleanly on LIFO rollback.",
                "category": "time",
                "control_type": "toggle",
                "default_hotkey": "F8",
                "mutates_memory": true,
                "supports_rollback": true,
                "risk_level": "low"
            },
            {
                "id": "game-speed",
                "name": "Game Speed Multiplier",
                "description": "Adjusts UnityEngine.Time.timeScale dynamically via NekodUnityBridge.",
                "category": "time",
                "control_type": "float",
                "value_range": {
                    "min": 0.1,
                    "max": 8.0,
                    "step": 0.25,
                    "default": 1.0
                },
                "mutates_memory": true,
                "supports_rollback": true,
                "risk_level": "low"
            },
            {
                "id": "unload-unused-assets",
                "name": "Unload Unused Assets",
                "description": "Calls UnityEngine.Resources.UnloadUnusedAssets on the Unity main thread.",
                "category": "other",
                "control_type": "action",
                "mutates_memory": false,
                "supports_rollback": false,
                "risk_level": "low"
            }
        ],
        "compatibility": {
            "singleplayer_only": true,
            "requires_offline_mode": false,
            "anti_cheat_incompatible": []
        },
        "artifacts": [{
            "filename": "scripts/unity_bridge.json",
            "sha256": payload_sha,
            "size_bytes": payload_len,
            "platform": "windows-x64"
        }]
    });
    let raw_tm = serde_json::to_string_pretty(&trainer_manifest).map_err(map_err)?;
    fs::write(pkg_dir.join("trainer-manifest.json"), raw_tm).map_err(map_err)?;

    state
        .core
        .ingest_local_trainer(&pkg_dir, TrustTier::Official)
        .map_err(map_err)?;
    state.core.rescan_games().map_err(map_err)?;

    Ok(QuickSetupResult {
        game_id,
        trainer_id,
        engine: env.engine,
    })
}

#[tauri::command]
fn list_companion_bridge_catalog(state: State<'_, AppState>) -> Vec<CompanionBridgeCatalogEntry> {
    list_bridge_catalog(&state.data_dir)
}

#[tauri::command]
fn auto_deploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
    custom_source_override: Option<String>,
) -> Result<BridgeDeploymentStatus, String> {
    let env = state.core.inspect_installed_game_environment(&install_dir);
    if !env.dir_exists {
        return Err(format!("game install directory '{install_dir}' not found"));
    }
    if !env.anti_cheat_files_present.is_empty() {
        return Err(format!(
            "refusing companion bridge deployment: anti-cheat markers present ({:?})",
            env.anti_cheat_files_present
        ));
    }

    let (bytes, expected_sha, target_relative_path) = resolve_bridge_bytes_for_engine(
        &state.data_dir,
        env.engine,
        custom_source_override.as_deref(),
    )?;
    validate_bridge_target_path(&target_relative_path)?;

    let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
    let mut ledger =
        DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
    ledger
        .stage_file(
            &game_key,
            "nekod.companion.bridge",
            &target_relative_path,
            &bytes,
            &expected_sha,
        )
        .map_err(map_err)?;

    get_bridge_deployment_status(state, game_key, install_dir)
}

#[tauri::command]
fn deploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
    source_file: String,
    target_relative_path: String,
) -> Result<BridgeDeploymentStatus, String> {
    validate_bridge_target_path(&target_relative_path)?;
    let env = state.core.inspect_installed_game_environment(&install_dir);
    if !env.dir_exists {
        return Err(format!("game install directory '{install_dir}' not found"));
    }
    if !env.anti_cheat_files_present.is_empty() {
        return Err(format!(
            "refusing companion bridge deployment: anti-cheat markers present ({:?})",
            env.anti_cheat_files_present
        ));
    }

    let bytes = fs::read(&source_file)
        .map_err(|e| format!("cannot read bridge source '{source_file}': {e}"))?;
    let expected_sha = sha256_hex(&bytes);
    let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
    let mut ledger =
        DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
    ledger
        .stage_file(
            &game_key,
            "nekod.companion.bridge",
            &target_relative_path,
            &bytes,
            &expected_sha,
        )
        .map_err(map_err)?;

    get_bridge_deployment_status(state, game_key, install_dir)
}

#[tauri::command]
fn undeploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeUndeploySummary, String> {
    let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
    let mut ledger =
        DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
    let report = ledger
        .undeploy_package("nekod.companion.bridge")
        .map_err(map_err)?;
    Ok(BridgeUndeploySummary {
        removed_count: report.removed_count,
        restored_originals: report.restored_originals,
        conflicts: report.conflicts,
        conflict_paths: report.conflict_paths,
    })
}

#[tauri::command]
fn sync_global_hotkeys(state: State<'_, AppState>, bindings: Vec<GlobalHotkeyBinding>) -> usize {
    state.hotkeys.set_bindings(bindings)
}

#[tauri::command]
fn get_game_artwork(
    store_source: String,
    store_app_id: Option<String>,
    install_dir: String,
) -> GameArtwork {
    resolve_game_artwork(&store_source, store_app_id.as_deref(), &install_dir)
}

fn validate_direct_launch_executable(
    install_dir: &str,
    executable_path: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let dir_canon = fs::canonicalize(install_dir)
        .map_err(|e| format!("cannot access game install directory '{install_dir}': {e}"))?;
    let exe_candidate = PathBuf::from(executable_path);
    let full_exe = if exe_candidate.is_absolute() {
        exe_candidate
    } else {
        dir_canon.join(exe_candidate)
    };
    let exe_canon = fs::canonicalize(&full_exe)
        .map_err(|e| format!("cannot locate executable '{}': {e}", full_exe.display()))?;
    if !exe_canon.starts_with(&dir_canon) {
        return Err(format!(
            "refusing to launch executable outside game install directory: '{}'",
            exe_canon.display()
        ));
    }
    if !exe_canon.is_file() {
        return Err(format!(
            "launch target is not a regular file: '{}'",
            exe_canon.display()
        ));
    }
    #[cfg(target_os = "windows")]
    {
        let is_exe = exe_canon
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false);
        if !is_exe {
            return Err(format!(
                "refusing direct launch of non-.exe target: '{}'",
                exe_canon.display()
            ));
        }
    }
    Ok((dir_canon, exe_canon))
}

#[tauri::command]
fn launch_game(
    store_source: String,
    store_app_id: Option<String>,
    install_dir: String,
    executable_path: Option<String>,
) -> Result<LaunchGameResult, String> {
    if store_source.eq_ignore_ascii_case("steam")
        && let Some(app_id) = store_app_id.as_deref()
        && !app_id.is_empty()
        && app_id.chars().all(|c| c.is_ascii_digit())
    {
        let uri = format!("steam://rungameid/{app_id}");
        #[cfg(target_os = "windows")]
        {
            Command::new("explorer.exe")
                .arg(&uri)
                .spawn()
                .map_err(|e| format!("failed to launch Steam URI '{uri}': {e}"))?;
        }
        #[cfg(not(target_os = "windows"))]
        {
            Command::new("xdg-open")
                .arg(&uri)
                .spawn()
                .map_err(|e| format!("failed to launch Steam URI '{uri}': {e}"))?;
        }
        return Ok(LaunchGameResult {
            method: "steam_uri".to_owned(),
            target: uri,
        });
    }

    let exe_str = executable_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "cannot launch game: no Steam AppID and no executable path recorded for this game"
                .to_owned()
        })?;
    let (dir_canon, exe_canon) = validate_direct_launch_executable(&install_dir, exe_str)?;
    Command::new(&exe_canon)
        .current_dir(&dir_canon)
        .spawn()
        .map_err(|e| format!("failed to launch '{}': {e}", exe_canon.display()))?;
    Ok(LaunchGameResult {
        method: "direct_executable".to_owned(),
        target: exe_canon.display().to_string(),
    })
}

impl AppState {
    pub fn new(core: NekodAppService) -> Self {
        Self {
            core,
            data_dir: std::env::temp_dir().join("nekod_desktop_state"),
            hotkeys: GlobalHotkeyService::new(),
        }
    }

    pub fn with_data_dir(core: NekodAppService, data_dir: PathBuf) -> Self {
        Self {
            core,
            data_dir,
            hotkeys: GlobalHotkeyService::new(),
        }
    }
}

pub fn desktop_invoke_handler<R: tauri::Runtime>()
-> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        rescan_games,
        list_library_games,
        list_trainers_for_game,
        grant_trainer_permissions,
        revoke_trainer_permissions,
        ingest_local_trainer,
        register_game_manifest,
        add_custom_game_path,
        remove_custom_game_path,
        list_custom_game_paths,
        get_security_policy,
        update_security_policy,
        load_profile,
        save_profile,
        export_profile_json,
        import_profile_json,
        get_diagnostic_logs,
        discover_running_processes,
        start_trainer_session,
        apply_trainer_feature,
        stop_trainer_session,
        get_session_diagnostics,
        list_registry_sources,
        add_third_party_registry_source,
        remove_registry_source,
        sync_registry_sources,
        list_community_trainers_for_game,
        check_trainer_updates,
        install_or_update_trainer_from_registry,
        inspect_game_environment,
        query_compatibility_matrix,
        get_bridge_deployment_status,
        list_companion_bridge_catalog,
        auto_deploy_companion_bridge,
        deploy_companion_bridge,
        undeploy_companion_bridge,
        install_bepinex_loader,
        undeploy_bepinex_loader,
        quick_setup_unity_starter,
        sync_global_hotkeys,
        get_game_artwork,
        launch_game,
    ]
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let core = NekodAppService::open_in_dir(
                &data_dir,
                Arc::new(CompositeRuntimeRouter::with_default_providers()),
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
            let state = AppState::with_data_dir(core, data_dir);
            let app_handle = app.handle().clone();
            state.hotkeys.start_watcher(move |event| {
                let _ = app_handle.emit("nekod://global-hotkey", event);
            });
            app.manage(state);
            Ok(())
        })
        .invoke_handler(desktop_invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running Nekod desktop application");
}
