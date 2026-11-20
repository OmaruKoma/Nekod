//! L2 Application Core modular monolith facade.
//!
//! Coordinates `game-detection`, `trainer-manager`, `permission-manager`,
//! `profile-manager`, `registry-client`, and L3 `RuntimeProvider` implementations.
//! Deliberately decoupled from `apps/desktop` (L1) and `crates/nekod-mcp` (L6).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nekod_game_detection::{
    CustomGameFolder, CustomPathDetector, DetectedGame, DetectionError, EpicDetector, GameDetector,
    GameEnvironmentProfile, GogDetector, SteamDetector, current_target_platform,
    inspect_game_directory, match_games_against_manifests,
};
use nekod_manifest::{
    CompatibilityStatus, GameManifest, ManifestError, PermissionDeclaration, PermissionScope,
    RuntimeDependencySpec, TrainerFeature, TrainerManifest, TrainerPolicy, TrustTier,
};
use nekod_permission_manager::{
    EvaluationOutcome, PermissionError, PermissionEvaluator, SecurityPolicyConfig, SqliteGrantStore,
};
use nekod_profile_manager::{CustomGamePathRecord, GameProfile, ProfileError, SqliteProfileStore};
use nekod_registry_client::{
    HttpTransport, RegistryError, RegistrySource, RegistrySourceState, RegistrySyncReport,
    ReqwestHttpTransport, SqliteRegistryCache, stage_verified_trainer_package,
};
use nekod_runtime_api::{
    FeatureCommand, ProcessIdentity, RuntimeDiagnostics, RuntimeError, RuntimeProvider,
    RuntimeSessionId, StartSessionRequest,
};
use nekod_trainer_manager::{
    CompatibilityDatabase, CompatibilityRecord, DependencyResolutionReport, InstalledTrainer,
    NewCompatibilityRecord, RuntimeDependencyManager, SqliteTrainerCatalog, TrainerCatalog,
    TrainerManagerError, compute_compatibility, load_verified_entrypoint_str,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

const SECURITY_POLICY_KEY: &str = "security_policy";

/// Top-level error returned by `NekodCore` and `NekodAppService` operations.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),

    #[error("game detection error: {0}")]
    Detection(#[from] DetectionError),

    #[error("permission error: {0}")]
    Permission(#[from] PermissionError),

    #[error("profile error: {0}")]
    Profile(#[from] ProfileError),

    #[error("trainer manager error: {0}")]
    Trainer(#[from] TrainerManagerError),

    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),

    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),

    #[error("explicit player permission confirmation required for: {0}")]
    ConsentRequired(String),

    #[error("incompatible game/trainer policy: {0}")]
    IncompatiblePolicy(String),

    #[error("storage initialization error: {0}")]
    Storage(String),
}

/// Player-facing game library entry surfaced to L1 UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryGameView {
    pub library_key: String,
    pub game: DetectedGame,
    pub trainer_count: usize,
}

/// Permission summary returned to the UI for transparent consent prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TrainerPermissionState {
    Approved {
        granted_scopes: Vec<PermissionScope>,
    },
    RequiresConfirmation {
        missing_confirmations: Vec<PermissionDeclaration>,
    },
    Blocked {
        reason: String,
    },
}

/// Player-facing trainer card and feature metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainerDetailView {
    pub trainer_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub source_repository: String,
    pub trust_tier: TrustTier,
    pub manifest_sha256: String,
    pub compatibility_status: CompatibilityStatus,
    pub compatibility_notes: Option<String>,
    pub revoked: bool,
    pub permissions: Vec<PermissionDeclaration>,
    pub permission_state: TrainerPermissionState,
    pub features: Vec<TrainerFeature>,
}

/// Structured audit/diagnostic log entry viewable in the player UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub level: String,
    pub category: String,
    pub message: String,
}

/// Installation/update status comparing a community index entry to local SQLite catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CommunityTrainerInstallState {
    NotInstalled,
    InstalledUpToDate {
        installed_version: String,
    },
    UpdateAvailable {
        installed_version: String,
        latest_version: String,
    },
    Revoked,
}

/// Player-facing community trainer discovery card aggregated from synced registries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityTrainerView {
    pub repository_id: String,
    pub trainer_id: String,
    pub game_id: String,
    pub latest_version: String,
    pub manifest_url: String,
    pub manifest_sha256: Option<String>,
    pub trust_tier: TrustTier,
    pub compatibility_status: Option<CompatibilityStatus>,
    pub last_verified_game_version: Option<String>,
    pub install_state: CommunityTrainerInstallState,
}

/// Available update summary for an already-installed trainer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerUpdateInfo {
    pub repository_id: String,
    pub trainer_id: String,
    pub game_id: String,
    pub installed_version: String,
    pub available_version: String,
    pub trust_tier: TrustTier,
}

/// Lightweight modular monolith coordinator (also used directly in unit tests).
pub struct NekodCore {
    detectors: Vec<Box<dyn GameDetector>>,
    runtime: Arc<dyn RuntimeProvider>,
    security_policy: SecurityPolicyConfig,
}

impl NekodCore {
    pub fn new(runtime: Arc<dyn RuntimeProvider>, security_policy: SecurityPolicyConfig) -> Self {
        Self {
            detectors: Vec::new(),
            runtime,
            security_policy,
        }
    }

    pub fn register_detector(&mut self, detector: Box<dyn GameDetector>) {
        self.detectors.push(detector);
    }

    pub fn discover_games(&self) -> Result<Vec<DetectedGame>, CoreError> {
        let mut games = Vec::new();
        for detector in &self.detectors {
            games.extend(detector.discover_installed()?);
        }
        Ok(games)
    }

    /// Enforces cross-manifest game policy, singleplayer scope, and anti-cheat conflict checks.
    pub fn validate_game_trainer_compatibility(
        game: &GameManifest,
        trainer: &TrainerManifest,
    ) -> Result<(), CoreError> {
        game.validate_semantic()?;
        trainer.validate_semantic()?;

        if trainer.target_game.game_id != game.id {
            return Err(CoreError::IncompatiblePolicy(format!(
                "trainer targets game '{}', expected '{}'",
                trainer.target_game.game_id, game.id
            )));
        }

        if game.compatibility_policy.trainer_policy == TrainerPolicy::ProhibitedOnlineCompetitive {
            return Err(CoreError::IncompatiblePolicy(format!(
                "game '{}' prohibits trainers due to competitive online policy",
                game.id
            )));
        }

        if game.compatibility_policy.trainer_policy == TrainerPolicy::RestrictedOfflineOnly
            && !trainer.compatibility.requires_offline_mode
        {
            return Err(CoreError::IncompatiblePolicy(format!(
                "game '{}' requires trainers to declare requires_offline_mode = true",
                game.id
            )));
        }

        for ac in &game.compatibility_policy.anti_cheat_detected {
            if trainer.compatibility.anti_cheat_incompatible.contains(ac)
                && !trainer.compatibility.requires_offline_mode
            {
                return Err(CoreError::IncompatiblePolicy(format!(
                    "trainer conflicts with active anti-cheat '{ac:?}' without offline mode"
                )));
            }
        }

        Ok(())
    }

