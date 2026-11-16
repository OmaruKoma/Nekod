# Nekod Technical Decisions & Architecture Decision Records (ADRs)

This document records significant architectural choices, the alternatives considered, their trade-offs, and a summary of unresolved engineering questions.

---

## ADR-001: Modular Monorepo Starting Structure

- **Status:** Accepted (P0)
- **Context:** Nekod spans six logical layers (UI, Core, Runtime, SDK/Schemas, Community Tooling, optional MCP). Splitting into five separate GitHub repositories on day one would slow down protocol iteration between `sdk/manifest`, `sdk/runtime-api`, and `crates/*`.
- **Decision:** Start with a single Cargo workspace monorepo (`Nekod/`) containing `apps/`, `crates/`, `sdk/`, `schemas/`, `tools/`, `community/`, `skills/`, `prompts/`, and `docs/`. Later, `Nekod-Community`, `Nekod-Official-Trainers`, `Nekod-Skills`, and `Nekod-Docs` can be mirrored or split cleanly along directory boundaries once L4 schemas stabilize.
- **Alternatives Considered:**
  1. *Multi-repo from day one:* Rejected due to cross-repo version lockstep overhead during P0–P2.
  2. *Single monolithic crate:* Rejected because it blurs boundaries between UI-independent SDK (`sdk/`), core (`crates/nekod-core`), and privileged runtime (`crates/trainer-runtime`).
- **Consequences:** Easy atomic refactors across protocol and core in P0–P2; requires strict Cargo dependency discipline so L2/L3 crates never depend on L1/L6.

---

## ADR-002: Modular Monolith with IPC-Ready Runtime Boundary

- **Status:** Accepted (P0), Re-evaluated in P2/P5
- **Context:** Section 2 requires separating trainer runtime execution from the desktop presentation layer while avoiding premature microservices.
- **Decision:** Implement L2 (`nekod-core`) and L3 (`trainer-runtime`) as separate Cargo crates communicating exclusively across the `sdk/runtime-api` (`RuntimeProvider`) trait boundary. In P1–P2, they run inside the same backend Rust process (hosted by Tauri), while the frontend (`apps/desktop`) communicates over Tauri IPC.
- **Alternatives Considered:**
  1. *Immediate separate runtime daemon / helper process:* Adds IPC serialization, lifecycle/orphan-process management, and Windows job-object complexity before core features exist.
  2. *Direct runtime calls from Tauri commands:* Rejected; violates layer separation and prevents headless CLI/MCP reuse.
- **Consequences:** Clean abstraction boundary (`RuntimeProvider`) enables future extraction into an isolated out-of-process worker (e.g., for crash containment or tighter sandboxing) without rewriting L1 or L2.

---

## ADR-003: Declarative-First Trainer Runtime (`declarative_patch_v1`) Over Arbitrary Native Binaries

- **Status:** Accepted (P0)
- **Context:** Traditional trainer ecosystems distribute opaque `.exe` or `.dll` payloads, creating severe malware risk and making static auditing impossible for ordinary players.
- **Decision:** Make `declarative_patch_v1` the primary, default-supported runtime kind. Declarative trainers describe AOB signatures, pointer chains, byte patches, value constraints, and feature metadata in validated JSON/structured data executed by Nekod's own audited Rust runtime. Native modules (`native_provider_v1`) are **untrusted and disabled by default**; a future `wasm_sandbox_v1` capability-based sandbox is planned for P5 for complex scripted logic.
- **Alternatives Considered:**
  1. *Native DLL/shared-library plugins only:* High security risk, platform-locked, impossible for most players to audit, violates default-deny untrusted code policy.
  2. *Embedded Lua/JS engine in P0:* Adds scripting surface and FFI binding maintenance before basic signature/patch primitives are proven.
- **Consequences:** Dramatically smaller attack surface, deterministic patch rollback (`PatchLedger`), cross-platform manifest portability, and straightforward static validation in `trainer-validator`. Trade-off: complex game-engine hooks (e.g. calling in-game functions) cannot be expressed purely declaratively until `wasm_sandbox_v1` or explicit opt-in providers exist.

---

## ADR-004: Hybrid Community Registry Without Central Binary Custody

