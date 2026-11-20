//! L1 Tauri 2 desktop shell delegating all business & security operations to `nekod-core`.

mod artwork;
mod bridges;
mod hotkeys;

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use artwork::{GameArtwork, resolve_game_artwork};
use bridges::{
    CompanionBridgeCatalogEntry, download_or_load_cached_bepinex_zip_with_progress,
    extract_bepinex_zip_to_ledger_with_progress, inspect_bepinex_loader_readiness,
    list_catalog as list_bridge_catalog, resolve_bridge_bytes_for_engine,
};
use hotkeys::{GlobalHotkeyBinding, GlobalHotkeyService};
use nekod_core::{
    CommunityTrainerView, DiagnosticEvent, LibraryGameView, NekodAppService, TrainerDetailView,
    TrainerPermissionState, TrainerUpdateInfo,
};
use nekod_game_detection::{GameEngineKind, GameEnvironmentProfile};
use nekod_manifest::{
    GameManifest, ModKind, ModManifest, PermissionScope, RuntimeKind, TrustTier,
    is_valid_kebab_slug, validate_relative_path,
};
use nekod_permission_manager::SecurityPolicyConfig;
use nekod_profile_manager::{CustomGamePathRecord, GameProfile};
use nekod_registry_client::{RegistrySourceState, RegistrySyncReport};
use nekod_runtime_api::{FeatureCommand, ProcessIdentity, RuntimeDiagnostics, RuntimeSessionId};
use nekod_trainer_manager::{CompatibilityRecord, sha256_hex};
use nekod_trainer_runtime::{CompositeRuntimeRouter, DeploymentLedger, DeploymentState};
use nekod_validator::ct_importer::{
    CtImportOptions, SkippedCtEntry, convert_ct_xml_with_diagnostics,
};
use nekod_validator::{
    validate_game_manifest_str, validate_mod_manifest_str, validate_nmod_package_dir,
    validate_trainer_package_dir,
};
use serde::Serialize;
use serde_json::json;
use tauri::{Emitter, Manager, State};
use zip::ZipArchive;

