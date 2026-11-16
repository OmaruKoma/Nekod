//! License-aware Runtime Dependency Manager (`ADR-019` / `ADR-020`).
//!
//! Resolves `ModManifest.dependencies` against inspected game directories and the
//! local SHA-256 verified dependency cache without bundling license-restricted loaders.

use std::fs;
use std::path::{Path, PathBuf};

use nekod_game_detection::{GameEnvironmentProfile, InstalledLoaderKind};
use nekod_manifest::{
    DependencyKind, DependencyRedistribution, RuntimeDependencySpec, is_valid_sha256_hex,
};
use serde::{Deserialize, Serialize};

use crate::{TrainerManagerError, sha256_hex};

/// Resolution outcome for a single declared mod dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DependencyResolutionState {
    SatisfiedByGameLoader {
        loader_path: String,
    },
    SatisfiedByLocalCache {
        cached_file: String,
        sha256: String,
    },
    SatisfiedByRecipe,
    RequiresUserUpstreamFetch {
        url: String,
        sha256: String,
        license: String,
    },
    MissingLocalInstall {
        instructions: String,
    },
}

/// Complete report for one declared dependency in a `ModManifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyResolutionReport {
    pub dependency_id: String,
    pub kind: DependencyKind,
    pub version_req: String,
    pub optional: bool,
    pub state: DependencyResolutionState,
}

/// Manages local dependency inspection and SHA-256-verified staging cache.
pub struct RuntimeDependencyManager {
    cache_dir: PathBuf,
}

impl RuntimeDependencyManager {
    pub fn new(cache_dir: impl AsRef<Path>) -> Result<Self, TrainerManagerError> {
        let dir = cache_dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
        Ok(Self { cache_dir: dir })
    }

    /// Evaluates declared dependencies against `env_profile` and the local cache.
    pub fn evaluate_dependencies(
        &self,
        dependencies: &[RuntimeDependencySpec],
        env_profile: Option<&GameEnvironmentProfile>,
    ) -> Vec<DependencyResolutionReport> {
        dependencies
            .iter()
            .map(|dep| self.evaluate_one(dep, env_profile))
            .collect()
    }

    /// Verifies downloaded upstream dependency bytes against the pinned SHA-256 digest
    /// before saving them into Nekod's managed cache.
    pub fn stage_verified_upstream_bytes(
        &self,
        dep: &RuntimeDependencySpec,
        bytes: &[u8],
    ) -> Result<PathBuf, TrainerManagerError> {
        let upstream = dep.upstream.as_ref().ok_or_else(|| {
            TrainerManagerError::InvalidManifest(format!(
                "dependency '{}' has no pinned upstream source",
                dep.dependency_id
            ))
        })?;
        let expected = upstream.sha256.to_ascii_lowercase();
        if !is_valid_sha256_hex(&expected) {
            return Err(TrainerManagerError::InvalidManifest(format!(
                "invalid upstream sha256 on '{}'",
                dep.dependency_id
            )));
        }
        let actual = sha256_hex(bytes);
        if actual != expected {
            return Err(TrainerManagerError::IntegrityMismatch(
                dep.dependency_id.clone(),
                format!("expected sha256 {expected}, got {actual}"),
            ));
        }

        let dep_dir = self.cache_dir.join(&dep.dependency_id);
        fs::create_dir_all(&dep_dir).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
        let target = dep_dir.join(format!("{expected}.pkg"));
        fs::write(&target, bytes).map_err(|e| TrainerManagerError::Io(e.to_string()))?;
        Ok(target)
    }

