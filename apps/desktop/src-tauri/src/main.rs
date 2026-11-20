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
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tauri::http::HeaderMap;
    use tauri::test::{MockRuntime, get_ipc_response, mock_builder};
    use tauri::webview::InvokeRequest;
    use zip::write::SimpleFileOptions;

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
            .manage(AppState::with_data_dir(core, data_dir.clone()))
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

        // 7. Verify V2-M2 engine directory inspection & compatibility matrix IPC endpoints.
        let env = invoke_json(
            &webview,
            "inspect_game_environment",
            serde_json::json!({ "installDir": custom_game_dir.to_string_lossy() }),
        )
        .expect("inspect_game_environment");
        assert_eq!(env["dir_exists"], true);
        assert_eq!(env["engine"], "native_unknown");

        let matrix = invoke_json(
            &webview,
            "query_compatibility_matrix",
            serde_json::json!({
                "gameId": "sample-singleplayer-rpg",
                "modId": null,
                "gameVersion": null
            }),
        )
        .expect("query_compatibility_matrix");
        assert!(matrix.as_array().unwrap().is_empty());

        // 7b. Verify companion bridge one-click deploy & undeploy (DeploymentLedger) + path safety checks.
        let bridge_src = root.join("NekodUnityBridge.Mono.dll");
        fs::write(&bridge_src, b"dummy-managed-plugin-bytes").unwrap();
        let initial_bridge = invoke_json(
            &webview,
            "get_bridge_deployment_status",
            serde_json::json!({
                "gameKey": "custom:sample-rpg",
                "installDir": custom_game_dir.to_string_lossy()
            }),
        )
        .expect("get_bridge_deployment_status");
        assert_eq!(initial_bridge["deployed"], false);

        // Reject unsafe target paths (outside allowlisted plugin subdirs) and blocked executable extensions.
        let bad_target = invoke_json(
            &webview,
            "deploy_companion_bridge",
            serde_json::json!({
                "gameKey": "custom:sample-rpg",
                "installDir": custom_game_dir.to_string_lossy(),
                "sourceFile": bridge_src.to_string_lossy(),
                "targetRelativePath": "bad_root.dll"
            }),
        );
        assert!(bad_target.is_err());

        let deployed = invoke_json(
            &webview,
            "deploy_companion_bridge",
            serde_json::json!({
                "gameKey": "custom:sample-rpg",
                "installDir": custom_game_dir.to_string_lossy(),
                "sourceFile": bridge_src.to_string_lossy(),
                "targetRelativePath": "BepInEx/plugins/NekodUnityBridge.Mono.dll"
            }),
        )
        .expect("deploy_companion_bridge");
        assert_eq!(deployed["deployed"], true);
        assert!(
            custom_game_dir
                .join("BepInEx")
                .join("plugins")
                .join("NekodUnityBridge.Mono.dll")
                .exists()
        );

        let undeploy = invoke_json(
            &webview,
            "undeploy_companion_bridge",
            serde_json::json!({
                "gameKey": "custom:sample-rpg",
                "installDir": custom_game_dir.to_string_lossy()
            }),
        )
        .expect("undeploy_companion_bridge");
        assert_eq!(undeploy["removed_count"], 1);
        assert_eq!(undeploy["conflicts"], 0);
        assert!(
            !custom_game_dir
                .join("BepInEx")
                .join("plugins")
                .join("NekodUnityBridge.Mono.dll")
                .exists()
        );

        // 7c. Verify Direction 1/2/3/4 IPC commands: global hotkeys, artwork, bridge catalog, auto-deploy, and launch security.
        let bound_count = invoke_json(
            &webview,
            "sync_global_hotkeys",
            serde_json::json!({
                "bindings": [
                    { "feature_id": "infinite-health", "hotkey": "Ctrl+F5" },
                    { "feature_id": "invalid-entry", "hotkey": "F99" }
                ]
            }),
        )
        .expect("sync_global_hotkeys");
        assert_eq!(bound_count, 1);

        let catalog = invoke_json(
            &webview,
            "list_companion_bridge_catalog",
            serde_json::json!({}),
        )
        .expect("list_companion_bridge_catalog");
        assert!(catalog.as_array().unwrap().len() >= 5);

        // Local cover.jpg resolution inside custom game dir
        fs::write(
            custom_game_dir.join("cover.jpg"),
            [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
        )
        .unwrap();
        let art = invoke_json(
            &webview,
            "get_game_artwork",
            serde_json::json!({
                "storeSource": "custom_path",
                "storeAppId": null,
                "installDir": custom_game_dir.to_string_lossy()
            }),
        )
        .expect("get_game_artwork");
        assert!(
            art["header_data_uri"]
                .as_str()
                .unwrap_or_default()
                .starts_with("data:image/jpeg;base64,")
        );

        // Auto-deploy companion bridge for a Unity Mono directory without typing any DLL path manually
        let unity_game_dir = root.join("UnityMonoDemo");
        fs::create_dir_all(unity_game_dir.join("UnityMonoDemo_Data").join("Managed")).unwrap();
        fs::write(unity_game_dir.join("UnityPlayer.dll"), b"MZ").unwrap();
        fs::write(unity_game_dir.join("UnityMonoDemo.exe"), b"MZ").unwrap();
        let auto_deployed = invoke_json(
            &webview,
            "auto_deploy_companion_bridge",
            serde_json::json!({
                "gameKey": "custom:unity-mono-demo",
                "installDir": unity_game_dir.to_string_lossy(),
                "customSourceOverride": bridge_src.to_string_lossy()
            }),
        )
        .expect("auto_deploy_companion_bridge");
        assert_eq!(auto_deployed["deployed"], true);
        assert_eq!(auto_deployed["loader_installed"], false);
        assert!(auto_deployed["loader_warning"].as_str().is_some());
        assert!(
            unity_game_dir
                .join("BepInEx")
                .join("plugins")
                .join("NekodUnityBridge.Mono.dll")
                .exists()
        );
        let auto_undeploy = invoke_json(
            &webview,
            "undeploy_companion_bridge",
            serde_json::json!({
                "gameKey": "custom:unity-mono-demo",
                "installDir": unity_game_dir.to_string_lossy()
            }),
        )
        .expect("undeploy auto-deployed bridge");
        assert_eq!(auto_undeploy["removed_count"], 1);

        // Unregistered game process discovery fallback via installDir (should return Vec cleanly without error)
        let unregistered_procs = invoke_json(
            &webview,
            "discover_running_processes",
            serde_json::json!({
                "gameId": null,
                "installDir": unity_game_dir.to_string_lossy()
            }),
        )
        .expect("discover_running_processes by install_dir");
        assert!(unregistered_procs.is_array());

        // One-click universal Unity starter setup: registers GameManifest + ingests starter Trainer in requires_confirmation state
        let quick = invoke_json(
            &webview,
            "quick_setup_unity_starter",
            serde_json::json!({
                "title": "Unity Mono Demo",
                "storeSource": "custom_path",
                "storeAppId": null,
                "storeBuildId": null,
                "installDir": unity_game_dir.to_string_lossy()
            }),
        )
        .expect("quick_setup_unity_starter");
        let quick_game_id = quick["game_id"].as_str().unwrap();
        let quick_trainer_id = quick["trainer_id"].as_str().unwrap();
        assert_eq!(quick["engine"], "unity_mono");
        let starter_trainers = invoke_json(
            &webview,
            "list_trainers_for_game",
            serde_json::json!({
                "gameId": quick_game_id,
                "detectedBuildId": null
            }),
        )
        .expect("list starter trainers");
        let starter = &starter_trainers.as_array().unwrap()[0];
        assert_eq!(starter["trainer_id"], quick_trainer_id);
        assert_eq!(
            starter["permission_state"]["status"],
            "requires_confirmation"
        );

        // Security check: launch_game must reject executables outside install_dir
        let outside_exe = root.join("outside.exe");
        fs::write(&outside_exe, b"MZ").unwrap();
        let bad_launch = invoke_json(
            &webview,
            "launch_game",
            serde_json::json!({
                "storeSource": "custom_path",
                "storeAppId": null,
                "installDir": custom_game_dir.to_string_lossy(),
                "executablePath": outside_exe.to_string_lossy()
            }),
        );
        assert!(bad_launch.is_err());

        // 8. Remove custom path & verify diagnostic audit logs captured workflow.
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

    #[test]
    fn tauri_ct_import_and_generic_nmod_discovery_ipc_flow() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nekod_tauri_ct_nmod_{}_{}",
            std::process::id(),
            nanos
        ));
        let data_dir = root.join("data");
        let ct_game_dir = root.join("CustomGames").join("CtSample");
        fs::create_dir_all(&ct_game_dir).unwrap();
        fs::write(ct_game_dir.join("CtSample.exe"), b"MZ").unwrap();

        let core = NekodAppService::open_in_dir(
            &data_dir,
            Arc::new(CompositeRuntimeRouter::with_default_providers()),
        )
        .unwrap();

        let app = mock_builder()
            .manage(AppState::with_data_dir(core, data_dir.clone()))
            .invoke_handler(desktop_invoke_handler())
            .build(tauri::generate_context!())
            .expect("build mock Tauri app");

        let win_cfg = app.config().app.windows[0].clone();
        let webview = tauri::WebviewWindowBuilder::from_config(&app, &win_cfg)
            .expect("valid window config")
            .build()
            .expect("build main webview window");

        // 1. Mixed convertible + unsafe Lua .CT import succeeds safely
        let mixed_ct_path = root.join("CtSample.CT");
        let mixed_ct_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="45">
  <CheatEntries>
    <CheatEntry>
      <ID>0</ID>
      <Description>"Player Gold"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>"CtSample.exe"+0012ABCD</Address>
      <Offsets>
        <Offset>10</Offset>
        <Offset>28</Offset>
      </Offsets>
    </CheatEntry>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Infinite Health"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>[ENABLE]