pub struct AppState {
    core: Arc<NekodAppService>,
    data_dir: PathBuf,
    hotkeys: GlobalHotkeyService,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeDeploymentStatus {
    pub deployed: bool,
    pub bridge_update_available: bool,
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

#[derive(Debug, Clone, Serialize)]
pub struct CtImportUiReport {
    pub imported: bool,
    pub trainer_id: String,
    pub game_id: String,
    pub package_dir: Option<String>,
    pub converted_features: Vec<String>,
    pub skipped_entries: Vec<SkippedCtEntry>,
    pub modules: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalNmodCandidate {
    pub mod_id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub mod_kind: ModKind,
    pub target_game_id: String,
    pub target_game_name: String,
    pub target_steam_app_id: Option<u64>,
    pub runtime_kind: RuntimeKind,
    pub feature_count: usize,
    pub source_path: String,
    pub source_format: String,
    pub already_installed: bool,
}

const MAX_CT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_NMOD_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_NMOD_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_NMOD_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_NMOD_ENTRIES: usize = 256;
const DISALLOWED_ARCHIVE_EXTENSIONS: &[&str] = &[".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"];

fn map_err(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn sanitize_package_id_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "nmod-pkg".to_owned()
    } else {
        cleaned
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(map_err)?;
    for entry in fs::read_dir(src).map_err(map_err)? {
        let entry = entry.map_err(map_err)?;
        let ty = entry.file_type().map_err(map_err)?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(map_err)?;
            }
            fs::copy(&from, &to).map_err(map_err)?;
        }
    }
    Ok(())
}

fn extract_and_validate_nmod_archive(
    archive_path: &Path,
    data_dir: &Path,
) -> Result<PathBuf, String> {
    let meta = fs::metadata(archive_path)
        .map_err(|e| format!("无法访问 .nmod 文件 '{}': {e}", archive_path.display()))?;
    if meta.len() > MAX_NMOD_ARCHIVE_BYTES {
        return Err(format!(
            ".nmod 压缩包体积 ({} bytes) 超过 32 MiB 安全上限。",
            meta.len()
        ));
    }

    let file = fs::File::open(archive_path)
        .map_err(|e| format!("无法打开 .nmod 文件 '{}': {e}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|e| {
        format!(
            "无效的 .nmod / ZIP 压缩包 '{}': {e}",
            archive_path.display()
        )
    })?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging_dir = data_dir
        .join("staging-nmods")
        .join(format!("{}_{nanos}", std::process::id()));
    fs::create_dir_all(&staging_dir).map_err(map_err)?;

    let result = (|| -> Result<PathBuf, String> {
        let mut entry_count = 0usize;
        let mut total_uncompressed = 0u64;

        for i in 0..archive.len() {
            let entry = archive
                .by_index(i)
                .map_err(|e| format!("读取 .nmod 归档条目 #{i} 失败: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            if let Some(mode) = entry.unix_mode()
                && (mode & 0o170000) == 0o120000
            {
                return Err(format!(
                    "拒绝包含符号链接的 .nmod 归档条目 '{}'",
                    entry.name()
                ));
            }
            entry_count += 1;
            if entry_count > MAX_NMOD_ENTRIES {
                return Err(format!(".nmod 归档文件数超过 {MAX_NMOD_ENTRIES} 安全上限"));
            }

            let normalized = entry.name().replace('\\', "/");
            validate_relative_path(&normalized).map_err(map_err)?;
            let lower = normalized.to_ascii_lowercase();
            if DISALLOWED_ARCHIVE_EXTENSIONS
                .iter()
                .any(|ext| lower.ends_with(ext))
            {
                return Err(format!(
                    "source-first .nmod 归档禁止包含可执行文件 '{normalized}'"
                ));
            }

            let remaining = MAX_NMOD_UNCOMPRESSED_BYTES.saturating_sub(total_uncompressed);
            let mut buf = Vec::new();
            entry
                .take(remaining + 1)
                .read_to_end(&mut buf)
                .map_err(map_err)?;
            total_uncompressed = total_uncompressed.saturating_add(buf.len() as u64);
            if total_uncompressed > MAX_NMOD_UNCOMPRESSED_BYTES {
                return Err(".nmod 解压总大小超过 64 MiB 安全上限".to_owned());
            }

            let dest = staging_dir.join(&normalized);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(map_err)?;
            }
            fs::write(&dest, &buf).map_err(map_err)?;
        }

        let (pkg_id, pkg_ver) = if staging_dir.join("mod-manifest.json").is_file() {
            let (m, _) = validate_nmod_package_dir(&staging_dir).map_err(map_err)?;
            (m.id, m.version.to_string())
        } else if staging_dir.join("trainer-manifest.json").is_file() {
            let (m, _) = validate_trainer_package_dir(&staging_dir).map_err(map_err)?;
            (m.id, m.version.to_string())
        } else {
            return Err(
                ".nmod 归档根目录缺少有效的 mod-manifest.json 或 trainer-manifest.json".to_owned(),
            );
        };

        let install_dir = data_dir
            .join("installed-nmods")
            .join(sanitize_package_id_segment(&pkg_id))
            .join(sanitize_package_id_segment(&pkg_ver));
        if install_dir.exists() {
            fs::remove_dir_all(&install_dir).map_err(map_err)?;
        }
        copy_dir_recursive(&staging_dir, &install_dir)?;
        Ok(install_dir)
    })();

    let _ = fs::remove_dir_all(&staging_dir);
    result
}

#[tauri::command]
async fn rescan_games(state: State<'_, AppState>) -> Result<Vec<LibraryGameView>, String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || core.rescan_games().map_err(map_err))
        .await
        .map_err(map_err)?
}

#[tauri::command]
async fn list_library_games(state: State<'_, AppState>) -> Result<Vec<LibraryGameView>, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        core.list_library_games().map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn list_trainers_for_game(
    state: State<'_, AppState>,
    game_id: String,
    detected_build_id: Option<String>,
) -> Result<Vec<TrainerDetailView>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.list_trainers_for_game(&game_id, detected_build_id.as_deref())
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn grant_trainer_permissions(
    state: State<'_, AppState>,
    trainer_id: String,
    confirmed_scopes: Vec<PermissionScope>,
) -> Result<TrainerPermissionState, String> {
    let scope_set: BTreeSet<PermissionScope> = confirmed_scopes.into_iter().collect();
    spawn_core(Arc::clone(&state.core), move |core| {
        core.grant_trainer_permissions(&trainer_id, scope_set)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn revoke_trainer_permissions(
    state: State<'_, AppState>,
    trainer_id: String,
) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.revoke_trainer_permissions(&trainer_id)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn ingest_local_trainer(
    state: State<'_, AppState>,
    package_dir: String,
    trust_tier: TrustTier,
) -> Result<(), String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let trimmed = package_dir.trim().trim_matches('"');
        let raw_path = PathBuf::from(trimmed);
        let pkg_path = if raw_path.is_file() {
            let ext = raw_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "nmod" || ext == "zip" {
                extract_and_validate_nmod_archive(&raw_path, &data_dir)?
            } else {
                return Err(format!(
                    "不支持的文件格式 '{}'；请选择 .nmod 归档包或包含 mod-manifest.json 的目录。",
                    raw_path.display()
                ));
            }
        } else if raw_path.is_dir() {
            raw_path
        } else {
            return Err(format!(
                "路径 '{}' 不存在（请填写本地 .nmod 文件或模组目录路径）。",
                raw_path.display()
            ));
        };

        let sibling_gm = pkg_path.join("game-manifest.json");
        if sibling_gm.is_file() {
            let raw_gm = fs::read_to_string(&sibling_gm).map_err(map_err)?;
            core.register_game_manifest(&raw_gm).map_err(map_err)?;
        }
        core.ingest_local_trainer(&pkg_path, trust_tier)
            .map(|_| ())
            .map_err(map_err)?;
        core.rescan_games().map_err(map_err)?;
        Ok(())
    })
    .await
    .map_err(map_err)?
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallTrainerResult {
    pub trainer_id: String,
    pub name: String,
    pub removed_managed_files: bool,
}

fn remove_dir_if_strictly_inside(target: &Path, managed_roots: &[PathBuf]) -> bool {
    let Ok(canon_target) = fs::canonicalize(target) else {
        return false;
    };
    for root in managed_roots {
        if let Ok(canon_root) = fs::canonicalize(root)
            && canon_target != canon_root
            && canon_target.starts_with(&canon_root)
            && fs::remove_dir_all(&canon_target).is_ok()
        {
            if let Some(parent) = canon_target.parent()
                && parent != canon_root
                && parent.starts_with(&canon_root)
            {
                let _ = fs::remove_dir(parent);
            }
            return true;
        }
    }
    false
}

#[tauri::command]
async fn uninstall_trainer(
    state: State<'_, AppState>,
    trainer_id: String,
) -> Result<UninstallTrainerResult, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (removed, mut removed_files) =
            core.uninstall_trainer(&trainer_id, true).map_err(map_err)?;
        if !removed_files {
            let extra_roots = [
                data_dir.join("installed-nmods"),
                data_dir.join("generated-starters"),
                data_dir.join("imported-ct"),
            ];
            removed_files =
                remove_dir_if_strictly_inside(Path::new(&removed.package_dir), &extra_roots);
        }
        let _ = core.rescan_games();
        Ok(UninstallTrainerResult {
            trainer_id: removed.manifest.id,
            name: removed.manifest.name,
            removed_managed_files: removed_files,
        })
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn pick_local_path(
    mode: String,
    title: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        {
            let mut dialog = rfd::FileDialog::new();
            if let Some(t) = title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                dialog = dialog.set_title(t);
            }
            if mode == "folder" {
                Ok(dialog.pick_folder().map(|p| p.display().to_string()))
            } else {
                if let Some(exts) = extensions.as_ref() {
                    let clean: Vec<&str> = exts
                        .iter()
                        .map(|s| s.trim().trim_start_matches('.'))
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !clean.is_empty() {
                        dialog = dialog.add_filter("Supported Files", &clean);
                    }
                }
                Ok(dialog.pick_file().map(|p| p.display().to_string()))
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut cmd = Command::new("zenity");
            cmd.arg("--file-selection");
            if mode == "folder" {
                cmd.arg("--directory");
            }
            if let Some(t) = title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                cmd.arg(format!("--title={t}"));
            }
            if mode != "folder"
                && let Some(exts) = extensions.as_ref()
                && !exts.is_empty()
            {
                let patterns: Vec<String> = exts
                    .iter()
                    .map(|s| format!("*.{}", s.trim().trim_start_matches('.')))
                    .collect();
                cmd.arg(format!("--file-filter={}", patterns.join(" ")));
            }
            match cmd.output() {
                Ok(out) if out.status.success() => {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                    Ok((!s.is_empty()).then_some(s))
                }
                _ => Ok(None),
            }
        }
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn register_game_manifest(
    state: State<'_, AppState>,
    raw_json: String,
) -> Result<String, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.register_game_manifest(&raw_json)
            .map(|m| m.id)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn add_custom_game_path(
    state: State<'_, AppState>,
    path: String,
    label: Option<String>,
) -> Result<Vec<CustomGamePathRecord>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        let trimmed = path.trim().trim_matches('"');
        if trimmed.is_empty() {
            return Err("请输入游戏目录或可执行文件 (.exe) 路径。".to_owned());
        }
        if !Path::new(trimmed).exists() {
            return Err(format!("路径 '{trimmed}' 不存在，请检查目录或文件位置。"));
        }
        core.add_custom_game_path(trimmed, label.as_deref())
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn remove_custom_game_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<CustomGamePathRecord>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.remove_custom_game_path(&path).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn list_custom_game_paths(
    state: State<'_, AppState>,
) -> Result<Vec<CustomGamePathRecord>, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        core.list_custom_game_paths().map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn get_security_policy(state: State<'_, AppState>) -> Result<SecurityPolicyConfig, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        core.get_security_policy().map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn update_security_policy(
    state: State<'_, AppState>,
    policy: SecurityPolicyConfig,
) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.update_security_policy(policy).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn load_profile(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
) -> Result<Option<GameProfile>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.load_profile(&game_id, &trainer_id).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn save_profile(state: State<'_, AppState>, profile: GameProfile) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.save_profile(&profile).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn export_profile_json(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
) -> Result<String, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.export_profile_json(&game_id, &trainer_id)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn import_profile_json(
    state: State<'_, AppState>,
    raw_json: String,
) -> Result<GameProfile, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.import_profile_json(&raw_json).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn get_diagnostic_logs(state: State<'_, AppState>) -> Result<Vec<DiagnosticEvent>, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        Ok(core.get_diagnostic_logs())
    })
    .await
}

#[tauri::command]
async fn discover_running_processes(
    state: State<'_, AppState>,
    game_id: Option<String>,
    install_dir: Option<String>,
) -> Result<Vec<ProcessIdentity>, String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(gid) = game_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let procs = core
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
            return core.discover_running_processes_in_dir(dir).map_err(map_err);
        }

        Ok(Vec::new())
    })
    .await
    .map_err(map_err)?
}

