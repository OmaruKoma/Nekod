//! Deterministic on-disk game engine, mod loader, and anti-cheat directory inspector.
//!
//! Inspects local game installation folders without launching processes or reading memory,
//! supporting both Player Dependency Resolution (L2) and AI Mod Forge Recommendation (L6).

use std::fs;
use std::path::Path;

use nekod_manifest::AntiCheatKind;
use serde::{Deserialize, Serialize};

/// Identified runtime/engine architecture of an installed game directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameEngineKind {
    UnityMono,
    UnityIl2cpp,
    UnrealEngine,
    ReEngine,
    Godot,
    NativeUnknown,
}

/// Known third-party mod loader or bridge installation detected inside a game folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledLoaderKind {
    BepInExMono,
    BepInExIl2cpp,
    MelonLoader,
    NekodUnityBridge,
    UnityExplorerMcpBridge,
    Ue4ss,
    ReFramework,
    ScriptHookV,
    ScriptHookRdr2,
}

/// Concrete mod loader installation observed inside a game's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedLoader {
    pub kind: InstalledLoaderKind,
    pub entry_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<String>,
}

/// Inspection snapshot of an installed game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameEnvironmentProfile {
    pub install_dir: String,
    pub dir_exists: bool,
    pub engine: GameEngineKind,
    pub installed_loaders: Vec<DetectedLoader>,
    pub anti_cheat_files_present: Vec<AntiCheatKind>,
    pub recommended_adapters: Vec<String>,
}

/// Inspects `install_dir` for game engine signatures, installed mod loaders, and known
/// anti-cheat redistributable directories.
pub fn inspect_game_directory(install_dir: impl AsRef<Path>) -> GameEnvironmentProfile {
    let root = install_dir.as_ref();
    let dir_str = root.display().to_string();
    if !root.is_dir() {
        return GameEnvironmentProfile {
            install_dir: dir_str,
            dir_exists: false,
            engine: GameEngineKind::NativeUnknown,
            installed_loaders: Vec::new(),
            anti_cheat_files_present: Vec::new(),
            recommended_adapters: vec!["declarative_patch_v1".to_owned()],
        };
    }

    let engine = detect_engine(root);
    let installed_loaders = detect_loaders(root);
    let anti_cheat_files_present = detect_anti_cheat_markers(root);
    let recommended_adapters = recommend_adapters(engine, &anti_cheat_files_present);

    GameEnvironmentProfile {
        install_dir: dir_str,
        dir_exists: true,
        engine,
        installed_loaders,
        anti_cheat_files_present,
        recommended_adapters,
    }
}

fn detect_engine(root: &Path) -> GameEngineKind {
    let has_unity_player = root.join("UnityPlayer.dll").is_file()
        || has_dir_matching(root, |name| name.ends_with("_Data"));
    if has_unity_player {
        if root.join("GameAssembly.dll").is_file() || root.join("il2cpp_data").is_dir() {
            return GameEngineKind::UnityIl2cpp;
        }
        if root.join("MonoBleedingEdge").is_dir()
            || root.join("Mono").is_dir()
            || has_unity_managed_dir(root)
        {
            return GameEngineKind::UnityMono;
        }
        return GameEngineKind::UnityMono;
    }

    if is_unreal_dir(root) {
        return GameEngineKind::UnrealEngine;
    }

    if has_file_matching(root, |name| {
        let low = name.to_ascii_lowercase();
        low.starts_with("re_chunk_") && low.ends_with(".pak")
    }) {
        return GameEngineKind::ReEngine;
    }

    if has_file_matching(root, |name| name.to_ascii_lowercase().ends_with(".pck")) {
        return GameEngineKind::Godot;
    }

    GameEngineKind::NativeUnknown
}

fn has_unity_managed_dir(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        path.is_dir()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_Data"))
            && path.join("Managed").is_dir()
    })
}

fn is_unreal_dir(root: &Path) -> bool {
    if root.join("Engine").join("Binaries").is_dir() {
        return true;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let p = entry.path();
        p.is_dir()
            && (p.join("Binaries").join("Win64").is_dir()
                || p.join("Content").join("Paks").is_dir())
    })
}

