//! L1 Tauri 2 desktop shell delegating all business & security operations to `nekod-core`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
use tauri::{Manager, State};

pub struct AppState {
    core: NekodAppService,
    data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeDeploymentStatus {
    pub deployed: bool,
    pub engine: GameEngineKind,
    pub active_files: Vec<String>,
    pub suggested_source_file: Option<String>,
    pub suggested_target_relative_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeUndeploySummary {
    pub removed_count: usize,
    pub restored_originals: usize,
    pub conflicts: usize,
    pub conflict_paths: Vec<String>,
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
    game_id: String,
) -> Result<Vec<ProcessIdentity>, String> {
    state
        .core
        .discover_running_processes_for_game(&game_id)
        .map_err(map_err)
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
    let mut active_files = Vec::new();
    if env.dir_exists {
        let (backup_dir, ledger_path) = bridge_ledger_paths(&state.data_dir, &game_key);
        if ledger_path.is_file()
            && let Ok(ledger) = DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path)
        {
            for rec in ledger.records() {
                if rec.state == DeploymentState::Deployed {
                    active_files.push(rec.relative_path.clone());
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
    })
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

impl AppState {
    pub fn new(core: NekodAppService) -> Self {
        Self {
            core,
            data_dir: std::env::temp_dir().join("nekod_desktop_state"),
        }
    }

    pub fn with_data_dir(core: NekodAppService, data_dir: PathBuf) -> Self {
        Self { core, data_dir }
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
        deploy_companion_bridge,
        undeploy_companion_bridge,
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
            app.manage(AppState::with_data_dir(core, data_dir));
            Ok(())
        })
        .invoke_handler(desktop_invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running Nekod desktop application");
}