    fn evaluate_one(
        &self,
        dep: &RuntimeDependencySpec,
        env_profile: Option<&GameEnvironmentProfile>,
    ) -> DependencyResolutionReport {
        if let Some(profile) = env_profile
            && let Some(loader) = profile
                .installed_loaders
                .iter()
                .find(|l| matches_loader_id(&dep.dependency_id, l.kind))
        {
            return DependencyResolutionReport {
                dependency_id: dep.dependency_id.clone(),
                kind: dep.kind,
                version_req: dep.version_req.clone(),
                optional: dep.optional,
                state: DependencyResolutionState::SatisfiedByGameLoader {
                    loader_path: loader.entry_path.clone(),
                },
            };
        }

        if dep.redistribution == DependencyRedistribution::BundledRecipeOnly {
            return DependencyResolutionReport {
                dependency_id: dep.dependency_id.clone(),
                kind: dep.kind,
                version_req: dep.version_req.clone(),
                optional: dep.optional,
                state: DependencyResolutionState::SatisfiedByRecipe,
            };
        }

        if let Some(upstream) = &dep.upstream {
            let expected = upstream.sha256.to_ascii_lowercase();
            let cached_file = self
                .cache_dir
                .join(&dep.dependency_id)
                .join(format!("{expected}.pkg"));
            if cached_file.is_file()
                && let Ok(bytes) = fs::read(&cached_file)
                && sha256_hex(&bytes) == expected
            {
                return DependencyResolutionReport {
                    dependency_id: dep.dependency_id.clone(),
                    kind: dep.kind,
                    version_req: dep.version_req.clone(),
                    optional: dep.optional,
                    state: DependencyResolutionState::SatisfiedByLocalCache {
                        cached_file: cached_file.display().to_string(),
                        sha256: expected,
                    },
                };
            }

            if dep.redistribution == DependencyRedistribution::OfficialUpstreamFetch {
                return DependencyResolutionReport {
                    dependency_id: dep.dependency_id.clone(),
                    kind: dep.kind,
                    version_req: dep.version_req.clone(),
                    optional: dep.optional,
                    state: DependencyResolutionState::RequiresUserUpstreamFetch {
                        url: upstream.url.clone(),
                        sha256: expected,
                        license: upstream.license.clone(),
                    },
                };
            }
        }

        DependencyResolutionReport {
            dependency_id: dep.dependency_id.clone(),
            kind: dep.kind,
            version_req: dep.version_req.clone(),
            optional: dep.optional,
            state: DependencyResolutionState::MissingLocalInstall {
                instructions: format!(
                    "Install '{}' ({}) locally or verify external tool configuration.",
                    dep.dependency_id, dep.version_req
                ),
            },
        }
    }
}

fn matches_loader_id(dependency_id: &str, kind: InstalledLoaderKind) -> bool {
    let low = dependency_id.to_ascii_lowercase();
    match kind {
        InstalledLoaderKind::BepInExMono => low == "bepinex-mono" || low == "bepinex",
        InstalledLoaderKind::BepInExIl2cpp => low == "bepinex-il2cpp" || low == "bepinex",
        InstalledLoaderKind::MelonLoader => low.contains("melonloader"),
        InstalledLoaderKind::Ue4ss => low.contains("ue4ss"),
        InstalledLoaderKind::ReFramework => low.contains("reframework"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    use nekod_game_detection::{DetectedLoader, GameEngineKind};
    use nekod_manifest::PinnedUpstreamSource;

    fn temp_cache(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nekod_dep_{name}_{nanos}"))
    }

    #[test]
    fn stages_upstream_dependency_with_sha256_and_detects_installed_loader() {
        let cache = temp_cache("stage");
        let mgr = RuntimeDependencyManager::new(&cache).unwrap();

        let archive = b"BEPINEX_5_4_22_ZIP_BYTES";
        let sha = sha256_hex(archive);
        let dep = RuntimeDependencySpec {
            dependency_id: "bepinex-mono".to_owned(),
            kind: DependencyKind::ModLoader,
            version_req: "^5.4.0".to_owned(),
            optional: false,
            redistribution: DependencyRedistribution::OfficialUpstreamFetch,
            upstream: Some(PinnedUpstreamSource {
                url: "https://github.com/BepInEx/BepInEx/releases/download/v5.4.22/BepInEx_win_x64_5.4.22.0.zip".to_owned(),
                sha256: sha.clone(),
                license: "LGPL-2.1".to_owned(),
            }),
        };

        // 1. Before staging or detection -> prompts user consent for official upstream fetch.
        let r1 = mgr.evaluate_dependencies(std::slice::from_ref(&dep), None);
        assert!(matches!(
            r1[0].state,
            DependencyResolutionState::RequiresUserUpstreamFetch { .. }
        ));

        // 2. Tampered upstream payload fails closed.
        assert!(
            mgr.stage_verified_upstream_bytes(&dep, b"TAMPERED")
                .is_err()
        );

        // 3. Verified upstream payload is cached and satisfies dependency.
        mgr.stage_verified_upstream_bytes(&dep, archive).unwrap();
        let r2 = mgr.evaluate_dependencies(std::slice::from_ref(&dep), None);
        assert!(matches!(
            r2[0].state,
            DependencyResolutionState::SatisfiedByLocalCache { .. }
        ));

        // 4. Detected loader inside game folder satisfies dependency directly.
        let profile = GameEnvironmentProfile {
            install_dir: "C:\\Games\\Sample".to_owned(),
            dir_exists: true,
            engine: GameEngineKind::UnityMono,
            installed_loaders: vec![DetectedLoader {
                kind: InstalledLoaderKind::BepInExMono,
                entry_path: "BepInEx/core".to_owned(),
                detected_version: None,
            }],
            anti_cheat_files_present: vec![],
            recommended_adapters: vec![],
        };
        let r3 = mgr.evaluate_dependencies(&[dep], Some(&profile));
        assert!(matches!(
            r3[0].state,
            DependencyResolutionState::SatisfiedByGameLoader { .. }
        ));

        let _ = fs::remove_dir_all(cache);
    }
}
