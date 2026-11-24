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
        version: "0.5.5",
        workspace_relative_path: "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.Mono.dll",
        filename: "NekodUnityBridge.Mono.dll",
        description: "独立 BepInEx 5 Mono 伴生桥接插件，无需 UnityExplorer，支持四元数无万向锁 LateUpdate 骨骼大陀螺、场景守卫、通用 DamageAura、Harmony 方法挂钩写字段/复制/克隆子对象并重绑 Timeline（含 ExposedReference 与 Cinemachine 相机Follow/LookAt）/补齐组件与经 SHA-256 验签的包内贴图材质替换。",
        download_url: None,
        pinned_sha256: None,
    },
    BridgeSpec {
        id: "nekod-unity-bridge-il2cpp",
        display_name: "NekodUnityBridge (IL2CPP · 零 UnityExplorer)",
        engine: GameEngineKind::UnityIl2cpp,
        version: "0.5.5",
        workspace_relative_path: "plugins/unity-nekod-bridge/dist/il2cpp/NekodUnityBridge.IL2CPP.dll",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.IL2CPP.dll",
        filename: "NekodUnityBridge.IL2CPP.dll",
        description: "独立 BepInEx 6 IL2CPP 伴生桥接插件，无需 UnityExplorer，支持四元数无万向锁 LateUpdate 骨骼大陀螺、场景守卫、通用 DamageAura、Harmony 方法挂钩写字段/复制/克隆子对象并重绑 Timeline（含 ExposedReference 与 Cinemachine 相机Follow/LookAt）/补齐组件与经 SHA-256 验签的包内贴图材质替换。",
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
        version: "0.7.3",
        workspace_relative_path: "plugins/ue4ss-nekod-bridge/Scripts/main.lua",
        target_relative_path: "ue4ss/Mods/NekodBridge/Scripts/main.lua",
        filename: "main.lua",
        description: "面向 Unreal Engine 4/5 的 RE-UE4SS 单线程游戏帧调度伴生桥接脚本（支持纯 Lua File-IPC、只读 MCP 探测、运行期 UI 中文汉化、粘性属性锁定、头部自瞄锁敌、自动开火、第三人称大陀螺、原生杀戮光环、仓库满仓与关卡直通）。",
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

const RE_UE4SS_LOADER_SPEC: BepInExLoaderSpec = BepInExLoaderSpec {
    display_name: "RE-UE4SS v4.0.0 (UE4/UE5 引导器)",
    cache_filename: "UE4SS_v4.0.0.zip",
    download_url: "https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip",
    pinned_sha256: "251ad0d1381a241750e036f5918cbf2b5d7ec58ebc608cb2dd09d459d0c81e45",
};

/// Locates an Unreal Engine `Binaries/Win64` directory relative to `install_dir`.
pub fn detect_ue_win64_relative_dir(install_dir: &Path) -> Option<String> {
    if install_dir.join("Binaries").join("Win64").is_dir() {
        return Some("Binaries/Win64".to_owned());
    }
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            let sub = entry.path();
            if !sub.is_dir() {
                continue;
            }
            let Some(name) = sub.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case("engine") {
                continue;
            }
            if sub.join("Binaries").join("Win64").is_dir() {
                candidates.push(format!("{name}/Binaries/Win64"));
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// Resolves the relative `(Scripts/main.lua, enabled.txt)` staging paths for `NekodBridge`
/// inside an Unreal Engine installation directory.
pub fn resolve_ue4ss_target_paths(install_dir: &Path) -> (String, String) {
    if let Some(win64) = detect_ue_win64_relative_dir(install_dir) {
        let win64_dir = install_dir.join(&win64);
        if win64_dir.join("Mods").is_dir() && !win64_dir.join("ue4ss").is_dir() {
            return (
                format!("{win64}/Mods/NekodBridge/Scripts/main.lua"),
                format!("{win64}/Mods/NekodBridge/enabled.txt"),
            );
        }
        return (
            format!("{win64}/ue4ss/Mods/NekodBridge/Scripts/main.lua"),
            format!("{win64}/ue4ss/Mods/NekodBridge/enabled.txt"),
        );
    }
    (
        "ue4ss/Mods/NekodBridge/Scripts/main.lua".to_owned(),
        "ue4ss/Mods/NekodBridge/enabled.txt".to_owned(),
    )
}

pub fn bepinex_loader_spec_for_engine(
    engine: GameEngineKind,
) -> Option<&'static BepInExLoaderSpec> {
    match engine {
        GameEngineKind::UnityMono => Some(&BEPINEX_MONO_SPEC),
        GameEngineKind::UnityIl2cpp => Some(&BEPINEX_IL2CPP_SPEC),
        GameEngineKind::UnrealEngine => Some(&RE_UE4SS_LOADER_SPEC),
        _ => None,
    }
}

/// Checks whether the appropriate BepInEx doorstop proxy + core runtime (or RE-UE4SS for
/// Unreal Engine) is present in `install_dir`.
/// Returns `(loader_installed, loader_display_name, warning_if_missing)`.
pub fn inspect_bepinex_loader_readiness(
    install_dir: &Path,
    engine: GameEngineKind,
) -> (bool, Option<String>, Option<String>) {
    if engine == GameEngineKind::UnrealEngine {
        let win64_rel = detect_ue_win64_relative_dir(install_dir);
        let search_dir = win64_rel
            .as_deref()
            .map(|r| install_dir.join(r))
            .unwrap_or_else(|| install_dir.to_path_buf());
        let has_dll = search_dir.join("ue4ss").join("UE4SS.dll").is_file()
            || search_dir.join("UE4SS.dll").is_file()
            || install_dir.join("ue4ss").join("UE4SS.dll").is_file();
        let has_proxy = search_dir.join("dwmapi.dll").is_file()
            || search_dir.join("xinput1_3.dll").is_file()
            || install_dir.join("dwmapi.dll").is_file()
            || install_dir.join("xinput1_3.dll").is_file();
        let ready = has_dll && has_proxy;
        let warning = if ready {
            None
        } else {
            let target_hint = win64_rel.as_deref().unwrap_or("<Project>/Binaries/Win64");
            Some(format!(
                "缺少 RE-UE4SS 引导器（dwmapi.dll / xinput1_3.dll 与 ue4ss/UE4SS.dll），Unreal Engine 启动时不会加载 NekodBridge。点击「安装」即可自动下载校验并解压到 {target_hint}（支持一键无痕卸载）。"
            ))
        };
        return (
            ready,
            Some(RE_UE4SS_LOADER_SPEC.display_name.to_owned()),
            warning,
        );
    }

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

const DISALLOWED_LOADER_EXECUTABLE_EXTS: &[&str] =
    &[".exe", ".bat", ".cmd", ".ps1", ".vbs", ".msi"];

fn normalize_and_check_archive_entry(
    rel_path: &str,
    loader_label: &str,
) -> Result<(String, String), String> {
    let normalized = rel_path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.contains(':')
    {
        return Err(format!(
            "unsafe {loader_label} archive entry path '{rel_path}'"
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    if DISALLOWED_LOADER_EXECUTABLE_EXTS
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return Err(format!(
            "disallowed executable extension in {loader_label} archive entry '{rel_path}'"
        ));
    }

    Ok((trimmed.to_owned(), lower))
}

/// Validates and normalizes a BepInEx archive entry path before staging into a game directory.
pub fn validate_bepinex_archive_entry_path(rel_path: &str) -> Result<String, String> {
    let (trimmed, lower) = normalize_and_check_archive_entry(rel_path, "BepInEx")?;
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
    Ok(trimmed)
}

/// Validates and normalizes a RE-UE4SS archive entry path before staging into `Binaries/Win64/`.
pub fn validate_ue4ss_archive_entry_path(rel_path: &str) -> Result<String, String> {
    let (trimmed, lower) = normalize_and_check_archive_entry(rel_path, "RE-UE4SS")?;
    let allowed =
        matches!(lower.as_str(), "dwmapi.dll" | "xinput1_3.dll") || lower.starts_with("ue4ss/");
    if !allowed {
        return Err(format!(
            "archive entry '{rel_path}' is outside the RE-UE4SS allowlisted file tree"
        ));
    }
    Ok(trimmed)
}

/// Resolves the relative `<Project>/Binaries/Win64` target prefix for RE-UE4SS staging,
/// failing closed if no Unreal Engine `Binaries/Win64` or shipping executable directory is found.
pub fn resolve_ue4ss_install_prefix(install_dir: &Path) -> Result<Option<String>, String> {
    if let Some(win64) = detect_ue_win64_relative_dir(install_dir) {
        return Ok(Some(win64));
    }
    let is_direct_win64 = install_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("Win64"))
        || fs::read_dir(install_dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let p = entry.path();
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.to_ascii_lowercase().ends_with("-win64-shipping.exe"))
            });
    if is_direct_win64 {
        Ok(None)
    } else {
        Err(format!(
            "未在 '{}' 下找到 Unreal Engine 的 <Project>/Binaries/Win64 主程序目录，无法安全安装 RE-UE4SS。",
            install_dir.display()
        ))
    }
}

struct LoaderExtractConfig<'a> {
    loader_label: &'a str,
    package_id: &'a str,
    target_prefix: Option<&'a str>,
    progress_every: usize,
    start_message_prefix: &'a str,
    step_message_prefix: &'a str,
    done_message: &'a str,
    validate_entry: fn(&str) -> Result<String, String>,
}

fn extract_loader_zip_to_ledger_with_progress<F>(
    zip_bytes: &[u8],
    game_key: &str,
    ledger: &mut DeploymentLedger,
    cfg: &LoaderExtractConfig<'_>,
    mut on_progress: F,
) -> Result<usize, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|e| format!("failed to parse {} zip archive: {e}", cfg.loader_label))?;

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
        return Err(format!(
            "{} archive contained no stageable files",
            cfg.loader_label
        ));
    }

    on_progress(LoaderProgressEvent {
        stage: "extracting".to_owned(),
        current: 0,
        total: total_files,
        message: format!("{} (0 / {total_files})...", cfg.start_message_prefix),
    });

    let mut staged_count = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("failed reading archive entry #{i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry
            .name()
            .map_err(|e| format!("failed reading archive entry #{i} name: {e}"))?
            .into_owned();
        let clean_rel = (cfg.validate_entry)(&raw_name)?;

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed reading archive entry '{clean_rel}': {e}"))?;
        let sha = sha256_hex(&buf);
        let target_rel = match cfg.target_prefix {
            Some(prefix) => format!("{prefix}/{clean_rel}"),
            None => clean_rel,
        };
        ledger
            .stage_file_deferred(game_key, cfg.package_id, &target_rel, &buf, &sha)
            .map_err(|e| e.to_string())?;
        staged_count += 1;

        if staged_count % 32 == 0 {
            ledger.flush().map_err(|e| e.to_string())?;
        }
        if staged_count % cfg.progress_every == 0 || staged_count as u64 == total_files {
            on_progress(LoaderProgressEvent {
                stage: "extracting".to_owned(),
                current: staged_count as u64,
                total: total_files,
                message: format!(
                    "{} ({staged_count} / {total_files})",
                    cfg.step_message_prefix
                ),
            });
        }
    }

    ledger.flush().map_err(|e| e.to_string())?;
    on_progress(LoaderProgressEvent {
        stage: "done".to_owned(),
        current: staged_count as u64,
        total: total_files,
        message: cfg.done_message.to_owned(),
    });
    Ok(staged_count)
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
    on_progress: F,
) -> Result<usize, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let cfg = LoaderExtractConfig {
        loader_label: "BepInEx",
        package_id: "nekod.loader.bepinex",
        target_prefix: None,
        progress_every: 8,
        start_message_prefix: "开始写入引导加载器文件",
        step_message_prefix: "安全写入引导文件",
        done_message: "BepInEx 安装完成",
        validate_entry: validate_bepinex_archive_entry_path,
    };
    extract_loader_zip_to_ledger_with_progress(zip_bytes, game_key, ledger, &cfg, on_progress)
}