async fn spawn_core<T: Send + 'static>(
    core: Arc<NekodAppService>,
    work: impl FnOnce(&NekodAppService) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || work(&core))
        .await
        .map_err(map_err)?
}

#[tauri::command]
async fn start_trainer_session(
    state: State<'_, AppState>,
    game_id: String,
    trainer_id: String,
    target_process: ProcessIdentity,
) -> Result<RuntimeSessionId, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.start_trainer_session(&game_id, &trainer_id, target_process)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn apply_trainer_feature(
    state: State<'_, AppState>,
    session_id: String,
    command: FeatureCommand,
) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.apply_trainer_feature(&RuntimeSessionId(session_id), command)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn stop_trainer_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<RuntimeDiagnostics, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.stop_trainer_session(&RuntimeSessionId(session_id))
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn get_session_diagnostics(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<RuntimeDiagnostics, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.get_session_diagnostics(&RuntimeSessionId(session_id))
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn list_registry_sources(
    state: State<'_, AppState>,
) -> Result<Vec<RegistrySourceState>, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        core.list_registry_sources().map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn add_third_party_registry_source(
    state: State<'_, AppState>,
    repository_id: String,
    index_url: String,
    enabled: bool,
) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.add_third_party_registry_source(&repository_id, &index_url, enabled)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn remove_registry_source(
    state: State<'_, AppState>,
    repository_id: String,
) -> Result<(), String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.remove_registry_source(&repository_id).map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn sync_registry_sources(state: State<'_, AppState>) -> Result<RegistrySyncReport, String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || core.sync_registry_sources().map_err(map_err))
        .await
        .map_err(map_err)?
}

#[tauri::command]
async fn list_community_trainers_for_game(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<CommunityTrainerView>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.list_community_trainers_for_game(&game_id)
            .map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn check_trainer_updates(
    state: State<'_, AppState>,
) -> Result<Vec<TrainerUpdateInfo>, String> {
    spawn_core(Arc::clone(&state.core), |core| {
        core.check_trainer_updates().map_err(map_err)
    })
    .await
}

#[tauri::command]
async fn install_or_update_trainer_from_registry(
    state: State<'_, AppState>,
    repository_id: String,
    trainer_id: String,
) -> Result<(), String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        core.install_or_update_trainer_from_registry(&repository_id, &trainer_id)
            .map(|_| ())
            .map_err(map_err)
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn inspect_game_environment(
    state: State<'_, AppState>,
    install_dir: String,
) -> Result<GameEnvironmentProfile, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        Ok(core.inspect_installed_game_environment(install_dir))
    })
    .await
}