fn detect_loaders(root: &Path) -> Vec<DetectedLoader> {
    let mut loaders = Vec::new();

    if root.join("BepInEx").join("core").is_dir() {
        let core_dir = root.join("BepInEx").join("core");
        let is_il2cpp = core_dir.join("BepInEx.IL2CPP.dll").is_file()
            || core_dir.join("BepInEx.Unity.IL2CPP.dll").is_file()
            || root.join("dotnet").is_dir();
        loaders.push(DetectedLoader {
            kind: if is_il2cpp {
                InstalledLoaderKind::BepInExIl2cpp
            } else {
                InstalledLoaderKind::BepInExMono
            },
            entry_path: "BepInEx/core".to_owned(),
            detected_version: None,
        });
    }

    if root.join("MelonLoader").is_dir() {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::MelonLoader,
            entry_path: "MelonLoader".to_owned(),
            detected_version: None,
        });
    }

    if let Some(nekod_rel) =
        find_plugin_rel(&root.join("BepInEx").join("plugins"), "nekodunitybridge")
    {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::NekodUnityBridge,
            entry_path: nekod_rel,
            detected_version: Some("0.1.0".to_owned()),
        });
    }

    if let Some(mcp_rel) = find_plugin_rel(&root.join("BepInEx").join("plugins"), "mcpbridge") {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::UnityExplorerMcpBridge,
            entry_path: mcp_rel,
            detected_version: Some("1.3.1".to_owned()),
        });
    }

    if root.join("ue4ss").join("UE4SS.dll").is_file()
        || root.join("UE4SS.dll").is_file()
        || find_nested_ue4ss(root)
    {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::Ue4ss,
            entry_path: "ue4ss/UE4SS.dll".to_owned(),
            detected_version: None,
        });
    }

    if root.join("reframework").is_dir() && root.join("dinput8.dll").is_file() {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::ReFramework,
            entry_path: "dinput8.dll".to_owned(),
            detected_version: None,
        });
    }

    if root.join("ScriptHookV.dll").is_file() {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::ScriptHookV,
            entry_path: "ScriptHookV.dll".to_owned(),
            detected_version: None,
        });
    }

    if root.join("ScriptHookRDR2.dll").is_file() {
        loaders.push(DetectedLoader {
            kind: InstalledLoaderKind::ScriptHookRdr2,
            entry_path: "ScriptHookRDR2.dll".to_owned(),
            detected_version: None,
        });
    }

    loaders
}

fn find_plugin_rel(plugins_dir: &Path, needle: &str) -> Option<String> {
    let entries = fs::read_dir(plugins_dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file()
            && let Some(name) = p.file_name().and_then(|n| n.to_str())
        {
            let lower = name.to_ascii_lowercase();
            if lower.contains(needle) && lower.ends_with(".dll") {
                return Some(format!("BepInEx/plugins/{name}"));
            }
        } else if p.is_dir()
            && let Some(dir_name) = p.file_name().and_then(|n| n.to_str())
            && let Ok(sub) = fs::read_dir(&p)
        {
            for sub_entry in sub.flatten() {
                let sp = sub_entry.path();
                if sp.is_file()
                    && let Some(name) = sp.file_name().and_then(|n| n.to_str())
                {
                    let lower = name.to_ascii_lowercase();
                    if lower.contains(needle) && lower.ends_with(".dll") {
                        return Some(format!("BepInEx/plugins/{dir_name}/{name}"));
                    }
                }
            }
        }
    }
    None
}

fn find_nested_ue4ss(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p.join("Binaries").join("Win64").join("ue4ss").is_dir() {
            return true;
        }
    }
    false
}

fn detect_anti_cheat_markers(root: &Path) -> Vec<AntiCheatKind> {
    let mut found = Vec::new();
    if root.join("EasyAntiCheat").is_dir() || root.join("EasyAntiCheat_EOS").is_dir() {
        found.push(AntiCheatKind::EasyAntiCheat);
    }
    if root.join("BattlEye").is_dir() {
        found.push(AntiCheatKind::Battleye);
    }
    if has_file_matching(root, |name| {
        name.to_ascii_lowercase().contains("vgkbootstatus")
    }) {
        found.push(AntiCheatKind::Vanguard);
    }
    found
}

fn recommend_adapters(engine: GameEngineKind, anti_cheat: &[AntiCheatKind]) -> Vec<String> {
    if !anti_cheat.is_empty() {
        return vec!["offline_check_required".to_owned()];
    }
    match engine {
        GameEngineKind::UnityMono | GameEngineKind::UnityIl2cpp => vec![
            "unity_bridge_v1".to_owned(),
            "declarative_patch_v1".to_owned(),
        ],
        GameEngineKind::UnrealEngine => vec![
            "ue4ss_bridge_v1".to_owned(),
            "declarative_patch_v1".to_owned(),
        ],
        GameEngineKind::ReEngine => vec![
            "managed_framework_v1".to_owned(),
            "declarative_patch_v1".to_owned(),
        ],
        GameEngineKind::Godot | GameEngineKind::NativeUnknown => {
            vec![
                "declarative_patch_v1".to_owned(),
                "ce_subset_v1".to_owned(),
                "frida_gum_v1".to_owned(),
            ]
        }
    }
}

fn has_dir_matching(root: &Path, pred: impl Fn(&str) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(&pred)
    })
}

