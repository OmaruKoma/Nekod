#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nekod_desktop_lib::run();
}

#[cfg(test)]
mod tests {
    use nekod_core::NekodAppService;
    use nekod_desktop_lib::{AppState, desktop_invoke_handler};
    use nekod_trainer_runtime::CompositeRuntimeRouter;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tauri::http::HeaderMap;
    use tauri::test::{MockRuntime, get_ipc_response, mock_builder};
    use tauri::webview::InvokeRequest;

    fn build_ipc_request(
        origin: tauri::Url,
        cmd: &str,
        payload: serde_json::Value,
    ) -> InvokeRequest {
        InvokeRequest {
            cmd: cmd.to_owned(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: origin,
            body: tauri::ipc::InvokeBody::Json(payload),
            headers: HeaderMap::new(),
            invoke_key: tauri::test::INVOKE_KEY.to_owned(),
        }
    }

    fn invoke_json(
        webview: &tauri::WebviewWindow<MockRuntime>,
        cmd: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let origin = webview.url().expect("webview url");
        let resp = get_ipc_response(webview, build_ipc_request(origin, cmd, payload))
            .map_err(|e| format!("{e:?}"))?;
        resp.deserialize::<serde_json::Value>()
            .map_err(|e| format!("deserialize error: {e}"))
    }

    #[test]
    fn tauri_desktop_window_and_game_management_ipc_flow() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("nekod_tauri_ipc_{}_{}", std::process::id(), nanos));
        let data_dir = root.join("data");
        let custom_game_dir = root.join("CustomGames").join("SampleRPG");
        fs::create_dir_all(&custom_game_dir).unwrap();
        fs::write(custom_game_dir.join("SampleRPG.exe"), b"MZ").unwrap();

        let core = NekodAppService::open_in_dir(
            &data_dir,
            Arc::new(CompositeRuntimeRouter::with_declarative_default()),
        )
        .unwrap();

        let app = mock_builder()
            .manage(AppState::new(core))
            .invoke_handler(desktop_invoke_handler())
            .build(tauri::generate_context!())
            .expect("build mock Tauri app with real context & capabilities");

        // Instantiate the main window directly from tauri.conf.json + capabilities/default.json ACL.
        let win_cfg = app.config().app.windows[0].clone();
        assert_eq!(win_cfg.title, "Nekod — Modding games, together");
        assert_eq!(win_cfg.width, 1180.0);
        assert_eq!(win_cfg.height, 760.0);
        let webview = tauri::WebviewWindowBuilder::from_config(&app, &win_cfg)
            .expect("valid window config")
            .build()
            .expect("build main webview window");
        assert_eq!(webview.label(), "main");

        // 1. Register custom game folder + register game manifest using exact camelCase IPC keys from apps/desktop/src/api/nekod.ts.
        let paths = invoke_json(
            &webview,
            "add_custom_game_path",
            serde_json::json!({
                "path": custom_game_dir.to_string_lossy(),
                "label": "Sample Custom Folder"
            }),
        )
        .expect("add_custom_game_path");
        assert_eq!(paths.as_array().unwrap().len(), 1);

        let game_manifest_raw =
            include_str!("../../../../schemas/examples/game-manifest.example.json");
        let registered_id = invoke_json(
            &webview,
            "register_game_manifest",
            serde_json::json!({ "rawJson": game_manifest_raw }),
        )
        .expect("register_game_manifest");
        assert_eq!(registered_id, "sample-singleplayer-rpg");

        // 2. Rescan game library and verify custom folder matches SampleRPG manifest.
        let library =
            invoke_json(&webview, "rescan_games", serde_json::json!({})).expect("rescan_games");
        let matched = library
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["game"]["matched_game_id"] == "sample-singleplayer-rpg")
            .expect("custom game matched against manifest");
        assert_eq!(matched["game"]["store_source"], "custom_path");

        // 3. Ingest example trainer package and inspect default-deny permission state.
        let pkg_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("examples")
            .join("sample-singleplayer-rpg");
        invoke_json(
            &webview,
            "ingest_local_trainer",
            serde_json::json!({
                "packageDir": pkg_dir.to_string_lossy(),
                "trustTier": "community_verified"
            }),
        )
        .expect("ingest_local_trainer");

        let trainers = invoke_json(
            &webview,
            "list_trainers_for_game",
            serde_json::json!({
                "gameId": "sample-singleplayer-rpg",
                "detectedBuildId": null
            }),
        )
        .expect("list_trainers_for_game");
        let trainer = &trainers.as_array().unwrap()[0];
        assert_eq!(trainer["trainer_id"], "dev.example.sample-singleplayer-rpg");
        assert_eq!(
            trainer["permission_state"]["status"],
            "requires_confirmation"
        );

        // 4. Grant declared scopes via IPC and verify transition to Approved.
        let scopes = trainer["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["scope"].clone())
            .collect::<Vec<_>>();
        let grant_state = invoke_json(
            &webview,
            "grant_trainer_permissions",
            serde_json::json!({
                "trainerId": "dev.example.sample-singleplayer-rpg",
                "confirmedScopes": scopes
            }),
        )
        .expect("grant_trainer_permissions");
        assert_eq!(grant_state["status"], "approved");

        // 5. Save and reload hotkey profile via IPC.
        invoke_json(
            &webview,
            "save_profile",
            serde_json::json!({
                "profile": {
                    "game_id": "sample-singleplayer-rpg",
                    "trainer_id": "dev.example.sample-singleplayer-rpg",
                    "hotkeys": { "infinite-health": "Ctrl+F5" },
                    "saved_feature_states": {}
                }
            }),
        )
        .expect("save_profile");
        let loaded = invoke_json(
            &webview,
            "load_profile",
            serde_json::json!({
                "gameId": "sample-singleplayer-rpg",
                "trainerId": "dev.example.sample-singleplayer-rpg"
            }),
        )
        .expect("load_profile");
        assert_eq!(loaded["hotkeys"]["infinite-health"], "Ctrl+F5");

        // 6. Negative IPC error handling checks (invalid manifest & missing package return Err, never panic).
        let bad_manifest = invoke_json(
            &webview,
            "register_game_manifest",
            serde_json::json!({ "rawJson": "{not-valid-json}" }),
        );
        assert!(bad_manifest.is_err());

        let missing_pkg = invoke_json(
            &webview,
            "ingest_local_trainer",
            serde_json::json!({
                "packageDir": root.join("does-not-exist").to_string_lossy(),
                "trustTier": "unverified"
            }),
        );
        assert!(missing_pkg.is_err());

        // 7. Remove custom path & verify diagnostic audit logs captured workflow.
        let after_remove = invoke_json(
            &webview,
            "remove_custom_game_path",
            serde_json::json!({ "path": custom_game_dir.to_string_lossy() }),
        )
        .expect("remove_custom_game_path");
        assert!(after_remove.as_array().unwrap().is_empty());

        let logs = invoke_json(&webview, "get_diagnostic_logs", serde_json::json!({}))
            .expect("get_diagnostic_logs");
        assert!(!logs.as_array().unwrap().is_empty());

        let _ = fs::remove_dir_all(&root);
    }
}
