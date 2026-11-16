# Nekod Development Roadmap

Nekod is built incrementally across six phases (`P0`–`P5`). Each phase has explicit deliverables and verification gates before advancing. Do not implement downstream phases out of order or introduce speculative abstractions ahead of their milestone.

---

## Phase Overview

| Phase | Name | Primary Goal | Status |
| :--- | :--- | :--- | :--- |
| **P0** | Architecture & Protocol | Repository layout, ADRs, manifest specs, JSON Schemas, security/permission model, crate boundaries | **Completed** |
| **P1** | Desktop Foundation | Tauri 2 + React desktop app, Rust core (`nekod-core`), Steam game discovery, local library & metadata | **Completed** |
| **P2** | Windows x64 Runtime | Controlled trainer execution, process verification, feature toggles, deterministic patch rollback, diagnostics | **Completed** |
| **P3** | Community Ecosystem | Registry client, third-party GitHub repo registration, artifact verification, trainer updates | **Completed** |
| **P4** | Developer Ecosystem | `trainer-sdk` authoring ergonomics, `nekod-mcp` server, Agent Skills, Prompt Library, CI validator | **Completed** |
| **P5** | Platform Expansion | Additional engines/runtime providers (e.g. WASM sandbox), Linux & Steam Deck support | **Completed** |

---

## P0 — Architecture and Protocol (Current Milestone)

**Objective:** Establish a verifiable technical foundation, versioned manifest contracts, security boundaries, and compilable crate/SDK interfaces.

**Deliverables:**
- `ARCHITECTURE.md`, `ROADMAP.md`, `TECHNICAL_DECISIONS.md`, `SECURITY.md`, `CONTRIBUTING.md`.
- Initial monorepo workspace (`Cargo.toml`, `crates/*`, `sdk/*`, `tools/*`, `schemas/*`, `docs/*`).
- Game, Trainer, and Registry Manifest specifications (`docs/specs/`).
- JSON Schema Draft 2020-12 files (`schemas/`) + valid reference examples (`schemas/examples/`).
- Security & permission model (`docs/security-and-permission-model.md`) and `permission-manager` policy core.
- Unresolved engineering questions & risk register (`docs/unresolved-questions.md`).

**Exit Gate:**
- All workspace crates compile cleanly (`cargo check --workspace`).
- JSON Schemas validate example manifests and manifest round-trip tests pass (`cargo test --workspace`).
- Unresolved engineering questions impacting P1/P2 are explicitly documented.

---

## P1 — Desktop Foundation

**Objective:** Deliver a working local desktop experience for players with zero AI or community-network dependency.

**Deliverables:**
- **L2 Core (`nekod-core`, `game-detection`, `profile-manager`, `trainer-manager`):**
  - Steam library detection on Windows (`libraryfolders.vdf` + `appmanifest_*.acf` parsing, install paths, `buildid`).
  - SQLite storage layer for local game library, profiles, hotkeys, and persistent permission grants.
  - Local trainer package ingestion from disk (schema-validated + permission-checked).
- **L1 Desktop (`apps/desktop`):**
  - Tauri 2 + React + TypeScript shell.
  - Game Library view (discovered Steam games + custom folder registration).
  - Game detail & trainer inspection page (features, hotkeys, trust badge, permission summary).
  - Settings, profiles, and log viewer.

**Exit Gate:**
- Steam games detected accurately on Windows test fixtures and local installs.
- Tauri UI communicates strictly through `nekod-core` IPC commands with zero direct OS/memory code in frontend.
- Unvalidated or malformed trainer manifests are rejected with actionable UI errors.

---

## P2 — Windows x64 Runtime

**Objective:** Execute declarative trainers safely against verified local Windows x64 singleplayer game processes.

**Deliverables:**
- **L3 Runtime (`trainer-runtime`):**
  - Windows x64 process enumeration & identity verification (executable path, architecture, optional SHA-256 & build ID match).
  - Anti-cheat / disallowed process guard (abort before handle creation if prohibited environment detected).
  - `declarative_patch_v1` execution engine: AOB signature scanning, bounded pointer-chain reads, byte patches, and periodic value freeze loop.
  - `PatchLedger` with deterministic rollback on feature off, session stop, and game exit detection.
  - Strict enforcement of `GrantedPermissions` (`permission-manager`) with user confirmation flow for write/allocate scopes.
  - Structured `tracing` runtime logs and `RuntimeDiagnostics` surfaced in Desktop UI.
- **L1 Integration:**
  - One-click trainer attach/detach, live feature toggles, global hotkeys, and rollback status indicators.

**Exit Gate:**
- Integration tests against a benign local test target binary verify attach, read, patch, rollback, and permission denial.
- Attempted write without `process.write_memory` grant fails closed and logs an audit event.
- Third-party native runtime modules remain disabled by default.

---

## P3 — Community Ecosystem

**Objective:** Connect players to `Nekod-Community` and independent third-party GitHub trainer repositories safely.

