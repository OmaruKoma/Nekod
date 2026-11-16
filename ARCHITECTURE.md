# Nekod Architecture

## 1. Architectural Goals & Constraints

Nekod is an **open-source community Mod platform** (`Modding games, together.`) built around strict separation of concerns, multi-engine runtime unification at the management boundary, explicit trust boundaries, and protocol stability.

Key architectural constraints:
1. **No AI dependency for players:** All player workflows (game & engine discovery, dependency management, mod installation, permission review, feature toggles, rollback, logging) execute deterministically in the local application core and runtime without requiring any LLM, MCP server, or cloud inference.
2. **Unified management interface, heterogeneous runtime adapters:** Nekod exposes unified lifecycle, permission, diagnostic, and compatibility contracts across `sdk/runtime-api`, while letting each engine adapter (`Declarative/CE`, `Dynamic Instrumentation`, `Unity Bridge`, `Unreal Bridge`, `Game-Specific Bridge`) handle its own execution semantics and upstream loader discovery.
3. **Modular monolith before microservices:** Application core domains live in cleanly separated Rust crates inside one workspace and execute inside a unified core service boundary (`nekod-core`), while keeping the `RuntimeProvider` boundary IPC-ready for future subprocess isolation.
4. **Default-deny security posture:** Unvalidated manifests are rejected; untrusted native modules and scripts are disabled by default; memory-write, allocation, disk deployment, and hook scopes require explicit user consent.
5. **Singleplayer scope enforcement:** Nekod does not implement anti-cheat bypasses, stealth injection, or target competitive online games.

---

## 2. Six-Layer Logical Model

```text
+-------------------------------------------------------------------------+
| L1: Desktop Application (apps/desktop — Tauri 2 + React + TypeScript)   |
|     Game Library · Mods · Trainers · Dependencies · Profiles · Security |
+------------------------------------+------------------------------------+
                                     | Typed IPC (Commands / Events)
                                     v
+-------------------------------------------------------------------------+
| L2: Application Core (crates/nekod-core + Domain Crates)                |
|  - game-detection (stores + engines) - trainer/mod-manager              |
|  - profile-manager                   - registry-client                  |
|  - permission-manager                - Dependency & Compatibility Mgmt  |
+------------------------------------+------------------------------------+
                                     | L4 Protocol: sdk/runtime-api (Unified Runtime Interface)
                                     v
+-------------------------------------------------------------------------+
| L3: Multi-Engine Mod Runtime (crates/trainer-runtime)                   |
|  CompositeRuntimeRouter · PatchLedger · DeploymentLedger · Diagnostics  |
|  Adapters: Declarative/CE | Frida | Unity Bridge | UE Bridge | Bridges  |
+------------------------------------+------------------------------------+
                                     | Process / Hook / Loader / Emulator IPC
                                     v
             [ Verified Local Game Process / Mod Loader / Emulator ]

Cross-cutting L4, L5, L6:
- L4 SDK & Protocol: sdk/manifest (Trainer/ModManifest), sdk/runtime-api, sdk/trainer-sdk, schemas/
- L5 Community Registry: Nekod-Community index + Compatibility DB + independent GitHub repos + trainer-validator
- L6 AI Mod Forge (Optional): crates/nekod-mcp, skills/, prompts/ (Game Analysis -> Mod Generation -> Validation)
```

---

## 3. Layer Breakdown & Crate Boundaries

### L1 — Desktop Application (`apps/desktop`)
- **Stack:** Tauri 2, React, TypeScript.
- **Role:** Thin presentation layer for ordinary players.
- **Rules:**
  - Contains no memory offsets, process logic, or schema parsing rules.
  - Consumes `nekod-core` exclusively through typed Tauri IPC commands and state-change event streams.
  - Surfaces clear, jargon-free permission confirmation dialogs before granting write/allocate or native-module scopes.

### L2 — Application Core (`crates/nekod-core` and Domain Crates)

`nekod-core` acts as the modular monolith coordinator that wires together single-responsibility domain crates:

| Crate | Primary Responsibility | Key Public Abstractions |
| :--- | :--- | :--- |
| `game-detection` | Discover installed games and read store metadata (Steam ACF/VDF first) | `GameDetector`, `DetectedGame`, `GameInstallInfo` |
| `trainer-manager` | Manage local trainer packages, compatibility matching, artifact hashing | `TrainerCatalog`, `InstalledTrainer`, `CompatibilityMatch` |
| `registry-client` | Fetch, validate, and cache `RegistryManifest` & `TrainerManifest` from GitHub/community | `RegistrySource`, `RegistrySyncService`, `RemoteTrainerPackage` |
| `permission-manager` | Evaluate trust tiers, enforce default-deny policy, persist user grants | `PermissionEvaluator`, `TrustTier`, `GrantPolicy`, `GrantedPermissions` |
| `profile-manager` | Persist per-game user profiles, hotkeys, default toggles, app config | `ProfileStore`, `GameProfile`, `AppConfig` |
| `nekod-core` | Unified facade coordinating managers, runtime sessions, updates, and event bus | `CoreFacade`, `CoreCommand`, `CoreEvent` |

