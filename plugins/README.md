# Nekod Companion Bridge Plugins

Per **ADR-021**, Nekod distributes lightweight, source-auditable companion bridge adapters separately from the core player desktop application:

| Engine / Ecosystem | Adapter (`RuntimeKind`) | Companion Bridge | Transport |
| :--- | :--- | :--- | :--- |
| **Unity (Mono & IL2CPP)** | `unity_bridge_v1` (`unity_explorer_mcp_http_v1`) | [`OmaruKoma/UnityExplorerMCP`](https://github.com/OmaruKoma/UnityExplorerMCP) (`MCPBridge.dll` + `mcs.dll` under `BepInEx/plugins/`) | Loopback HTTP `POST /` (`127.0.0.1:<port>`) + `%LOCALAPPDATA%\UnityExplorerMCP\bridges\bridge-<pid>.json` discovery |
| **Unreal Engine 4/5 (`UE4SS`)** | `ue4ss_bridge_v1` | [Scripts/main.lua](ue4ss-nekod-bridge/Scripts/main.lua) | Loopback JSON-line IPC (`127.0.0.1:19541`, token-authenticated) |
| **Capcom RE Engine (`REFramework`)** | `managed_framework_v1` (`reframework`) | [nekod_bridge.lua](reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua) | Managed `reframework/autorun/` Lua + optional loopback IPC |
| **Godot Engine 3/4** | `adapter_ipc_v1` / `managed_framework_v1` | [nekod_bridge.gd](godot-nekod-bridge/nekod_bridge.gd) | Loopback TCP JSON-line server (`127.0.0.1:19543`) with LIFO property rollback |

All companion scripts enforce **loopback-only binding (`127.0.0.1`)**, avoid remote shell execution, and participate in LIFO rollback when a Nekod session stops.