/// Extracts a verified RE-UE4SS zip archive into `ledger` under `nekod.loader.ue4ss`,
/// targeting `<Project>/Binaries/Win64/`.
#[allow(dead_code)]
pub fn extract_ue4ss_zip_to_ledger(
    zip_bytes: &[u8],
    game_key: &str,
    install_dir: &Path,
    ledger: &mut DeploymentLedger,
) -> Result<usize, String> {
    extract_ue4ss_zip_to_ledger_with_progress(zip_bytes, game_key, install_dir, ledger, |_| {})
}

/// Extracts a verified RE-UE4SS zip archive into `ledger` under `nekod.loader.ue4ss`
/// with throttled progress callbacks.
pub fn extract_ue4ss_zip_to_ledger_with_progress<F>(
    zip_bytes: &[u8],
    game_key: &str,
    install_dir: &Path,
    ledger: &mut DeploymentLedger,
    on_progress: F,
) -> Result<usize, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let prefix = resolve_ue4ss_install_prefix(install_dir)?;
    let cfg = LoaderExtractConfig {
        loader_label: "RE-UE4SS",
        package_id: "nekod.loader.ue4ss",
        target_prefix: prefix.as_deref(),
        progress_every: 4,
        start_message_prefix: "开始写入 RE-UE4SS 引导文件",
        step_message_prefix: "安全写入 RE-UE4SS 引导文件",
        done_message: "RE-UE4SS 安装完成",
        validate_entry: validate_ue4ss_archive_entry_path,
    };
    extract_loader_zip_to_ledger_with_progress(zip_bytes, game_key, ledger, &cfg, on_progress)
}

