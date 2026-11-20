//! Built-in Companion Bridge Bucket (Direction 4).
//!
//! Automatically resolves engine-matched companion bridge plugins (`NekodUnityBridge` Mono/IL2CPP,
//! `REFramework`, `UE4SS`, `Godot`) from local cache, workspace dist artifacts, or a SHA-256-verified
//! HTTPS download — eliminating manual DLL path entry while enforcing `DeploymentLedger` rollback.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nekod_game_detection::GameEngineKind;
use nekod_trainer_manager::sha256_hex;
use nekod_trainer_runtime::DeploymentLedger;
use serde::Serialize;
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize)]
pub struct LoaderProgressEvent {
    pub stage: String,
    pub current: u64,
    pub total: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct BridgeSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub engine: GameEngineKind,
    pub version: &'static str,
    pub workspace_relative_path: &'static str,
    pub target_relative_path: &'static str,
    pub filename: &'static str,
    pub description: &'static str,
    /// Optional official release HTTPS URL + pinned SHA-256 for remote bucket download.
    pub download_url: Option<&'static str>,
    pub pinned_sha256: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionBridgeCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub engine: GameEngineKind,
    pub version: String,
    pub target_relative_path: String,
    pub description: String,
    pub ready_locally: bool,
    pub source_kind: String,
    pub resolved_path: Option<String>,
    pub sha256: Option<String>,
    pub download_url: Option<String>,
}

pub const BUILTIN_BRIDGES: &[BridgeSpec] = &[
    BridgeSpec {
        id: "nekod-unity-bridge-mono",
        display_name: "NekodUnityBridge (Mono · 零 UnityExplorer)",
        engine: GameEngineKind::UnityMono,
        version: "0.4.0",
        workspace_relative_path: "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.Mono.dll",
        filename: "NekodUnityBridge.Mono.dll",
        description: "独立 BepInEx 5 Mono 伴生桥接插件，无需 UnityExplorer，自动发现 PID 与本地一次性 Token。",
        download_url: None,
        pinned_sha256: None,
    },
    BridgeSpec {
        id: "nekod-unity-bridge-il2cpp",
        display_name: "NekodUnityBridge (IL2CPP · 零 UnityExplorer)",
        engine: GameEngineKind::UnityIl2cpp,
        version: "0.4.0",
        workspace_relative_path: "plugins/unity-nekod-bridge/dist/il2cpp/NekodUnityBridge.IL2CPP.dll",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.IL2CPP.dll",
        filename: "NekodUnityBridge.IL2CPP.dll",
        description: "独立 BepInEx 6 IL2CPP 伴生桥接插件，无需 UnityExplorer，通过反射与场景遍历支持 IL2CPP 游戏。",
        download_url: None,
        pinned_sha256: None,
    },
    BridgeSpec {
        id: "nekod-reframework-bridge",
        display_name: "Nekod REFramework Lua Bridge",
        engine: GameEngineKind::ReEngine,
        version: "0.1.0",
        workspace_relative_path: "plugins/reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua",
        target_relative_path: "reframework/autorun/nekod_bridge.lua",
        filename: "nekod_bridge.lua",
        description: "面向 RE Engine 的 REFramework Lua 伴生脚本。",
        download_url: None,
        pinned_sha256: None,
    },
    BridgeSpec {
        id: "nekod-ue4ss-bridge",
        display_name: "Nekod UE4SS Lua Bridge",
        engine: GameEngineKind::UnrealEngine,
        version: "0.1.0",
        workspace_relative_path: "plugins/ue4ss-nekod-bridge/Scripts/main.lua",
        target_relative_path: "ue4ss/Mods/NekodBridge/Scripts/main.lua",
        filename: "main.lua",
        description: "面向 Unreal Engine 4/5 的 UE4SS Lua 伴生脚本。",
        download_url: None,
        pinned_sha256: None,
    },
    BridgeSpec {
        id: "nekod-godot-bridge",
        display_name: "Nekod Godot GDScript Bridge",
        engine: GameEngineKind::Godot,
        version: "0.1.0",
        workspace_relative_path: "plugins/godot-nekod-bridge/nekod_bridge.gd",
        target_relative_path: "scripts/nekod_bridge.gd",
        filename: "nekod_bridge.gd",
        description: "面向 Godot 单机游戏的本地 GDScript 桥接脚本。",
        download_url: None,
        pinned_sha256: None,
    },
];

