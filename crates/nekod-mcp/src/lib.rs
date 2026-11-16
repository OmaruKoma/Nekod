//! L6 optional developer-only MCP server for trainer authoring, validation, and diagnostics.
//!
//! Players never need this binary. It exists solely as an optional assistant
//! bridge for trainer developers using MCP-compatible AI coding clients.

use serde::{Deserialize, Serialize};

pub mod server;
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

    fn test_server(suffix: &str) -> (NekodMcpServer, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("nekod_mcp_test_{suffix}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let service = NekodAppService::open_in_memory_with_transport(
            Arc::new(DeclarativeRuntime::new()),
            None,
            Arc::new(nekod_registry_client::transport::mock::MockHttpTransport::new()),
            root.join("packages"),
        )
        .unwrap();
        (NekodMcpServer::new(Arc::new(service)), root)
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
        assert_eq!(tools.len(), 16);

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
}