**Dependency Rules (Acyclic):**
- Domain crates in L2 may depend on `sdk/manifest` and `sdk/runtime-api`, never on `apps/desktop` or `crates/nekod-mcp`.
- `nekod-core` coordinates L2 crates and holds a `dyn RuntimeProvider` (from `sdk/runtime-api`) implemented by `crates/trainer-runtime`.
- `crates/trainer-runtime` depends on `sdk/runtime-api`, `sdk/manifest`, and `crates/permission-manager` types (specifically immutable `GrantedPermissions`), never on UI, registry networking, or MCP.

### L3 — Trainer Runtime (`crates/trainer-runtime`)
- **Role:** Isolated execution boundary for trainer lifecycles and target process interaction.
- **Responsibilities:**
  1. **Process Discovery & Identity Verification:** Match running process name, executable path, architecture, and (when available) binary SHA-256 / store build ID against `GameManifest` and `TrainerManifest` before attaching.
  2. **Anti-Cheat & Policy Guard:** Abort attachment if known anti-cheat drivers/processes or disallowed online flags are detected.
  3. **Permission Enforcement:** Every runtime operation checks an immutable `GrantedPermissions` snapshot issued by `permission-manager`. Unapproved write/allocate/native scopes fail closed.
  4. **Execution Modes:**
     - **Declarative Runtime (`declarative_patch_v1`):** Primary, default-safe mode where trainers are structured patch/signature/freeze definitions executed by Nekod's audited runtime engine.
     - **Sandboxed / Native Providers (`wasm_sandbox_v1`, `native_provider_v1`):** Future/advanced extension paths. Native modules from third-party or unverified sources are **disabled by default**.
  5. **Patch Rollback:** Maintain an ordered `PatchLedger` recording `(address, original_bytes, patched_bytes, feature_id)`. Deactivating a feature, stopping a trainer, or orderly shutdown restores original bytes deterministically where the process remains alive.
  6. **Diagnostics & Audit Log:** Structured `tracing` + ring-buffer execution logs exported via `RuntimeDiagnostics`.

### L4 — SDK & Protocol (`sdk/` & `schemas/`)
- **`schemas/`:** Canonical JSON Schema (Draft 2020-12) definitions for `GameManifest`, `TrainerManifest`, and `RegistryManifest`, independently versioned via `schema_version` (SemVer).
- **`sdk/manifest`:** Strongly typed Rust structs matching the JSON Schemas 1:1, with SemVer and path-safety validation.
- **`sdk/runtime-api`:** UI-independent traits (`RuntimeProvider`), commands (`AttachRequest`, `FeatureCommand`), state snapshots (`SessionState`, `FeatureStatus`), `PatchRecord`, and typed `RuntimeError`s.
- **`sdk/trainer-sdk`:** Authoring primitives (`TrainerDefinition`, `DeclarativeFeature`, `MemoryPatchOp`) so trainer authors target a stable SDK rather than Nekod internal crates.

### L5 — Community Registry & Ecosystem (`community/`, `tools/trainer-validator/`)
- **Hybrid Repository Model:**
  - `Nekod` — Main application, core, runtime, SDK.
  - `Nekod-Community` — Metadata index aggregating official and independent third-party trainer repositories.
  - `Nekod-Official-Trainers` — Officially maintained trainers (`TrustTier::Official`).
  - `Nekod-Skills` & `Nekod-Docs` — Documentation and optional contributor workflows (hosted inside `skills/`, `prompts/`, `docs/` in this monorepo during early development).
- **Ownership Principle:** `Nekod-Community` stores pointers and verified metadata (`RegistryManifest`), not copies of all third-party trainer binaries. Authors keep full ownership of their GitHub repositories.
- **Validation Gate:** `tools/trainer-validator` (`nekod-validator`) runs both locally and in GitHub Actions to enforce JSON Schema validity, SemVer rules, permission sanity, and prohibited path/URL patterns before registry inclusion.