- **Status:** Accepted (P0)
- **Context:** Community authors should be able to publish, version, and maintain trainers in their own GitHub repositories without transferring ownership to a central organization.
- **Decision:** `Nekod-Community` acts as a curated metadata index (`RegistryManifest` entries pointing to external GitHub repos + SHA-256 integrity records + trust tiers), not a binary host for all third-party trainers. Clients fetch release artifacts directly from the attributed repository (or official mirrors) and verify hashes + schemas locally.
- **Alternatives Considered:**
  1. *Centralized binary registry (all trainers uploaded to one repo/bucket):* Creates moderation/legal/bandwidth bottlenecks and discourages independent maintainers.
  2. *Unindexed peer-to-peer URLs only:* Poor discoverability and no consistent trust-tier or revocation mechanism.
- **Consequences:** Preserves developer independence and clear source attribution. Trade-off: upstream GitHub repos can delete tags or go private (mitigated via client caching, hash pinning, and revocation flags).

---

## ADR-005: JSON Schema (Draft 2020-12) with Strict Top-Level Fields & `extensions` Escape Hatch

- **Status:** Accepted (P0)
- **Context:** Manifests must be validated strictly ("never trust an unvalidated manifest") while remaining backward-compatible and extensible across schema versions.
- **Decision:**
  - Use JSON Schema Draft 2020-12 with self-contained schema files in `schemas/` (`*.v1.schema.json`) for maximum offline tool compatibility.
  - Version every manifest with `schema_version` (`SemVer`, currently `"1.0.0"`).
  - Set `"additionalProperties": false` on core manifest objects so typos and unexpected untrusted fields fail validation immediately, while providing an explicit optional `"extensions"` object (`type: "object"`) for forward-compatible, non-executable namespaced metadata.
- **Alternatives Considered:**
  1. *`additionalProperties: true` everywhere:* Silently accepts misspelled fields (e.g., `"permisions"`), causing dangerous silent defaults.
  2. *External `$ref` across multiple schema files:* Fails in lightweight or offline JSON Schema tools that do not configure custom URI resolvers.
- **Consequences:** High validation reliability across Rust (`jsonschema` crate with network resolution disabled), TypeScript, and CI workflows; clear upgrade path for `1.x` minor additions vs `2.0.0` breaking schemas.

---

## ADR-006: Capability-Based Explicit Permission Model (Default-Deny)

- **Status:** Accepted (P0)
- **Context:** Memory-modifying software must never silently run arbitrary operations or hide risk from players.
- **Decision:** Every `TrainerManifest` must declare fine-grained `PermissionDeclaration`s (`process.discover`, `process.read_memory`, `process.write_memory`, `process.allocate_memory`, `process.suspend_resume`, `filesystem.read_save_dir`, `filesystem.write_save_dir`, `runtime.native_module`). `permission-manager` enforces default-deny rules by `TrustTier` and materializes an immutable `GrantedPermissions` token bound to `(trainer_id, version, manifest_sha256)`. Any manifest update that adds permissions automatically invalidates prior grants.
- **Alternatives Considered:**
  1. *All-or-nothing "Trust this trainer" toggle:* Hides whether a trainer only reads memory vs. allocates code caves or loads native binaries.
  2. *OS-level sandboxing alone:* Insufficient for communicating risk clearly to ordinary players before execution.
- **Consequences:** Auditable permission checks in `trainer-runtime` and transparent consent UX in `apps/desktop`.

---

## ADR-007: Optional Developer MCP Server Decoupled from Player Runtime

- **Status:** Accepted (P0)
- **Context:** AI tooling accelerates trainer authoring and manifest debugging, but players must never depend on AI clients or MCP servers.
- **Decision:** Isolate all MCP functionality inside `crates/nekod-mcp` (L6), compiled as a separate developer binary that consumes `nekod-core`, `sdk/manifest`, and `tools/trainer-validator`. `apps/desktop` has zero dependency on `nekod-mcp`.
- **Alternatives Considered:**
  1. *Embed MCP server inside the desktop app by default:* Increases attack surface (exposes local RPC/tooling to external AI agents) and blurs player vs. developer boundaries.
- **Consequences:** Zero AI overhead or security exposure for ordinary players; clean standalone developer tooling for contributors.

---

## ADR-008: Tauri 2 + React + TypeScript for Desktop Presentation