**Deliverables:**
- **L2 Registry (`registry-client`, Update Manager):**
  - Fetch & cache `RegistryManifest`s from `Nekod-Community` and user-added third-party repositories (ETag/conditional HTTP via `reqwest`).
  - Enforce `TrustTier` distinctions (`Official`, `CommunityVerified`, `Unverified`) in core and UI.
  - Download trainer releases, validate `TrainerManifest` against JSON Schema, and verify artifact SHA-256 digests.
  - Revocation handling (`status: "revoked"`) and version update checks.
- **L5 Community (`community/`, `tools/trainer-validator`):**
  - Complete CLI validator (`nekod-validator`) with JSON Schema + semantic/security linting.
  - GitHub Actions workflow templates for `Nekod-Community` and independent trainer repositories.

**Exit Gate:**
- Client rejects tampered artifacts, mismatched hashes, path-traversal entrypoints, and revoked trainers.
- UI clearly distinguishes Official, Community-Verified, and Unverified third-party trainers.

---

## P4 — Developer Ecosystem (Optional AI Tooling)

**Objective:** Streamline trainer creation, testing, and publishing for contributors with optional MCP and Agent Skills support.

**Deliverables:**
- **L4 SDK (`sdk/trainer-sdk`):** Ergonomic Rust builders, signature testing helpers, and manifest generation utilities.
- **L6 MCP Server (`crates/nekod-mcp`):**
  - Standard MCP transport supporting the five scoped tool groups (Game Discovery, Trainer Development, Compatibility, Repository, Runtime Inspection).
  - Read-only vs. write separation with explicit authorization checks.
- **L6 Skills & Prompts (`skills/`, `prompts/`):**
  - Model-independent Agent Skills for `game-discovery`, `trainer-development`, `manifest-validation`, `trainer-debugging`, `compatibility-analysis`, and `trainer-publishing`.
  - Contributor prompt templates.

**Exit Gate:**
- MCP tools operate against `nekod-core` and `nekod-validator` with typed schemas and refuse unauthorized process writes or anti-cheat circumvention requests.
- End-user desktop application continues to build and run cleanly with `nekod-mcp` completely absent.

---

## P5 — Platform & Runtime Expansion

**Objective:** Broaden OS, store, and runtime coverage without breaking L4 protocol compatibility.

**Deliverables:**
- **Runtime Providers:** Evaluate and implement `wasm_sandbox_v1` (Wasmtime/capability-scoped host functions) for complex trainer logic without raw native DLL injection.
- **OS & Store Support:**
  - Linux x64 (`process_vm_readv` / `process_vm_writev` / `/proc/<pid>/mem` capability considerations) and SteamOS / Steam Deck (Proton prefix & Wine PID mapping).
  - Additional store detectors (GOG, Epic Games Store, Xbox/MS Store, custom paths).
- **Process Isolation Hardening:** Evaluate moving `trainer-runtime` into a dedicated helper subprocess over local IPC if threat modeling warrants OS-level process separation from the Tauri UI process.

---

## Next Horizon — V1.1 & Multi-Engine Mod Platform Evolution

Building on the completed `P0`–`P5` baseline, Nekod is evolving into a unified **Multi-Engine Mod Runtime & AI-Assisted Mod Forge Platform** (see [ADR-019](TECHNICAL_DECISIONS.md#adr-019-multi-engine-mod-platform-evolution-modmanifest-runtime-adapters-dependency-manager-and-compatibility-database) and [`docs/runtime-engine-ecosystem.md`](docs/runtime-engine-ecosystem.md)):

| Phase | Focus | Core Deliverables | Status |
| :--- | :--- | :--- | :--- |
| **V1.1-S1** | Bridge & Importer Foundations | `.CT -> declarative_patch_v1` static converter (`nekod-validator import-ct`), `DeploymentLedger` disk rollback, loopback `adapter_ipc_v1` (`retroarch_udp_v1`) | **Completed** |
| **V2-M1** | Core Protocol Refactor (`.nmod`) | `ModManifest` v2 (`mod_kind`: trainer/script/plugin/asset/config/patch/emulator), source-first `.nmod` package format (ADR-020), `RuntimeProvider` adapter capabilities & v1 upgrader | **Completed** |
| **V2-M2** | Dependency & Compatibility | `Runtime Dependency Manager` (license-aware loader detection/staging) + multi-dimensional `Compatibility Database` | **Completed** |
| **V2-M3** | Traditional + Engine Adapters | Constrained CE `.CT` subset evaluator, `Unity Runtime Bridge` (Mono & IL2CPP via BepInEx/MelonLoader), `Unreal Runtime Bridge` (`UE4SS`) | **Next** |
| **V2-M4** | Dynamic & Specialized Bridges | Optional isolated `Frida Gum` dynamic instrumentation adapter + `REFramework` / `ScriptHook` managed adapters | **Planned** |
| **V2-M5** | AI Mod Forge & UI Polish | L6 `nekod-mcp` game/engine analysis → runtime recommendation → mod generation → sandbox/runtime verification pipeline + WeMod-grade desktop UX (`docs/v1.1-roadmap.md`) | **Planned** |

See also [`docs/v1.1-roadmap.md`](docs/v1.1-roadmap.md) for player-facing desktop UX improvements and reference trainer examples.
