use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::{GameManifest, TargetPlatform};
use tracing::{debug, warn};

use crate::vdf::parse_vdf;
use crate::{DetectedGame, DetectionError, GameDetector, GameStoreSource};

/// Discovers installed Steam games by parsing `libraryfolders.vdf` and `appmanifest_*.acf`.
#[derive(Debug, Clone)]
pub struct SteamDetector {
    steam_roots: Vec<PathBuf>,
}

impl SteamDetector {
    /// Creates a Steam detector targeting explicit root path(s) (useful for tests or custom roots).
    pub fn from_root(steam_root: impl Into<PathBuf>) -> Self {
        Self {
            steam_roots: vec![steam_root.into()],
        }
    }

    /// Returns the discovered Steam root directories.
    pub fn steam_roots(&self) -> &[PathBuf] {
        &self.steam_roots
    }

    /// Creates a Steam detector probing standard OS installation locations.
    pub fn from_system() -> Self {
        let mut roots = Vec::new();

        #[cfg(target_os = "windows")]
        {
            use winreg::RegKey;
            use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

            if let Ok(hkcu) =
                RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam")
                && let Ok(path) = hkcu.get_value::<String, _>("SteamPath")
            {
                roots.push(PathBuf::from(path));
            }
            if let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE)
                .open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
                && let Ok(path) = hklm.get_value::<String, _>("InstallPath")
            {
                roots.push(PathBuf::from(path));
            }
            for fallback in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
                let pb = PathBuf::from(fallback);
                if pb.exists() {
                    roots.push(pb);
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                let home_pb = PathBuf::from(home);
                for rel in [
                    ".local/share/Steam",
                    ".steam/steam",
                    ".var/app/com.valvesoftware.Steam/.local/share/Steam",
                ] {
                    let pb = home_pb.join(rel);
                    if pb.exists() {
                        roots.push(pb);
                    }
                }
            }
        }

        // Deduplicate preserving discovery order.
        let mut seen = BTreeSet::new();
        roots.retain(|p| seen.insert(p.clone()));

