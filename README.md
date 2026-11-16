# Nekod

> **Work in Progress (WIP):** Nekod is currently under active early development (`v0.1.0-alpha.0`). The core P0–P5 architecture, Tauri 2 desktop foundation, declarative runtime, `.CT` static importer, `DeploymentLedger`, and `adapter_ipc_v1` are implemented and verified; `.nmod` source-first packaging (`ADR-020`), `ModManifest` v2, multi-engine adapters (`Unity`/`UE4SS`/`Frida`), and AI Mod Forge (`V2-M1`–`V2-M5`) are actively in progress (see [`ROADMAP.md`](ROADMAP.md) & [`docs/runtime-engine-ecosystem.md`](docs/runtime-engine-ecosystem.md)).

**Modding games, together.**  
*Open-source Community Mod Platform — Discover, create, manage, and share game modifications.*

Nekod unifies fragmented PC game modification ecosystems—Trainers, Cheat Engine `.CT` tables, script hooks, Unity (`BepInEx` / `MelonLoader`), Unreal (`UE4SS`), game-specific bridges (`REFramework`, `ScriptHook`, `RetroArch UDP`), and AI-assisted creator workflows—under one open, verifiable, security-first platform.

> **Dual-Ecosystem Design:**  
> - **Player Ecosystem (Zero AI Required):** Ordinary players never need an AI model, API key, MCP server, or reverse-engineering knowledge to discover games, manage runtime dependencies, toggle features, or roll back cleanly.  
> - **Developer & AI Mod Forge (Optional):** Contributors and AI Agents can inspect game directories, recommend runtime adapters, generate/import Mod packages, run automated validation & rollback checks, and publish to community repositories.

---

## Core Principles