- **Status:** Accepted (P0, Implementation P1)
- **Context:** Nekod requires a responsive Windows-first (and future Linux/SteamOS) desktop UI tightly integrated with a Rust core.
- **Decision:** Use Tauri 2 with React and TypeScript in `apps/desktop/`, backed by `crates/nekod-core`.
- **Alternatives Considered:**
  1. *Electron:* Heavy memory footprint alongside AAA games and splits native Rust core across Node/N-API boundaries.
  2. *Pure Rust GUI (egui / iced):* Faster binary footprint, but slower iteration for rich community catalog layouts, localization, and accessible UI polish.
- **Consequences:** Native Rust core with low idle overhead and strong typed IPC (`tauri::command`). Requires careful IPC boundary hygiene so frontend remains strictly presentation-only.

---

## ADR-009: Repository License (MIT)

- **Status:** Accepted (P0 — Confirmed for U10)
- **Context:** Repository structure requires a root `LICENSE`, and ecosystem contributors need a clear, minimal permissive license before P1 implementation begins.
- **Decision:** Adopt the **MIT License** (`LICENSE`) for the Nekod core repository and workspace crates, while allowing trainers in independent third-party repositories to declare their own SPDX license in `TrainerManifest.license`.
- **Alternatives Considered:**
  1. *Apache-2.0:* Includes explicit patent grant/retaliation terms, slightly heavier notice obligations.
  2. *Dual MIT OR Apache-2.0:* Common in Rust crates, unnecessary dual-licensing overhead for this project.
- **Consequences:** Simple permissive adoption for contributors and downstream tooling.

---

## ADR-010: Canonical Game ID Namespace Governance (Strict Kebab-Case Slugs)

- **Status:** Accepted (P0 — Confirmed for U11)
- **Context:** Multiple community trainers must reliably reference the same game across Steam, GOG, Epic, and custom installs without ID drift or storefront lock-in.
- **Decision:** Enforce strict lowercase kebab-case canonical slugs (`^[a-z0-9]+(?:-[a-z0-9]+)*$`, length 2–128) for `GameManifest.id` (e.g. `hollow-knight`, `skyrim-special-edition`). Store-specific identifiers belong exclusively in `store_ids`. Distinct editions with separate executables or store IDs receive distinct edition-suffixed slugs, and `Nekod-Community` CI enforces global uniqueness.
- **Alternatives Considered:**
  1. *Reverse-DNS publisher namespaces (`game.bethesda.skyrim-se`):* Verbose and inconsistent when publishers change.
  2. *Store-prefixed IDs (`steam:367520`):* Couples canonical game identity to a single storefront.
- **Consequences:** Store-independent, readable identifiers enforced uniformly by both JSON Schema and `nekod-manifest` semantic validation.

---

## ADR-011: `declarative_patch_v1` Bounded Instruction Set (U1)

- **Status:** Accepted (P2)
- **Context:** `declarative_patch_v1` requires a small, deterministic, auditable operation set that covers common singleplayer trainer patches without enabling arbitrary native execution or Turing-complete scripts.
- **Decision:** Define `declarative-patch.v1.schema.json` supporting two bounded operations (`byte_patch` up to 64 bytes with optional `expected_original_bytes` guard; `write_value` for `i32`/`u32`/`i64`/`u64`/`f32`/`f64`) and three bounded `AddressLocator` variants (`module_offset`, `aob_pattern` up to 128 bytes, `pointer_chain` up to depth 8). No executable code-cave allocation is included in `declarative_patch_v1`.
- **Consequences:** Static schema validation, module-constraint enforcement, and deterministic `PatchLedger` rollback are guaranteed for every operation.

---

## ADR-012: Windows x64 Process Identity, PID-Reuse Detection & Non-Elevated Access Policy (U2, U3)

- **Status:** Accepted (P2)
- **Context:** PIDs on Windows are recycled aggressively by the kernel, and games launched elevated cause `ERROR_ACCESS_DENIED` (`5`) for standard-user Nekod processes.
- **Decision:** Bind every runtime session and `PatchLedger` record to `(pid, process_creation_time_100ns, executable_path)` obtained via `GetProcessTimes` and `QueryFullProcessImageNameW`. Before every write or rollback, re-verify process liveness, creation timestamp, and committed memory protection (`VirtualQueryEx`). When `OpenProcess` fails with `ERROR_ACCESS_DENIED`, surface `RuntimeError::ProcessElevatedAccessDenied` instructing the user to run the game without elevation rather than escalating Nekod privileges.
- **Consequences:** Eliminates silent PID-reuse corruption and preserves the no-auto-elevation security guarantee.