#[tauri::command]
async fn query_compatibility_matrix(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: Option<String>,
    game_version: Option<String>,
) -> Result<Vec<CompatibilityRecord>, String> {
    spawn_core(Arc::clone(&state.core), move |core| {
        core.query_compatibility_matrix(&game_id, mod_id.as_deref(), game_version.as_deref())
            .map_err(map_err)
    })
    .await
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

fn compute_bridge_deployment_status(
    core: &NekodAppService,
    data_dir: &Path,
    game_key: &str,
    install_dir: &str,
) -> Result<BridgeDeploymentStatus, String> {
    let env = core.inspect_installed_game_environment(install_dir);
    let (suggested_source_file, suggested_target_relative_path) =
        suggest_companion_bridge(env.engine);
    let matched_bridge = list_bridge_catalog(data_dir)
        .into_iter()
        .find(|b| b.engine == env.engine);
    let (loader_installed, loader_display_name, loader_warning) = if env.dir_exists {
        inspect_bepinex_loader_readiness(Path::new(install_dir), env.engine)
    } else {
        (false, None, None)
    };
    let mut active_files = Vec::new();
    let mut loader_managed_by_nekod = false;
    if env.dir_exists {
        let (backup_dir, ledger_path) = bridge_ledger_paths(data_dir, game_key);
        if ledger_path.is_file()
            && let Ok(ledger) = DeploymentLedger::open(install_dir, &backup_dir, &ledger_path)
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
    let deployed = !active_files.is_empty();
    let bridge_update_available = if deployed
        && let Some(catalog_sha) = matched_bridge.as_ref().and_then(|b| b.sha256.as_deref())
        && let Some(first_rel) = active_files.first()
        && let Ok(bytes) = fs::read(Path::new(install_dir).join(first_rel))
    {
        sha256_hex(&bytes) != catalog_sha
    } else {
        false
    };
    Ok(BridgeDeploymentStatus {
        deployed,
        bridge_update_available,
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
async fn get_bridge_deployment_status(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeDeploymentStatus, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        compute_bridge_deployment_status(&core, &data_dir, &game_key, &install_dir)
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn install_bepinex_loader<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeDeploymentStatus, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let env = core.inspect_installed_game_environment(&install_dir);
        if !env.dir_exists {
            return Err(format!("game install directory '{install_dir}' not found"));
        }
        if !env.anti_cheat_files_present.is_empty() {
            return Err(format!(
                "refusing BepInEx loader installation: anti-cheat markers present ({:?})",
                env.anti_cheat_files_present
            ));
        }

        let zip_bytes =
            download_or_load_cached_bepinex_zip_with_progress(&data_dir, env.engine, |ev| {
                let _ = app.emit("nekod://loader-progress", ev);
            })?;
        let (backup_dir, ledger_path) = bridge_ledger_paths(&data_dir, &game_key);
        let mut ledger =
            DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
        extract_bepinex_zip_to_ledger_with_progress(&zip_bytes, &game_key, &mut ledger, |ev| {
            let _ = app.emit("nekod://loader-progress", ev);
        })?;

        compute_bridge_deployment_status(&core, &data_dir, &game_key, &install_dir)
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn undeploy_bepinex_loader(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeUndeploySummary, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (backup_dir, ledger_path) = bridge_ledger_paths(&data_dir, &game_key);
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
    })
    .await
    .map_err(map_err)?
}

fn ascii_kebab_segment(raw: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut prev_hyphen = true;
    for ch in raw.chars() {
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
    while out.len() > max_len {
        out.pop();
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn slugify_kebab(title: &str, store_app_id: Option<&str>) -> String {
    let out = ascii_kebab_segment(title, 44);
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
async fn quick_setup_unity_starter(
    state: State<'_, AppState>,
    title: String,
    store_source: String,
    store_app_id: Option<String>,
    store_build_id: Option<String>,
    install_dir: String,
) -> Result<QuickSetupResult, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        quick_setup_unity_starter_sync(
            &core,
            &data_dir,
            title,
            store_source,
            store_app_id,
            store_build_id,
            install_dir,
        )
    })
    .await
    .map_err(map_err)?
}

fn parse_steam_app_u64(store_source: Option<&str>, store_app_id: Option<&str>) -> Option<u64> {
    if store_source.is_some_and(|s| s.eq_ignore_ascii_case("steam")) {
        store_app_id.and_then(|s| s.trim().parse::<u64>().ok())
    } else {
        None
    }
}

fn resolve_game_id_candidate(
    existing_manifests: &[nekod_manifest::GameManifest],
    title: &str,
    requested_game_id: Option<&str>,
    steam_app_u64: Option<u64>,
    primary_exe: Option<&str>,
    install_dir: Option<&str>,
    store_app_id: Option<&str>,
) -> Result<(String, bool), String> {
    if let Some(req) = requested_game_id.map(str::trim).filter(|s| !s.is_empty()) {
        if !is_valid_kebab_slug(req) {
            return Err(format!(
                "game_id '{req}' must be a valid kebab-case slug (lowercase ASCII letters, digits, hyphens)"
            ));
        }
        let exists = existing_manifests.iter().any(|gm| gm.id == req);
        return Ok((req.to_owned(), exists));
    }

    if let Some(gm) = existing_manifests.iter().find(|gm| {
        if let Some(aid) = steam_app_u64
            && gm.store_ids.steam_app_id == Some(aid)
        {
            return true;
        }
        if let Some(exe) = primary_exe
            && gm
                .executables
                .iter()
                .any(|ex| ex.process_name.eq_ignore_ascii_case(exe))
        {
            return true;
        }
        false
    }) {
        return Ok((gm.id.clone(), true));
    }

    let base_slug = slugify_kebab(title, store_app_id);
    let candidate_id = if existing_manifests.iter().any(|gm| gm.id == base_slug) {
        if let Some(aid) = steam_app_u64 {
            format!("{base_slug}-{aid}")
        } else {
            let disambig_input = install_dir.unwrap_or(title);
            let hash_suffix = &sha256_hex(disambig_input.as_bytes())[..8];
            format!("{base_slug}-{hash_suffix}")
        }
    } else {
        base_slug
    };
    Ok((candidate_id, false))
}

#[allow(clippy::too_many_arguments)]
fn register_minimal_game_manifest(
    core: &NekodAppService,
    candidate_id: &str,
    title: &str,
    steam_app_u64: Option<u64>,
    store_build_id: Option<&str>,
    primary_exe: &str,
) -> Result<String, String> {
    let mut store_ids = serde_json::Map::new();
    if let Some(aid) = steam_app_u64 {
        store_ids.insert("steam_app_id".to_owned(), json!(aid));
    }

    let known_versions = if let Some(bid) = store_build_id.map(str::trim).filter(|s| !s.is_empty())
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

    let display_name = if title.trim().is_empty() {
        candidate_id.to_owned()
    } else {
        title.trim().to_owned()
    };

    let game_manifest = json!({
        "schema_version": "1.0.0",
        "id": candidate_id,
        "name": display_name,
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
    core.register_game_manifest(&raw_gm)
        .map(|m| m.id)
        .map_err(map_err)
}

fn quick_setup_unity_starter_sync(
    core: &NekodAppService,
    data_dir: &Path,
    title: String,
    store_source: String,
    store_app_id: Option<String>,
    store_build_id: Option<String>,
    install_dir: String,
) -> Result<QuickSetupResult, String> {
    let env = core.inspect_installed_game_environment(&install_dir);
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

    let steam_app_u64 = parse_steam_app_u64(Some(&store_source), store_app_id.as_deref());
    let existing_manifests = core.list_registered_game_manifests().map_err(map_err)?;
    let (candidate_id, already_exists) = resolve_game_id_candidate(
        &existing_manifests,
        &title,
        None,
        steam_app_u64,
        Some(&primary_exe),
        Some(&install_dir),
        store_app_id.as_deref(),
    )?;
    let game_id = if already_exists {
        candidate_id
    } else {
        register_minimal_game_manifest(
            core,
            &candidate_id,
            &title,
            steam_app_u64,
            store_build_id.as_deref(),
            &primary_exe,
        )?
    };

    let trainer_id = format!("org.nekod.unity-starter.{game_id}");
    let pkg_dir = data_dir.join("generated-starters").join(&game_id);
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

    core.ingest_local_trainer(&pkg_dir, TrustTier::Official)
        .map_err(map_err)?;
    core.rescan_games().map_err(map_err)?;

    Ok(QuickSetupResult {
        game_id,
        trainer_id,
        engine: env.engine,
    })
}

#[tauri::command]
async fn list_companion_bridge_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<CompanionBridgeCatalogEntry>, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || list_bridge_catalog(&data_dir))
        .await
        .map_err(map_err)
}

#[tauri::command]
async fn auto_deploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
    custom_source_override: Option<String>,
) -> Result<BridgeDeploymentStatus, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let env = core.inspect_installed_game_environment(&install_dir);
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
            &data_dir,
            env.engine,
            custom_source_override.as_deref(),
        )?;
        validate_bridge_target_path(&target_relative_path)?;

        let (backup_dir, ledger_path) = bridge_ledger_paths(&data_dir, &game_key);
        let mut ledger =
            DeploymentLedger::open(&install_dir, &backup_dir, &ledger_path).map_err(map_err)?;
        if ledger.records().iter().any(|r| {
            r.state == DeploymentState::Deployed && r.package_id == "nekod.companion.bridge"
        }) {
            ledger
                .undeploy_package("nekod.companion.bridge")
                .map_err(map_err)?;
        }
        ledger
            .stage_file(
                &game_key,
                "nekod.companion.bridge",
                &target_relative_path,
                &bytes,
                &expected_sha,
            )
            .map_err(map_err)?;

        compute_bridge_deployment_status(&core, &data_dir, &game_key, &install_dir)
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn deploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
    source_file: String,
    target_relative_path: String,
) -> Result<BridgeDeploymentStatus, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_bridge_target_path(&target_relative_path)?;
        let env = core.inspect_installed_game_environment(&install_dir);
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
        let (backup_dir, ledger_path) = bridge_ledger_paths(&data_dir, &game_key);
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

        compute_bridge_deployment_status(&core, &data_dir, &game_key, &install_dir)
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn undeploy_companion_bridge(
    state: State<'_, AppState>,
    game_key: String,
    install_dir: String,
) -> Result<BridgeUndeploySummary, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (backup_dir, ledger_path) = bridge_ledger_paths(&data_dir, &game_key);
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
    })
    .await
    .map_err(map_err)?
}

