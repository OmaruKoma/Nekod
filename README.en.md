# Nekod

[简体中文](README.md) | **English** | [日本語](README.ja.md)

> ✨ **This project was built entirely with the Gemini Neon model :p**

**Modding games, together.**  
*Open-source Community Mod Platform (`v1.0.0`) — Discover, create, manage, and share singleplayer game modifications.*

[![License: MIT](https://img.shields.io/badge/License-MIT-38B788.svg)](LICENSE)
[![Release](https://img.shields.io/badge/Release-v1.0.0-111514.svg)](.github/workflows/release.yml)

Nekod is an open-source singleplayer game modification and trainer platform for both everyday players and mod creators. Built with a restrained **Deep Emerald Obsidian** workbench UI and instant multi-language switching (**简体中文 / English / 日本語**), it unifies standalone trainers, Cheat Engine `.CT` tables, Unity (`BepInEx` Mono / IL2CPP) mods, and Unreal Engine (`RE-UE4SS`) mods under the open, verifiable `.nmod` package standard.

> **Two Distinct Experiences — Cleanly Separated:**
> - **For Players (Zero AI / Zero Code):** Launch the Nekod desktop app to auto-discover installed games, install `.nmod` packages from the online store in one click, stack multiple mods simultaneously, bind global hotkeys, and toggle the click-through RGB HUD overlay. **No AI account, API key, MCP server, or CLI required.**
> - **For Creators & "Wish-to-Mod via MCP" (Optional):** Connect the optional `nekod-mcp.exe` server to any MCP-compatible AI assistant, describe what mod features you want in plain natural language, and let AI inspect the game safely, forge a verifiable `.nmod` package, and pack it ready to drop into Nekod.

---

## Highlights

- **High-Density Deep Emerald Obsidian Workbench:** `#0B1B16` deep forest green sidebar + `#0B0E0D` obsidian canvas + crisp `1px` borders and `#38B788` mint emerald highlights; launches straight into the two-column workbench with live **简体中文 / English / 日本語** switching.
- **Built-in Official Mod Bucket:** Automatically subscribes to [`OmaruKoma/Nekod-Mods`](https://github.com/OmaruKoma/Nekod-Mods) (source-first, SHA-256 verified, auto-registering `game-manifest.json`) and supports any custom third-party `registry.json` feed.
- **Multi-Mod Stacking & Auditable Rollback:** Run multiple `.nmod` packages on the same game simultaneously (e.g., a gameplay/character mod alongside a UI localization pack) and roll them back cleanly in LIFO order via `PatchLedger`.
- **Zero-Friction Runtime Experience:** **Play & Inject**, Auto-Attach, click-to-record global hotkey capsules, system tray background mode, and top-right click-through **RGB ArrayList HUD Overlay**.
- **Managed Loaders & Built-in Companion Bridges (`DeploymentLedger`):**
  - **Unity Mono / IL2CPP (`v0.5.5`):** Zero UnityExplorer dependency; auto-stages BepInEx + `NekodUnityBridge` for persistent field pins, Harmony method hooks, quaternion SpinBot, DamageAura, UI GameObject toggling, and SHA-256-verified texture/material replacement.
  - **Unreal Engine 4/5 (`v0.7.3`):** Auto-stages `RE-UE4SS` + `NekodBridge` with single-threaded game-thread ticking, live UMG localization, sticky UProperty locks, smooth target lock, rapid fire, world-decoupled third-person SpinBot, and native `ApplyDamage` aura.
- **Fail-Closed `.CT` Static Importer:** Converts safe static module offsets, pointer chains, and `aobscanmodule + db` patches into SHA-256-verified declarative packages while strictly rejecting `{$lua}`, `alloc()`, and remote scripts.

---

## Quick Start (For Players)

### 1. Download & Install (Windows x64)

Download the latest release assets from the **Releases** page (verify checksums using `SHA256SUMS.txt`):

- **Installer (Recommended):** `nekod-windows-x64-setup.exe` (NSIS per-user installer)
- **Portable Archive:** `nekod-windows-x64-portable.zip` (includes `nekod-desktop.exe`, `nekod-validator.exe`, and optional `nekod-mcp.exe`)

```powershell
# Optional integrity check
Get-FileHash .\nekod-windows-x64-setup.exe -Algorithm SHA256
```

### 2. Four Steps to Mod a Singleplayer Game

1. **Discover Games:** Nekod automatically scans Steam, Epic, and GOG installs into the left sidebar. Click `+` to add any portable or standalone `.exe`.
2. **Get `.nmod` Packages:**
   - Open **Mod Store** -> click **Sync All Sources** -> install packs from official `OmaruKoma/Nekod-Mods`;
   - Or **drag & drop** any local `.nmod` / `.ct` file directly anywhere onto the Nekod window.
3. **Stage Bridge & Loader (If Needed):** If a Unity or Unreal game needs a companion bridge, click **Install BepInEx / RE-UE4SS** and **Deploy Bridge** via `DeploymentLedger` (cleanly reversible back to original files).
4. **Approve & Play:** Review the Default-Deny permission prompt, then click **Play & Inject** (or **Inject Mod** if the game is already running) and use switches or custom hotkeys.

---

## Make a Wish to AI via MCP (Zero-Code Mod Forge)

> **Anyone can create custom `.nmod` mods without writing code!**  
> If the Mod Store does not yet have a mod for your niche singleplayer game—or you want a bespoke trainer (Infinite HP, Gold Multiplier, Character Swap, or Live Translation)—connect **`nekod-mcp.exe`** to your MCP AI client and make a wish in plain language.

### Step 1: Locate `nekod-mcp.exe`

- **Portable ZIP users:** `nekod-mcp.exe` is right next to `nekod-desktop.exe`.
- **Source builders:** Run `cargo build --release -p nekod-mcp` -> `target\release\nekod-mcp.exe`.

### Step 2: Connect `nekod-mcp` to Your AI Client

Add this snippet to your client's MCP config file (use the actual absolute path to `nekod-mcp.exe` with escaped `\\` on Windows):

#### Claude Desktop / Antigravity / Qoder / Windsurf

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe",
      "args": [
        "--data-dir",
        "C:\\Users\\YOUR_USER\\AppData\\Roaming\\org.nekod.desktop"
      ]
    }
  }
}
```

#### Cursor (`.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe"
    }
  }
}
```

> **Tip:** Point `--data-dir` at Nekod's data folder so the MCP server sees your added games directly (or omit `--data-dir` to use the isolated sandbox temp workspace). After restarting the AI client, verify the **24 `nekod` tools** appear.

### Step 3: Prep the Game (1 Minute)

1. Select or add your offline singleplayer game in the **Nekod** desktop app.
2. For Unity / Unreal games, deploy the **Companion Bridge** in Nekod and start the game into actual gameplay.

### Step 4: Copy & Send a Wish Prompt

#### Wish 1 — Unity Trainer (God Mode / Currencies / Move Speed)
> "Use the `nekod` MCP tools to inspect my Unity singleplayer game at `D:\Games\MyUnityGame`:
> 1. Run `inspect_game_environment` and `inspect_unity_assemblies` (read-only) to find the player HP, currency/ammo, and movement speed classes & fields.
> 2. The game and Nekod bridge are running; verify exact live instances/members with `probe_unity_bridge` if needed.
> 3. Call `forge_nmod_package` to generate an `.nmod` mod with God Mode (`F5`), +99999 Currency (`F6`), and 2x Move Speed slider, validate it via `verify_forged_mod_package`, pack it with `pack_nmod_archive`, and give me the `.nmod` file path!"

#### Wish 2 — Unreal Engine 4/5 (`ue4ss_bridge_v1`) Mod / Localization
> "Use `nekod` MCP to inspect my running Unreal singleplayer game with the UE4SS bridge loaded:
> 1. Call `probe_ue4ss_bridge` (read-only) to inspect player controller, pawn, and inventory UProperty / UFunction names.
> 2. Use `forge_nmod_package`, `verify_forged_mod_package`, and `pack_nmod_archive` to build a clean `ue4ss_bridge_v1` `.nmod` package for Infinite Stamina, No Recoil, and Refill Inventory."

#### Wish 3 — Convert a Cheat Engine `.CT` File into a Safe `.nmod`
> "Inspect `D:\Downloads\Game.CT` using `analyze_ct_file` to identify which entries are safe static offsets, pointer chains, or AOB patches (and filter out any Lua/alloc scripts), then forge and pack the safe features into a clean `.nmod` package."

### Why MCP Wishing Stays Safe

| Tool Group | Read-Only Inspection | Controlled Authoring / Packing (Explicit Auth Required) |
| :--- | :--- | :--- |
| **Game & Engine Discovery** | `list_installed_games`, `get_game_info`, `inspect_game_environment`, `recommend_runtime_adapter`, `inspect_unity_assemblies` | — |
| **Live Bridge Probing** | `probe_unity_bridge` (`ping` / `list_types` / `list_members` / `find_instances` / `dump_hierarchy` / `get_field`), `probe_ue4ss_bridge`, `inspect_authorized_game_process`, `inspect_trainer_state`, `get_execution_logs` | — |
| **Mod Forging & `.CT`** | `analyze_ct_file`, `validate_trainer_manifest`, `inspect_trainer` | `forge_nmod_package`, `pack_nmod_archive`, `create_trainer_manifest`, `generate_trainer_template` |
| **Compatibility & Release** | `check_game_version`, `validate_compatibility`, `inspect_runtime_diagnostics`, `validate_repository`, `inspect_registry_entry` | `verify_forged_mod_package`, `prepare_trainer_release` |

AI cannot bypass anti-cheat, auto-elevate privileges, or silently grant runtime capabilities; every generated `.nmod` still passes strict JSON Schema validation, SHA-256 verification, and player consent inside Nekod Desktop.

---

## Six-Layer Architecture & Security Model

| Layer | Name | Location |
| :--- | :--- | :--- |
| **L1** | Desktop Application (Tauri 2 + React, zero AI/MCP) | [`apps/desktop/`](apps/desktop/README.md) |
| **L2** | Application Core (Games, Trainers, Profiles, Default-Deny Permissions, Registry) | [`crates/nekod-core/`](crates/nekod-core/Cargo.toml) |
| **L3** | Trainer Runtime (Win32/Linux backends, Unity/UE4SS bridges, `PatchLedger`, `DeploymentLedger`) | [`crates/trainer-runtime/`](crates/trainer-runtime/Cargo.toml) |
| **L4** | SDK & Protocol (`GameManifest`, `TrainerManifest`, `ModManifest` v2 / `nmod_v1`) | [`sdk/`](sdk/manifest/Cargo.toml), [`schemas/`](schemas/README.md) |
| **L5** | Community Registry (`OmaruKoma/Nekod-Mods` bucket spec & `nekod-validator`) | [`community/`](community/README.md), [`tools/trainer-validator/`](tools/trainer-validator/Cargo.toml) |
| **L6** | AI Developer Tooling (Optional `nekod-mcp`, Agent Skills, Prompts) | [`crates/nekod-mcp/`](crates/nekod-mcp/Cargo.toml), [`skills/`](skills/README.md) |

- **Default-Deny Permissions:** Grants are bound to `(trainer_id, version, manifest_sha256)`. Any file modification forces fresh user approval.
- **Singleplayer-Only Boundary:** Anti-cheat evasion, stealth injection, and online multiplayer cheating are strictly prohibited. Detected anti-cheat markers trigger fail-closed abort. See [`SECURITY.md`](SECURITY.md).

---

## Contributing & Building from Source

Real-game `.nmod` packages live in [**`OmaruKoma/Nekod-Mods`**](https://github.com/OmaruKoma/Nekod-Mods), while `examples/` keeps minimal reference starter packs (`sample-singleplayer-rpg`, `unity-standalone-starter`, `unity-explorer-mcp-starter`).

```bash
# 1. Build frontend
npm --prefix apps/desktop install
npm --prefix apps/desktop run build

# 2. Check formatting, clippy (0 warnings), and run full test suite
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. Validate starter packages
cargo run -p nekod-validator -- package examples/sample-singleplayer-rpg
cargo run -p nekod-validator -- nmod examples/unity-standalone-starter
cargo run -p nekod-validator -- nmod examples/unity-explorer-mcp-starter

# 4. Run Tauri dev app or build release binaries
cd apps/desktop && npm run tauri dev
cargo build --release -p nekod-desktop -p nekod-mcp -p nekod-validator
```

---

## License

Licensed under the [MIT License](LICENSE) (see [ADR-009](TECHNICAL_DECISIONS.md#adr-009-repository-license-mit)).