pub fn spec_for_engine(engine: GameEngineKind) -> Option<&'static BridgeSpec> {
    BUILTIN_BRIDGES.iter().find(|s| s.engine == engine)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn cache_path(data_dir: &Path, spec: &BridgeSpec) -> PathBuf {
    data_dir
        .join("bridge-bucket")
        .join(spec.id)
        .join(spec.version)
        .join(spec.filename)
}

/// Resolves an existing local bridge artifact from cache or workspace build output.
pub fn resolve_local_artifact(
    data_dir: &Path,
    spec: &BridgeSpec,
) -> Option<(PathBuf, &'static str)> {
    let cached = cache_path(data_dir, spec);
    if cached.is_file() {
        return Some((cached, "local_cache"));
    }

    let ws = workspace_root().join(spec.workspace_relative_path);
    if ws.is_file() {
        return Some((ws, "workspace_dist"));
    }

    // Also check next to the running desktop executable (`bridges/<id>/<filename>`)
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let bundled = dir.join("bridges").join(spec.id).join(spec.filename);
        if bundled.is_file() {
            return Some((bundled, "bundled_release"));
        }
    }

    None
}

/// Lists all built-in companion bridges with live availability & SHA-256 fingerprint.
pub fn list_catalog(data_dir: &Path) -> Vec<CompanionBridgeCatalogEntry> {
    BUILTIN_BRIDGES
        .iter()
        .map(|spec| {
            let resolved = resolve_local_artifact(data_dir, spec);
            let (ready_locally, source_kind, resolved_path, sha256) =
                if let Some((path, kind)) = resolved {
                    let digest = fs::read(&path).ok().map(|b| sha256_hex(&b));
                    (
                        true,
                        kind.to_owned(),
                        Some(path.display().to_string()),
                        digest,
                    )
                } else {
                    (
                        false,
                        if spec.download_url.is_some() {
                            "remote_bucket".to_owned()
                        } else {
                            "build_required".to_owned()
                        },
                        None,
                        spec.pinned_sha256.map(ToOwned::to_owned),
                    )
                };

            CompanionBridgeCatalogEntry {
                id: spec.id.to_owned(),
                display_name: spec.display_name.to_owned(),
                engine: spec.engine,
                version: spec.version.to_owned(),
                target_relative_path: spec.target_relative_path.to_owned(),
                description: spec.description.to_owned(),
                ready_locally,
                source_kind,
                resolved_path,
                sha256,
                download_url: spec.download_url.map(ToOwned::to_owned),
            }
        })
        .collect()
}

/// Downloads a bridge artifact over HTTPS and verifies its exact SHA-256 digest before caching.
pub fn fetch_verified_remote_bridge(
    data_dir: &Path,
    spec: &BridgeSpec,
    url: &str,
    expected_sha256: &str,
) -> Result<PathBuf, String> {
    if !url.starts_with("https://") {
        return Err(format!(
            "refusing non-HTTPS bridge bucket download URL '{url}'"
        ));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("failed to download companion bridge from '{url}': {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "bridge bucket download failed with HTTP {}",
            resp.status()
        ));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| format!("failed reading bridge download body: {e}"))?;
    let actual_sha = sha256_hex(&bytes);
    if !actual_sha.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "SHA-256 integrity mismatch for '{}': expected {expected_sha256}, got {actual_sha}",
            spec.id
        ));
    }

    let dest = cache_path(data_dir, spec);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed creating bridge cache directory: {e}"))?;
    }
    fs::write(&dest, &bytes).map_err(|e| format!("failed writing bridge cache file: {e}"))?;
    Ok(dest)
}