        Self { steam_roots: roots }
    }

    fn collect_library_folders(steam_root: &Path) -> Vec<PathBuf> {
        let mut libraries = BTreeSet::new();
        if steam_root.exists() {
            libraries.insert(steam_root.to_path_buf());
        }

        let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
        let Ok(content) = fs::read_to_string(&vdf_path) else {
            return libraries.into_iter().collect();
        };

        match parse_vdf(&content) {
            Ok(root) => {
                let section = root
                    .get_ci("libraryfolders")
                    .or_else(|| root.get_ci("LibraryFolders"));
                if let Some(obj) = section.and_then(|v| v.as_object()) {
                    for (key, val) in obj {
                        if !key.chars().all(|c| c.is_ascii_digit()) {
                            continue;
                        }
                        if let Some(path_str) = val.get_str_ci("path").or_else(|| val.as_str()) {
                            let pb = PathBuf::from(path_str);
                            if pb.exists() {
                                libraries.insert(pb);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                warn!(path = %vdf_path.display(), error = %err, "failed to parse libraryfolders.vdf");
            }
        }

        libraries.into_iter().collect()
    }

    fn parse_acf_file(library_root: &Path, acf_path: &Path) -> Option<DetectedGame> {
        let content = fs::read_to_string(acf_path).ok()?;
        let parsed = match parse_vdf(&content) {
            Ok(v) => v,
            Err(err) => {
                warn!(path = %acf_path.display(), error = %err, "skipping malformed appmanifest");
                return None;
            }
        };

        let app_state = parsed.get_ci("AppState")?;
        let app_id = app_state.get_str_ci("appid")?.trim().to_owned();
        let name = app_state.get_str_ci("name")?.trim().to_owned();
        let install_rel = app_state.get_str_ci("installdir")?.trim();
        let build_id = app_state.get_str_ci("buildid").map(|s| s.trim().to_owned());

        if app_id.is_empty() || name.is_empty() || install_rel.is_empty() {
            return None;
        }
        // Reject path traversal inside malformed ACF installdir fields.
        if install_rel.contains("..") || install_rel.contains('/') || install_rel.contains('\\') {
            warn!(path = %acf_path.display(), installdir = install_rel, "rejected unsafe installdir");
            return None;
        }

        // Skip Steamworks Common Redistributables (228980) noise if desired, or keep standard apps.
        let install_dir = library_root
            .join("steamapps")
            .join("common")
            .join(install_rel);
        if !install_dir.exists() {
            debug!(install_dir = %install_dir.display(), "skipping uninstalled/missing steam common folder");
            return None;
        }

        Some(DetectedGame {
            matched_game_id: None,
            title: name,
            store_source: GameStoreSource::Steam,
            store_app_id: Some(app_id),
            store_build_id: build_id,
            install_dir: install_dir.to_string_lossy().into_owned(),
            executable_path: None,
            platform: current_target_platform(),
        })
    }
}

impl GameDetector for SteamDetector {
    fn source(&self) -> GameStoreSource {
        GameStoreSource::Steam
    }

    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError> {
        let mut games = Vec::new();
        let mut seen_app_ids = BTreeSet::new();

        for root in &self.steam_roots {
            for lib in Self::collect_library_folders(root) {
                let steamapps = lib.join("steamapps");
                let Ok(entries) = fs::read_dir(&steamapps) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if file_name.starts_with("appmanifest_") && file_name.ends_with(".acf") {
                        if let Some(game) = Self::parse_acf_file(&lib, &path) {
                            let dedup_key = game
                                .store_app_id
                                .clone()
                                .unwrap_or_else(|| game.install_dir.clone());
                            if seen_app_ids.insert(dedup_key) {
                                games.push(game);
                            }
                        }
                    }
                }
            }
        }

        games.sort_by_key(|a| a.title.to_lowercase());
        Ok(games)
    }
}

/// Enriches discovered games with canonical `GameManifest` matches and resolved executable paths.
pub fn match_games_against_manifests(games: &mut [DetectedGame], manifests: &[GameManifest]) {
    for game in games.iter_mut() {
        let matched = manifests.iter().find(|manifest| {
            let Some(found_id) = game.store_app_id.as_deref() else {
                return false;
            };
            match game.store_source {
                GameStoreSource::Steam => manifest
                    .store_ids
                    .steam_app_id
                    .is_some_and(|id| id.to_string() == found_id),
                GameStoreSource::Epic => manifest
                    .store_ids
                    .epic_catalog_item_id
                    .as_deref()
                    .is_some_and(|id| id == found_id),
                GameStoreSource::Gog => manifest
                    .store_ids
                    .gog_product_id
                    .as_deref()
                    .is_some_and(|id| id == found_id),
                GameStoreSource::Xbox => manifest
                    .store_ids
                    .xbox_product_id
                    .as_deref()
                    .is_some_and(|id| id == found_id),
                GameStoreSource::CustomPath => false,
            }
        });

        let Some(manifest) = matched else {
            continue;
        };
        game.matched_game_id = Some(manifest.id.clone());

        let install_root = Path::new(&game.install_dir);
        for exe in &manifest.executables {
            if exe.platform == game.platform {
                let candidate = install_root.join(&exe.relative_path);
                if candidate.is_file() {
                    game.executable_path = Some(candidate.to_string_lossy().into_owned());
                    break;
                }
            }
        }
    }
}

pub const fn current_target_platform() -> TargetPlatform {
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        TargetPlatform::WindowsArm64
    }
    #[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
    {
        TargetPlatform::WindowsX64
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        TargetPlatform::LinuxArm64
    }
    #[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
    {
        TargetPlatform::LinuxX64
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        TargetPlatform::WindowsX64
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekod_{prefix}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_steam_games_across_libraries_and_matches_manifest() {
        let root = temp_test_dir("steam_fixture");
        let lib2 = temp_test_dir("steam_lib2");

        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::create_dir_all(lib2.join("steamapps").join("common").join("SampleRPG")).unwrap();
        fs::write(
            lib2.join("steamapps")
                .join("common")
                .join("SampleRPG")
                .join("SampleRPG.exe"),
            b"MZ",
        )
        .unwrap();

        let vdf = format!(
            r#""libraryfolders" {{ "0" {{ "path" "{}" }} "1" {{ "path" "{}" }} }}"#,
            root.to_string_lossy().replace('\\', "\\\\"),
            lib2.to_string_lossy().replace('\\', "\\\\")
        );
        fs::write(root.join("steamapps").join("libraryfolders.vdf"), vdf).unwrap();

        let acf = r#"
        "AppState"
        {
            "appid"      "999999"
            "name"       "Sample Singleplayer RPG"
            "installdir" "SampleRPG"
            "buildid"    "15000001"
        }
        "#;
        fs::write(lib2.join("steamapps").join("appmanifest_999999.acf"), acf).unwrap();

        let detector = SteamDetector::from_root(&root);
        let mut discovered = detector.discover_installed().expect("discovery succeeds");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].title, "Sample Singleplayer RPG");
        assert_eq!(discovered[0].store_app_id.as_deref(), Some("999999"));
        assert_eq!(discovered[0].store_build_id.as_deref(), Some("15000001"));

        let manifest_raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let manifest: GameManifest = serde_json::from_str(manifest_raw).unwrap();
        match_games_against_manifests(&mut discovered, &[manifest]);

        assert_eq!(
            discovered[0].matched_game_id.as_deref(),
            Some("sample-singleplayer-rpg")
        );
        assert!(discovered[0].executable_path.is_some());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(lib2);
    }

    #[test]
    fn ignores_acf_with_path_traversal_installdir() {
        let root = temp_test_dir("steam_traversal");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        let acf = r#"
        "AppState"
        {
            "appid"      "12345"
            "name"       "Evil Game"
            "installdir" "../../Windows"
        }
        "#;
        fs::write(root.join("steamapps").join("appmanifest_12345.acf"), acf).unwrap();

        let detector = SteamDetector::from_root(&root);
        let discovered = detector.discover_installed().unwrap();
        assert!(discovered.is_empty());

        let _ = fs::remove_dir_all(root);
    }
}
