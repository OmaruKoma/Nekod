//! L2 Game Detection (Steam VDF/ACF, Epic `.item`, GOG `.info`, & custom directory discovery).

pub mod custom;
pub mod engine;
pub mod epic;
pub mod gog;
pub mod steam;
pub mod vdf;

pub use custom::{CustomGameFolder, CustomPathDetector};
pub use engine::{
    DetectedLoader, GameEngineKind, GameEnvironmentProfile, InstalledLoaderKind,
    inspect_game_directory,
};
pub use epic::EpicDetector;
pub use gog::GogDetector;
pub use steam::{SteamDetector, current_target_platform, match_games_against_manifests};

use nekod_manifest::TargetPlatform;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Storefront or source from which a local game installation was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStoreSource {
    Steam,
    Gog,
    Epic,
    Xbox,
    CustomPath,
}

/// Discovered local game installation metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedGame {
    pub matched_game_id: Option<String>,
    pub title: String,
    pub store_source: GameStoreSource,
    pub store_app_id: Option<String>,
    pub store_build_id: Option<String>,
    pub install_dir: String,
    pub executable_path: Option<String>,
    pub platform: TargetPlatform,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DetectionError {
    #[error("game detection provider unavailable on this platform: {0}")]
    UnsupportedPlatform(&'static str),

    #[error("failed to parse store metadata: {0}")]
    MetadataParse(String),
}

/// Trait implemented by store-specific game detectors (e.g. Steam VDF/ACF in P1).
pub trait GameDetector: Send + Sync {
    fn source(&self) -> GameStoreSource;
    fn discover_installed(&self) -> Result<Vec<DetectedGame>, DetectionError>;
}