---

## ADR-013: Bounded Value Freeze Polling Cadence (U4)

- **Status:** Accepted (P2)
- **Context:** Frozen scalar values (`write_value` with `freeze_interval_ms`) must keep in-game counters steady without spinning CPU cores or flooding `WriteProcessMemory`.
- **Decision:** Restrict `freeze_interval_ms` to `50..=1000` ms (default `100` ms) executed per-session in a cancellable background ticker that verifies process identity and stops immediately on session teardown or process exit.
- **Consequences:** Predictable low overhead and clean lifecycle shutdown.

---

## ADR-014: Artifact Integrity Chain & Publisher Verification Policy (U5)

- **Status:** Accepted (P3)
- **Context:** Community trainers hosted on independent GitHub repositories require tamper-resistant release verification without imposing heavy key-management barriers on contributors.
- **Decision:** Enforce a strict cryptographic chain over HTTPS (`validate_https_url`): `RegistryTrainerEntry.manifest_sha256` (when present, required for `CommunityVerified` and `Official`) pins downloaded `trainer-manifest.json` bytes, and `TrainerManifest.artifacts[*]` pins byte size and lowercase SHA-256 of every staged artifact (including `runtime.entrypoint`). Signed attestations / detached Ed25519 signatures may be supplied via optional schema extensions (`nekod.attestation`) and will never override authoritative `TrustTier` assignment from client root config.
- **Consequences:** Silent release asset replacement or manifest tampering fails closed prior to installation or grant evaluation.

---

## ADR-015: Static Index Feeds & Conditional HTTP Caching (`ETag`) (U6)

- **Status:** Accepted (P3)
- **Context:** Unauthenticated GitHub REST API polling hits 60 requests/hour limits and would degrade player startup or sync reliability.
- **Decision:** Registry indexes should be served as static JSON documents (`static_index_json` or release asset feeds via `raw.githubusercontent.com` / GitHub Releases / CDN) fetched with conditional HTTP (`If-None-Match` / `ETag` and `If-Modified-Since`) via `reqwest` blocking client, capped response sizes, `304 Not Modified` short-circuiting, explicit `429`/`Retry-After` rate-limit errors, and SQLite offline fallback.
- **Consequences:** Players require zero GitHub API tokens and zero accounts; offline startup continues using cached SQLite metadata.

---

## ADR-016: Upstream Repository Deletion & Tag Mutation Protection (U7)

- **Status:** Accepted (P3)
- **Context:** Independent authors can delete repositories, unpublish releases, or force-push mutable tags at any time.
- **Decision:** `registry-client` retains cached SQLite metadata with per-source sync status (`ok`, `not_modified`, `not_found`, `rate_limited`, `error`) if an upstream feed disappears (`404`/`410`), while refusing to replace verified installed packages when downloaded `manifest_sha256`, `trainer_id`, `game_id`, `latest_version`, or artifact SHA-256 differs. Upstream entries marked `status: "revoked"` propagate immediately to local `SqliteTrainerCatalog`, blocking future sessions.
- **Consequences:** Tag mutation cannot silently swap trainer payloads, and repository deletion preserves installed singleplayer packages unless explicitly revoked.

---

## ADR-017: Cross-Platform Linux/SteamOS Process Discovery & Multi-Store Expansion (U8)

- **Status:** Accepted (P5)
- **Context:** Games on Linux/SteamOS frequently run inside Proton/Wine prefixes where Windows binaries appear as arguments in `/proc/<pid>/cmdline`, while Windows players also install singleplayer games via GOG Galaxy and Epic Games Store alongside Steam.
- **Decision:**
  1. Expand `crates/game-detection` behind the existing `GameDetector` trait with `EpicDetector` (`.item` manifests with relative path validation) and `GogDetector` (`goggame-*.info` metadata + registry/root scan).
  2. Define deterministic `/proc/<pid>/stat`, `/proc/<pid>/maps`, and Proton `cmdline` parsing utilities so Linux `procfs` / `process_vm_readv` / `process_vm_writev` backends reuse `PatchLedger` and `(pid, starttime, executable)` identity binding without automatic privilege escalation or modifying SteamOS read-only rootfs.