/// Resolves bytes + SHA-256 for the engine's companion bridge (automatically checking local cache,
/// workspace build artifacts, or optional pinned HTTPS download).
pub fn resolve_bridge_bytes_for_engine(
    data_dir: &Path,
    engine: GameEngineKind,
    custom_source_override: Option<&str>,
) -> Result<(Vec<u8>, String, String), String> {
    let spec = spec_for_engine(engine).ok_or_else(|| {
        format!("no built-in companion bridge is registered for detected engine '{engine:?}'")
    })?;

    if let Some(override_path) = custom_source_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let bytes = fs::read(override_path)
            .map_err(|e| format!("cannot read custom bridge source '{override_path}': {e}"))?;
        let sha = sha256_hex(&bytes);
        return Ok((bytes, sha, spec.target_relative_path.to_owned()));
    }

    if let Some((local_path, _kind)) = resolve_local_artifact(data_dir, spec) {
        let bytes = fs::read(&local_path).map_err(|e| {
            format!(
                "cannot read local bridge artifact '{}': {e}",
                local_path.display()
            )
        })?;
        let sha = sha256_hex(&bytes);
        return Ok((bytes, sha, spec.target_relative_path.to_owned()));
    }

    if let (Some(url), Some(pinned_sha)) = (spec.download_url, spec.pinned_sha256) {
        let downloaded = fetch_verified_remote_bridge(data_dir, spec, url, pinned_sha)?;
        let bytes = fs::read(&downloaded).map_err(|e| {
            format!(
                "cannot read downloaded bridge '{}': {e}",
                downloaded.display()
            )
        })?;
        return Ok((
            bytes,
            pinned_sha.to_owned(),
            spec.target_relative_path.to_owned(),
        ));
    }

    Err(format!(
        "companion bridge '{}' is not built or cached locally yet (expected '{}'). Run `plugins/unity-nekod-bridge/build.ps1` or select a local DLL.",
        spec.display_name, spec.workspace_relative_path
    ))
}

#[derive(Debug, Clone, Copy)]
pub struct BepInExLoaderSpec {
    pub display_name: &'static str,
    pub cache_filename: &'static str,
    pub download_url: &'static str,
    pub pinned_sha256: &'static str,
}

const BEPINEX_MONO_SPEC: BepInExLoaderSpec = BepInExLoaderSpec {
    display_name: "BepInEx 5 (Mono)",
    cache_filename: "BepInEx_win_x64_5.4.23.5.zip",
    download_url: "https://github.com/BepInEx/BepInEx/releases/download/v5.4.23.5/BepInEx_win_x64_5.4.23.5.zip",
    pinned_sha256: "82f9878551030f54657792c0740d9d51a09500eeae1fba21106b0c441e6732c4",
};

const BEPINEX_IL2CPP_SPEC: BepInExLoaderSpec = BepInExLoaderSpec {
    display_name: "BepInEx 6 (IL2CPP · Unity 6)",
    cache_filename: "BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.793+dbc82f0.zip",
    download_url: "https://builds.bepinex.dev/projects/bepinex_be/793/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.793%2Bdbc82f0.zip",
    pinned_sha256: "5b63d7d0e92a2448ccd13b738ff95ff1cc1026c6d718555d4df8e6c824953ac4",
};

pub fn bepinex_loader_spec_for_engine(
    engine: GameEngineKind,
) -> Option<&'static BepInExLoaderSpec> {
    match engine {
        GameEngineKind::UnityMono => Some(&BEPINEX_MONO_SPEC),
        GameEngineKind::UnityIl2cpp => Some(&BEPINEX_IL2CPP_SPEC),
        _ => None,
    }
}