### L6 — AI Developer Tooling (`crates/nekod-mcp`, `skills/`, `prompts/`)
- **Optional Developer Tooling:** Used solely by contributors developing/validating trainers in MCP-compatible editors/agents.
- **`crates/nekod-mcp`:** Standard MCP server wrapping `nekod-core`, `sdk/manifest`, and `tools/trainer-validator` capabilities across five scoped tool groups:
  1. Game Discovery (`list_installed_games`, `get_game_info`, `inspect_game_environment`)
  2. Trainer Development (`create_trainer_manifest`, `validate_trainer_manifest`, `inspect_trainer`, `generate_trainer_template`)
  3. Compatibility (`check_game_version`, `validate_compatibility`, `inspect_runtime_diagnostics`)
  4. Repository (`validate_repository`, `inspect_registry_entry`, `prepare_trainer_release`)
  5. Runtime Inspection (`inspect_authorized_game_process`, `inspect_trainer_state`, `get_execution_logs`)
- **Safety Boundary:** Read-only inspection tools are strictly separated from any state-mutating operations; runtime memory inspection requires an explicitly authorized session, and no tool may bypass anti-cheat or target competitive online games.

---

## 4. End-to-End Flows

### Flow A: Player Activates a Trainer
1. `game-detection` discovers an installed game (e.g., Steam ACF + executable path) and matches it to a `GameManifest`.
2. `registry-client` / `trainer-manager` presents compatible trainers labeled clearly by `TrustTier` (`Official`, `CommunityVerified`, `Unverified`).
3. Player clicks **Install / Enable** on a trainer.
4. `trainer-manager` validates the `TrainerManifest` against JSON Schema, verifies artifact SHA-256 hashes, and checks game version compatibility.
5. `permission-manager` inspects declared `permissions` and `runtime.kind`:
   - If `native_provider_v1` from a non-official source: blocked by default unless explicitly overridden in advanced security settings.
   - If `process.write_memory` or `process.allocate_memory` is requested: surfaces a plain-language confirmation prompt to the player.
6. Upon player confirmation, `permission-manager` issues a scoped `GrantedPermissions` snapshot for that session.
7. `nekod-core` calls `RuntimeProvider::start_session(...)` across `sdk/runtime-api`.
8. `trainer-runtime` verifies the running game process identity & anti-cheat absence, attaches with least required OS access rights, and awaits feature commands.
9. Player toggles a feature; `trainer-runtime` applies the patch, records original bytes in `PatchLedger`, and logs the action.
10. Player toggles off or stops the trainer; `trainer-runtime` rolls back original bytes in reverse order and emits updated state.

### Flow B: Community Developer Publishes a Trainer
1. Developer authors a trainer using `sdk/trainer-sdk` (optionally assisted by `nekod-mcp` and `skills/`).
2. Developer generates `trainer-manifest.json` and runs `nekod-validator` locally.
3. Developer publishes the trainer release and `trainer-manifest.json` to their own GitHub repository.
4. Developer submits/updates a `RegistryManifest` PR in `Nekod-Community` referencing their GitHub repo.
5. CI runs `nekod-validator` on the PR (schema checks, HTTPS provenance, non-empty permissions, no path traversal, singleplayer compatibility declaration).
6. Once merged, Nekod desktop clients fetch the updated metadata, display source repository attribution, and verify artifact integrity before execution.

---

## 5. Data & Persistence Model (Planned P1–P3)

- **SQLite (`profile-manager`, `trainer-manager`, `registry-client`):**
  - Local game library cache & custom paths
  - Installed trainer metadata & verification timestamps
  - Cached registry indexes & ETags
  - Persistent permission grants (scoped to `trainer_id` + `version` + `manifest_sha256`, invalidated on permission escalation)
  - User profiles, hotkeys, and toggle presets
- **TOML / JSON Config Files:**
  - Human-editable application preferences (`config.toml`)
  - Canonical interchange manifests (`*.manifest.json`)
- **Structured Logs:**
  - `tracing` JSON/text rotating session logs for diagnostics and reproducible bug reports.

---

## 6. Process & Memory Isolation Evolution

- **P1–P2 (Initial):** `trainer-runtime` executes behind the `sdk/runtime-api` trait in-process with `nekod-core` inside the Tauri backend process, using minimal Windows APIs (`OpenProcess` with scoped `PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION` only when permitted, never `PROCESS_ALL_ACCESS` unless strictly justified).
- **Future Hardening (P3+/P5):** Because all calls cross `sdk/runtime-api`, `trainer-runtime` can be moved to a dedicated helper/worker subprocess over local IPC without modifying L1 UI or L2 business logic. See [ADR-002](TECHNICAL_DECISIONS.md#adr-002-modular-monolith-with-ipc-ready-runtime-boundary) and [`docs/unresolved-questions.md`](docs/unresolved-questions.md).