#[tauri::command]
async fn sync_global_hotkeys(
    state: State<'_, AppState>,
    bindings: Vec<GlobalHotkeyBinding>,
) -> Result<usize, String> {
    let hotkeys = state.hotkeys.clone();
    tauri::async_runtime::spawn_blocking(move || hotkeys.set_bindings(bindings))
        .await
        .map_err(map_err)
}

#[tauri::command]
async fn get_game_artwork(
    store_source: String,
    store_app_id: Option<String>,
    install_dir: String,
) -> Result<GameArtwork, String> {
    tauri::async_runtime::spawn_blocking(move || {
        resolve_game_artwork(&store_source, store_app_id.as_deref(), &install_dir)
    })
    .await
    .map_err(map_err)
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
async fn launch_game(
    store_source: String,
    store_app_id: Option<String>,
    install_dir: String,
    executable_path: Option<String>,
) -> Result<LaunchGameResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
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

        let resolved_exe = executable_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                nekod_game_detection::custom::detect_primary_executable(Path::new(&install_dir))
                    .or_else(|| {
                        NekodAppService::collect_candidate_exes_in_dir(&install_dir)
                            .into_iter()
                            .next()
                            .map(|name| Path::new(&install_dir).join(name))
                    })
                    .map(|p| p.display().to_string())
            });
        let exe_str = resolved_exe.as_deref().ok_or_else(|| {
            "cannot launch game: no Steam AppID and no executable path found for this game"
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
    })
    .await
    .map_err(map_err)?
}

