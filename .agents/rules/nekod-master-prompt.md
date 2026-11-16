---
trigger: always_on
description: Nekod Project Master Prompt — architectural principles, six-layer boundaries, U11 game ID governance, security model, runtime strategy, and engineering workflow.
---

> **Current Baseline Sync Note:** U10 has been resolved as **MIT** (`LICENSE`, `Cargo.toml`, `ADR-009`) and U11 as **strict kebab-case canonical slugs** (`ADR-010`). Phase P0 architecture, schemas, and audit remediation are complete; consult `ROADMAP.md` and `TECHNICAL_DECISIONS.md` for live milestone status before starting new work.

# Nekod — Project Master Prompt

## 1. Project Identity

**Project Name:** Nekod  
**Tagline:** Modding games, together.  
**Category:** Open-source Community Game Trainer Platform  
**License:** MIT (resolved in U10 / ADR-009; original draft proposed Apache-2.0)  
**Primary Platform:** Windows x64  
**Primary Language:** Rust  
**Desktop Framework:** Tauri 2 + React + TypeScript

Nekod is an open-source, community-driven game trainer platform.

Its purpose is to make game modification accessible to ordinary players while providing a structured development ecosystem for contributors.

Nekod is not simply a cheat engine, a collection of memory-editing scripts, or an AI-powered game modification tool.

It is a complete platform consisting of:

- A desktop application for players.
- A modular runtime for executing authorized trainer operations.
- A versioned SDK and manifest protocol.
- A distributed community registry.
- Official and third-party trainer repositories.
- Optional AI-assisted developer tooling.

The project must prioritize security, maintainability, extensibility, transparency, and community ownership.

---

## 2. Core Product Philosophy

### 2.1 Two distinct user experiences

Nekod serves two different audiences.

**Ordinary Players**

Players should be able to:

1. Install Nekod.
2. Discover installed games.
3. Browse available trainers.
4. Select a trainer.
5. Enable or disable individual features.
6. Manage trainer versions and configurations.
7. View diagnostics and compatibility information.

Ordinary players must NOT need:

- An AI model.
- An API key.
- An MCP server.
- An AI subscription.
- Developer knowledge.
- A command-line interface.

The player experience should be straightforward and comparable in usability to mainstream trainer applications.

**Developers and Contributors**

Contributors should be able to:

1. Create game manifests.
2. Develop trainers using the Nekod SDK.
3. Validate trainer manifests.
4. Test compatibility.
5. Diagnose runtime behavior.
6. Publish trainers in independent GitHub repositories.
7. Register their repositories in the community index.
8. Use optional AI tools to accelerate development.

AI is a developer productivity enhancement, not a mandatory platform dependency.

### 2.2 Open ecosystem

Nekod must support both official and independent community development.

The official project provides:

- Desktop client.
- Core application logic.
- Runtime.
- SDK.
- Manifest specifications.
- Validation tools.
- Official trainers.
- Community registry specifications.

Third-party developers may maintain independent repositories without requiring their code to be merged into the official repository.

The client aggregates trainer metadata through a unified community registry.

The registry is an index and discovery mechanism, not the exclusive storage location for trainer binaries.

---

## 3. Architectural Principles

Use a modular monolith initially.

Do not introduce microservices without a concrete requirement.

Keep module boundaries explicit and ensure that the architecture can evolve toward separate processes and runtime isolation.

### Six logical layers

#### L1 — Desktop Application

Responsibilities:

- Game library.
- Game detection.
- Trainer discovery.
- Trainer details.
- Feature toggles.
- Version management.
- Configuration.
- Logs and diagnostics.
- Settings and permissions.

Technology:

- Tauri 2.
- React.
- TypeScript.

The frontend must not directly manipulate game memory.

All sensitive operations must pass through the Rust backend and runtime permission system.

#### L2 — Application Core

Responsibilities:

- Game Manager.
- Trainer Manager.
- Profile Manager.
- Registry Client.
- Permission Manager.
- Update Manager.
- Runtime Manager.

This layer coordinates application workflows.

It must not contain platform-specific memory manipulation implementations.

#### L3 — Trainer Runtime

Responsibilities:

- Process discovery.
- Process identity verification.
- Memory operations.
- Patch lifecycle.
- Activation and deactivation.
- Rollback.
- Runtime diagnostics.
- Compatibility checks.

Use a provider abstraction so that different runtime backends can be introduced later.

The initial implementation should prioritize declarative patch operations.

Do not begin with arbitrary native executable trainer plugins.

#### L4 — SDK and Protocol

Responsibilities:

- Game Manifest.
- Trainer Manifest.
- Registry Manifest.
- JSON Schemas.
- Rust SDK.
- Runtime API.
- Permission declarations.
- Compatibility specifications.

All public protocol formats must be versioned.

Schema definitions and Rust types must remain consistent.

#### L5 — Community Registry

Responsibilities:

- Official repository registration.
- Third-party repository registration.
- Trainer discovery.
- Version indexing.
- Repository metadata.
- Integrity verification.
- Trust information.

The registry must support independently maintained GitHub repositories.

Do not make a central server mandatory for all trainer hosting.

#### L6 — AI Developer Tooling

Responsibilities:

- MCP server.
- Agent Skills.
- Prompt templates.
- Manifest validation workflows.
- Compatibility analysis.
- Development assistance.
- Repository diagnostics.

This layer is optional and must remain decoupled from the ordinary player's runtime experience.

---

## 4. Recommended Repository Structure

Maintain a coherent monorepo for the official project.

Current workspace directories map to `apps/desktop/`, `crates/*`, `sdk/*`, `tools/trainer-validator/`, `schemas/`, `community/`, `skills/`, `prompts/`, `docs/`, and `tests/` (with architectural boundaries kept strict as the structure evolves).

Suggested future repositories:

- `Nekod`: Official application and SDK.
- `Nekod-Community`: Community registry and repository index.
- `Nekod-Official-Trainers`: Officially maintained trainers.
- `Nekod-Skills`: AI agent skills.
- `Nekod-Docs`: Documentation and specifications.

Third-party developers may maintain completely independent repositories.

---

## 5. Technical Stack

Prefer the following technologies unless a documented technical reason justifies an alternative.

### Backend

- Rust.
- Tokio.
- Serde.
- Axum only where HTTP services are needed.
- Reqwest.
- SQLite.
- tracing.
- JSON Schema validation.

### Frontend

- Tauri 2.
- React.
- TypeScript.
- A maintainable component architecture.

### Development and CI

- Cargo workspace.
- GitHub Actions.
- Automated tests.
- Formatting checks.
- Clippy.
- Manifest validation.
- Dependency auditing.

Avoid unnecessary dependencies.

Prefer well-maintained libraries and stable APIs.

Do not introduce complex abstractions without a real use case.

---

## 6. Manifest and Protocol Design

Three manifest types must remain separate: `GameManifest`, `TrainerManifest`, and `RegistryManifest`.

### Protocol requirements

- Use JSON Schema Draft 2020-12.
- Version all public schemas.
- Use strict validation by default.
- Provide an explicitly defined `extensions` mechanism.
- Reject unsupported required capabilities.
- Keep metadata declarative.
- Never interpret arbitrary manifest fields as executable instructions.
- Maintain example manifests and automated schema tests.

---

## 7. U11 — Canonical Game ID Namespace

**Decision: Strict kebab-case canonical slugs.**

Examples: `hollow-knight`, `skyrim-special-edition`, `elden-ring`, `cyberpunk-2077`.

The canonical ID is owned by Nekod and must be independent of distribution platforms.

### Rules

1. IDs must use lowercase ASCII letters, digits, and hyphens (`^[a-z0-9]+(?:-[a-z0-9]+)*$`).
2. IDs must not start or end with a hyphen.
3. Consecutive hyphens must be rejected.
4. IDs must be globally unique within the Nekod ecosystem.
5. IDs must be stable across game updates and publisher changes.
6. IDs must never be reassigned to another game.
7. Deleted or deprecated IDs must remain reserved.
8. Similar or ambiguous titles must be distinguished through explicit canonical slugs.
9. Store identifiers must not be embedded in canonical IDs (external identifiers belong in `store_ids`).
10. Game renames must not automatically change canonical IDs.

### Identity governance

Before accepting a new canonical ID:

- Check the existing community registry.
- Check for duplicate games.
- Check for different editions and remasters.
- Check for ambiguous titles.
- Document exceptional naming decisions.

Do not allow independent repositories to silently redefine an existing canonical game ID.

---

## 8. Security Model

Security is a fundamental architectural requirement. Do not treat trainer packages as automatically trustworthy.

### Trust tiers

- `Official`
- `CommunityVerified`
- `Unverified`

Trust levels must be visible to users. Trust status must not automatically grant arbitrary runtime capabilities.

### Capability-based permissions

Initial capability vocabulary:

```text
process.discover
process.read_memory
process.write_memory
process.allocate_memory
process.suspend_resume
filesystem.read_save_dir
filesystem.write_save_dir
runtime.native_module
```

Use default-deny authorization. Every trainer must explicitly declare its required capabilities.

Permission grants must be bound to `(trainer_id, version, manifest_sha256)`. A modified manifest or new trainer version must require fresh authorization.

### Safety requirements

- Never automatically elevate privileges.
- Never execute untrusted native code by default.
- Never grant undeclared capabilities.
- Require explicit user authorization for sensitive operations.
- Validate process identity before memory access.
- Validate process state before applying patches.
- Detect process exit and PID reuse.
- Verify addresses and memory regions.
- Maintain patch rollback records.
- Provide clear diagnostics.
- Fail closed when verification fails.

### PatchLedger

PatchLedger must track:

- Target process identity.
- Memory address.
- Original bytes or value.
- Applied bytes or value.
- Patch identifier.
- Activation state.
- Timestamp.
- Rollback state.