    pub fn evaluate_trainer_permissions(
        &self,
        manifest: &TrainerManifest,
        manifest_sha256: &str,
        trust_tier: TrustTier,
        confirmed_scopes: &BTreeSet<PermissionScope>,
    ) -> Result<EvaluationOutcome, CoreError> {
        Ok(PermissionEvaluator::evaluate(
            manifest,
            manifest_sha256,
            trust_tier,
            &self.security_policy,
            confirmed_scopes,
        )?)
    }

    pub fn runtime(&self) -> &Arc<dyn RuntimeProvider> {
        &self.runtime
    }
}

/// Full P1–P3 Application Core service backed by SQLite stores and L3 `RuntimeProvider`.
pub struct NekodAppService {
    profiles: SqliteProfileStore,
    grants: SqliteGrantStore,
    trainers: SqliteTrainerCatalog,
    compatibility_db: CompatibilityDatabase,
    dependencies: RuntimeDependencyManager,
    registry: SqliteRegistryCache,
    runtime: Arc<dyn RuntimeProvider>,
    http_transport: Arc<dyn HttpTransport>,
    packages_dir: PathBuf,
    steam_override: Option<PathBuf>,
    cached_games: Mutex<Vec<DetectedGame>>,
    logs: Mutex<Vec<DiagnosticEvent>>,
}

impl NekodAppService {
    /// Opens persistent SQLite storage inside `data_dir`.
    pub fn open_in_dir(
        data_dir: impl AsRef<Path>,
        runtime: Arc<dyn RuntimeProvider>,
    ) -> Result<Self, CoreError> {
        let dir = data_dir.as_ref();
        fs::create_dir_all(dir).map_err(|e| CoreError::Storage(e.to_string()))?;
        let packages_dir = dir.join("packages");
        fs::create_dir_all(&packages_dir).map_err(|e| CoreError::Storage(e.to_string()))?;

        let profiles = SqliteProfileStore::open(dir.join("profiles.db"))?;
        let grants = SqliteGrantStore::open(dir.join("grants.db"))?;
        let trainers = SqliteTrainerCatalog::open(dir.join("trainers.db"))?;
        let compatibility_db = CompatibilityDatabase::open(dir.join("compatibility.db"))?;
        let dependencies = RuntimeDependencyManager::new(dir.join("deps_cache"))?;
        let registry = SqliteRegistryCache::open(dir.join("registry.db"))?;
        let http_transport = Arc::new(ReqwestHttpTransport::new()?);

        let service = Self {
            profiles,
            grants,
            trainers,
            compatibility_db,
            dependencies,
            registry,
            runtime,
            http_transport,
            packages_dir,
            steam_override: None,
            cached_games: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
        };
        service.record_log("info", "core", "Initialized Nekod core service");
        Ok(service)
    }

    /// Creates an isolated in-memory core service for tests (with optional Steam root fixture).
    pub fn open_in_memory(
        runtime: Arc<dyn RuntimeProvider>,
        steam_override: Option<PathBuf>,
    ) -> Result<Self, CoreError> {
        let http_transport = Arc::new(ReqwestHttpTransport::new()?);
        let packages_dir = std::env::temp_dir().join("nekod_in_memory_packages");
        Self::open_in_memory_with_transport(runtime, steam_override, http_transport, packages_dir)
    }

    /// Creates an isolated in-memory core service wired to a custom/mock HTTP transport.
    pub fn open_in_memory_with_transport(
        runtime: Arc<dyn RuntimeProvider>,
        steam_override: Option<PathBuf>,
        http_transport: Arc<dyn HttpTransport>,
        packages_dir: PathBuf,
    ) -> Result<Self, CoreError> {
        fs::create_dir_all(&packages_dir).map_err(|e| CoreError::Storage(e.to_string()))?;
        let profiles = SqliteProfileStore::open_in_memory()?;
        let grants = SqliteGrantStore::open_in_memory()?;
        let trainers = SqliteTrainerCatalog::open_in_memory()?;
        let compatibility_db = CompatibilityDatabase::open_in_memory()?;
        let dependencies = RuntimeDependencyManager::new(packages_dir.join(".deps_cache"))?;
        let registry = SqliteRegistryCache::open_in_memory()?;

        Ok(Self {
            profiles,
            grants,
            trainers,
            compatibility_db,
            dependencies,
            registry,
            runtime,
            http_transport,
            packages_dir,
            steam_override,
            cached_games: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
        })
    }

    fn record_log(&self, level: &str, category: &str, message: impl Into<String>) {
        let msg = message.into();
        match level {
            "warn" => warn!(category, "{msg}"),
            _ => info!(category, "{msg}"),
        }
        if let Ok(mut guard) = self.logs.lock() {
            guard.push(DiagnosticEvent {
                level: level.to_owned(),
                category: category.to_owned(),
                message: msg,
            });
            if guard.len() > 250 {
                let overflow = guard.len() - 250;
                guard.drain(..overflow);
            }
        }
    }

    pub fn get_security_policy(&self) -> Result<SecurityPolicyConfig, CoreError> {
        Ok(self
            .profiles
            .load_setting::<SecurityPolicyConfig>(SECURITY_POLICY_KEY)?
            .unwrap_or_default())
    }

    pub fn update_security_policy(&self, policy: SecurityPolicyConfig) -> Result<(), CoreError> {
        self.profiles.save_setting(SECURITY_POLICY_KEY, &policy)?;
        self.record_log(
            "info",
            "security",
            format!(
                "Updated security policy (allow_unverified={}, allow_third_party_native={})",
                policy.allow_unverified_sources, policy.allow_third_party_native_modules
            ),
        );
        Ok(())
    }

    pub fn register_game_manifest(&self, raw_json: &str) -> Result<GameManifest, CoreError> {
        let manifest = self.trainers.register_game_manifest_str(raw_json)?;
        self.record_log(
            "info",
            "catalog",
            format!("Registered game manifest '{}'", manifest.id),
        );
        Ok(manifest)
    }

    pub fn list_game_manifests(&self) -> Result<Vec<GameManifest>, CoreError> {
        Ok(self.trainers.list_game_manifests()?)
    }