aobscanmodule(HEALTH_SUB,CtSample.exe,29 73 10 8B 43 10)
HEALTH_SUB:
  db 90 90 90
[DISABLE]
HEALTH_SUB:
  db 29 73 10
</AssemblerScript>
    </CheatEntry>
    <CheatEntry>
      <ID>2</ID>
      <Description>"Unsafe Lua Ad Shell"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>{$lua}
[ENABLE]
shellExecute("https://modengine.net")
[DISABLE]
</AssemblerScript>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;
        fs::write(&mixed_ct_path, mixed_ct_xml).unwrap();

        let ct_report = invoke_json(
            &webview,
            "import_ct_file",
            serde_json::json!({
                "ctPath": mixed_ct_path.to_string_lossy(),
                "gameId": "ct-sample",
                "gameName": "CT Sample",
                "defaultModule": "CtSample.exe",
                "installDir": ct_game_dir.to_string_lossy(),
                "storeSource": "custom_path",
                "storeAppId": null,
                "storeBuildId": null,
                "trustTier": "community_verified"
            }),
        )
        .expect("import_ct_file mixed");
        assert_eq!(ct_report["imported"], true);
        assert_eq!(ct_report["game_id"], "ct-sample");
        assert_eq!(ct_report["converted_features"].as_array().unwrap().len(), 2);
        assert_eq!(ct_report["skipped_entries"].as_array().unwrap().len(), 1);
        assert!(
            ct_report["skipped_entries"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("{$lua}")
        );

        let ct_trainers = invoke_json(
            &webview,
            "list_trainers_for_game",
            serde_json::json!({
                "gameId": "ct-sample",
                "detectedBuildId": null
            }),
        )
        .expect("list ct-sample trainers");
        assert_eq!(ct_trainers.as_array().unwrap().len(), 1);
        assert_eq!(
            ct_trainers[0]["permission_state"]["status"],
            "requires_confirmation"
        );

        // 2. Lua-only fake ModEngine .CT reports zero converted features with clear reasons
        let lua_ct_path = root.join("LuaOnlySample.CT");
        let lua_ct_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="45">
  <CheatEntries>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Download ModEngine"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>{$lua}
[ENABLE]
shellExecute("https://modengine.net")
[DISABLE]
</AssemblerScript>
    </CheatEntry>
    <CheatEntry>
      <ID>2</ID>
      <Description>"Get more trainers"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>{$lua}
[ENABLE]
shellExecute("https://modengine.net")
[DISABLE]
</AssemblerScript>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;
        fs::write(&lua_ct_path, lua_ct_xml).unwrap();

        let rejected_report = invoke_json(
            &webview,
            "import_ct_file",
            serde_json::json!({
                "ctPath": lua_ct_path.to_string_lossy(),
                "gameId": null,
                "gameName": "Sample Unity Game",
                "defaultModule": "SampleUnity.exe",
                "installDir": null,
                "storeSource": "custom_path",
                "storeAppId": null,
                "storeBuildId": null,
                "trustTier": "unverified"
            }),
        )
        .expect("import_ct_file lua-only returns structured diagnostics");
        assert_eq!(rejected_report["imported"], false);
        assert!(
            rejected_report["converted_features"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            rejected_report["skipped_entries"].as_array().unwrap().len(),
            2
        );
        assert!(
            rejected_report["skipped_entries"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("{$lua}")
        );

        // 3. Generic .nmod discovery locates matching unpacked .nmod via metadata (Steam AppID), then ingests it
        let discovered = invoke_json(
            &webview,
            "discover_local_nmods",
            serde_json::json!({
                "gameId": null,
                "steamAppId": "1948700",
                "installDir": null,
                "executablePath": null
            }),
        )
        .expect("discover_local_nmods");
        let cand_list = discovered.as_array().unwrap();
        assert_eq!(cand_list.len(), 1);
        let cand = &cand_list[0];
        assert_eq!(cand["mod_id"], "org.nekod.gun-soul-girl-2");
        assert_eq!(cand["feature_count"], 21);
        assert_eq!(cand["source_format"], "nmod_dir");
        assert_eq!(cand["already_installed"], false);

        invoke_json(
            &webview,
            "ingest_local_trainer",
            serde_json::json!({
                "packageDir": cand["source_path"],
                "trustTier": "community_verified"
            }),
        )
        .expect("ingest discovered .nmod directory");

        let gsg2_trainers = invoke_json(
            &webview,
            "list_trainers_for_game",
            serde_json::json!({
                "gameId": "gun-soul-girl-2",
                "detectedBuildId": "13294226"
            }),
        )
        .expect("list gsg2 trainers");
        let gsg2_trainer = &gsg2_trainers.as_array().unwrap()[0];
        assert_eq!(gsg2_trainer["trainer_id"], "org.nekod.gun-soul-girl-2");
        assert_eq!(gsg2_trainer["features"].as_array().unwrap().len(), 21);
        assert_eq!(
            gsg2_trainer["permission_state"]["status"],
            "requires_confirmation"
        );

        // Re-discover shows already_installed == true
        let rediscovered = invoke_json(
            &webview,
            "discover_local_nmods",
            serde_json::json!({
                "gameId": "gun-soul-girl-2",
                "steamAppId": "1948700",
                "installDir": null,
                "executablePath": null
            }),
        )
        .expect("rediscover local nmods");
        assert_eq!(rediscovered[0]["already_installed"], true);

        // 4. Single-file .nmod archive discovery + safe ingestion + unsafe .exe/.nmod rejection
        let nmod_dir = data_dir.join("nmods");
        fs::create_dir_all(&nmod_dir).unwrap();
        let archive_path = nmod_dir.join("archive-demo.nmod");
        let archive_mod_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("examples")
            .join("unity-standalone-starter");
        let bridge_bytes = fs::read(archive_mod_dir.join("scripts/unity_bridge.json")).unwrap();
        let mut mod_val: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(archive_mod_dir.join("mod-manifest.json")).unwrap(),
        )
        .unwrap();
        mod_val["id"] = serde_json::json!("dev.example.archive-nmod");
        mod_val["target_game"]["game_id"] = serde_json::json!("archive-demo-game");
        mod_val["target_game"]["game_name"] = serde_json::json!("Archive Demo Game");
        mod_val["artifacts"][0]["sha256"] =
            serde_json::json!(nekod_trainer_manager::sha256_hex(&bridge_bytes));
        mod_val["artifacts"][0]["size_bytes"] = serde_json::json!(bridge_bytes.len() as u64);

        {
            let f = fs::File::create(&archive_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            zw.start_file("mod-manifest.json", opts).unwrap();
            zw.write_all(serde_json::to_string_pretty(&mod_val).unwrap().as_bytes())
                .unwrap();
            zw.start_file("scripts/unity_bridge.json", opts).unwrap();
            zw.write_all(&bridge_bytes).unwrap();
            zw.finish().unwrap();
        }

        let archive_candidates = invoke_json(
            &webview,
            "discover_local_nmods",
            serde_json::json!({
                "gameId": "archive-demo-game",
                "steamAppId": null,
                "installDir": null,
                "executablePath": null
            }),
        )
        .expect("discover .nmod archive");
        assert_eq!(archive_candidates.as_array().unwrap().len(), 1);
        assert_eq!(archive_candidates[0]["source_format"], "nmod_archive");

        invoke_json(
            &webview,
            "ingest_local_trainer",
            serde_json::json!({
                "packageDir": archive_path.to_string_lossy(),
                "trustTier": "community_verified"
            }),
        )
        .expect("ingest single-file .nmod archive");

        let archive_trainers = invoke_json(
            &webview,
            "list_trainers_for_game",
            serde_json::json!({
                "gameId": "archive-demo-game",
                "detectedBuildId": null
            }),
        )
        .expect("list archive-demo-game trainers");
        assert_eq!(
            archive_trainers[0]["trainer_id"],
            "dev.example.archive-nmod"
        );

        // Reject unsafe .nmod containing raw .exe payload
        let unsafe_archive_path = root.join("unsafe.nmod");
        {
            let f = fs::File::create(&unsafe_archive_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            zw.start_file("payload.exe", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(b"MZ").unwrap();
            zw.finish().unwrap();
        }
        let bad_nmod = invoke_json(
            &webview,
            "ingest_local_trainer",
            serde_json::json!({
                "packageDir": unsafe_archive_path.to_string_lossy(),
                "trustTier": "unverified"
            }),
        );
        assert!(bad_nmod.is_err());

        // 5. Generic quick_setup_unity_starter remains 100% generic (3 features even if dir looks like any Mono game)
        let gsg2_dir = root.join("MonoSampleGame");
        fs::create_dir_all(gsg2_dir.join("MonoSampleGame_Data").join("Managed")).unwrap();
        fs::create_dir_all(gsg2_dir.join("MonoBleedingEdge")).unwrap();
        fs::write(gsg2_dir.join("UnityPlayer.dll"), b"MZ").unwrap();
        fs::write(gsg2_dir.join("GunSoulGirl 2.exe"), b"MZ").unwrap();

        let generic_setup = invoke_json(
            &webview,
            "quick_setup_unity_starter",
            serde_json::json!({
                "title": "GunSoul Girl 2",
                "storeSource": "steam",
                "storeAppId": "1948700",
                "storeBuildId": "13294226",
                "installDir": gsg2_dir.to_string_lossy()
            }),
        )
        .expect("quick_setup_unity_starter stays generic");
        assert_ne!(
            generic_setup["trainer_id"], "org.nekod.gun-soul-girl-2",
            "quick_setup_unity_starter must stay generic and never hardcode game-specific packs"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
