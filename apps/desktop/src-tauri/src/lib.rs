//! L1 Tauri 2 desktop shell delegating all business & security operations to `nekod-core`.

use std::collections::BTreeSet;
use std::sync::Arc;

use nekod_core::{
    CommunityTrainerView, DiagnosticEvent, LibraryGameView, NekodAppService, TrainerDetailView,
    TrainerPermissionState, TrainerUpdateInfo,
};
use nekod_manifest::{PermissionScope, TrustTier};
use nekod_permission_manager::SecurityPolicyConfig;
use nekod_profile_manager::{CustomGamePathRecord, GameProfile};
use nekod_registry_client::{RegistrySourceState, RegistrySyncReport};
use nekod_runtime_api::{FeatureCommand, ProcessIdentity, RuntimeDiagnostics, RuntimeSessionId};
use nekod_trainer_runtime::CompositeRuntimeRouter;
use tauri::{Manager, State};

pub struct AppState {
    core: NekodAppService,
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

impl AppState {
    pub fn new(core: NekodAppService) -> Self {
        Self { core }
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
                data_dir,
                Arc::new(CompositeRuntimeRouter::with_default_providers()),
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
            app.manage(AppState { core });
            Ok(())
        })
        .invoke_handler(desktop_invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running Nekod desktop application");
}