Rollback must not blindly overwrite memory. Before restoring original values:

1. Verify that the process still matches the original identity.
2. Verify that the target address is still valid.
3. Compare current memory against the expected applied value.
4. Restore only when the state is safe.
5. Report conflicts instead of silently overwriting unexpected changes.

Use LIFO rollback ordering where applicable.

### Anti-cheat boundaries

Nekod must not implement anti-cheat bypass, anti-cheat evasion, stealth injection, kernel-level evasion, online multiplayer cheating, or unauthorized account manipulation.

Single-player-only policies must be explicit. If a supported game environment is detected to have incompatible anti-cheat protections, the runtime must abort rather than attempt evasion.

---

## 9. Runtime Architecture

Use a provider abstraction (`RuntimeProvider` in `sdk/runtime-api`), remaining independent of the desktop UI.

Prioritize `declarative_patch_v1`. Do not introduce arbitrary native binaries as the initial trainer execution mechanism.

Define a small, explicit, auditable instruction set. Every operation must have input validation, capability checks, process identity checks, failure handling, rollback semantics, and diagnostic output. Do not expand the instruction set without tests and a security review.

---

## 10. Community and Distribution Model

The registry is an index, not a centralized binary hosting platform. Third-party repositories must be independently maintainable.

- Support GitHub-hosted repositories initially without assuming GitHub is the only future host.
- Handle unavailable repositories gracefully, cache metadata, and respect API rate limits.
- Detect deleted repositories and changed/moved tags.
- Verify downloaded artifact integrity and never silently replace an installed trainer with an unverified artifact.
- `CommunityVerified` must have documented verification criteria.

---

## 11. MCP and AI Developer Tooling

MCP is exclusively an optional developer-facing integration, never a dependency of the desktop player application.

- Separate read-only tools from modifying tools.
- Modifying tools must require explicit authorization and produce inspectable changes.
- Never allow an AI agent to silently publish a trainer, modify registry trust, grant runtime permissions, execute arbitrary native modules, modify protected game processes, or bypass security restrictions.
- Agent Skills must be model-independent.

---

## 12. Desktop UX Principles

- Prioritize clear game library, fast search, simple trainer discovery, clear compatibility status, feature-level controls, visible trust indicators, permission transparency, update management, and useful diagnostics.
- Do not expose MCP, AI configuration, manifest editing, or low-level memory operations in the default player interface.

---

## 13. Testing and Quality Requirements

Run before completing milestones:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Test both protocol edge cases (invalid manifests, unsupported versions, duplicate/invalid canonical IDs, schema-to-Rust consistency) and security failure modes (default-deny permissions, unauthorized memory access, modified manifest hashes, version changes, PID reuse, process exit during patching, invalid addresses, rollback conflicts, repository deletion, artifact integrity failures).

Do not claim security guarantees based solely on unit tests.

---

## 14. Project Roadmap

Respect the `P0`–`P5` roadmap (`ROADMAP.md`):

- **P0:** Architecture and Protocol
- **P1:** Core Application Foundation (game discovery, SQLite persistence, registry client, trainer metadata management, basic Tauri shell, game library UI)
- **P2:** Runtime Foundation (`RuntimeProvider` implementation, declarative patch execution, permission enforcement, `PatchLedger`, diagnostics, rollback testing)
- **P3:** Player Experience (trainer discovery, details, feature toggles, profiles, update management, trust & permission UI)
- **P4:** Developer Ecosystem (stable Trainer SDK, CLI validator, MCP tools, Agent Skills, docs, official examples, contribution workflow)
- **P5:** Community and Expansion (community registry, third-party integration, trust verification, artifact signing/attestations, runtime isolation improvements, cross-platform support)

Do not skip milestones or implement future milestone features prematurely unless a documented dependency requires it.

---

## 15. Engineering Workflow

1. Read existing project documentation before modifying code.
2. Inspect the existing repository structure and Git state.
3. Preserve completed milestone work; do not overwrite working implementations without a clear reason.
4. Do not introduce undocumented architectural changes; keep crate dependencies aligned with the documented architecture.
5. Prefer small, reviewable changes; add tests alongside implementation.
6. Run relevant checks after each meaningful change and report actual command results.
7. Distinguish implemented functionality from placeholders; do not claim a stub is complete.
8. Explicitly document unresolved technical decisions and record protocol/security/compatibility decisions as ADRs.
9. Do not silently weaken security requirements to make tests pass.

### Mandatory milestone reporting format

At the end of each milestone, report:
1. **Summary** (implemented vs. deliberately omitted)
2. **Changed Files**
3. **Architecture Impact** (crate deps, public API, protocol)
4. **Verification** (commands, actual results, known limitations)
5. **Security Review** (capabilities, permissions, attack surface, remaining risks)
6. **Unresolved Decisions** (remaining questions, recommendations, P-next blockers)
7. **Next Milestone** (proposed tasks and acceptance criteria)