fn has_file_matching(root: &Path, pred: impl Fn(&str) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(&pred)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("nekod_engine_{name}_{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn distinguishes_unity_mono_vs_il2cpp_and_detects_bepinex_and_eac() {
        let mono_root = temp_dir("mono");
        fs::write(mono_root.join("UnityPlayer.dll"), b"MZ").unwrap();
        fs::create_dir_all(mono_root.join("Sample_Data").join("Managed")).unwrap();
        fs::create_dir_all(mono_root.join("BepInEx").join("core")).unwrap();
        fs::write(
            mono_root.join("BepInEx").join("core").join("BepInEx.dll"),
            b"MZ",
        )
        .unwrap();

        let mono_profile = inspect_game_directory(&mono_root);
        assert_eq!(mono_profile.engine, GameEngineKind::UnityMono);
        assert_eq!(mono_profile.installed_loaders.len(), 1);
        assert_eq!(
            mono_profile.installed_loaders[0].kind,
            InstalledLoaderKind::BepInExMono
        );
        assert!(
            mono_profile
                .recommended_adapters
                .contains(&"unity_bridge_v1".to_owned())
        );

        let il2cpp_root = temp_dir("il2cpp");
        fs::write(il2cpp_root.join("UnityPlayer.dll"), b"MZ").unwrap();
        fs::write(il2cpp_root.join("GameAssembly.dll"), b"MZ").unwrap();
        fs::create_dir_all(il2cpp_root.join("EasyAntiCheat")).unwrap();
        fs::create_dir_all(il2cpp_root.join("BepInEx").join("core")).unwrap();
        fs::create_dir_all(il2cpp_root.join("BepInEx").join("plugins")).unwrap();
        fs::write(
            il2cpp_root
                .join("BepInEx")
                .join("core")
                .join("BepInEx.Unity.IL2CPP.dll"),
            b"MZ",
        )
        .unwrap();
        fs::write(
            il2cpp_root
                .join("BepInEx")
                .join("plugins")
                .join("MCPBridge.dll"),
            b"MZ",
        )
        .unwrap();

        let il2cpp_profile = inspect_game_directory(&il2cpp_root);
        assert_eq!(il2cpp_profile.engine, GameEngineKind::UnityIl2cpp);
        assert_eq!(
            il2cpp_profile.installed_loaders[0].kind,
            InstalledLoaderKind::BepInExIl2cpp
        );
        assert!(
            il2cpp_profile
                .installed_loaders
                .iter()
                .any(|l| l.kind == InstalledLoaderKind::UnityExplorerMcpBridge)
        );
        assert_eq!(
            il2cpp_profile.anti_cheat_files_present,
            vec![AntiCheatKind::EasyAntiCheat]
        );
        assert_eq!(
            il2cpp_profile.recommended_adapters,
            vec!["offline_check_required".to_owned()]
        );

        let _ = fs::remove_dir_all(mono_root);
        let _ = fs::remove_dir_all(il2cpp_root);
    }

    #[test]
    fn detects_unreal_ue4ss_and_re_engine_reframework() {
        let ue_root = temp_dir("ue5");
        fs::create_dir_all(
            ue_root
                .join("MyGame")
                .join("Binaries")
                .join("Win64")
                .join("ue4ss"),
        )
        .unwrap();
        let ue_profile = inspect_game_directory(&ue_root);
        assert_eq!(ue_profile.engine, GameEngineKind::UnrealEngine);
        assert_eq!(
            ue_profile.installed_loaders[0].kind,
            InstalledLoaderKind::Ue4ss
        );

        let re_root = temp_dir("re");
        fs::write(re_root.join("re_chunk_000.pak"), b"KPKA").unwrap();
        fs::write(re_root.join("dinput8.dll"), b"MZ").unwrap();
        fs::create_dir_all(re_root.join("reframework")).unwrap();
        let re_profile = inspect_game_directory(&re_root);
        assert_eq!(re_profile.engine, GameEngineKind::ReEngine);
        assert_eq!(
            re_profile.installed_loaders[0].kind,
            InstalledLoaderKind::ReFramework
        );
        assert!(
            re_profile
                .recommended_adapters
                .contains(&"managed_framework_v1".to_owned())
        );

        let sh_root = temp_dir("scripthook");
        fs::write(sh_root.join("ScriptHookV.dll"), b"MZ").unwrap();
        let sh_profile = inspect_game_directory(&sh_root);
        assert_eq!(
            sh_profile.installed_loaders[0].kind,
            InstalledLoaderKind::ScriptHookV
        );

        let _ = fs::remove_dir_all(ue_root);
        let _ = fs::remove_dir_all(re_root);
        let _ = fs::remove_dir_all(sh_root);
    }
}