fn build_ct_trainer_id(game_id: &str, ct_path: &Path) -> String {
    let stem = ct_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("table");
    let stem_slug = ascii_kebab_segment(stem, 40);
    let effective_stem = if stem_slug.is_empty() {
        "table".to_owned()
    } else {
        stem_slug
    };
    let mut tid = format!("local.ct.{game_id}.{effective_stem}");
    if tid.len() > 120 {
        tid.truncate(120);
        while tid.ends_with(['-', '.', '_']) {
            tid.pop();
        }
    }
    tid
}

#[allow(clippy::too_many_arguments)]
fn import_ct_file_sync(
    core: &NekodAppService,
    data_dir: &Path,
    ct_path: String,
    game_id: Option<String>,
    game_name: String,
    default_module: Option<String>,
    install_dir: Option<String>,
    store_source: Option<String>,
    store_app_id: Option<String>,
    store_build_id: Option<String>,
    trust_tier: TrustTier,
) -> Result<CtImportUiReport, String> {
    let trimmed_path = ct_path.trim().trim_matches('"');
    if trimmed_path.is_empty() {
        return Err("请填写 .CT Cheat Engine 表格文件路径。".to_owned());
    }
    let ct_file = Path::new(trimmed_path);
    let meta = fs::metadata(ct_file)
        .map_err(|e| format!("无法访问 .CT 文件 '{}': {e}", ct_file.display()))?;
    if !meta.is_file() {
        return Err(format!("路径 '{}' 不是有效文件。", ct_file.display()));
    }
    if meta.len() > MAX_CT_FILE_BYTES {
        return Err(format!(
            ".CT 文件体积 ({} bytes) 超过 16 MiB 安全上限。",
            meta.len()
        ));
    }

    let raw_xml = fs::read_to_string(ct_file)
        .map_err(|e| format!("无法以 UTF-8 读取 .CT 文件 '{}': {e}", ct_file.display()))?;

    let clean_install_dir = install_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let primary_exe = clean_install_dir.and_then(|dir| {
        NekodAppService::collect_candidate_exes_in_dir(dir)
            .into_iter()
            .next()
    });
    let effective_module = default_module
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| primary_exe.clone());

    let steam_app_u64 = parse_steam_app_u64(store_source.as_deref(), store_app_id.as_deref());
    let existing_manifests = core.list_registered_game_manifests().map_err(map_err)?;
    let (resolved_game_id, already_exists) = resolve_game_id_candidate(
        &existing_manifests,
        &game_name,
        game_id.as_deref(),
        steam_app_u64,
        primary_exe.as_deref(),
        clean_install_dir,
        store_app_id.as_deref(),
    )?;

    let effective_game_name = if game_name.trim().is_empty() {
        resolved_game_id.clone()
    } else {
        game_name.trim().to_owned()
    };
    let trainer_id = build_ct_trainer_id(&resolved_game_id, ct_file);
    let table_title = ct_file
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|stem| format!("CT Import: {stem}"))
        .unwrap_or_else(|| format!("{effective_game_name} (.CT Import)"));

    let opts = CtImportOptions {
        trainer_id: trainer_id.clone(),
        trainer_name: table_title,
        game_id: resolved_game_id.clone(),
        game_name: effective_game_name.clone(),
        source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
        default_module: effective_module.clone(),
    };

    let (maybe_pkg, summary) = convert_ct_xml_with_diagnostics(&raw_xml, &opts).map_err(map_err)?;

    let Some(pkg) = maybe_pkg else {
        return Ok(CtImportUiReport {
            imported: false,
            trainer_id,
            game_id: resolved_game_id,
            package_dir: None,
            converted_features: summary.converted_features,
            skipped_entries: summary.skipped_entries.clone(),
            modules: summary.modules,
            message: format!(
                "未找到可安全静态转换的声明式修改项（已安全跳过 {} 条脚本/动态条目，例如 {{$lua}} 或 alloc）。",
                summary.skipped_entries.len()
            ),
        });
    };

    if !already_exists {
        let exe_for_manifest = primary_exe
            .or_else(|| effective_module.clone())
            .unwrap_or_else(|| format!("{resolved_game_id}.exe"));
        register_minimal_game_manifest(
            core,
            &resolved_game_id,
            &effective_game_name,
            steam_app_u64,
            store_build_id.as_deref(),
            &exe_for_manifest,
        )?;
    }

    let out_dir = data_dir.join("imported-ct").join(&trainer_id);
    if out_dir.exists() {
        let _ = fs::remove_dir_all(&out_dir);
    }
    fs::create_dir_all(&out_dir).map_err(map_err)?;
    pkg.write_to_dir(&out_dir).map_err(map_err)?;
    validate_trainer_package_dir(&out_dir).map_err(map_err)?;

    core.ingest_local_trainer(&out_dir, trust_tier)
        .map_err(map_err)?;
    core.rescan_games().map_err(map_err)?;

    Ok(CtImportUiReport {
        imported: true,
        trainer_id,
        game_id: resolved_game_id,
        package_dir: Some(out_dir.display().to_string()),
        converted_features: summary.converted_features.clone(),
        skipped_entries: summary.skipped_entries.clone(),
        modules: summary.modules,
        message: format!(
            "已静态转换并导入 {} 项声明式功能（安全跳过 {} 项不安全或非声明式条目）。",
            summary.converted_features.len(),
            summary.skipped_entries.len()
        ),
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn import_ct_file(
    state: State<'_, AppState>,
    ct_path: String,
    game_id: Option<String>,
    game_name: String,
    default_module: Option<String>,
    install_dir: Option<String>,
    store_source: Option<String>,
    store_app_id: Option<String>,
    store_build_id: Option<String>,
    trust_tier: TrustTier,
) -> Result<CtImportUiReport, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_ct_file_sync(
            &core,
            &data_dir,
            ct_path,
            game_id,
            game_name,
            default_module,
            install_dir,
            store_source,
            store_app_id,
            store_build_id,
            trust_tier,
        )
    })
    .await
    .map_err(map_err)?
}