/// Checks whether the appropriate BepInEx doorstop proxy + core runtime is present in `install_dir`.
/// Returns `(loader_installed, loader_display_name, warning_if_missing)`.
pub fn inspect_bepinex_loader_readiness(
    install_dir: &Path,
    engine: GameEngineKind,
) -> (bool, Option<String>, Option<String>) {
    let Some(spec) = bepinex_loader_spec_for_engine(engine) else {
        return (true, None, None);
    };

    let has_proxy =
        install_dir.join("winhttp.dll").is_file() || install_dir.join("version.dll").is_file();
    let core_dir = install_dir.join("BepInEx").join("core");
    let has_core = match engine {
        GameEngineKind::UnityIl2cpp => {
            core_dir.join("BepInEx.Unity.IL2CPP.dll").is_file()
                || core_dir.join("BepInEx.IL2CPP.dll").is_file()
        }
        GameEngineKind::UnityMono => core_dir.join("BepInEx.dll").is_file(),
        _ => true,
    };

    let ready = has_proxy && has_core;
    let warning = if ready {
        None
    } else {
        let label = match engine {
            GameEngineKind::UnityIl2cpp => "BepInEx 6 (IL2CPP)",
            GameEngineKind::UnityMono => "BepInEx 5 (Mono)",
            _ => "BepInEx",
        };
        Some(format!(
            "缺少 {label} 引导加载器（winhttp.dll / BepInEx/core），Unity 启动时不会加载桥接插件。请先一键安装加载器并重启游戏。"
        ))
    };

    (ready, Some(spec.display_name.to_owned()), warning)
}

/// Validates and normalizes an archive entry path before staging into a game directory.
pub fn validate_bepinex_archive_entry_path(rel_path: &str) -> Result<String, String> {
    let normalized = rel_path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.contains(':')
    {
        return Err(format!("unsafe BepInEx archive entry path '{rel_path}'"));
    }

    let lower = trimmed.to_ascii_lowercase();
    let allowed = matches!(
        lower.as_str(),
        "winhttp.dll" | "doorstop_config.ini" | ".doorstop_version" | "changelog.txt"
    ) || lower.starts_with("bepinex/")
        || lower.starts_with("dotnet/");
    if !allowed {
        return Err(format!(
            "archive entry '{rel_path}' is outside the BepInEx allowlisted file tree"
        ));
    }

    if [".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return Err(format!(
            "disallowed executable extension in BepInEx archive entry '{rel_path}'"
        ));
    }

    Ok(trimmed.to_owned())
}

/// Extracts a verified BepInEx zip archive into `ledger` under `nekod.loader.bepinex`,
/// persisting in checkpoints (`flush` every 32 files and once at completion).
#[allow(dead_code)]
pub fn extract_bepinex_zip_to_ledger(
    zip_bytes: &[u8],
    game_key: &str,
    ledger: &mut DeploymentLedger,
) -> Result<usize, String> {
    extract_bepinex_zip_to_ledger_with_progress(zip_bytes, game_key, ledger, |_| {})
}

/// Extracts a verified BepInEx zip archive into `ledger` with throttled progress callbacks.
pub fn extract_bepinex_zip_to_ledger_with_progress<F>(
    zip_bytes: &[u8],
    game_key: &str,
    ledger: &mut DeploymentLedger,
    mut on_progress: F,
) -> Result<usize, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|e| format!("failed to parse BepInEx zip archive: {e}"))?;

    let mut total_files = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("failed inspecting archive entry #{i}: {e}"))?;
        if !entry.is_dir() {
            total_files += 1;
        }
    }

    if total_files == 0 {
        return Err("BepInEx archive contained no stageable files".to_owned());
    }

    on_progress(LoaderProgressEvent {
        stage: "extracting".to_owned(),
        current: 0,
        total: total_files,
        message: format!("开始写入引导加载器文件 (0 / {total_files})..."),
    });

    let mut staged_count = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("failed reading archive entry #{i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().to_owned();
        let clean_rel = validate_bepinex_archive_entry_path(&raw_name)?;

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed reading archive entry '{clean_rel}': {e}"))?;
        let sha = sha256_hex(&buf);
        ledger
            .stage_file_deferred(game_key, "nekod.loader.bepinex", &clean_rel, &buf, &sha)
            .map_err(|e| e.to_string())?;
        staged_count += 1;

        if staged_count % 32 == 0 {
            ledger.flush().map_err(|e| e.to_string())?;
        }
        if staged_count % 8 == 0 || staged_count as u64 == total_files {
            on_progress(LoaderProgressEvent {
                stage: "extracting".to_owned(),
                current: staged_count as u64,
                total: total_files,
                message: format!("安全写入引导文件 ({staged_count} / {total_files})"),
            });
        }
    }

    ledger.flush().map_err(|e| e.to_string())?;
    on_progress(LoaderProgressEvent {
        stage: "done".to_owned(),
        current: staged_count as u64,
        total: total_files,
        message: "BepInEx 安装完成".to_owned(),
    });
    Ok(staged_count)
}

