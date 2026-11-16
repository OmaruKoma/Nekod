//! L2 Profile Manager with SQLite persistence for player profiles, hotkeys, and custom paths.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use nekod_runtime_api::FeatureValue;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Saved player configuration for a specific game & trainer pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameProfile {
    pub game_id: String,
    pub trainer_id: String,
    pub hotkeys: BTreeMap<String, String>,
    pub saved_feature_states: BTreeMap<String, FeatureValue>,
}

/// Custom local game folder persisted by the player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGamePathRecord {
    pub path: String,
    pub label: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProfileError {
    #[error("profile not found for game '{0}'")]
    NotFound(String),

    #[error("storage error: {0}")]
    Storage(String),
}

/// Trait abstracting profile persistence.
pub trait ProfileStore: Send + Sync {
    fn load_profile(
        &self,
        game_id: &str,
        trainer_id: &str,
    ) -> Result<Option<GameProfile>, ProfileError>;
    fn save_profile(&self, profile: &GameProfile) -> Result<(), ProfileError>;
}

/// SQLite-backed implementation for player profiles, custom game paths, and settings.
pub struct SqliteProfileStore {
    conn: Mutex<Connection>,
}

impl SqliteProfileStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let conn = Connection::open(path).map_err(|e| ProfileError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, ProfileError> {
        let conn =
            Connection::open_in_memory().map_err(|e| ProfileError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), ProfileError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS profiles (
                game_id TEXT NOT NULL,
                trainer_id TEXT NOT NULL,
                hotkeys_json TEXT NOT NULL,
                feature_states_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (game_id, trainer_id)
            );
            CREATE TABLE IF NOT EXISTS custom_game_paths (
                path TEXT PRIMARY KEY,
                label TEXT,
                added_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| ProfileError::Storage(e.to_string()))?;
        Ok(())
    }

    pub fn add_custom_game_path(
        &self,
        path: &str,
        label: Option<&str>,
    ) -> Result<(), ProfileError> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(ProfileError::Storage(
                "custom game path cannot be empty".to_owned(),
            ));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO custom_game_paths (path, label) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET label = excluded.label",
            params![trimmed, label],
        )
        .map_err(|e| ProfileError::Storage(e.to_string()))?;
        Ok(())
    }

    pub fn remove_custom_game_path(&self, path: &str) -> Result<(), ProfileError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        conn.execute(
            "DELETE FROM custom_game_paths WHERE path = ?1",
            params![path],
        )
        .map_err(|e| ProfileError::Storage(e.to_string()))?;
        Ok(())
    }

    pub fn list_custom_game_paths(&self) -> Result<Vec<CustomGamePathRecord>, ProfileError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        let mut stmt = conn
            .prepare("SELECT path, label FROM custom_game_paths ORDER BY added_at ASC, path ASC")
            .map_err(|e| ProfileError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CustomGamePathRecord {
                    path: row.get(0)?,
                    label: row.get(1)?,
                })
            })
            .map_err(|e| ProfileError::Storage(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| ProfileError::Storage(e.to_string()))
    }

    pub fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), ProfileError> {
        let json =
            serde_json::to_string(value).map_err(|e| ProfileError::Storage(e.to_string()))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO app_settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, json],
        )
        .map_err(|e| ProfileError::Storage(e.to_string()))?;
        Ok(())
    }

    pub fn load_setting<T: for<'de> Deserialize<'de>>(
        &self,
        key: &str,
    ) -> Result<Option<T>, ProfileError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ProfileError::Storage(e.to_string()))?;

        match raw {
            Some(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| ProfileError::Storage(e.to_string())),
            None => Ok(None),
        }
    }
}

impl ProfileStore for SqliteProfileStore {
    fn load_profile(
        &self,
        game_id: &str,
        trainer_id: &str,
    ) -> Result<Option<GameProfile>, ProfileError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT hotkeys_json, feature_states_json FROM profiles WHERE game_id = ?1 AND trainer_id = ?2",
                params![game_id, trainer_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| ProfileError::Storage(e.to_string()))?;

        match row {
            Some((hotkeys_json, states_json)) => {
                let hotkeys = serde_json::from_str(&hotkeys_json)
                    .map_err(|e| ProfileError::Storage(e.to_string()))?;
                let saved_feature_states = serde_json::from_str(&states_json)
                    .map_err(|e| ProfileError::Storage(e.to_string()))?;
                Ok(Some(GameProfile {
                    game_id: game_id.to_owned(),
                    trainer_id: trainer_id.to_owned(),
                    hotkeys,
                    saved_feature_states,
                }))
            }
            None => Ok(None),
        }
    }

    fn save_profile(&self, profile: &GameProfile) -> Result<(), ProfileError> {
        let hotkeys_json = serde_json::to_string(&profile.hotkeys)
            .map_err(|e| ProfileError::Storage(e.to_string()))?;
        let states_json = serde_json::to_string(&profile.saved_feature_states)
            .map_err(|e| ProfileError::Storage(e.to_string()))?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| ProfileError::Storage("database mutex poisoned".to_owned()))?;
        conn.execute(
            "INSERT INTO profiles (game_id, trainer_id, hotkeys_json, feature_states_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(game_id, trainer_id) DO UPDATE SET
                hotkeys_json = excluded.hotkeys_json,
                feature_states_json = excluded.feature_states_json,
                updated_at = unixepoch()",
            params![profile.game_id, profile.trainer_id, hotkeys_json, states_json],
        )
        .map_err(|e| ProfileError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_profile_and_custom_paths_in_sqlite() {
        let store = SqliteProfileStore::open_in_memory().unwrap();
        let mut hotkeys = BTreeMap::new();
        hotkeys.insert("infinite-health".to_owned(), "F1".to_owned());
        let mut states = BTreeMap::new();
        states.insert("infinite-health".to_owned(), FeatureValue::Toggle(false));

        let profile = GameProfile {
            game_id: "sample-singleplayer-rpg".to_owned(),
            trainer_id: "dev.example.sample-singleplayer-rpg".to_owned(),
            hotkeys,
            saved_feature_states: states,
        };
        store.save_profile(&profile).unwrap();

        let loaded = store
            .load_profile(
                "sample-singleplayer-rpg",
                "dev.example.sample-singleplayer-rpg",
            )
            .unwrap()
            .expect("profile exists");
        assert_eq!(loaded, profile);

        store
            .add_custom_game_path("D:\\Games\\SampleRPG", Some("Sample RPG"))
            .unwrap();
        let paths = store.list_custom_game_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].label.as_deref(), Some("Sample RPG"));
    }
}