    pub fn ingest_local_trainer(
        &self,
        package_dir: impl AsRef<Path>,
        trust_tier: TrustTier,
    ) -> Result<InstalledTrainer, CoreError> {
        let installed = self
            .trainers
            .ingest_local_package(package_dir, trust_tier)?;

        // Enforce cross-manifest game compatibility if a matching GameManifest is registered.
        let games = self.trainers.list_game_manifests()?;
        if let Some(game_manifest) = games
            .iter()
            .find(|g| g.id == installed.manifest.target_game.game_id)
        {
            NekodCore::validate_game_trainer_compatibility(game_manifest, &installed.manifest)?;
        }

        self.record_log(
            "info",
            "trainer",
            format!(
                "Validated and installed trainer '{}@{}' ({trust_tier:?})",
                installed.manifest.id, installed.manifest.version
            ),
        );
        Ok(installed)
    }

    pub fn add_custom_game_path(
        &self,
        path: &str,
        label: Option<&str>,
    ) -> Result<Vec<CustomGamePathRecord>, CoreError> {
        self.profiles.add_custom_game_path(path, label)?;
        self.record_log(
            "info",
            "detection",
            format!("Added custom game folder '{path}'"),
        );
        Ok(self.profiles.list_custom_game_paths()?)
    }

    pub fn remove_custom_game_path(
        &self,
        path: &str,
    ) -> Result<Vec<CustomGamePathRecord>, CoreError> {
        self.profiles.remove_custom_game_path(path)?;
        self.record_log(
            "info",
            "detection",
            format!("Removed custom game folder '{path}'"),
        );
        Ok(self.profiles.list_custom_game_paths()?)
    }

    pub fn list_custom_game_paths(&self) -> Result<Vec<CustomGamePathRecord>, CoreError> {
        Ok(self.profiles.list_custom_game_paths()?)
    }

    /// Scans Steam & custom folders, matches against known `GameManifest`s, and caches results.
    pub fn rescan_games(&self) -> Result<Vec<LibraryGameView>, CoreError> {
        let steam = match &self.steam_override {
            Some(root) => SteamDetector::from_root(root),
            None => SteamDetector::from_system(),
        };
        let known_manifests = self.trainers.list_game_manifests()?;
        let custom_folders: Vec<CustomGameFolder> = self
            .profiles
            .list_custom_game_paths()?
            .into_iter()
            .map(|r| CustomGameFolder {
                path: PathBuf::from(r.path),
                label: r.label,
            })
            .collect();
        let custom = CustomPathDetector::new(custom_folders, known_manifests.clone());

        let epic = EpicDetector::from_system();
        let gog = GogDetector::from_system();

        let mut discovered = steam.discover_installed()?;
        discovered.extend(epic.discover_installed()?);
        discovered.extend(gog.discover_installed()?);
        discovered.extend(custom.discover_installed()?);
        match_games_against_manifests(&mut discovered, &known_manifests);

        if let Ok(mut guard) = self.cached_games.lock() {
            *guard = discovered.clone();
        }
        self.record_log(
            "info",
            "detection",
            format!("Discovered {} installed game(s)", discovered.len()),
        );

        self.build_library_views(&discovered)
    }

    pub fn list_library_games(&self) -> Result<Vec<LibraryGameView>, CoreError> {
        let cached = self
            .cached_games
            .lock()
            .map_err(|_| CoreError::Storage("game cache mutex poisoned".to_owned()))?
            .clone();
        if cached.is_empty() {
            return self.rescan_games();
        }
        self.build_library_views(&cached)
    }

    fn build_library_views(
        &self,
        games: &[DetectedGame],
    ) -> Result<Vec<LibraryGameView>, CoreError> {
        let mut views = Vec::with_capacity(games.len());
        for game in games {
            let trainer_count = if let Some(game_id) = &game.matched_game_id {
                self.trainers.list_for_game(game_id)?.len()
            } else {
                0
            };
            let library_key = game
                .matched_game_id
                .clone()
                .or_else(|| game.store_app_id.as_ref().map(|id| format!("steam:{id}")))
                .unwrap_or_else(|| game.install_dir.clone());
            views.push(LibraryGameView {
                library_key,
                game: game.clone(),
                trainer_count,
            });
        }
        Ok(views)
    }

    /// Lists compatible trainers and their current permission state for a canonical `game_id`.
    pub fn list_trainers_for_game(
        &self,
        game_id: &str,
        detected_build_id: Option<&str>,
    ) -> Result<Vec<TrainerDetailView>, CoreError> {
        let installed = self.trainers.list_for_game(game_id)?;
        let policy = self.get_security_policy()?;
        let platform = current_target_platform();

        let mut views = Vec::with_capacity(installed.len());
        for item in installed {
            let compat = compute_compatibility(&item.manifest, platform, detected_build_id);
            let permission_state = if item.revoked {
                TrainerPermissionState::Blocked {
                    reason: "Trainer has been revoked".to_owned(),
                }
            } else {
                let confirmed = self.grants.load_confirmed_scopes(
                    &item.manifest.id,
                    &item.manifest.version.to_string(),
                    &item.manifest_sha256,
                )?;
                match PermissionEvaluator::evaluate(
                    &item.manifest,
                    &item.manifest_sha256,
                    item.trust_tier,
                    &policy,
                    &confirmed,
                ) {
                    Ok(EvaluationOutcome::Approved(grants)) => TrainerPermissionState::Approved {
                        granted_scopes: grants.granted_scopes.into_iter().collect(),
                    },
                    Ok(EvaluationOutcome::RequiresConfirmation {
                        missing_confirmations,
                    }) => TrainerPermissionState::RequiresConfirmation {
                        missing_confirmations,
                    },
                    Err(err) => TrainerPermissionState::Blocked {
                        reason: err.to_string(),
                    },
                }
            };

            views.push(TrainerDetailView {
                trainer_id: item.manifest.id.clone(),
                name: item.manifest.name.clone(),
                version: item.manifest.version.to_string(),
                description: item.manifest.description.clone(),
                authors: item
                    .manifest
                    .authors
                    .iter()
                    .map(|a| a.name.clone())
                    .collect(),
                source_repository: item.manifest.source_repository.clone(),
                trust_tier: item.trust_tier,
                manifest_sha256: item.manifest_sha256,
                compatibility_status: compat.status,
                compatibility_notes: compat.notes,
                revoked: item.revoked,
                permissions: item.manifest.permissions,
                permission_state,
                features: item.manifest.features,
            });
        }
        Ok(views)
    }

    pub fn list_all_installed_trainers(&self) -> Result<Vec<InstalledTrainer>, CoreError> {
        Ok(self.trainers.list_all_trainers()?)
    }

