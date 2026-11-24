//! L6 optional developer-only MCP server for trainer authoring, validation, and diagnostics.
//!
//! Players never need this binary. It exists solely as an optional assistant
//! bridge for trainer developers using MCP-compatible AI coding clients.

use serde::{Deserialize, Serialize};

pub mod server;
pub mod unity_metadata;
pub use server::{McpError, NekodMcpServer};

/// Five scoped tool groups exposed by `nekod-mcp` in Phase P4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpToolGroup {
    GameDiscovery,
    TrainerDevelopment,
    Compatibility,
    Repository,
    RuntimeInspection,
}

/// Planned MCP tool descriptor separating read-only inspection from authorized mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub group: McpToolGroup,
    pub read_only: bool,
    pub requires_explicit_authorization: bool,
}

/// Canonical catalog of planned P4 MCP tools.
pub const PLANNED_MCP_TOOLS: &[McpToolSpec] = &[
    // Game Discovery
    McpToolSpec {
        name: "list_installed_games",
        group: McpToolGroup::GameDiscovery,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "get_game_info",
        group: McpToolGroup::GameDiscovery,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "inspect_game_environment",
        group: McpToolGroup::GameDiscovery,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "recommend_runtime_adapter",
        group: McpToolGroup::GameDiscovery,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "inspect_unity_assemblies",
        group: McpToolGroup::GameDiscovery,
        read_only: true,
        requires_explicit_authorization: false,
    },
    // Trainer Development
    McpToolSpec {
        name: "create_trainer_manifest",
        group: McpToolGroup::TrainerDevelopment,
        read_only: false,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "validate_trainer_manifest",
        group: McpToolGroup::TrainerDevelopment,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "inspect_trainer",
        group: McpToolGroup::TrainerDevelopment,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "generate_trainer_template",
        group: McpToolGroup::TrainerDevelopment,
        read_only: false,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "forge_nmod_package",
        group: McpToolGroup::TrainerDevelopment,
        read_only: false,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "pack_nmod_archive",
        group: McpToolGroup::TrainerDevelopment,
        read_only: false,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "analyze_ct_file",
        group: McpToolGroup::TrainerDevelopment,
        read_only: true,
        requires_explicit_authorization: false,
    },
    // Compatibility
    McpToolSpec {
        name: "check_game_version",
        group: McpToolGroup::Compatibility,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "validate_compatibility",
        group: McpToolGroup::Compatibility,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "verify_forged_mod_package",
        group: McpToolGroup::Compatibility,
        read_only: false,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "inspect_runtime_diagnostics",
        group: McpToolGroup::Compatibility,
        read_only: true,
        requires_explicit_authorization: false,
    },
    // Repository
    McpToolSpec {
        name: "validate_repository",
        group: McpToolGroup::Repository,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "inspect_registry_entry",
        group: McpToolGroup::Repository,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "prepare_trainer_release",
        group: McpToolGroup::Repository,
        read_only: false,
        requires_explicit_authorization: true,
    },
    // Runtime Inspection
    McpToolSpec {
        name: "inspect_authorized_game_process",
        group: McpToolGroup::RuntimeInspection,
        read_only: true,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "probe_unity_bridge",
        group: McpToolGroup::RuntimeInspection,
        read_only: true,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "probe_ue4ss_bridge",
        group: McpToolGroup::RuntimeInspection,
        read_only: true,
        requires_explicit_authorization: true,
    },
    McpToolSpec {
        name: "inspect_trainer_state",
        group: McpToolGroup::RuntimeInspection,
        read_only: true,
        requires_explicit_authorization: false,
    },
    McpToolSpec {
        name: "get_execution_logs",
        group: McpToolGroup::RuntimeInspection,
        read_only: true,
        requires_explicit_authorization: false,
    },
];

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use nekod_core::NekodAppService;
    use nekod_trainer_runtime::DeclarativeRuntime;
    use serde_json::{Value, json};

    use super::*;

    fn test_server_with_data(
        suffix: &str,
        use_data_dir: bool,
    ) -> (NekodMcpServer, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("nekod_mcp_test_{suffix}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let data_dir = root.join("data");
        let service = NekodAppService::open_in_memory_with_transport(
            Arc::new(DeclarativeRuntime::new()),
            None,
            Arc::new(nekod_registry_client::transport::mock::MockHttpTransport::new()),
            root.join("packages"),
        )
        .unwrap();
        let server = if use_data_dir {
            NekodMcpServer::with_data_dir(Arc::new(service), data_dir)
        } else {
            NekodMcpServer::new(Arc::new(service))
        };
        (server, root)
    }

    fn test_server(suffix: &str) -> (NekodMcpServer, std::path::PathBuf) {
        test_server_with_data(suffix, false)
    }

    #[test]
    fn json_rpc_initialize_and_tools_list_expose_all_16_tools() {
        let (server, root) = test_server("init");

        let init_reply = server
            .handle_json_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            )
            .unwrap();
        let init_val: Value = serde_json::from_str(&init_reply).unwrap();
        assert_eq!(init_val["result"]["serverInfo"]["name"], "nekod-mcp");

        // Notifications return no reply.
        assert!(
            server
                .handle_json_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );

        let list_reply = server
            .handle_json_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .unwrap();
        let list_val: Value = serde_json::from_str(&list_reply).unwrap();
        let tools = list_val["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), PLANNED_MCP_TOOLS.len());
        assert_eq!(tools.len(), 24);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn enforces_explicit_authorization_and_refuses_prohibited_operations() {
        let (server, root) = test_server("authz");
        let out_dir = root.join("sample-trainer");

        // Mutating tool rejected without authorize_write: true
        let err = server
            .dispatch_tool(
                "generate_trainer_template",
                &json!({
                    "output_dir": out_dir.to_string_lossy(),
                    "trainer_id": "dev.example.mcp-test",
                    "name": "MCP Test Trainer",
                    "game_id": "sample-singleplayer-rpg",
                    "game_name": "Sample Singleplayer RPG",
                    "source_repository": "https://github.com/example/mcp-test"
                }),
            )
            .unwrap_err();
        assert!(matches!(err, McpError::AuthorizationRequired(_)));

        // Prohibited anti-cheat bypass / multiplayer flags rejected
        let policy_err = server
            .dispatch_tool(
                "validate_trainer_manifest",
                &json!({
                    "bypass_anti_cheat": true,
                    "manifest_json": "{}"
                }),
            )
            .unwrap_err();
        assert!(matches!(policy_err, McpError::PolicyRefusal(_)));

        // Process inspection rejected without authorize_process_inspection: true
        let proc_err = server
            .dispatch_tool(
                "inspect_authorized_game_process",
                &json!({ "game_id": "sample-singleplayer-rpg" }),
            )
            .unwrap_err();
        assert!(matches!(proc_err, McpError::AuthorizationRequired(_)));

        // Prohibited competitive online game process inspection rejected even when authorized
        let comp_game_json = include_str!("../../../schemas/examples/game-manifest.example.json")
            .replace(
                "\"trainer_policy\": \"allowed_singleplayer\"",
                "\"trainer_policy\": \"prohibited_online_competitive\"",
            );
        assert!(comp_game_json.contains("\"prohibited_online_competitive\""));
        server
            .core()
            .register_game_manifest(&comp_game_json)
            .unwrap();
        let comp_err = server
            .dispatch_tool(
                "inspect_authorized_game_process",
                &json!({
                    "game_id": "sample-singleplayer-rpg",
                    "authorize_process_inspection": true
                }),
            )
            .unwrap_err();
        assert!(matches!(comp_err, McpError::PolicyRefusal(_)));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn developer_workflow_scaffolds_validates_and_prepares_release() {
        let (server, root) = test_server("workflow");
        let pkg_dir = root.join("authored-trainer");

        let game_json = include_str!("../../../schemas/examples/game-manifest.example.json");
        server.core().register_game_manifest(game_json).unwrap();

        let scaffolded = server
            .dispatch_tool(
                "generate_trainer_template",
                &json!({
                    "authorize_write": true,
                    "output_dir": pkg_dir.to_string_lossy(),
                    "trainer_id": "dev.example.workflow-trainer",
                    "name": "Workflow Demo Trainer",
                    "game_id": "sample-singleplayer-rpg",
                    "game_name": "Sample Singleplayer RPG",
                    "source_repository": "https://github.com/example/workflow-trainer",
                    "module_name": "SampleRPG.exe"
                }),
            )
            .unwrap();
        assert!(scaffolded["manifest_sha256"].as_str().is_some());

        let validated = server
            .dispatch_tool(
                "validate_trainer_manifest",
                &json!({ "package_dir": pkg_dir.to_string_lossy() }),
            )
            .unwrap();
        assert_eq!(validated["valid"], true);

        let manifest_raw = fs::read_to_string(pkg_dir.join("trainer-manifest.json")).unwrap();
        let compat = server
            .dispatch_tool(
                "validate_compatibility",
                &json!({ "trainer_manifest_json": manifest_raw }),
            )
            .unwrap();
        assert_eq!(compat["compatible"], true);

        let release = server
            .dispatch_tool(
                "prepare_trainer_release",
                &json!({
                    "authorize_write": true,
                    "package_dir": pkg_dir.to_string_lossy(),
                    "manifest_url": "https://github.com/example/workflow-trainer/releases/download/v1.0.0/trainer-manifest.json",
                    "tag": "v1.0.0"
                }),
            )
            .unwrap();
        assert_eq!(release["validated"], true);
        assert!(pkg_dir.join("release-entry-summary.json").is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ai_mod_forge_recommends_forges_nmod_and_enforces_verification_gate() {
        let (server, root) = test_server("ai_forge");
        let unity_game_dir = root.join("UnityGame");
        fs::create_dir_all(unity_game_dir.join("BepInEx").join("core")).unwrap();
        fs::write(unity_game_dir.join("UnityPlayer.dll"), b"MZ").unwrap();
        fs::write(unity_game_dir.join("GameAssembly.dll"), b"MZ").unwrap();
        fs::write(
            unity_game_dir
                .join("BepInEx")
                .join("core")
                .join("BepInEx.Unity.IL2CPP.dll"),
            b"MZ",
        )
        .unwrap();

        let rec = server
            .dispatch_tool(
                "recommend_runtime_adapter",
                &json!({
                    "game_id": "sample-unity-game",
                    "install_dir": unity_game_dir.to_string_lossy()
                }),
            )
            .unwrap();
        assert_eq!(rec["engine"], "unity_il2cpp");
        assert_eq!(rec["primary_adapter"], "unity_bridge_v1");
        assert_eq!(
            rec["plugin_bucket_strategy"],
            "on_demand_sha256_bucket_plugin"
        );

        let nmod_dir = root.join("forged-nmod");
        let forged = server
            .dispatch_tool(
                "forge_nmod_package",
                &json!({
                    "authorize_write": true,
                    "output_dir": nmod_dir.to_string_lossy(),
                    "mod_id": "dev.example.unity-stamina-forge",
                    "name": "Forged Infinite Stamina",
                    "game_id": "sample-unity-game",
                    "game_name": "Sample Unity Game",
                    "source_repository": "https://github.com/example/unity-stamina",
                    "runtime_kind": "unity_bridge_v1",
                    "mod_kind": "script_mod",
                    "module_name": "GameAssembly.dll"
                }),
            )
            .unwrap();
        assert_eq!(forged["runtime_kind"], "unity_bridge_v1");
        assert_eq!(forged["declared_status"], "experimental");
        assert!(nmod_dir.join("mod-manifest.json").is_file());

        // Without runtime_rollback_verified, status stays Experimental (AiDraftUnverified).
        let draft_verify = server
            .dispatch_tool(
                "verify_forged_mod_package",
                &json!({
                    "authorize_write": true,
                    "package_dir": nmod_dir.to_string_lossy(),
                    "game_version": "1.2.0",
                    "store_platform": "steam",
                    "runtime_rollback_verified": false
                }),
            )
            .unwrap();
        assert_eq!(draft_verify["valid"], true);
        assert_eq!(
            draft_verify["recorded_compatibility"]["status"],
            "experimental"
        );
        assert_eq!(
            draft_verify["recorded_compatibility"]["evidence"],
            "ai_draft_unverified"
        );

        // With runtime_rollback_verified: true, compatibility is recorded as Verified / AutomatedRuntimeTest.
        let runtime_verify = server
            .dispatch_tool(
                "verify_forged_mod_package",
                &json!({
                    "authorize_write": true,
                    "package_dir": nmod_dir.to_string_lossy(),
                    "game_version": "1.2.0",
                    "store_platform": "steam",
                    "runtime_rollback_verified": true
                }),
            )
            .unwrap();
        assert_eq!(
            runtime_verify["recorded_compatibility"]["status"],
            "verified"
        );
        assert_eq!(
            runtime_verify["recorded_compatibility"]["evidence"],
            "automated_runtime_test"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn forge_nmod_with_custom_features_packs_valid_pure_nmod_archive() {
        let (server, root) = test_server_with_data("forge_pack_nmod", true);
        let nmod_dir = root.join("forged-mono-mod");

        let mut gm: Value = serde_json::from_str(include_str!(
            "../../../schemas/examples/game-manifest.example.json"
        ))
        .unwrap();
        gm["id"] = json!("mono-demo-game");
        gm["name"] = json!("Mono Demo Game");
        gm["store_ids"]["steam_app_id"] = json!(123456);

        let custom_payload = json!({
            "schema_version": "1.0.0",
            "protocol": "nekod_json_line_v1",
            "flavor": "mono",
            "loader": "bepinex_mono",
            "host": "127.0.0.1",
            "port": 19540,
            "auto_discover_by_pid": true,
            "features": [
                {
                    "kind": "toggle_field",
                    "feature_id": "player-invincible",
                    "class_name": "Demo.PlayerController",
                    "field_name": "isImmortal",
                    "enabled_value": true,
                    "disabled_value": false
                },
                {
                    "kind": "invoke_method",
                    "feature_id": "reset-health",
                    "class_name": "Demo.PlayerController",
                    "method_name": "ResetHealth"
                }
            ]
        });

        let custom_features = json!([
            {
                "id": "player-invincible",
                "name": "Player Invincible",
                "description": "Toggle Demo.PlayerController.isImmortal.",
                "category": "player",
                "control_type": "toggle",
                "default_hotkey": "F1",
                "mutates_memory": true,
                "supports_rollback": true,
                "risk_level": "low"
            },
            {
                "id": "reset-health",
                "name": "Reset Health",
                "description": "Invoke Demo.PlayerController.ResetHealth().",
                "category": "player",
                "control_type": "action",
                "default_hotkey": "F2",
                "mutates_memory": true,
                "supports_rollback": false,
                "risk_level": "low"
            }
        ]);

        let forged = server
            .dispatch_tool(
                "forge_nmod_package",
                &json!({
                    "authorize_write": true,
                    "output_dir": nmod_dir.to_string_lossy(),
                    "mod_id": "dev.example.mono-demo",
                    "name": "Mono Demo Mod",
                    "version": "1.2.0",
                    "game_id": "mono-demo-game",
                    "game_name": "Mono Demo Game",
                    "steam_app_id": 123456,
                    "source_repository": "https://github.com/example/mono-demo-nmod",
                    "runtime_kind": "unity_bridge_v1",
                    "mod_kind": "trainer",
                    "flavor": "mono",
                    "features": custom_features,
                    "entrypoint_payload": custom_payload,
                    "game_manifest": gm
                }),
            )
            .unwrap();

        assert_eq!(forged["wrote_legacy_v1_manifest"], false);
        assert_eq!(forged["wrote_game_manifest"], true);
        assert_eq!(forged["feature_count"], 2);
        assert!(nmod_dir.join("mod-manifest.json").is_file());
        assert!(nmod_dir.join("game-manifest.json").is_file());
        assert!(!nmod_dir.join("trainer-manifest.json").exists());

        let validated = server
            .dispatch_tool(
                "validate_trainer_manifest",
                &json!({ "package_dir": nmod_dir.to_string_lossy() }),
            )
            .unwrap();
        assert_eq!(validated["valid"], true);
        assert_eq!(validated["manifest_format"], "mod_manifest_v2");

        // Mutate the entrypoint payload slightly and verify pack_nmod_archive recomputes checksums.
        let entrypoint_rel = forged["entrypoint_file"].as_str().unwrap();
        let entry_path = nmod_dir.join(entrypoint_rel);
        let mut edited_payload: Value =
            serde_json::from_str(&fs::read_to_string(&entry_path).unwrap()).unwrap();
        edited_payload["port"] = json!(19549);
        fs::write(
            &entry_path,
            serde_json::to_string_pretty(&edited_payload).unwrap(),
        )
        .unwrap();

        let packed = server
            .dispatch_tool(
                "pack_nmod_archive",
                &json!({
                    "authorize_write": true,
                    "package_dir": nmod_dir.to_string_lossy(),
                    "recompute_checksums": true,
                    "install_to_local_nmods": true
                }),
            )
            .unwrap();
        assert_eq!(packed["packed"], true);
        assert_eq!(packed["feature_count"], 2);

        let archive_path = std::path::PathBuf::from(packed["archive_path"].as_str().unwrap());
        assert!(archive_path.is_file());
        assert!(archive_path.starts_with(root.join("data").join("nmods")));

        let file = fs::File::open(&archive_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().unwrap().into_owned());
        }
        assert!(names.contains(&"mod-manifest.json".to_owned()));
        assert!(names.contains(&"game-manifest.json".to_owned()));
        assert!(names.contains(&entrypoint_rel.to_owned()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn analyzes_ct_files_and_inspects_synthetic_unity_dll_and_refuses_anticheat() {
        let (server, root) = test_server("ct_unity_inspect");

        let ct_path = root.join("sample.CT");
        let sample_ct = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="45">
  <CheatEntries>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Demo Byte Patch"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>[ENABLE]
aobscanmodule(MOD_AOB,DemoGame.exe,48 89 5C 24 08)
MOD_AOB:
  db 90 90 90 90 90
[DISABLE]
MOD_AOB:
  db 48 89 5C 24 08
</AssemblerScript>
    </CheatEntry>
    <CheatEntry>
      <ID>2</ID>
      <Description>"Lua Helper"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>{$lua}
if syntaxcheck then return end
[ENABLE]
print("hi")
[DISABLE]
</AssemblerScript>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;
        fs::write(&ct_path, sample_ct).unwrap();

        let ct_res = server
            .dispatch_tool(
                "analyze_ct_file",
                &json!({
                    "ct_path": ct_path.to_string_lossy(),
                    "game_id": "demo-game",
                    "game_name": "Demo Game",
                    "default_module": "DemoGame.exe"
                }),
            )
            .unwrap();
        assert!(ct_res["convertible_count"].as_u64().unwrap() >= 1);
        assert!(ct_res["skipped_count"].as_u64().unwrap() >= 1);
        assert!(
            ct_res["recommendation"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );

        let unity_dir = root.join("DemoUnityGame");
        let managed = unity_dir.join("Demo_Data").join("Managed");
        fs::create_dir_all(&managed).unwrap();
        fs::write(
            managed.join("Assembly-CSharp.dll"),
            crate::unity_metadata::build_synthetic_unity_dll_for_test(),
        )
        .unwrap();

        let filtered = server
            .dispatch_tool(
                "inspect_unity_assemblies",
                &json!({
                    "install_dir": unity_dir.to_string_lossy(),
                    "filter": "Player"
                }),
            )
            .unwrap();
        let types = filtered["types"].as_array().unwrap();
        assert!(
            types
                .iter()
                .any(|t| t["full_name"] == "Demo.PlayerController")
        );

        let detail = server
            .dispatch_tool(
                "inspect_unity_assemblies",
                &json!({
                    "install_dir": unity_dir.to_string_lossy(),
                    "type_name": "Demo.PlayerController"
                }),
            )
            .unwrap();
        assert_eq!(
            detail["inheritance_chain"],
            json!([
                "Demo.PlayerController",
                "Demo.BaseHealth",
                "UnityEngine.MonoBehaviour"
            ])
        );
        assert!(
            detail["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "isImmortal" && f["bridge_accessible"] == true)
        );
        assert!(
            detail["properties"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "AllAmmoInfinity")
        );
        assert!(
            detail["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == "ResetHealth" && m["invoke_method_compatible"] == true)
        );

        // Refuses inspection when anti-cheat markers exist in install_dir.
        fs::create_dir_all(unity_dir.join("EasyAntiCheat")).unwrap();
        let ac_err = server
            .dispatch_tool(
                "inspect_unity_assemblies",
                &json!({
                    "install_dir": unity_dir.to_string_lossy()
                }),
            )
            .unwrap_err();
        assert!(matches!(ac_err, McpError::PolicyRefusal(_)));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_unity_bridge_enforces_authz_and_readonly_allowlist() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::thread;

        let (server, root) = test_server("probe_bridge");
        let discovery_dir = root.join("bridge_discovery");
        fs::create_dir_all(&discovery_dir).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let pid = 42424u32;
        let token = "0123456789abcdef0123456789abcdef";

        fs::write(
            discovery_dir.join(format!("unity-{pid}.json")),
            json!({
                "pid": pid,
                "port": port,
                "flavor": "mono",
                "loader": "bepinex_mono",
                "auth_token": token,
                "unity_version": "2022.3.51f1",
                "bridge_version": "0.2.0"
            })
            .to_string(),
        )
        .unwrap();

        let bg = thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["token"], token);
                match req["op"].as_str().unwrap() {
                    "ping" => {
                        writeln!(
                            stream,
                            "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":{pid},\"port\":{port}}}}}"
                        )
                        .unwrap();
                    }
                    "get_field" => {
                        assert_eq!(req["params"]["class_name"], "Demo.PlayerController");
                        assert_eq!(req["params"]["field_name"], "isImmortal");
                        writeln!(stream, "{{\"ok\":true,\"data\":{{\"value\":false}}}}").unwrap();
                    }
                    "list_members" => {
                        assert_eq!(req["params"]["class_name"], "Demo.PlayerController");
                        writeln!(stream, "{{\"ok\":true,\"data\":{{\"fields\":[]}}}}").unwrap();
                    }
                    "find_instances" => {
                        assert_eq!(req["params"]["class_name"], "Demo.PlayerController");
                        writeln!(
                            stream,
                            "{{\"ok\":true,\"data\":{{\"total\":1,\"instances\":[]}}}}"
                        )
                        .unwrap();
                    }
                    "dump_hierarchy" => {
                        assert_eq!(req["params"]["class_name"], "Demo.PlayerController");
                        writeln!(
                            stream,
                            "{{\"ok\":true,\"data\":{{\"root\":\"Player\",\"nodes\":[]}}}}"
                        )
                        .unwrap();
                    }
                    other => panic!("unexpected op sent to read-only mock bridge: {other}"),
                }
            }
        });

        // 1. Missing authorize_process_inspection must fail with AuthorizationRequired.
        let unauth_err = server
            .dispatch_tool(
                "probe_unity_bridge",
                &json!({
                    "pid": pid,
                    "op": "ping",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap_err();
        assert!(matches!(unauth_err, McpError::AuthorizationRequired(_)));

        // 2. Mutating operations must fail closed with PolicyRefusal before touching network.
        let mutate_err = server
            .dispatch_tool(
                "probe_unity_bridge",
                &json!({
                    "authorize_process_inspection": true,
                    "pid": pid,
                    "op": "set_field",
                    "class_name": "Demo.PlayerController",
                    "field_name": "isImmortal",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap_err();
        assert!(matches!(mutate_err, McpError::PolicyRefusal(_)));

        // 3. Authorized read-only ping succeeds.
        let ping_res = server
            .dispatch_tool(
                "probe_unity_bridge",
                &json!({
                    "authorize_process_inspection": true,
                    "pid": pid,
                    "op": "ping",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap();
        assert_eq!(ping_res["read_only"], true);
        assert_eq!(ping_res["op"], "ping");
        assert_eq!(ping_res["data"]["flavor"], "mono");

        // 4. Authorized read-only get_field succeeds.
        let field_res = server
            .dispatch_tool(
                "probe_unity_bridge",
                &json!({
                    "authorize_process_inspection": true,
                    "pid": pid,
                    "op": "get_field",
                    "class_name": "Demo.PlayerController",
                    "field_name": "isImmortal",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap();
        assert_eq!(field_res["read_only"], true);
        assert_eq!(field_res["op"], "get_field");
        assert_eq!(field_res["data"]["value"], false);

        // 5. Authorized read-only list_members, find_instances, dump_hierarchy succeed.
        for op in ["list_members", "find_instances", "dump_hierarchy"] {
            let res = server
                .dispatch_tool(
                    "probe_unity_bridge",
                    &json!({
                        "authorize_process_inspection": true,
                        "pid": pid,
                        "op": op,
                        "class_name": "Demo.PlayerController",
                        "discovery_dir": discovery_dir.to_string_lossy()
                    }),
                )
                .unwrap();
            assert_eq!(res["read_only"], true);
            assert_eq!(res["op"], op);
        }

        bg.join().unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_ue4ss_bridge_enforces_authz_and_readonly_allowlist() {
        use std::thread;

        let (server, root) = test_server("probe_ue4ss");
        let discovery_dir = root.join("bridges");
        fs::create_dir_all(&discovery_dir).unwrap();
        let token = "ue4ss_mcp_token_9876543210abcdef";

        fs::write(
            discovery_dir.join("ue4ss-active.json"),
            json!({
                "bridge_version": "0.2.0",
                "transport": "file_ipc",
                "ipc_dir": discovery_dir.to_string_lossy(),
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        // 1. Missing authorize_process_inspection fails with AuthorizationRequired.
        let unauth_err = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({
                    "op": "ping",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap_err();
        assert!(matches!(unauth_err, McpError::AuthorizationRequired(_)));

        // 2. Prohibited competitive game fails with PolicyRefusal.
        let game_json = include_str!("../../../schemas/examples/game-manifest.example.json")
            .replace("allowed_singleplayer", "prohibited_online_competitive");
        server.core().register_game_manifest(&game_json).unwrap();
        let comp_err = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({
                    "authorize_process_inspection": true,
                    "game_id": "sample-singleplayer-rpg",
                    "op": "ping",
                    "discovery_dir": discovery_dir.to_string_lossy()
                }),
            )
            .unwrap_err();
        assert!(matches!(comp_err, McpError::PolicyRefusal(_)));

        // 3. Mutating operations must fail closed with PolicyRefusal before touching IPC.
        for forbidden in [
            "cheat_manager_exec",
            "set_uproperty",
            "call_ufunction",
            "apply_localization",
            "clear_localization",
        ] {
            let mutate_err = server
                .dispatch_tool(
                    "probe_ue4ss_bridge",
                    &json!({
                        "authorize_process_inspection": true,
                        "op": forbidden,
                        "discovery_dir": discovery_dir.to_string_lossy()
                    }),
                )
                .unwrap_err();
            assert!(matches!(mutate_err, McpError::PolicyRefusal(_)));
        }

        // 4. Spawn mock File-IPC responder covering all 6 read-only introspection ops.
        let ipc_clone = discovery_dir.clone();
        let bg = thread::spawn(move || {
            let mut handled = 0usize;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            while handled < 6 && std::time::Instant::now() < deadline {
                let req_path = ipc_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: Value = serde_json::from_str(&raw).unwrap();
                    assert_eq!(req["token"], token);
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap();
                    let data = match op {
                        "ping" => {
                            json!({ "bridge_version": "0.3.0", "engine_version": "4.27", "transport": "file_ipc" })
                        }
                        "find_objects" => {
                            json!({ "class_name": "PlayerController", "count": 1, "objects": [{ "index": 1, "full_name": "PlayerController /Game/Map.PersistentLevel.PC" }] })
                        }
                        "dump_ui_texts" => {
                            json!({
                                "total_scanned": 2,
                                "count": 1,
                                "texts": [{ "index": 1, "full_name": "TextBlock /Engine/Transient.Menu:Button", "text": "RESUME", "is_transient": true }],
                                "untranslated_count": 1,
                                "untranslated_sample": ["RESUME"]
                            })
                        }
                        "get_uproperty" => {
                            json!({ "object_class": "PlayerController", "property_name": "bShowMouseCursor", "value": false, "value_type": "boolean" })
                        }
                        "list_properties" => {
                            json!({ "object_class": "PlayerController", "count": 1, "properties": [{ "name": "bShowMouseCursor", "type": "BoolProperty", "owner": "PlayerController" }] })
                        }
                        "list_functions" => {
                            json!({ "object_class": "PlayerController", "count": 1, "functions": [{ "name": "Pause", "full_name": "Function /Script/Engine.PlayerController:Pause", "owner": "PlayerController" }] })
                        }
                        other => panic!("unexpected op sent to mock UE4SS bridge: {other}"),
                    };
                    fs::write(
                        ipc_clone.join(format!("ue4ss-response-{id}.json")),
                        json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let d = discovery_dir.to_string_lossy();
        let ping_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "ping", "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(ping_res["read_only"], true);
        assert_eq!(ping_res["data"]["engine_version"], "4.27");

        let objs_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "find_objects", "class_name": "PlayerController", "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(objs_res["data"]["count"], 1);

        let dump_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "dump_ui_texts", "live_only": true, "unique_only": true, "limit": 100, "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(dump_res["data"]["count"], 1);
        assert_eq!(dump_res["data"]["texts"][0]["text"], "RESUME");

        let prop_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "get_uproperty", "object_class": "PlayerController", "property_name": "bShowMouseCursor", "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(prop_res["data"]["value"], false);

        let props_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "list_properties", "object_class": "PlayerController", "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(props_res["data"]["count"], 1);

        let fns_res = server
            .dispatch_tool(
                "probe_ue4ss_bridge",
                &json!({ "authorize_process_inspection": true, "op": "list_functions", "object_class": "PlayerController", "discovery_dir": d }),
            )
            .unwrap();
        assert_eq!(fns_res["data"]["count"], 1);

        bg.join().unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