- **Consequences:** Multi-store discovery and cross-platform parsing reuse existing L4 `GameManifest` and `DetectedGame` schemas without breaking protocol compatibility.

---

## ADR-018: Multi-Runtime Dispatch & Capability-Scoped `wasm_sandbox_v1` Host Boundary (U9)

- **Status:** Accepted (P5)
- **Context:** Trainers requiring stateful logic beyond static declarative sequences need sandboxed scripting (`wasm_sandbox_v1`) without exposing arbitrary OS syscalls, raw sockets, filesystem access, or unrestricted memory writes.
- **Decision:**
  1. Select capability-scoped WebAssembly (`wasm_sandbox_v1`) host mediation rather than arbitrary native DLLs: guest calls execute strictly inside host-mediated functions (`SandboxHostBridge`) gated by `GrantedPermissions`, `PermissionConstraints` (`allowed_modules`, `max_write_bytes`), fuel/operation budgets, and `PatchLedger` recording.
  2. Provide `CompositeRuntimeRouter` implementing `RuntimeProvider` (`sdk/runtime-api`) to route sessions cleanly by `trainer.runtime.kind` (`DeclarativePatchV1` vs. sandboxed providers) while allowing future OS-process isolation hardening cleanly behind the same trait.
- **Consequences:** Desktop UI (`L1`) and Application Core (`L2`) require zero changes as new `RuntimeProvider` implementations are introduced.

---

## ADR-019: Multi-Engine Mod Platform Evolution (`ModManifest`, Runtime Adapters, Dependency Manager, and Compatibility Database)

- **Status:** Accepted (Architecture Direction for V1.1+ / V2 Protocol Evolution)
- **Context:** Real-world PC game modification spans far more than Cheat Engine-style numeric trainers: community resources live across `.CT` tables, Frida scripts, Unity (`BepInEx` / `MelonLoader` on Mono & IL2CPP), Unreal (`UE4SS`), and game/emulator bridges (`REFramework`, `ScriptHook`, `RetroArch UDP`), and encompass QoL/script/plugin/asset/config/patch mods as well as trainers. At the same time, L6 AI tooling (`nekod-mcp` + Agent Skills) can act as an optional **AI Mod Forge** that analyzes games and drafts verifiable mod packages—provided runtime verification remains mandatory.
- **Decision:**
  1. **Broaden Platform Positioning & Package Model:** Evolve Nekod from a single-focus Trainer Platform into an **Open-source Community Mod Platform** (`Modding games, together.`). Generalize `TrainerManifest` into `ModManifest` supporting `mod_kind` variants (`trainer`, `script_mod`, `plugin_mod`, `asset_mod`, `configuration_mod`, `patch_mod`, `emulator_mod`) while preserving lossless backwards compatibility for existing v1 `TrainerManifest` packages.
  2. **Five Runtime Adapter Classes Behind Unified Management:** Rather than forcing one execution technology or reimplementing entire upstream mod loaders, define five distinct Runtime Adapter categories behind `RuntimeProvider`:
     - **CE Script Runtime** (static `.CT -> declarative_patch_v1` converter first; constrained AA/Lua subset later)
     - **Dynamic Instrumentation Runtime** (`frida-gum` optional/isolated adapter)
     - **Unity Runtime Bridge** (separate Mono vs. IL2CPP handling across `BepInEx` / `MelonLoader`)
     - **Unreal Engine Runtime Bridge** (`UE4SS` versioned loader discovery & `UObject`/reflection adapter)
     - **Game-Specific Runtime Bridge** (independent per-ecosystem adapters such as `REFramework`, `ScriptHook`, and loopback-only `adapter_ipc_v1` for `RetroArch UDP`).
  3. **Runtime Dependency Manager & Clean Disk Lifecycle:** Model upstream loaders and bridges (`BepInEx`, `UE4SS`, `REFramework`, etc.) as explicit declared dependencies managed through detection, SHA-256 verification, upstream license/redistribution compliance, and `DeploymentLedger` backup/rollback—never blindly bundling license-restricted binaries.
  4. **Compatibility Database & AI Validation Gate:** Track multi-dimensional compatibility tuples `(game, game_version, store_platform, runtime_adapter, mod_version, dependencies, tested_status)`. AI-assisted generation (`AI Mod Forge`) must pass automated validation and runtime rollback verification before receiving verified status; unverified memory offsets or scripts discovered by AI cannot self-assert compatibility.