    /// Persists explicit player consent bound to `(trainer_id, version, manifest_sha256)`.
    pub fn grant_trainer_permissions(
        &self,
        trainer_id: &str,
        confirmed_scopes: BTreeSet<PermissionScope>,
    ) -> Result<TrainerPermissionState, CoreError> {
        let item = self.trainers.get_trainer(trainer_id)?;
        if item.revoked {
            return Err(CoreError::Trainer(TrainerManagerError::Revoked(
                trainer_id.to_owned(),
            )));
        }

        let policy = self.get_security_policy()?;
        let version_str = item.manifest.version.to_string();

        let eval = PermissionEvaluator::evaluate(
            &item.manifest,
            &item.manifest_sha256,
            item.trust_tier,
            &policy,
            &confirmed_scopes,
        )?;

        self.grants.save_confirmed_scopes(
            &item.manifest.id,
            &version_str,
            &item.manifest_sha256,
            &confirmed_scopes,
        )?;

        self.record_log(
            "info",
            "permissions",
            format!("Saved player permission grant for '{trainer_id}@{version_str}'"),
        );

        match eval {
            EvaluationOutcome::Approved(grants) => Ok(TrainerPermissionState::Approved {
                granted_scopes: grants.granted_scopes.into_iter().collect(),
            }),
            EvaluationOutcome::RequiresConfirmation {
                missing_confirmations,
            } => Ok(TrainerPermissionState::RequiresConfirmation {
                missing_confirmations,
            }),
        }
    }

    pub fn revoke_trainer_permissions(&self, trainer_id: &str) -> Result<(), CoreError> {
        self.grants.revoke_trainer_grants(trainer_id)?;
        self.record_log(
            "warn",
            "permissions",
            format!("Revoked permission grants for '{trainer_id}'"),
        );
        Ok(())
    }

    pub fn load_profile(
        &self,
        game_id: &str,
        trainer_id: &str,
    ) -> Result<Option<GameProfile>, CoreError> {
        use nekod_profile_manager::ProfileStore;
        Ok(self.profiles.load_profile(game_id, trainer_id)?)
    }

    pub fn save_profile(&self, profile: &GameProfile) -> Result<(), CoreError> {
        use nekod_profile_manager::ProfileStore;
        self.profiles.save_profile(profile)?;
        self.record_log(
            "info",
            "profile",
            format!(
                "Saved profile for '{}/{}'",
                profile.game_id, profile.trainer_id
            ),
        );
        Ok(())
    }

    pub fn export_profile_json(
        &self,
        game_id: &str,
        trainer_id: &str,
    ) -> Result<String, CoreError> {
        let profile = self
            .load_profile(game_id, trainer_id)?
            .unwrap_or_else(|| GameProfile {
                game_id: game_id.to_owned(),
                trainer_id: trainer_id.to_owned(),
                hotkeys: std::collections::BTreeMap::new(),
                saved_feature_states: std::collections::BTreeMap::new(),
                presets: std::collections::BTreeMap::new(),
            });
        Ok(profile.to_portable_json()?)
    }

    pub fn import_profile_json(&self, raw_json: &str) -> Result<GameProfile, CoreError> {
        let profile = GameProfile::from_portable_json(raw_json)?;
        self.save_profile(&profile)?;
        Ok(profile)
    }

    pub fn get_diagnostic_logs(&self) -> Vec<DiagnosticEvent> {
        self.logs.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn registry_cache(&self) -> &SqliteRegistryCache {
        &self.registry
    }

    pub fn runtime(&self) -> &Arc<dyn RuntimeProvider> {
        &self.runtime
    }

    /// Discovers running target processes matching known executable names for `game_id`.
    pub fn discover_running_processes_for_game(
        &self,
        game_id: &str,
    ) -> Result<Vec<ProcessIdentity>, CoreError> {
        let mut candidate_names: BTreeSet<String> = BTreeSet::new();

        for gm in self.trainers.list_game_manifests()? {
            if gm.id == game_id {
                for exe in gm.executables {
                    candidate_names.insert(exe.process_name);
                }
            }
        }

        if let Ok(cached) = self.cached_games.lock() {
            for detected in cached.iter() {
                if detected.matched_game_id.as_deref() == Some(game_id)
                    && let Some(exe_path) = &detected.executable_path
                    && let Some(name) = Path::new(exe_path).file_name().and_then(|n| n.to_str())
                {
                    candidate_names.insert(name.to_owned());
                }
            }
        }

        if candidate_names.is_empty() {
            return Ok(Vec::new());
        }

        let names: Vec<String> = candidate_names.into_iter().collect();
        Ok(self.runtime.discover_matching_processes(&names)?)
    }

    /// Lists all registered `GameManifest` definitions in the local catalog.
    pub fn list_registered_game_manifests(&self) -> Result<Vec<GameManifest>, CoreError> {
        Ok(self.trainers.list_game_manifests()?)
    }

    /// Collects primary top-level game executable names inside `install_dir`, skipping helper
    /// crash handlers and installers.
    pub fn collect_candidate_exes_in_dir(install_dir: &str) -> Vec<String> {
        let dir = Path::new(install_dir);
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names = BTreeSet::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = file_name.to_ascii_lowercase();
            if !lower.ends_with(".exe") {
                continue;
            }
            if matches!(
                lower.as_str(),
                "unitycrashhandler64.exe"
                    | "unitycrashhandler32.exe"
                    | "crashreportclient.exe"
                    | "unins000.exe"
                    | "ueprereqsetup_x64.exe"
                    | "vc_redist.x64.exe"
                    | "dxsetup.exe"
            ) || lower.contains("crashhandler")
                || lower.starts_with("unins")
            {
                continue;
            }
            names.insert(file_name.to_owned());
        }
        names.into_iter().collect()
    }