/// Loads a cached SHA-256-verified loader zip or downloads and verifies it over HTTPS.
#[allow(dead_code)]
pub fn download_or_load_cached_bepinex_zip(
    data_dir: &Path,
    engine: GameEngineKind,
) -> Result<Vec<u8>, String> {
    download_or_load_cached_bepinex_zip_with_progress(data_dir, engine, |_| {})
}

/// Streams a loader zip download in 64 KiB chunks with live progress callbacks, then verifies
/// the exact pinned SHA-256 checksum and writes it into `<data_dir>/loader-cache/`.
pub fn download_or_load_cached_bepinex_zip_with_progress<F>(
    data_dir: &Path,
    engine: GameEngineKind,
    mut on_progress: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(LoaderProgressEvent),
{
    let spec = bepinex_loader_spec_for_engine(engine)
        .ok_or_else(|| format!("no loader package applies to detected engine '{engine:?}'"))?;

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
        .map_err(|e| format!("failed to build HTTP client for loader download: {e}"))?;
    let mut resp = client
        .get(spec.download_url)
        .send()
        .map_err(|e| format!("failed downloading {}: {e}", spec.display_name))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{} download failed with HTTP {} from {}",
            spec.display_name,
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
            .map_err(|e| format!("failed reading {} download body: {e}", spec.display_name))?;
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
        .map_err(|e| format!("failed caching downloaded loader zip: {e}"))?;
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

    #[test]
    fn resolves_ue4ss_win64_paths_and_loader_readiness() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_ue4ss_paths_{nanos}"));
        let win64 = root.join("MyGame").join("Binaries").join("Win64");
        fs::create_dir_all(&win64).unwrap();

        let (main_rel, enabled_rel) = resolve_ue4ss_target_paths(&root);
        assert_eq!(
            main_rel,
            "MyGame/Binaries/Win64/ue4ss/Mods/NekodBridge/Scripts/main.lua"
        );
        assert_eq!(
            enabled_rel,
            "MyGame/Binaries/Win64/ue4ss/Mods/NekodBridge/enabled.txt"
        );

        let (ready_before, display, warn_before) =
            inspect_bepinex_loader_readiness(&root, GameEngineKind::UnrealEngine);
        assert!(!ready_before);
        assert!(display.unwrap().contains("RE-UE4SS"));
        assert!(warn_before.unwrap().contains("MyGame/Binaries/Win64"));

        // Install mock RE-UE4SS proxy + DLL inside <Project>/Binaries/Win64/
        fs::create_dir_all(win64.join("ue4ss")).unwrap();
        fs::write(win64.join("dwmapi.dll"), b"MZ").unwrap();
        fs::write(win64.join("ue4ss").join("UE4SS.dll"), b"MZ").unwrap();

        let (ready_after, _, warn_after) =
            inspect_bepinex_loader_readiness(&root, GameEngineKind::UnrealEngine);
        assert!(ready_after);
        assert!(warn_after.is_none());

        // When legacy Win64/Mods exists without Win64/ue4ss, deploy into Win64/Mods
        let _ = fs::remove_dir_all(win64.join("ue4ss"));
        fs::create_dir_all(win64.join("Mods")).unwrap();
        let (legacy_main, legacy_enabled) = resolve_ue4ss_target_paths(&root);
        assert_eq!(
            legacy_main,
            "MyGame/Binaries/Win64/Mods/NekodBridge/Scripts/main.lua"
        );
        assert_eq!(
            legacy_enabled,
            "MyGame/Binaries/Win64/Mods/NekodBridge/enabled.txt"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validates_and_stages_re_ue4ss_archive_with_clean_rollback() {
        let spec = bepinex_loader_spec_for_engine(GameEngineKind::UnrealEngine).unwrap();
        assert!(spec.download_url.starts_with("https://"));
        assert!(spec.download_url.contains("RE-UE4SS"));
        assert_eq!(spec.pinned_sha256.len(), 64);
        assert!(spec.pinned_sha256.chars().all(|c| c.is_ascii_hexdigit()));

        assert_eq!(
            validate_ue4ss_archive_entry_path("dwmapi.dll").unwrap(),
            "dwmapi.dll"
        );
        assert_eq!(
            validate_ue4ss_archive_entry_path("./xinput1_3.dll").unwrap(),
            "xinput1_3.dll"
        );
        assert_eq!(
            validate_ue4ss_archive_entry_path("ue4ss/UE4SS.dll").unwrap(),
            "ue4ss/UE4SS.dll"
        );
        assert_eq!(
            validate_ue4ss_archive_entry_path(
                "ue4ss\\Mods\\CheatManagerEnablerMod\\Scripts\\main.lua"
            )
            .unwrap(),
            "ue4ss/Mods/CheatManagerEnablerMod/Scripts/main.lua"
        );

        assert!(validate_ue4ss_archive_entry_path("../evil.dll").is_err());
        assert!(validate_ue4ss_archive_entry_path("ue4ss/payload.exe").is_err());
        assert!(validate_ue4ss_archive_entry_path("ue4ss/run.bat").is_err());
        assert!(validate_ue4ss_archive_entry_path("random.dll").is_err());
        assert!(validate_ue4ss_archive_entry_path("C:/dwmapi.dll").is_err());

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_ue4ss_extract_{nanos}"));
        let game_dir = root.join("GameRoot");
        let win64 = game_dir
            .join("SakuraSegment_10")
            .join("Binaries")
            .join("Win64");
        let bridge_dir = win64
            .join("ue4ss")
            .join("Mods")
            .join("NekodBridge")
            .join("Scripts");
        fs::create_dir_all(&bridge_dir).unwrap();
        let backup_dir = root.join("backups");
        let ledger_path = root.join("ledger.json");

        // Pre-existing dwmapi.dll and NekodBridge main.lua must be preserved safely across install + undeploy.
        fs::write(win64.join("dwmapi.dll"), b"ORIG-DWMAPI").unwrap();
        fs::write(bridge_dir.join("main.lua"), b"-- nekod bridge").unwrap();

        let (ready_before, _, _) =
            inspect_bepinex_loader_readiness(&game_dir, GameEngineKind::UnrealEngine);
        assert!(!ready_before);

        let mut zip_buf = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buf);
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("dwmapi.dll", opts).unwrap();
            writer.write_all(b"MZ-UE4SS-PROXY").unwrap();
            writer.start_file("ue4ss/UE4SS.dll", opts).unwrap();
            writer.write_all(b"MZ-UE4SS-CORE").unwrap();
            writer
                .start_file("ue4ss/Mods/CheatManagerEnablerMod/Scripts/main.lua", opts)
                .unwrap();
            writer.write_all(b"-- cheat manager enabler").unwrap();
            writer.finish().unwrap();
        }
        let zip_bytes = zip_buf.into_inner();

        let mut ledger = DeploymentLedger::open(&game_dir, &backup_dir, &ledger_path).unwrap();
        let mut events = Vec::new();
        let staged = extract_ue4ss_zip_to_ledger_with_progress(
            &zip_bytes,
            "sakura-segment",
            &game_dir,
            &mut ledger,
            |ev| events.push(ev),
        )
        .unwrap();
        assert_eq!(staged, 3);
        assert!(events.iter().any(|e| e.stage == "done"));
        assert_eq!(
            fs::read(win64.join("dwmapi.dll")).unwrap(),
            b"MZ-UE4SS-PROXY"
        );
        assert!(win64.join("ue4ss").join("UE4SS.dll").is_file());
        assert!(
            win64
                .join("ue4ss")
                .join("Mods")
                .join("CheatManagerEnablerMod")
                .join("Scripts")
                .join("main.lua")
                .is_file()
        );

        let (ready_after, _, warn_after) =
            inspect_bepinex_loader_readiness(&game_dir, GameEngineKind::UnrealEngine);
        assert!(ready_after);
        assert!(warn_after.is_none());

        // Undeploying nekod.loader.ue4ss restores original dwmapi.dll and keeps NekodBridge intact.
        let report = ledger.undeploy_package("nekod.loader.ue4ss").unwrap();
        assert_eq!(report.removed_count, 3);
        assert_eq!(report.restored_originals, 1);
        assert_eq!(report.conflicts, 0);
        assert_eq!(fs::read(win64.join("dwmapi.dll")).unwrap(), b"ORIG-DWMAPI");
        assert!(!win64.join("ue4ss").join("UE4SS.dll").exists());
        assert!(
            !win64
                .join("ue4ss")
                .join("Mods")
                .join("CheatManagerEnablerMod")
                .join("Scripts")
                .join("main.lua")
                .exists()
        );
        assert_eq!(
            fs::read(bridge_dir.join("main.lua")).unwrap(),
            b"-- nekod bridge"
        );

        // Must fail closed when <Project>/Binaries/Win64 is absent.
        let no_win64 = root.join("InvalidDir");
        fs::create_dir_all(&no_win64).unwrap();
        let mut bad_ledger = DeploymentLedger::open(
            &no_win64,
            root.join("bad_backups"),
            root.join("bad_ledger.json"),
        )
        .unwrap();
        let err = extract_ue4ss_zip_to_ledger(&zip_bytes, "bad-game", &no_win64, &mut bad_ledger)
            .unwrap_err();
        assert!(err.contains("Binaries/Win64"));

        let _ = fs::remove_dir_all(&root);
    }
}
