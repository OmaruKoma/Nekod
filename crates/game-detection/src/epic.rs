use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::validate_relative_path;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::steam::current_target_platform;
use crate::{DetectedGame, DetectionError, GameDetector, GameStoreSource};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EpicItemManifest {
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    catalog_item_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    install_location: Option<String>,
    #[serde(default)]
    launch_executable: Option<String>,
    #[serde(default)]
    app_version_string: Option<String>,
    #[serde(default)]
    b_is_incomplete_install: Option<bool>,
}

/// Discovers installed Epic Games Store titles from `.item` manifest directories.
#[derive(Debug, Clone)]
pub struct EpicDetector {
    manifests_dirs: Vec<PathBuf>,
}

impl EpicDetector {
    /// Creates an `EpicDetector` scanning explicit manifest directories (ideal for tests).
    pub fn from_manifests_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            manifests_dirs: vec![dir.into()],
        }
    }

    /// Probes default OS Epic Games Launcher manifest directories.
    pub fn from_system() -> Self {
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut dirs = Vec::new();
        #[cfg(target_os = "windows")]
        {
            if let Some(program_data) = std::env::var_os("ProgramData") {
                let path = PathBuf::from(program_data)
                    .join("Epic")
                    .join("EpicGamesLauncher")
                    .join("Data")
                    .join("Manifests");
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
        Self {
            manifests_dirs: dirs,
        }
    }
}

impl GameDetector for EpicDetector {
    fn source(&self) -> GameStoreSource {
        GameStoreSource::Epic
    }

    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError> {
        let platform = current_target_platform();
        let mut games = Vec::new();

        for dir in &self.manifests_dirs {
            if !dir.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("item") {
                    continue;
                }
                let Ok(raw) = fs::read_to_string(&path) else {
                    continue;
                };
                let parsed: EpicItemManifest = match serde_json::from_str(&raw) {
                    Ok(item) => item,
                    Err(err) => {
                        warn!(path = %path.display(), "Skipping malformed Epic .item manifest: {err}");
                        continue;
                    }
                };
                if parsed.b_is_incomplete_install == Some(true) {
                    continue;
                }
                let (Some(title), Some(install_dir)) =
                    (parsed.display_name, parsed.install_location)
                else {
                    continue;
                };

                let install_path = Path::new(&install_dir);
                if !install_path.is_dir() {
                    debug!(title, install_dir, "Epic install directory missing on disk");
                    continue;
                }

                let executable_path = parsed.launch_executable.and_then(|rel| {
                    let normalized = rel.replace('\\', "/");
                    if validate_relative_path(&normalized).is_err() {
                        warn!(title, rel, "Ignoring unsafe Epic LaunchExecutable path");
                        return None;
                    }
                    let full = install_path.join(&normalized);
                    full.is_file().then(|| full.to_string_lossy().into_owned())
                });

                let store_id = parsed.catalog_item_id.or(parsed.app_name);
                games.push(DetectedGame {
                    matched_game_id: None,
                    title,
                    store_source: GameStoreSource::Epic,
                    store_app_id: store_id,
                    store_build_id: parsed.app_version_string,
                    install_dir,
                    executable_path,
                    platform,
                });
            }
        }

        Ok(games)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
    }

    #[test]
    fn discovers_epic_item_and_rejects_traversal_executable() {
        let root = temp_dir("nekod_epic");
        let manifests_dir = root.join("Manifests");
        let game_dir = root.join("Games").join("SampleEgsGame");
        fs::create_dir_all(&manifests_dir).unwrap();
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Game.exe"), b"MZ").unwrap();

        let item_ok = serde_json::json!({
            "AppName": "SampleEgsApp",
            "CatalogItemId": "epic-cat-123",
            "DisplayName": "Sample Epic RPG",
            "InstallLocation": game_dir.to_string_lossy(),
            "LaunchExecutable": "Game.exe",
            "AppVersionString": "1.4.2",
            "bIsIncompleteInstall": false
        });
        fs::write(manifests_dir.join("ok.item"), item_ok.to_string()).unwrap();

        let bad_dir = root.join("Games").join("TraversalGame");
        fs::create_dir_all(&bad_dir).unwrap();
        let item_traversal = serde_json::json!({
            "CatalogItemId": "epic-bad",
            "DisplayName": "Bad Path Game",
            "InstallLocation": bad_dir.to_string_lossy(),
            "LaunchExecutable": "../outside.exe"
        });
        fs::write(manifests_dir.join("bad.item"), item_traversal.to_string()).unwrap();

        let found = EpicDetector::from_manifests_dir(&manifests_dir)
            .discover_installed()
            .unwrap();
        assert_eq!(found.len(), 2);

        let ok = found.iter().find(|g| g.title == "Sample Epic RPG").unwrap();
        assert_eq!(ok.store_source, GameStoreSource::Epic);
        assert_eq!(ok.store_app_id.as_deref(), Some("epic-cat-123"));
        assert_eq!(ok.store_build_id.as_deref(), Some("1.4.2"));
        assert!(ok.executable_path.is_some());

        let bad = found.iter().find(|g| g.title == "Bad Path Game").unwrap();
        assert!(bad.executable_path.is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