/// Loads a cached SHA-256-verified BepInEx zip or downloads and verifies it over HTTPS.
#[allow(dead_code)]
pub fn download_or_load_cached_bepinex_zip(
    data_dir: &Path,
    engine: GameEngineKind,
) -> Result<Vec<u8>, String> {
    download_or_load_cached_bepinex_zip_with_progress(data_dir, engine, |_| {})
}

/// Streams a BepInEx zip download in 64 KiB chunks with live progress callbacks, then verifies
/// the exact pinned SHA-256 checksum and writes it into `<data_dir>/loader-cache/`.
pub fn download_or_load_cached_bepinex_zip_with_progress<F>(
    data_dir: &Path,
    engine: GameEngineKind,
    mut on_progress: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let spec = bepinex_loader_spec_for_engine(engine).ok_or_else(|| {
        format!("no BepInEx loader package applies to detected engine '{engine:?}'")
    })?;

    if !spec.download_url.starts_with("https://") {
        return Err(format!(
            "refusing non-HTTPS loader URL '{}'",
            spec.download_url
        ));
    }

    let cache_file = data_dir.join("loader-cache").join(spec.cache_filename);
    if cache_file.is_file()
        && let Ok(cached_bytes) = fs::read(&cache_file)
    {
        let cached_sha = sha256_hex(&cached_bytes);
        if cached_sha.eq_ignore_ascii_case(spec.pinned_sha256) {
            let len = cached_bytes.len() as u64;
            on_progress(LoaderProgressEvent {
                stage: "cache_hit".to_owned(),
                current: len,
                total: len,
                message: "命中本地 SHA-256 缓存，准备解压...".to_owned(),
            });
            return Ok(cached_bytes);
        }
    }

    on_progress(LoaderProgressEvent {
        stage: "connecting".to_owned(),
        current: 0,
        total: 0,
        message: format!("正在连接官方镜像下载 {}...", spec.display_name),
    });

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("failed to build HTTP client for BepInEx download: {e}"))?;
    let mut resp = client
        .get(spec.download_url)
        .send()
        .map_err(|e| format!("failed downloading {}: {e}", spec.display_name))?;
    if !resp.status().is_success() {
        return Err(format!(
            "BepInEx download failed with HTTP {} from {}",
            resp.status(),
            spec.download_url
        ));
    }

    let total_bytes = resp.content_length().unwrap_or(0);
    let mut bytes = if total_bytes > 0 {
        Vec::with_capacity(total_bytes as usize)
    } else {
        Vec::new()
    };
    let mut chunk = [0u8; 64 * 1024];
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_millis(250))
        .unwrap_or_else(Instant::now);

    loop {
        let n = resp
            .read(&mut chunk)
            .map_err(|e| format!("failed reading BepInEx download body: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if last_emit.elapsed() >= Duration::from_millis(200) {
            last_emit = Instant::now();
            let downloaded_mb = bytes.len() as f64 / (1024.0 * 1024.0);
            let msg = if total_bytes > 0 {
                let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
                let pct = ((bytes.len() as u64).saturating_mul(100)) / total_bytes.max(1);
                format!(
                    "下载 {}: {downloaded_mb:.1} / {total_mb:.1} MB ({pct}%)",
                    spec.display_name
                )
            } else {
                format!("下载 {}: {downloaded_mb:.1} MB", spec.display_name)
            };
            on_progress(LoaderProgressEvent {
                stage: "downloading".to_owned(),
                current: bytes.len() as u64,
                total: total_bytes,
                message: msg,
            });
        }
    }

    let len = bytes.len() as u64;
    on_progress(LoaderProgressEvent {
        stage: "verifying".to_owned(),
        current: len,
        total: len,
        message: "正在校验 SHA-256 完整性...".to_owned(),
    });

    let actual_sha = sha256_hex(&bytes);
    if !actual_sha.eq_ignore_ascii_case(spec.pinned_sha256) {
        return Err(format!(
            "SHA-256 integrity mismatch for '{}': expected {}, got {actual_sha}",
            spec.cache_filename, spec.pinned_sha256
        ));
    }

    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed creating loader cache dir: {e}"))?;
    }
    fs::write(&cache_file, &bytes)
        .map_err(|e| format!("failed caching downloaded BepInEx zip: {e}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolves_builtin_catalog_and_rejects_http_downloads() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("nekod_bridge_bucket_{nanos}"));
        let entries = list_catalog(&data_dir);
        assert!(entries.len() >= 5);
        assert!(
            entries
                .iter()
                .any(|e| e.engine == GameEngineKind::UnityMono)
        );
        assert!(
            entries
                .iter()
                .any(|e| e.engine == GameEngineKind::UnityIl2cpp)
        );

        // Security: non-HTTPS download URL must be rejected
        let spec = spec_for_engine(GameEngineKind::UnityMono).unwrap();
        let err =
            fetch_verified_remote_bridge(&data_dir, spec, "http://example.com/bridge.dll", "0000")
                .unwrap_err();
        assert!(err.contains("non-HTTPS"));

        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn detects_missing_bepinex_loader_and_extracts_mock_archive_safely() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_bepinex_test_{nanos}"));
        let game_dir = root.join("Game");
        let backup_dir = root.join("backups");
        let ledger_path = root.join("ledger.json");

        // Even with NekodUnityBridge plugin present, missing winhttp.dll + BepInEx/core must report not installed.
        fs::create_dir_all(game_dir.join("BepInEx").join("plugins")).unwrap();
        fs::write(
            game_dir
                .join("BepInEx")
                .join("plugins")
                .join("NekodUnityBridge.IL2CPP.dll"),
            b"dll",
        )
        .unwrap();

        let (ready_before, name_before, warn_before) =
            inspect_bepinex_loader_readiness(&game_dir, GameEngineKind::UnityIl2cpp);
        assert!(!ready_before);
        assert!(name_before.unwrap().contains("BepInEx 6"));
        assert!(warn_before.is_some());

        // Reject zip-slip and executable entries
        assert!(validate_bepinex_archive_entry_path("../evil.dll").is_err());
        assert!(validate_bepinex_archive_entry_path("BepInEx/payload.exe").is_err());
        assert!(validate_bepinex_archive_entry_path("random_root_file.dll").is_err());

        // Build mock BepInEx zip and extract via DeploymentLedger
        let mut zip_buf = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buf);
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("winhttp.dll", opts).unwrap();
            writer.write_all(b"MZ-proxy").unwrap();
            writer
                .start_file("BepInEx/core/BepInEx.Unity.IL2CPP.dll", opts)
                .unwrap();
            writer.write_all(b"MZ-core").unwrap();
            writer.finish().unwrap();
        }

        let mut ledger = DeploymentLedger::open(&game_dir, &backup_dir, &ledger_path).unwrap();
        let staged =
            extract_bepinex_zip_to_ledger(&zip_buf.into_inner(), "test-game", &mut ledger).unwrap();
        assert_eq!(staged, 2);

        let (ready_after, _, warn_after) =
            inspect_bepinex_loader_readiness(&game_dir, GameEngineKind::UnityIl2cpp);
        assert!(ready_after);
        assert!(warn_after.is_none());

        // Clean LIFO undeploy removes BepInEx loader without touching NekodUnityBridge plugin
        let report = ledger.undeploy_package("nekod.loader.bepinex").unwrap();
        assert_eq!(report.removed_count, 2);
        assert!(!game_dir.join("winhttp.dll").exists());
        assert!(
            game_dir
                .join("BepInEx")
                .join("plugins")
                .join("NekodUnityBridge.IL2CPP.dll")
                .exists()
        );

        let _ = fs::remove_dir_all(&root);
    }
}
