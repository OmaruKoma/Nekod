use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::GameManifest;

use crate::steam::current_target_platform;
use crate::{DetectedGame, DetectionError, GameDetector, GameStoreSource};

/// User-registered custom game folder (or direct executable) entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomGameFolder {
    pub path: PathBuf,
    pub label: Option<String>,
}

/// Detector for player-registered local game folders or executables outside Steam/GOG/Epic.
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

fn is_ignored_helper_exe(lower: &str) -> bool {
    matches!(
        lower,
        "unitycrashhandler64.exe"
            | "unitycrashhandler32.exe"
            | "crashreportclient.exe"
            | "unins000.exe"
            | "ueprereqsetup_x64.exe"
            | "vc_redist.x64.exe"
            | "dxsetup.exe"
    ) || lower.contains("crashhandler")
        || lower.starts_with("unins")
}

/// Scans top-level `.exe` files in `install_dir`, skipping helper/uninstaller binaries,
/// preferring a binary whose stem matches the directory name, else choosing the largest `.exe`.
pub fn detect_primary_executable(install_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(install_dir).ok()?;
    let dir_stem = install_dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);

    let mut candidates: Vec<(bool, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".exe") || is_ignored_helper_exe(&lower) {
            continue;
        }
        let stem_match = p
            .file_stem()
            .and_then(|s| s.to_str())
            .zip(dir_stem.as_deref())
            .is_some_and(|(s, d)| s.eq_ignore_ascii_case(d));
        let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        candidates.push((stem_match, size, p));
    }

    candidates
        .into_iter()
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, _, p)| p)
}

impl GameDetector for CustomPathDetector {
    fn source(&self) -> GameStoreSource {
        GameStoreSource::CustomPath
    }

    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError> {
        let platform = current_target_platform();
        let mut results = Vec::new();

        for folder in &self.folders {
            let (install_dir, explicit_exe) = if folder.path.is_file() {
                let Some(parent) = folder.path.parent() else {
                    continue;
                };
                (parent.to_path_buf(), Some(folder.path.clone()))
            } else if folder.path.is_dir() {
                (folder.path.clone(), None)
            } else {
                continue;
            };

            let mut matched_id = None;
            let mut matched_title = None;
            let mut exe_path = None;

            for manifest in &self.known_manifests {
                if let Some(candidate) =
                    crate::steam::find_matching_executable(&install_dir, manifest, platform)
                {
                    matched_id = Some(manifest.id.clone());
                    matched_title = Some(manifest.name.clone());
                    exe_path = Some(candidate.to_string_lossy().into_owned());
                    break;
                }
            }

            if exe_path.is_none() {
                exe_path = explicit_exe
                    .as_ref()
                    .cloned()
                    .or_else(|| detect_primary_executable(&install_dir))
                    .map(|p| p.to_string_lossy().into_owned());
            }

            let fallback_title = folder
                .label
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    explicit_exe
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .or_else(|| {
                    install_dir
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
                install_dir: install_dir.to_string_lossy().into_owned(),
                executable_path: exe_path,
                platform,
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn custom_directory_skips_crash_handler_and_selects_primary_exe() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_custom_dir_{nanos}"));
        let game_dir = root.join("CoolIndieGame");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("UnityCrashHandler64.exe"), vec![b'M'; 4096]).unwrap();
        fs::write(game_dir.join("CoolIndieGame.exe"), vec![b'M'; 128]).unwrap();

        let detector = CustomPathDetector::new(
            vec![CustomGameFolder {
                path: game_dir.clone(),
                label: None,
            }],
            vec![],
        );
        let found = detector.discover_installed().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "CoolIndieGame");
        assert_eq!(found[0].install_dir, game_dir.to_string_lossy());
        assert_eq!(
            found[0].executable_path.as_deref(),
            Some(
                game_dir
                    .join("CoolIndieGame.exe")
                    .to_string_lossy()
                    .as_ref()
            )
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn custom_exe_file_derives_parent_install_dir_and_stem_title() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_custom_exe_{nanos}"));
        let game_dir = root.join("bin");
        fs::create_dir_all(&game_dir).unwrap();
        let exe = game_dir.join("MyPortableGame.exe");
        fs::write(&exe, b"MZ").unwrap();

        let detector = CustomPathDetector::new(
            vec![CustomGameFolder {
                path: exe.clone(),
                label: None,
            }],
            vec![],
        );
        let found = detector.discover_installed().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "MyPortableGame");
        assert_eq!(found[0].install_dir, game_dir.to_string_lossy());
        assert_eq!(
            found[0].executable_path.as_deref(),
            Some(exe.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(&root);
    }
}