fn nmod_search_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![data_dir.join("nmods")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        roots.push(parent.join("nmods"));
    }
    let workspace_examples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("examples");
    roots.push(workspace_examples);
    roots
}

fn read_zip_json_member<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    member_name: &str,
) -> Option<String> {
    let entry = archive.by_name(member_name).ok()?;
    let mut bytes = Vec::new();
    entry
        .take(MAX_NMOD_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_NMOD_MANIFEST_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn inspect_nmod_dir(dir: &Path) -> Option<(ModManifest, Option<GameManifest>)> {
    let raw_mod = fs::read_to_string(dir.join("mod-manifest.json")).ok()?;
    let mod_manifest = validate_mod_manifest_str(&raw_mod).ok()?;
    let companion_gm = fs::read_to_string(dir.join("game-manifest.json"))
        .ok()
        .and_then(|raw| validate_game_manifest_str(&raw).ok());
    Some((mod_manifest, companion_gm))
}

fn inspect_nmod_archive(file: &Path) -> Option<(ModManifest, Option<GameManifest>)> {
    let meta = fs::metadata(file).ok()?;
    if !meta.is_file() || meta.len() > MAX_NMOD_ARCHIVE_BYTES {
        return None;
    }
    let f = fs::File::open(file).ok()?;
    let mut archive = ZipArchive::new(f).ok()?;
    let raw_mod = read_zip_json_member(&mut archive, "mod-manifest.json")?;
    let mod_manifest = validate_mod_manifest_str(&raw_mod).ok()?;
    let companion_gm = read_zip_json_member(&mut archive, "game-manifest.json")
        .and_then(|raw| validate_game_manifest_str(&raw).ok());
    Some((mod_manifest, companion_gm))
}

fn nmod_matches_game(
    mod_manifest: &ModManifest,
    companion_gm: Option<&GameManifest>,
    req_game_id: Option<&str>,
    req_steam_id: Option<u64>,
    req_exes: &[String],
) -> bool {
    if let Some(gid) = req_game_id
        && mod_manifest.target_game.game_id == gid
    {
        return true;
    }
    if let Some(aid) = req_steam_id
        && mod_manifest
            .target_game
            .store_ids
            .as_ref()
            .and_then(|s| s.steam_app_id)
            == Some(aid)
    {
        return true;
    }
    if let Some(gm) = companion_gm {
        if let Some(aid) = req_steam_id
            && gm.store_ids.steam_app_id == Some(aid)
        {
            return true;
        }
        if gm.executables.iter().any(|ex| {
            req_exes
                .iter()
                .any(|rx| rx.eq_ignore_ascii_case(&ex.process_name))
        }) {
            return true;
        }
    }
    false
}

fn scan_nmod_root(
    root: &Path,
    core: &NekodAppService,
    req_game_id: Option<&str>,
    req_steam_id: Option<u64>,
    req_exes: &[String],
    seen: &mut BTreeSet<(String, String)>,
    out: &mut Vec<LocalNmodCandidate>,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let (parsed, source_format) = if path.is_dir() && path.join("mod-manifest.json").is_file() {
            (inspect_nmod_dir(&path), "nmod_dir")
        } else if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("nmod"))
        {
            (inspect_nmod_archive(&path), "nmod_archive")
        } else {
            continue;
        };

        let Some((mod_manifest, companion_gm)) = parsed else {
            continue;
        };
        if !nmod_matches_game(
            &mod_manifest,
            companion_gm.as_ref(),
            req_game_id,
            req_steam_id,
            req_exes,
        ) {
            continue;
        }

        let ver_str = mod_manifest.version.to_string();
        if !seen.insert((mod_manifest.id.clone(), ver_str.clone())) {
            continue;
        }

        let already_installed = core
            .list_trainers_for_game(&mod_manifest.target_game.game_id, None)
            .map(|list| {
                list.iter()
                    .any(|t| !t.revoked && t.trainer_id == mod_manifest.id && t.version == ver_str)
            })
            .unwrap_or(false);

        out.push(LocalNmodCandidate {
            mod_id: mod_manifest.id,
            version: ver_str,
            name: mod_manifest.name,
            description: mod_manifest.description,
            mod_kind: mod_manifest.mod_kind,
            target_game_id: mod_manifest.target_game.game_id,
            target_game_name: mod_manifest.target_game.game_name,
            target_steam_app_id: mod_manifest
                .target_game
                .store_ids
                .and_then(|s| s.steam_app_id)
                .or_else(|| companion_gm.and_then(|gm| gm.store_ids.steam_app_id)),
            runtime_kind: mod_manifest.runtime.kind,
            feature_count: mod_manifest.features.len(),
            source_path: path.display().to_string(),
            source_format: source_format.to_owned(),
            already_installed,
        });
    }
}

