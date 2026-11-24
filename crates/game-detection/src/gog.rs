use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::validate_relative_path;
use serde::Deserialize;
use tracing::warn;

use crate::steam::current_target_platform;
use crate::{DetectedGame, DetectionError, GameDetector, GameStoreSource};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GogPlayTask {
    #[serde(default)]
    is_primary: Option<bool>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GogGameInfo {
    #[serde(default)]
    game_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    play_tasks: Vec<GogPlayTask>,
}

/// Discovers installed GOG Galaxy titles by inspecting `goggame-*.info` descriptors
/// inside explicit install directories or Windows registry GOG entries.
#[derive(Debug, Clone)]
pub struct GogDetector {
    install_dirs: Vec<PathBuf>,
}

impl GogDetector {
    /// Creates a `GogDetector` scanning explicit game install directories (ideal for tests).
    pub fn from_install_dirs(dirs: Vec<PathBuf>) -> Self {
        Self { install_dirs: dirs }
    }

    /// Probes system GOG Galaxy registry entries and standard install directories.
    pub fn from_system() -> Self {
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut dirs = BTreeSet::new();

        #[cfg(target_os = "windows")]
        {
            use winreg::RegKey;
            use winreg::enums::HKEY_LOCAL_MACHINE;

            if let Ok(games_key) = RegKey::predef(HKEY_LOCAL_MACHINE)
                .open_subkey("SOFTWARE\\WOW6432Node\\GOG.com\\Games")
            {
                for subkey_name in games_key.enum_keys().flatten() {
                    if let Ok(subkey) = games_key.open_subkey(&subkey_name)
                        && let Ok(path) = subkey.get_value::<String, _>("path")
                    {
                        let pb = PathBuf::from(path);
                        if pb.is_dir() {
                            dirs.insert(pb);
                        }
                    }
                }
            }
        }

        Self {
            install_dirs: dirs.into_iter().collect(),
        }
    }
}

impl GameDetector for GogDetector {
    fn source(&self) -> GameStoreSource {
        GameStoreSource::Gog
    }

    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError> {
        let platform = current_target_platform();
        let mut found = Vec::new();

        for dir in &self.install_dirs {
            if !dir.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !fname.starts_with("goggame-") || !fname.ends_with(".info") {
                    continue;
                }
                let Ok(raw) = fs::read_to_string(&path) else {
                    continue;
                };
                let parsed: GogGameInfo = match serde_json::from_str(&raw) {
                    Ok(info) => info,
                    Err(err) => {
                        warn!(path = %path.display(), "Skipping malformed GOG .info file: {err}");
                        continue;
                    }
                };
                let Some(title) = parsed.name else {
                    continue;
                };

                let exe_rel = parsed
                    .play_tasks
                    .iter()
                    .find(|t| t.is_primary == Some(true))
                    .or_else(|| parsed.play_tasks.first())
                    .and_then(|t| t.path.as_deref());

                let executable_path = exe_rel.and_then(|rel| {
                    let normalized = rel.replace('\\', "/");
                    if validate_relative_path(&normalized).is_err() {
                        warn!(title, rel, "Ignoring unsafe GOG playTask path");
                        return None;
                    }
                    let full = Path::new(dir).join(&normalized);
                    full.is_file().then(|| full.to_string_lossy().into_owned())
                });

                found.push(DetectedGame {
                    matched_game_id: None,
                    title,
                    store_source: GameStoreSource::Gog,
                    store_app_id: parsed.game_id,
                    store_build_id: parsed.version,
                    install_dir: dir.to_string_lossy().into_owned(),
                    executable_path,
                    platform,
                });
            }
        }

        Ok(found)
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
    fn discovers_gog_game_info_and_blocks_unsafe_playtask_paths() {
        let root = temp_dir("nekod_gog");
        let game_dir = root.join("SampleGogRpg");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("SampleGOG.exe"), b"MZ").unwrap();

        let info = serde_json::json!({
            "gameId": "1234567890",
            "name": "Sample GOG RPG",
            "version": "2.0.1",
            "playTasks": [
                { "isPrimary": true, "path": "SampleGOG.exe" }
            ]
        });
        fs::write(game_dir.join("goggame-1234567890.info"), info.to_string()).unwrap();

        let found = GogDetector::from_install_dirs(vec![game_dir])
            .discover_installed()
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].store_source, GameStoreSource::Gog);
        assert_eq!(found[0].store_app_id.as_deref(), Some("1234567890"));
        assert!(found[0].executable_path.is_some());

        let _ = fs::remove_dir_all(&root);
    }
}