1. **Player First** — Automatic game & engine discovery, clean feature toggles, hotkeys, managed loader cleanup (`DeploymentLedger`), and safe rollback (`PatchLedger`).
2. **Unified Mod Management, Heterogeneous Runtimes** — Common lifecycle, capability checks, and compatibility metadata across distinct runtime adapters without forcing one execution model onto every game engine.
3. **Community Recipe Buckets (Homebrew / Scoop Model)** — Independent GitHub buckets host source-first `.nmod` packages (readable declarative configs, AOB/pointer rules, and lightweight scripts) with SHA-256-pinned upstream loader references instead of opaque closed-source `.exe` mirrors ([ADR-020](TECHNICAL_DECISIONS.md#adr-020-homebrewscoop-style-decentralized-script-buckets--nmod-source-first-package-format)).
4. **Open Standards** — Versioned JSON Schemas for games, mods/trainers, compatibility records, and registries.
5. **AI Mod Forge (Developer-Only & Optional)** — MCP tools, model-independent Agent Skills, and prompts help contributors analyze games and generate verifiable mods while requiring runtime validation and human review before publication.
6. **Security First** — Default-deny execution for untrusted scripts/modules, explicit permission & license-aware dependency governance, process identity/version checks, and strict anti-cheat/singleplayer boundaries.

---

## Six-Layer Architecture

| Layer | Name | Responsibility | Primary Location |
| :--- | :--- | :--- | :--- |
| **L1** | Desktop Application | Player-facing Tauri 2 + React + TypeScript UI | [`apps/desktop/`](apps/desktop/README.md) |
| **L2** | Application Core | Modular Rust monolith (games, trainers, profiles, permissions, registry, updates) | [`crates/nekod-core/`](crates/nekod-core/Cargo.toml) |
| **L3** | Trainer Runtime | Controlled process identification, memory ops, lifecycle, rollback, permission enforcement | [`crates/trainer-runtime/`](crates/trainer-runtime/Cargo.toml) |
| **L4** | SDK & Protocol | Stable versioned manifests, JSON Schemas, Runtime API, and Trainer SDK | [`sdk/`](sdk/manifest/Cargo.toml), [`schemas/`](schemas/README.md) |
| **L5** | Community Registry | Hybrid GitHub registry model & CI/local manifest validator | [`community/`](community/README.md), [`tools/trainer-validator/`](tools/trainer-validator/Cargo.toml) |
| **L6** | AI Developer Tooling | Optional MCP server, Agent Skills, and contributor prompt library | [`crates/nekod-mcp/`](crates/nekod-mcp/Cargo.toml), [`skills/`](skills/README.md), [`prompts/`](prompts/README.md) |

---

## Repository Status

Core platform milestones **P0 – P5** are implemented and verified (see [`ROADMAP.md`](ROADMAP.md), [`TECHNICAL_DECISIONS.md`](TECHNICAL_DECISIONS.md), and the upcoming **[V1.1 WeMod-Alternative Feature Plan](docs/v1.1-roadmap.md)**).

---

## Installation & Quick Start

### For Players (Windows x64)

1. Download `nekod-windows-x64.zip` (and `SHA256SUMS.txt` to verify integrity) from the repository's **Releases** page, or build locally from source below.
2. Verify the archive digest and extract to a local directory:
   ```powershell
   Get-FileHash .\nekod-windows-x64.zip -Algorithm SHA256
   ```
3. Launch `nekod-desktop.exe` (requires Windows 10/11 WebView2 Runtime, pre-installed on modern Windows).
4. **First-time setup (zero account or AI setup required):**
   - Click **Rescan Library** to auto-detect Steam, Epic, and GOG installs, or add portable folders in **Settings & Security**.
   - Load community feeds in **Community & Updates**, or import a verified local trainer folder (such as [`examples/sample-singleplayer-rpg/`](examples/sample-singleplayer-rpg/)) under **Local Packages**.
   - Review the trainer's declared permissions, confirm consent, attach to the running singleplayer game process, and toggle features or hotkeys.

### Building from Source (Players & Contributors)

**Prerequisites:**
- [Rust](https://rustup.rs/) >= 1.85 (`rustfmt` and `clippy` components)
- [Node.js](https://nodejs.org/) >= 20
- **Windows:** Visual Studio Build Tools (C++ workload) + WebView2 Runtime
- **Linux:** `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

```bash
# 1. Build Tauri desktop frontend (React + TypeScript)
cd apps/desktop && npm install && npm run build && cd ../..

# 2. Format, lint, and run all workspace tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. Build optimized release binaries (desktop app, validator CLI, optional MCP server)
cargo build --release -p nekod-desktop -p nekod-mcp -p nekod-validator

# 4. Run the desktop app directly in development mode
cd apps/desktop && npm run tauri dev && cd ../..

# 5. Validate a complete declarative trainer package directory
cargo run -p nekod-validator -- package examples/sample-singleplayer-rpg

# 6. Run optional developer MCP server over stdio (contributors only)
cargo run -p nekod-mcp
```

- **Architecture & Decisions:** [`ARCHITECTURE.md`](ARCHITECTURE.md), [`TECHNICAL_DECISIONS.md`](TECHNICAL_DECISIONS.md)
- **Roadmap & Engine Plans:** [`ROADMAP.md`](ROADMAP.md), [`docs/v1.1-roadmap.md`](docs/v1.1-roadmap.md), [`docs/runtime-engine-ecosystem.md`](docs/runtime-engine-ecosystem.md)
- **CI & Releases:** [`.github/workflows/ci.yml`](.github/workflows/ci.yml), [`.github/workflows/release.yml`](.github/workflows/release.yml)
- **Manifest Specifications:**
  - [`docs/specs/game-manifest.md`](docs/specs/game-manifest.md)
  - [`docs/specs/trainer-manifest.md`](docs/specs/trainer-manifest.md)
  - [`docs/specs/registry-manifest.md`](docs/specs/registry-manifest.md)
  - [`docs/specs/declarative-patch.md`](docs/specs/declarative-patch.md)
- **JSON Schemas:** [`schemas/`](schemas/README.md)
- **Security & Permissions:** [`SECURITY.md`](SECURITY.md), [`docs/security-and-permission-model.md`](docs/security-and-permission-model.md)
- **Decision Register:** [`docs/unresolved-questions.md`](docs/unresolved-questions.md)

---

## Workspace Layout

```text
Nekod/
├── .github/workflows/       # Workspace CI checks (fmt, clippy, tests, validator, audit)
├── apps/desktop/            # L1: Tauri 2 + React desktop app (player-only, zero MCP)
├── crates/
│   ├── nekod-core/          # L2: Modular application core facade
│   ├── game-detection/      # L2: Steam, Epic, GOG & custom folder game discovery
│   ├── trainer-manager/     # L2: Trainer cache, compatibility, SHA-256 verification
│   ├── trainer-runtime/     # L3: Controlled Win32/Linux backends, PatchLedger, router & sandbox host
│   ├── registry-client/     # L2: Conditional HTTP (ETag/304) registry client & staging
│   ├── permission-manager/  # L2: Default-deny policy & hashed grant store
│   ├── profile-manager/     # L2: User profiles, hotkeys, settings
│   └── nekod-mcp/           # L6: Optional developer MCP JSON-RPC 2.0 server
├── sdk/
│   ├── manifest/            # L4: Typed Game, Trainer, Registry manifest models
│   ├── runtime-api/         # L4: Stable L2 <-> L3 runtime boundary traits & DTOs
│   └── trainer-sdk/         # L4: Declarative patch schemas, builders & AOB signature helpers
├── schemas/                 # L4: Versioned JSON Schemas & valid examples
├── examples/                # Runnable sample singleplayer trainer package fixture
├── community/               # L5: Community registry templates & governance docs
├── tools/trainer-validator/ # L5: Local & CI schema/package validator CLI
├── skills/                  # L6: 6 model-independent contributor Agent Skills
├── prompts/                 # L6: Contributor prompt library
├── docs/                    # Specifications, security model, decision register
└── tests/                   # Cross-crate integration & protocol test docs
```

---

## Contributing & License

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution guidelines and [`SECURITY.md`](SECURITY.md) for our security policy and singleplayer-only boundaries.

Licensed under the [MIT License](LICENSE) (see [ADR-009](TECHNICAL_DECISIONS.md#adr-009-repository-license-mit)).
