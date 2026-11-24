use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

fn send_and_recv(stdin: &mut ChildStdin, reader: &mut BufReader<ChildStdout>, msg: Value) -> Value {
    let line = serde_json::to_string(&msg).unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();

    let mut resp_line = String::new();
    let n = reader.read_line(&mut resp_line).unwrap();
    assert!(n > 0, "expected JSON-RPC response from nekod-mcp stdout");
    serde_json::from_str(&resp_line).unwrap()
}

#[test]
fn nekod_mcp_binary_communicates_over_real_stdio_json_rpc() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("nekod_mcp_stdio_{}_{}", std::process::id(), nanos));
    let data_dir = root.join("data");
    let out_dir = root.join("generated-trainer");
    fs::create_dir_all(&data_dir).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_nekod-mcp"))
        .arg("--data-dir")
        .arg(&data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nekod-mcp binary");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);

    // 1. Initialize handshake
    let init = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "nekod-mcp");

    // 2. Notification produces no response; tools/list comes immediately next on id 2.
    writeln!(
        stdin,
        "{}",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    )
    .unwrap();
    stdin.flush().unwrap();

    let listed = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 24);

    // 3. Unauthorized write fails closed with isError=true inside MCP CallToolResult.
    let unauth = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "generate_trainer_template",
                "arguments": {
                    "output_dir": out_dir.to_string_lossy(),
                    "trainer_id": "dev.example.stdio-test",
                    "name": "Stdio Test Trainer",
                    "game_id": "sample-singleplayer-rpg",
                    "game_name": "Sample Singleplayer RPG",
                    "source_repository": "https://github.com/example/stdio-test"
                }
            }
        }),
    );
    assert_eq!(unauth["result"]["isError"], true);
    assert!(!out_dir.join("trainer-manifest.json").exists());

    // 4. Anti-cheat bypass parameters are refused even for read-only tools.
    let ac_refused = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "validate_trainer_manifest",
                "arguments": {
                    "bypass_anti_cheat": true,
                    "manifest_json": "{}"
                }
            }
        }),
    );
    assert_eq!(ac_refused["result"]["isError"], true);

    // 5. Authorized template generation + real disk validation succeed end-to-end.
    let generated = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "generate_trainer_template",
                "arguments": {
                    "authorize_write": true,
                    "output_dir": out_dir.to_string_lossy(),
                    "trainer_id": "dev.example.stdio-test",
                    "name": "Stdio Test Trainer",
                    "game_id": "sample-singleplayer-rpg",
                    "game_name": "Sample Singleplayer RPG",
                    "source_repository": "https://github.com/example/stdio-test",
                    "module_name": "SampleRPG.exe"
                }
            }
        }),
    );
    assert_eq!(generated["result"]["isError"], false);
    assert!(out_dir.join("trainer-manifest.json").is_file());

    let validated = send_and_recv(
        &mut stdin,
        &mut reader,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "validate_trainer_manifest",
                "arguments": {
                    "package_dir": out_dir.to_string_lossy()
                }
            }
        }),
    );
    assert_eq!(validated["result"]["isError"], false);
    assert_eq!(validated["result"]["structuredContent"]["valid"], true);

    // Close stdin cleanly and verify exit code 0.
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "nekod-mcp exited abnormally: {status}");

    let _ = fs::remove_dir_all(&root);
}