#[tauri::command]
async fn discover_local_nmods(
    state: State<'_, AppState>,
    game_id: Option<String>,
    steam_app_id: Option<String>,
    install_dir: Option<String>,
    executable_path: Option<String>,
) -> Result<Vec<LocalNmodCandidate>, String> {
    let core = Arc::clone(&state.core);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let req_game_id = game_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let req_steam_id = steam_app_id
            .as_deref()
            .and_then(|s| s.trim().parse::<u64>().ok());

        let mut req_exes = Vec::new();
        if let Some(exe_path) = executable_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && let Some(name) = Path::new(exe_path).file_name().and_then(|n| n.to_str())
        {
            req_exes.push(name.to_owned());
        }
        if let Some(dir) = install_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            for exe in NekodAppService::collect_candidate_exes_in_dir(dir) {
                if !req_exes.iter().any(|x| x.eq_ignore_ascii_case(&exe)) {
                    req_exes.push(exe);
                }
            }
        }

        if req_game_id.is_none() && req_steam_id.is_none() && req_exes.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen = BTreeSet::new();
        let mut results = Vec::new();
        for root in nmod_search_roots(&data_dir) {
            if root.is_dir() {
                scan_nmod_root(
                    &root,
                    &core,
                    req_game_id.as_deref(),
                    req_steam_id,
                    &req_exes,
                    &mut seen,
                    &mut results,
                );
            }
        }
        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    })
    .await
    .map_err(map_err)?
}

impl AppState {
    pub fn new(core: NekodAppService) -> Self {
        Self {
            core: Arc::new(core),
            data_dir: std::env::temp_dir().join("nekod_desktop_state"),
            hotkeys: GlobalHotkeyService::new(),
        }
    }

    pub fn with_data_dir(core: NekodAppService, data_dir: PathBuf) -> Self {
        Self {
            core: Arc::new(core),
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
        uninstall_trainer,
        pick_local_path,
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
        import_ct_file,
        discover_local_nmods,
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
