use std::path::PathBuf;

use nekod_manifest::GameManifest;

use crate::steam::current_target_platform;
use crate::{DetectedGame, DetectionError, GameDetector, GameStoreSource};

/// User-registered custom game folder entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomGameFolder {
    pub path: PathBuf,
    pub label: Option<String>,
}

/// Detector for player-registered local game folders outside Steam.
#[derive(Debug, Clone, Default)]
pub struct CustomPathDetector {
    folders: Vec<CustomGameFolder>,
    known_manifests: Vec<GameManifest>,
}

impl CustomPathDetector {
    pub fn new(folders: Vec<CustomGameFolder>, known_manifests: Vec<GameManifest>) -> Self {
        Self {
            folders,
            known_manifests,
        }
    }
}

impl GameDetector for CustomPathDetector {
    fn source(&self) -> GameStoreSource {
        GameStoreSource::CustomPath
    }

    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError> {
        let platform = current_target_platform();
        let mut results = Vec::new();

        for folder in &self.folders {
            if !folder.path.is_dir() {
                continue;
            }

            let mut matched_id = None;
            let mut matched_title = None;
            let mut exe_path = None;

            for manifest in &self.known_manifests {
                for exe in &manifest.executables {
                    if exe.platform == platform {
                        let candidate = folder.path.join(&exe.relative_path);
                        if candidate.is_file() {
                            matched_id = Some(manifest.id.clone());
                            matched_title = Some(manifest.name.clone());
                            exe_path = Some(candidate.to_string_lossy().into_owned());
                            break;
                        }
                    }
                }
                if matched_id.is_some() {
                    break;
                }
            }

            let fallback_title = folder
                .label
                .clone()
                .or_else(|| {
                    folder
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "Custom Game".to_owned());

            results.push(DetectedGame {
                matched_game_id: matched_id,
                title: matched_title.unwrap_or(fallback_title),
                store_source: GameStoreSource::CustomPath,
                store_app_id: None,
                store_build_id: None,
                install_dir: folder.path.to_string_lossy().into_owned(),
                executable_path: exe_path,
                platform,
            });
        }

        Ok(results)
    }
}
