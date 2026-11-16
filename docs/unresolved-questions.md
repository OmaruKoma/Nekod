# Unresolved Engineering Questions & Technical Risk Register

This register tracks open design questions, trade-offs, and technical risks identified during **P0 (Architecture & Protocol)**. Each item states its target decision milestone, options under consideration, and why premature commitment is avoided now.

---

## 1. Runtime & Memory Engine (Resolved for P2)

### U1 — `declarative_patch_v1` Payload Instruction Set (**Resolved**)
- **Resolution:** Defined in [`docs/specs/declarative-patch.md`](specs/declarative-patch.md), [`schemas/declarative-patch.v1.schema.json`](../schemas/declarative-patch.v1.schema.json), and [ADR-011](../TECHNICAL_DECISIONS.md#adr-011-declarative_patch_v1-bounded-instruction-set-u1). Supports bounded `byte_patch` (with `expected_original_bytes` guard) and typed `write_value` across `module_offset`, `aob_pattern`, and `pointer_chain` (max depth 8).

### U2 — Process Identity Verification & PID Reuse (**Resolved**)
- **Resolution:** Runtime binds every session and `PatchLedger` record to `(pid, process_creation_time_100ns, resolved_executable_path)` via `GetProcessTimes` and `QueryFullProcessImageNameW` ([ADR-012](../TECHNICAL_DECISIONS.md#adr-012-windows-x64-process-identity-pid-reuse-detection--non-elevated-access-policy-u2-u3)).

### U3 — Elevated Game Processes vs. Non-Elevated Nekod (**Resolved**)
- **Resolution:** Nekod strictly refuses automatic elevation and surfaces `RuntimeError::ProcessElevatedAccessDenied` on `ERROR_ACCESS_DENIED`, guiding players to launch the game as standard user ([ADR-012](../TECHNICAL_DECISIONS.md#adr-012-windows-x64-process-identity-pid-reuse-detection--non-elevated-access-policy-u2-u3)).

### U4 — Value Freeze Cadence & Thread Safety (**Resolved**)
- **Resolution:** Clamped `freeze_interval_ms` (`50..=1000` ms, default `100` ms) per active session with process-identity re-verification and clean teardown on stop/exit ([ADR-013](../TECHNICAL_DECISIONS.md#adr-013-bounded-value-freeze-polling-cadence-u4)).

---

## 2. Security, Integrity & Community Ecosystem Questions (Resolved for P3)

### U5 — Artifact Signing & Publisher Identity Beyond SHA-256 (**Resolved**)
- **Resolution:** Enforced cryptographic digest chaining over strict HTTPS ([ADR-014](../TECHNICAL_DECISIONS.md#adr-014-artifact-integrity-chain--publisher-identity-verification-policy-u5)): `RegistryTrainerEntry.manifest_sha256` pins downloaded `trainer-manifest.json` bytes (required for `CommunityVerified` and `Official`), `TrainerManifest.artifacts[*]` pins file size and lowercase SHA-256 of every staged payload including `runtime.entrypoint`, and `TrustTier` stays strictly clamped by client root authority.

### U6 — GitHub API Rate Limits & Registry Hosting Scalability (**Resolved**)
- **Resolution:** Feeds use static `RegistryManifest` JSON documents paired with conditional HTTP (`ETag` / `If-None-Match` and `Last-Modified` / `If-Modified-Since`), `304 Not Modified` short-circuiting, explicit `429`/`Retry-After` handling, and SQLite offline fallback ([ADR-015](../TECHNICAL_DECISIONS.md#adr-015-static-index-feeds--conditional-http-caching-etag-u6)).

### U7 — Handling Upstream Repository Deletion or Tag Mutation (**Resolved**)
- **Resolution:** Tag mutation or upstream asset substitution fails closed on `manifest_sha256` / version / `artifacts[*].sha256` verification; HTTP `404`/`410` records source health without erasing cached metadata or verified local packages, while `status: "revoked"` immediately revokes local execution ([ADR-016](../TECHNICAL_DECISIONS.md#adr-016-upstream-repository-deletion--tag-mutation-protection-u7)).

---

## 3. Cross-Platform & Future Runtime Questions (Resolved for P5)

### U8 — Linux & Steam Deck (Proton/Wine) Process Mapping (**Resolved**)
- **Resolution:** Addressed in [ADR-017](../TECHNICAL_DECISIONS.md#adr-017-cross-platform-linuxsteamos-process-discovery--multi-store-expansion-u8). Uses `/proc/<pid>/stat` startup ticks + `/proc/<pid>/cmdline` Proton Windows-exe extraction + `/proc/<pid>/maps` module range parsing alongside `EpicDetector` and `GogDetector` behind `GameDetector`, without privilege escalation or SteamOS rootfs writes.

### U9 — Sandboxed Scripted Runtime (`wasm_sandbox_v1`) Design (**Resolved**)
- **Resolution:** Selected capability-scoped WebAssembly host mediation (`wasm_sandbox_v1`) over unrestricted native modules in [ADR-018](../TECHNICAL_DECISIONS.md#adr-018-multi-runtime-dispatch--capability-scoped-wasm_sandbox_v1-host-boundary-u9), routed via `CompositeRuntimeRouter` (`RuntimeProvider`) and mediated by `SandboxHostBridge` + `PatchLedger` with strict fuel/write-byte budgets.

---

## 4. Governance & Licensing (Resolved for P0)

### U10 — Repository License Confirmation (**Resolved: MIT**)
- **Resolution:** Confirmed **MIT License** ([`LICENSE`](../LICENSE), [`Cargo.toml`](../Cargo.toml), [ADR-009](../TECHNICAL_DECISIONS.md#adr-009-repository-license-mit)). Third-party trainers retain their own SPDX license declaration in `TrainerManifest.license`.

### U11 — Game ID Namespace Governance (**Resolved: Strict Kebab-Case Canonical Slugs**)
- **Resolution:** `GameManifest.id` uses strict lowercase kebab-case canonical slugs (`^[a-z0-9]+(?:-[a-z0-9]+)*$`, 2–128 chars; e.g., `hollow-knight`, `skyrim-special-edition`) enforced in both JSON Schemas and `nekod-manifest` ([ADR-010](../TECHNICAL_DECISIONS.md#adr-010-canonical-game-id-namespace-governance-strict-kebab-case-slugs)). Store identifiers reside solely in `store_ids`, distinct editions receive distinct slugs, and `Nekod-Community` CI enforces uniqueness.