    /// Discovers running processes matching top-level `.exe` files in `install_dir`
    /// (works even before a `GameManifest` has been registered for the game).
    pub fn discover_running_processes_in_dir(
        &self,
        install_dir: &str,
    ) -> Result<Vec<ProcessIdentity>, CoreError> {
        let names = Self::collect_candidate_exes_in_dir(install_dir);
        if names.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.runtime.discover_matching_processes(&names)?)
    }

    /// Verifies policy, manifest/artifact integrity, and player capability grants, then starts
    /// a controlled trainer runtime session bound to `target_process`.
    pub fn start_trainer_session(
        &self,
        game_id: &str,
        trainer_id: &str,
        target_process: ProcessIdentity,
    ) -> Result<RuntimeSessionId, CoreError> {
        let item = self.trainers.get_trainer(trainer_id)?;
        if item.revoked {
            return Err(CoreError::Trainer(TrainerManagerError::Revoked(
                trainer_id.to_owned(),
            )));
        }

        if item.manifest.target_game.game_id != game_id {
            return Err(CoreError::IncompatiblePolicy(format!(
                "trainer '{trainer_id}' targets game '{}', expected '{game_id}'",
                item.manifest.target_game.game_id
            )));
        }

        if let Some(gm) = self
            .trainers
            .list_game_manifests()?
            .into_iter()
            .find(|g| g.id == game_id)
        {
            NekodCore::validate_game_trainer_compatibility(&gm, &item.manifest)?;
        }

        let policy = self.get_security_policy()?;
        let version_str = item.manifest.version.to_string();
        let confirmed = self.grants.load_confirmed_scopes(
            &item.manifest.id,
            &version_str,
            &item.manifest_sha256,
        )?;

        let granted_permissions = match PermissionEvaluator::evaluate(
            &item.manifest,
            &item.manifest_sha256,
            item.trust_tier,
            &policy,
            &confirmed,
        )? {
            EvaluationOutcome::Approved(grants) => grants,
            EvaluationOutcome::RequiresConfirmation {
                missing_confirmations,
            } => {
                let scopes = missing_confirmations
                    .iter()
                    .map(|p| format!("{:?}", p.scope))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CoreError::ConsentRequired(scopes));
            }
        };

        let payload_json = load_verified_entrypoint_str(&item)?;
        let pid = target_process.pid;
        let session_id = self.runtime.start_session(StartSessionRequest {
            trainer: item.manifest,
            target_process,
            granted_permissions,
            declarative_payload_json: Some(payload_json),
        })?;

        self.record_log(
            "info",
            "runtime",
            format!(
                "Started runtime session '{}' for '{trainer_id}' (PID {pid})",
                session_id.0
            ),
        );
        Ok(session_id)
    }

    /// Applies a feature toggle or scalar update within an active runtime session.
    pub fn apply_trainer_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), CoreError> {
        let feat_id = command.feature_id.clone();
        match self.runtime.apply_feature(session_id, command) {
            Ok(()) => {
                self.record_log(
                    "info",
                    "runtime",
                    format!("Applied feature '{feat_id}' on session '{}'", session_id.0),
                );
                Ok(())
            }
            Err(err) => {
                self.record_log(
                    "warn",
                    "runtime",
                    format!(
                        "Feature '{feat_id}' error on session '{}': {err}",
                        session_id.0
                    ),
                );
                Err(CoreError::Runtime(err))
            }
        }
    }

    /// Stops an active runtime session, rolling back all remaining patches in LIFO order.
    pub fn stop_trainer_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, CoreError> {
        let diag = self.runtime.stop_session(session_id)?;
        self.record_log(
            "info",
            "runtime",
            format!(
                "Stopped session '{}' (active_patches={}, conflicts={}, failures={})",
                session_id.0, diag.active_patches, diag.rollback_conflicts, diag.rollback_failures
            ),
        );
        Ok(diag)
    }

    /// Reads current runtime session diagnostics & patch ledger counts.
    pub fn get_session_diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, CoreError> {
        Ok(self.runtime.diagnostics(session_id)?)
    }

    // -------------------------------------------------------------------------
    // Phase P3 — Community Registry, Verification, Installation & Update Checks
    // -------------------------------------------------------------------------

    /// Lists configured community & third-party registry feeds paired with sync health.
    pub fn list_registry_sources(&self) -> Result<Vec<RegistrySourceState>, CoreError> {
        Ok(self.registry.list_sources()?)
    }

    /// Registers a user-added third-party registry feed constrained to `TrustTier::Unverified`.
    pub fn add_third_party_registry_source(
        &self,
        repository_id: &str,
        index_url: &str,
        enabled: bool,
    ) -> Result<(), CoreError> {
        let source = RegistrySource {
            repository_id: repository_id.to_owned(),
            index_url: index_url.to_owned(),
            authoritative_trust_tier: TrustTier::Unverified,
            enabled,
        };
        self.add_configured_registry_source(source)
    }

    /// Registers or updates a registry source with an explicit authoritative trust tier.
    pub fn add_configured_registry_source(&self, source: RegistrySource) -> Result<(), CoreError> {
        let repo_id = source.repository_id.clone();
        self.registry.upsert_source(&source)?;
        self.record_log(
            "info",
            "registry",
            format!(
                "Configured registry source '{repo_id}' (authority={:?})",
                source.authoritative_trust_tier
            ),
        );
        Ok(())
    }

    /// Removes a configured registry source and purges its cached index.
    pub fn remove_registry_source(&self, repository_id: &str) -> Result<(), CoreError> {
        self.registry.remove_source(repository_id)?;
        self.record_log(
            "info",
            "registry",
            format!("Removed registry source '{repository_id}'"),
        );
        Ok(())
    }

    /// Syncs all enabled registry sources via conditional HTTP and propagates any
    /// upstream `revoked` statuses directly into the local trainer catalog.
    pub fn sync_registry_sources(&self) -> Result<RegistrySyncReport, CoreError> {
        let report = self.registry.sync_all_enabled(&*self.http_transport)?;
        for revoked_id in &report.revoked_trainer_ids {
            if self.trainers.mark_revoked_if_installed(revoked_id)? {
                self.record_log(
                    "warn",
                    "security",
                    format!(
                        "Revoked locally installed trainer '{revoked_id}' per community registry"
                    ),
                );
            }
        }
        self.record_log(
            "info",
            "registry",
            format!(
                "Registry sync: checked={}, updated={}, not_modified={}, failed={}",
                report.sources_checked, report.updated, report.not_modified, report.failed
            ),
        );
        Ok(report)
    }

    /// Lists discovered community trainers for `game_id`, annotating local install/update status.
    pub fn list_community_trainers_for_game(
        &self,
        game_id: &str,
    ) -> Result<Vec<CommunityTrainerView>, CoreError> {
        let indexed = self.registry.find_trainers_for_game(game_id)?;
        let installed = self.trainers.list_for_game(game_id)?;

        Ok(indexed
            .into_iter()
            .map(|rec| {
                let local = installed
                    .iter()
                    .find(|t| t.manifest.id == rec.entry.trainer_id);
                let install_state = match local {
                    Some(t) if t.revoked => CommunityTrainerInstallState::Revoked,
                    Some(t) if rec.entry.latest_version > t.manifest.version => {
                        CommunityTrainerInstallState::UpdateAvailable {
                            installed_version: t.manifest.version.to_string(),
                            latest_version: rec.entry.latest_version.to_string(),
                        }
                    }
                    Some(t) => CommunityTrainerInstallState::InstalledUpToDate {
                        installed_version: t.manifest.version.to_string(),
                    },
                    None => CommunityTrainerInstallState::NotInstalled,
                };
                CommunityTrainerView {
                    repository_id: rec.repository_id,
                    trainer_id: rec.entry.trainer_id,
                    game_id: rec.entry.game_id,
                    latest_version: rec.entry.latest_version.to_string(),
                    manifest_url: rec.entry.manifest_url,
                    manifest_sha256: rec.entry.manifest_sha256,
                    trust_tier: rec.effective_trust_tier,
                    compatibility_status: rec
                        .entry
                        .compatibility_summary
                        .as_ref()
                        .and_then(|c| c.status),
                    last_verified_game_version: rec
                        .entry
                        .compatibility_summary
                        .and_then(|c| c.last_verified_game_version),
                    install_state,
                }
            })
            .collect())
    }

    /// Checks all locally installed trainers against cached community registries for updates.
    pub fn check_trainer_updates(&self) -> Result<Vec<TrainerUpdateInfo>, CoreError> {
        let installed = self.trainers.list_all_trainers()?;
        let indexed = self.registry.list_all_indexed_trainers()?;
        let mut updates = Vec::new();

        for local in installed {
            if local.revoked {
                continue;
            }
            if let Some(best) = indexed
                .iter()
                .filter(|r| r.entry.trainer_id == local.manifest.id)
                .max_by(|a, b| a.entry.latest_version.cmp(&b.entry.latest_version))
                && best.entry.latest_version > local.manifest.version
            {
                updates.push(TrainerUpdateInfo {
                    repository_id: best.repository_id.clone(),
                    trainer_id: local.manifest.id.clone(),
                    game_id: local.manifest.target_game.game_id.clone(),
                    installed_version: local.manifest.version.to_string(),
                    available_version: best.entry.latest_version.to_string(),
                    trust_tier: best.effective_trust_tier,
                });
            }
        }
        Ok(updates)
    }

    /// Downloads, SHA-256 verifies, stages, and installs/updates a trainer from a synced registry.
    /// Existing permission grants remain bound to the previous `(version, manifest_sha256)`,
    /// forcing explicit re-consent after any update.
    pub fn install_or_update_trainer_from_registry(
        &self,
        repository_id: &str,
        trainer_id: &str,
    ) -> Result<InstalledTrainer, CoreError> {
        if self
            .registry
            .list_revoked_trainer_ids()?
            .iter()
            .any(|id| id == trainer_id)
        {
            return Err(CoreError::Registry(RegistryError::Revoked(
                trainer_id.to_owned(),
            )));
        }

        let record = self
            .registry
            .find_entry(repository_id, trainer_id)?
            .ok_or_else(|| {
                CoreError::Registry(RegistryError::UpstreamNotFound(format!(
                    "{repository_id}/{trainer_id}"
                )))
            })?;

        let staging_root = self.packages_dir.join(".staging");
        let staged_dir =
            stage_verified_trainer_package(&record, &staging_root, &*self.http_transport)?;

        let install_res = self.trainers.install_staged_package(
            &staged_dir,
            &self.packages_dir,
            record.effective_trust_tier,
        );
        let _ = fs::remove_dir_all(&staged_dir);

        let installed = install_res?;
        self.record_log(
            "info",
            "registry",
            format!(
                "Installed verified trainer '{}@{}' from '{repository_id}' (tier={:?})",
                installed.manifest.id, installed.manifest.version, installed.trust_tier
            ),
        );
        Ok(installed)
    }

    /// Inspects a game directory for engine architecture, installed loaders, and anti-cheat markers.
    pub fn inspect_installed_game_environment(
        &self,
        install_dir: impl AsRef<Path>,
    ) -> GameEnvironmentProfile {
        inspect_game_directory(install_dir)
    }

    /// Evaluates declared `ModManifest.dependencies` against the inspected game directory and local cache.
    pub fn evaluate_mod_dependencies(
        &self,
        dependencies: &[RuntimeDependencySpec],
        install_dir: Option<&Path>,
    ) -> Vec<DependencyResolutionReport> {
        let env = install_dir.map(inspect_game_directory);
        self.dependencies
            .evaluate_dependencies(dependencies, env.as_ref())
    }

    /// Stages a SHA-256-verified upstream dependency archive into Nekod's managed cache.
    pub fn stage_verified_upstream_dependency(
        &self,
        dependency: &RuntimeDependencySpec,
        bytes: &[u8],
    ) -> Result<PathBuf, CoreError> {
        Ok(self
            .dependencies
            .stage_verified_upstream_bytes(dependency, bytes)?)
    }

    /// Persists a multi-dimensional compatibility observation (rejecting unverified AI `Verified` claims).
    pub fn record_compatibility_result(
        &self,
        entry: &NewCompatibilityRecord,
    ) -> Result<CompatibilityRecord, CoreError> {
        let saved = self.compatibility_db.record_entry(entry)?;
        self.record_log(
            "info",
            "compatibility",
            format!(
                "Recorded {:?} compatibility for '{}@{}' ({:?})",
                saved.status, saved.mod_id, saved.mod_version, saved.evidence
            ),
        );
        Ok(saved)
    }

    /// Queries compatibility matrix entries for `game_id`.
    pub fn query_compatibility_matrix(
        &self,
        game_id: &str,
        mod_id: Option<&str>,
        game_version: Option<&str>,
    ) -> Result<Vec<CompatibilityRecord>, CoreError> {
        Ok(self
            .compatibility_db
            .query_entries(game_id, mod_id, game_version)?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use nekod_manifest::AntiCheatKind;
    use nekod_trainer_manager::sha256_hex;
    use nekod_trainer_runtime::DeclarativeRuntime;

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekod_core_{prefix}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_trainer_for_prohibited_competitive_game() {
        let game_raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let trainer_raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut game: GameManifest = serde_json::from_str(game_raw).unwrap();
        let trainer: TrainerManifest = serde_json::from_str(trainer_raw).unwrap();

        game.compatibility_policy.trainer_policy = TrainerPolicy::ProhibitedOnlineCompetitive;
        let err = NekodCore::validate_game_trainer_compatibility(&game, &trainer).unwrap_err();
        assert!(matches!(err, CoreError::IncompatiblePolicy(_)));
    }

    #[test]
    fn rejects_anti_cheat_conflict_without_offline_requirement() {
        let game_raw = include_str!("../../../schemas/examples/game-manifest.example.json");
        let trainer_raw = include_str!("../../../schemas/examples/trainer-manifest.example.json");
        let mut game: GameManifest = serde_json::from_str(game_raw).unwrap();
        let mut trainer: TrainerManifest = serde_json::from_str(trainer_raw).unwrap();

        game.compatibility_policy
            .anti_cheat_detected
            .push(AntiCheatKind::EasyAntiCheat);
        trainer
            .compatibility
            .anti_cheat_incompatible
            .push(AntiCheatKind::EasyAntiCheat);
        trainer.compatibility.requires_offline_mode = false;

        let err = NekodCore::validate_game_trainer_compatibility(&game, &trainer).unwrap_err();
        assert!(matches!(err, CoreError::IncompatiblePolicy(_)));
    }

    #[test]
    fn end_to_end_p1_steam_discovery_trainer_ingestion_and_grant_flow() {
        let steam_root = temp_dir("steam");
        let pkg_dir = temp_dir("pkg");

        fs::create_dir_all(
            steam_root
                .join("steamapps")
                .join("common")
                .join("SampleRPG"),
        )
        .unwrap();
        fs::write(
            steam_root
                .join("steamapps")
                .join("common")
                .join("SampleRPG")
                .join("SampleRPG.exe"),
            b"MZ",
        )
        .unwrap();
        let vdf = format!(
            r#""libraryfolders" {{ "0" {{ "path" "{}" }} }}"#,
            steam_root.to_string_lossy().replace('\\', "\\\\")
        );
        fs::write(steam_root.join("steamapps").join("libraryfolders.vdf"), vdf).unwrap();
        fs::write(
            steam_root.join("steamapps").join("appmanifest_999999.acf"),
            r#"
            "AppState"
            {
                "appid" "999999"
                "name" "Sample Singleplayer RPG"
                "installdir" "SampleRPG"
                "buildid" "15000001"
            }
            "#,
        )
        .unwrap();

        let payload = br#"{"patches":[]}"#;
        fs::write(pkg_dir.join("trainer.json"), payload).unwrap();
        let mut trainer: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        trainer.artifacts[0].sha256 = sha256_hex(payload);
        trainer.artifacts[0].size_bytes = payload.len() as u64;
        fs::write(
            pkg_dir.join("trainer-manifest.json"),
            serde_json::to_string_pretty(&trainer).unwrap(),
        )
        .unwrap();

        let service = NekodAppService::open_in_memory(
            Arc::new(DeclarativeRuntime::default()),
            Some(steam_root.clone()),
        )
        .unwrap();

        service
            .register_game_manifest(include_str!(
                "../../../schemas/examples/game-manifest.example.json"
            ))
            .unwrap();
        service
            .ingest_local_trainer(&pkg_dir, TrustTier::CommunityVerified)
            .unwrap();

        let library = service.rescan_games().unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(
            library[0].game.matched_game_id.as_deref(),
            Some("sample-singleplayer-rpg")
        );
        assert_eq!(library[0].trainer_count, 1);

        let before = service
            .list_trainers_for_game("sample-singleplayer-rpg", Some("15000001"))
            .unwrap();
        assert!(matches!(
            before[0].permission_state,
            TrainerPermissionState::RequiresConfirmation { .. }
        ));

        let mut confirmed = BTreeSet::new();
        confirmed.insert(PermissionScope::ProcessReadMemory);
        confirmed.insert(PermissionScope::ProcessWriteMemory);
        let after = service
            .grant_trainer_permissions("dev.example.sample-singleplayer-rpg", confirmed)
            .unwrap();
        assert!(matches!(after, TrainerPermissionState::Approved { .. }));

        let _ = fs::remove_dir_all(steam_root);
        let _ = fs::remove_dir_all(pkg_dir);
    }

    #[test]
    fn service_session_requires_consent_verifies_integrity_and_rolls_back() {
        use nekod_manifest::{BinaryArch, TargetPlatform};
        use nekod_runtime_api::{FeatureValue, SessionLifecycleState};
        use nekod_trainer_runtime::backend::ProcessAccessMode;
        use nekod_trainer_runtime::backend::ProcessMemoryBackend;
        use nekod_trainer_runtime::backend::mock::MockMemoryBackend;

        let backend = Arc::new(MockMemoryBackend::new());
        let mut mem = vec![0u8; 0x3000];
        mem[0x0100..0x0106].copy_from_slice(&[0x29, 0x73, 0x10, 0x8B, 0x43, 0x10]);
        let base: u64 = 0x1400_00000;
        mem[0x1234..0x123C].copy_from_slice(&(base + 0x2000).to_le_bytes());
        mem[0x2010..0x2018].copy_from_slice(&(base + 0x2400).to_le_bytes());
        mem[0x2420..0x2424].copy_from_slice(&50i32.to_le_bytes());
        backend.insert_process(MockMemoryBackend::sample_windows_process(
            9001,
            "SampleRPG.exe",
            mem,
        ));

        let pkg_dir = temp_dir("p2_pkg");
        let payload = include_str!("../../../schemas/examples/declarative-patch.example.json");
        fs::write(pkg_dir.join("trainer.json"), payload.as_bytes()).unwrap();
        let mut trainer: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        trainer.artifacts[0].sha256 = sha256_hex(payload.as_bytes());
        trainer.artifacts[0].size_bytes = payload.len() as u64;
        fs::write(
            pkg_dir.join("trainer-manifest.json"),
            serde_json::to_string_pretty(&trainer).unwrap(),
        )
        .unwrap();

        let runtime = Arc::new(DeclarativeRuntime::with_backend(backend.clone()));
        let service = NekodAppService::open_in_memory(runtime, None).unwrap();
        service
            .register_game_manifest(include_str!(
                "../../../schemas/examples/game-manifest.example.json"
            ))
            .unwrap();
        service
            .ingest_local_trainer(&pkg_dir, TrustTier::CommunityVerified)
            .unwrap();

        // 1. Discover matching running process for game
        let procs = service
            .discover_running_processes_for_game("sample-singleplayer-rpg")
            .unwrap();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 9001);
        assert_eq!(procs[0].platform, TargetPlatform::WindowsX64);
        assert_eq!(procs[0].architecture, BinaryArch::X86_64);

        // 2. Starting session before explicit consent fails closed
        let err = service
            .start_trainer_session(
                "sample-singleplayer-rpg",
                "dev.example.sample-singleplayer-rpg",
                procs[0].clone(),
            )
            .unwrap_err();
        assert!(matches!(err, CoreError::ConsentRequired(_)));

        // 3. Grant scopes -> start session -> toggle infinite-health & stop session
        let mut confirmed = BTreeSet::new();
        confirmed.insert(PermissionScope::ProcessReadMemory);
        confirmed.insert(PermissionScope::ProcessWriteMemory);
        service
            .grant_trainer_permissions("dev.example.sample-singleplayer-rpg", confirmed)
            .unwrap();

        let sid = service
            .start_trainer_session(
                "sample-singleplayer-rpg",
                "dev.example.sample-singleplayer-rpg",
                procs[0].clone(),
            )
            .unwrap();

        service
            .apply_trainer_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let ro = backend
            .open_process(9001, ProcessAccessMode::ReadOnly)
            .unwrap();
        assert_eq!(
            ro.read_memory(0x1400_00100, 3).unwrap(),
            vec![0x90, 0x90, 0x90]
        );

        let diag = service.stop_trainer_session(&sid).unwrap();
        assert_eq!(diag.state, SessionLifecycleState::Stopped);
        assert_eq!(diag.active_patches, 0);
        assert_eq!(
            ro.read_memory(0x1400_00100, 3).unwrap(),
            vec![0x29, 0x73, 0x10]
        );

        // 4. Tampering trainer.json on disk is caught before starting another session
        fs::write(pkg_dir.join("trainer.json"), b"{}").unwrap();
        let tamper_err = service
            .start_trainer_session(
                "sample-singleplayer-rpg",
                "dev.example.sample-singleplayer-rpg",
                procs[0].clone(),
            )
            .unwrap_err();
        assert!(matches!(
            tamper_err,
            CoreError::Trainer(TrainerManagerError::IntegrityMismatch(_, _))
        ));

        let _ = fs::remove_dir_all(pkg_dir);
    }

    #[test]
    fn registry_sync_update_reconsent_and_revocation_workflow() {
        use nekod_registry_client::transport::mock::{MockHttpReply, MockHttpTransport};
        use nekod_trainer_runtime::DeclarativeRuntime;

        let index_url = "https://raw.githubusercontent.com/example-contributor/sample-rpg-trainer/main/registry.json";
        let payload_bytes =
            include_bytes!("../../../schemas/examples/declarative-patch.example.json").to_vec();
        let mut trainer_v1: TrainerManifest = serde_json::from_str(include_str!(
            "../../../schemas/examples/trainer-manifest.example.json"
        ))
        .unwrap();
        trainer_v1.artifacts[0].sha256 = sha256_hex(&payload_bytes);
        trainer_v1.artifacts[0].size_bytes = payload_bytes.len() as u64;
        let m_v1_bytes = serde_json::to_vec_pretty(&trainer_v1).unwrap();
        let m_v1_sha = sha256_hex(&m_v1_bytes);

        let mut trainer_v2 = trainer_v1.clone();
        trainer_v2.version = "1.1.0".parse().unwrap();
        let m_v2_bytes = serde_json::to_vec_pretty(&trainer_v2).unwrap();
        let m_v2_sha = sha256_hex(&m_v2_bytes);

        let m_v1_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer-manifest.json";
        let p_v1_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer.json";
        let m_v2_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.1.0/trainer-manifest.json";
        let p_v2_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.1.0/trainer.json";

        let reg_v1 = include_str!("../../../schemas/examples/registry-manifest.example.json")
            .replace(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                &m_v1_sha,
            );
        let reg_v2 = reg_v1
            .replace("\"1.0.0\"", "\"1.1.0\"")
            .replace("v1.0.0", "v1.1.0")
            .replace(&m_v1_sha, &m_v2_sha)
            // Restore schema_version which also matched "1.0.0"
            .replacen("\"1.1.0\"", "\"1.0.0\"", 1);
        let reg_revoked = reg_v2.replace("\"active\"", "\"revoked\"");

        let transport = Arc::new(MockHttpTransport::new());
        transport.enqueue(
            index_url,
            MockHttpReply::Ok {
                body: reg_v1.as_bytes().to_vec(),
                etag: Some("\"e1\"".to_owned()),
                last_modified: None,
            },
        );
        transport.enqueue(
            index_url,
            MockHttpReply::Ok {
                body: reg_v2.as_bytes().to_vec(),
                etag: Some("\"e2\"".to_owned()),
                last_modified: None,
            },
        );
        transport.enqueue(
            index_url,
            MockHttpReply::Ok {
                body: reg_revoked.as_bytes().to_vec(),
                etag: Some("\"e3\"".to_owned()),
                last_modified: None,
            },
        );
        transport.set_ok(m_v1_url, m_v1_bytes, None);
        transport.set_ok(p_v1_url, payload_bytes.clone(), None);
        transport.set_ok(m_v2_url, m_v2_bytes, None);
        transport.set_ok(p_v2_url, payload_bytes, None);

        let pkgs_dir = temp_dir("p3_pkgs");
        let service = NekodAppService::open_in_memory_with_transport(
            Arc::new(DeclarativeRuntime::default()),
            None,
            transport,
            pkgs_dir.clone(),
        )
        .unwrap();

        service
            .add_configured_registry_source(RegistrySource {
                repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
                index_url: index_url.to_owned(),
                authoritative_trust_tier: TrustTier::CommunityVerified,
                enabled: true,
            })
            .unwrap();

        // 1. Sync v1 & install verified release
        let s1 = service.sync_registry_sources().unwrap();
        assert_eq!(s1.updated, 1);

        let installed_v1 = service
            .install_or_update_trainer_from_registry(
                "com.github.example-contributor.sample-trainers",
                "dev.example.sample-singleplayer-rpg",
            )
            .unwrap();
        assert_eq!(installed_v1.manifest.version.to_string(), "1.0.0");

        let mut confirmed = BTreeSet::new();
        confirmed.insert(PermissionScope::ProcessReadMemory);
        confirmed.insert(PermissionScope::ProcessWriteMemory);
        service
            .grant_trainer_permissions("dev.example.sample-singleplayer-rpg", confirmed)
            .unwrap();

        // 2. Sync v2 -> detect update -> update invalidates v1 permission grant!
        service.sync_registry_sources().unwrap();
        let updates = service.check_trainer_updates().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].available_version, "1.1.0");

        let installed_v2 = service
            .install_or_update_trainer_from_registry(
                "com.github.example-contributor.sample-trainers",
                "dev.example.sample-singleplayer-rpg",
            )
            .unwrap();
        assert_eq!(installed_v2.manifest.version.to_string(), "1.1.0");

        let views_after_update = service
            .list_trainers_for_game("sample-singleplayer-rpg", None)
            .unwrap();
        assert!(matches!(
            views_after_update[0].permission_state,
            TrainerPermissionState::RequiresConfirmation { .. }
        ));

        // 3. Sync revoked -> marks installed trainer revoked & blocks execution!
        service.sync_registry_sources().unwrap();
        let views_revoked = service
            .list_trainers_for_game("sample-singleplayer-rpg", None)
            .unwrap();
        assert!(views_revoked[0].revoked);

        let _ = fs::remove_dir_all(pkgs_dir);
    }

    #[test]
    fn collects_primary_exes_and_filters_crash_handlers_in_dir() {
        let dir = temp_dir("collect_exes");
        fs::write(dir.join("Sugar Lust - Hentai Harem.exe"), b"MZ").unwrap();
        fs::write(dir.join("UnityCrashHandler64.exe"), b"MZ").unwrap();
        fs::write(dir.join("unins000.exe"), b"MZ").unwrap();
        fs::write(dir.join("UnityPlayer.dll"), b"MZ").unwrap();

        let exes = NekodAppService::collect_candidate_exes_in_dir(dir.to_string_lossy().as_ref());
        assert_eq!(exes, vec!["Sugar Lust - Hentai Harem.exe".to_owned()]);

        let _ = fs::remove_dir_all(dir);
    }
}
