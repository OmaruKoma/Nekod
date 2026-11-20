//! Built-in Companion Bridge Bucket (Direction 4).
//!
//! Automatically resolves engine-matched companion bridge plugins (`NekodUnityBridge` Mono/IL2CPP,
//! `REFramework`, `UE4SS`, `Godot`) from local cache, workspace dist artifacts, or a SHA-256-verified
//! HTTPS download — eliminating manual DLL path entry while enforcing `DeploymentLedger` rollback.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nekod_game_detection::GameEngineKind;
use nekod_trainer_manager::sha256_hex;
use serde::Serialize;

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
        version: "0.2.0",
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
        version: "0.2.0",
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