- **Consequences:** Subsequent protocol work will introduce `ModManifest` v2, dependency specifications, compatibility records, and adapter capability metadata cleanly at the L4 boundary instead of continually patching v1 trainer-only structs, while keeping zero AI dependency for ordinary players.

---

## ADR-020: Homebrew/Scoop-Style Decentralized Script Buckets & `.nmod` Source-First Package Format

- **Status:** Accepted (Architecture Direction for Distribution & Packaging)
- **Context:** Redistributing third-party closed-source `.exe` trainers or copyrighted game asset archives creates severe copyright, license, malware supply-chain, and PR review bottlenecks. By contrast, package ecosystems such as Homebrew (formulas/taps) and Scoop (JSON app manifests/buckets) prove that lightweight, human-readable text recipes referencing verified upstream download URLs + SHA-256 digests scale openly via Git.
- **Decision:**
  1. **Recipe & Script Distribution Over Binary Mirroring:** Nekod operates as a decentralized script/recipe bucket platform rather than a binary mirror for opaque third-party `.exe` trainers or pirated asset packs.
  2. **`.nmod` Standardized Source-First Format:** Encapsulate mod logic as inspectable declarative configurations + lightweight auditable scripts (JSON/TOML manifests, AOB/pointer/config rules, constrained Lua/JS scripts) packaged in `.nmod` bundles.
  3. **Upstream Pinned Coordinates:** Required external frameworks/loaders (`BepInEx`, `UE4SS`, `REFramework`, etc.) are referenced via pinned upstream URLs, explicit `sha256` checksums, and upstream license declarations—resolved and verified locally by the Runtime Dependency Manager rather than bundled illegally or blindly trusted.
- **Consequences:** Resolves four core ecosystem challenges simultaneously:
  - **Copyright & Licensing Compliance:** No unauthorized redistribution of closed-source trainers or copyrighted assets.
  - **Supply-Chain Security:** Every recipe and script line is plain-text inspectable and hash-pinned.
  - **Git PR Reviewability:** Community buckets live cleanly in standard GitHub repositories with line-by-line diffs and CI linting.
  - **AI Mod Forge Native Fit:** AI Agents naturally generate structured `.nmod` text recipes and scripts that humans and automated validators can audit before publication.

---

## ADR-021: On-Demand Plugin Bucket Distribution for Isolated Runtime Workers & Companion Bridge Plugins

- **Status:** Accepted (`V2-M4` / `V2-M5`)
- **Context:** `FridaGumRuntime` (`frida_gum_v1`) requires an isolated local worker process, and `UnityBridgeRuntime` / `Ue4ssBridgeRuntime` / `ManagedFrameworkRuntime` communicate with lightweight in-loader companion plugins over loopback IPC. Bundling these binaries into the core Nekod installer would bloat the player client and trigger false-positive AV heuristics.
- **Decision:** Distribute isolated runtime workers and companion bridge plugins as versioned, SHA-256-pinned plugin packages in a dedicated **Nekod Plugin Bucket** (`DependencyKind::RuntimeBridge` with `DependencyRedistribution::OfficialUpstreamFetch`), fetched and verified on demand by `RuntimeDependencyManager` and staged/rolled back via `DeploymentLedger`.
- **Consequences:** Core player installations remain lightweight, auditable, and zero-bloat; optional dynamic/loader plugins obey the same cryptographic SHA-256 verification and LIFO rollback guarantees as third-party mod recipes.

---

## Summary of Unresolved Engineering Questions

The full register of unresolved questions, trade-offs, and impact timelines lives in [`docs/unresolved-questions.md`](docs/unresolved-questions.md). All initial **U1**–**U11** architectural questions now have accepted ADRs:

*(Resolved in P0–P5: **U1**–**U11**; Platform Expansion ADR-019, `.nmod` Bucket Distribution ADR-020, and Plugin Bucket Distribution ADR-021 recorded for V1.1+/V2 evolution.)*
