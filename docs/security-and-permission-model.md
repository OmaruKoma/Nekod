# Nekod Security and Permission Model

This document defines Nekod's threat model, trust tiers, capability-based permission taxonomy, grant lifecycle, process verification rules, and runtime enforcement invariants.

---

## 1. Threat Model & Non-Goals

### Assets to Protect
- The player's operating system, user files, credentials, and game save data.
- The target game process (preventing unintended crashes, memory corruption, or unrecoverable patches).
- Player account safety (preventing accidental execution against anti-cheat or competitive online titles).

### Threat Vectors Addressed
1. **Malicious or compromised community trainer packages:** Arbitrary native DLLs/EXEs, hidden filesystem reads/writes, remote payload downloaders, or path traversal in bundle entrypoints.
2. **Silent permission creep across updates:** A previously trusted trainer adding `process.allocate_memory` or `runtime.native_module` in `v1.1.0`.
3. **Process misidentification / version mismatch:** Attaching to the wrong process or patching shifted offsets on an updated game binary, corrupting memory or saves.
4. **Tampered manifests or downloads:** MITM or modified release assets not matching published SHA-256 digests.
5. **AI-generated unsafe manifests/trainers:** Contributors using LLM/MCP tooling that outputs invalid paths, over-broad permissions, or disallowed targets.

### Explicit Non-Goals
- Nekod does **not** protect against a malicious local administrator or pre-existing kernel malware on the user's host.
- Nekod does **not** defeat, bypass, or hide from anti-cheat systems.

---

## 2. Trust Tiers & Provenance

Every trainer resolved by `nekod-core` receives an effective `TrustTier` computed by `registry-client` and `permission-manager` (never blindly copied from self-asserted JSON fields):

| Tier | Source | Declarative Trainers (`declarative_patch_v1`) | Native Modules (`native_provider_v1`) |
| :--- | :--- | :--- | :--- |
| **`Official`** | `Nekod-Official-Trainers` (maintainer-reviewed & pinned) | Allowed after explicit user permission confirmation | Allowed only if signed/verified & user-confirmed |
| **`CommunityVerified`** | Indexed in `Nekod-Community` (CI schema + policy validated, source-attributed) | Allowed after explicit user permission confirmation | **Blocked by default** |
| **`Unverified`** | Direct third-party repo URL or local disk import | Allowed only after prominent warning + explicit confirmation | **Blocked** (requires explicit advanced security override) |

> **Provenance Principle:** A matching SHA-256 hash only proves *artifact integrity* relative to a manifest; it never proves an artifact is benign. Provenance (`source_repository`, registry index membership, and trust tier) plus runtime capability enforcement remain mandatory.

---

## 3. Permission Taxonomy

All capabilities requested by a trainer must be declared in `TrainerManifest.permissions`:

| Scope | Risk Level | Description | User Confirmation Required? |
| :--- | :--- | :--- | :--- |
| `process.discover` | Low | Enumerate running OS processes matching `GameManifest` executables | Implicit on trainer activation |
| `process.read_memory` | Low–Medium | Read virtual memory of the verified target game process | Confirmed on first grant |
| `process.write_memory` | **High** | Patch bytes / write values in verified target game process memory | **Always explicit confirmation** |
| `process.allocate_memory` | **Critical** | Allocate executable or writable memory pages inside target process | **Always explicit confirmation + elevated warning** |
| `process.suspend_resume` | Medium | Temporarily suspend/resume game threads during multi-byte patch | Explicit confirmation |
| `filesystem.read_save_dir` | Medium | Read files inside the declared game save/config directory | Explicit confirmation |
| `filesystem.write_save_dir` | **High** | Modify files inside the declared game save/config directory | **Always explicit confirmation** |
| `runtime.native_module` | **Critical** | Load a native dynamic library provider (`native_provider_v1`) | **Denied by default** for non-official trainers |

### Optional Constraints
Each `PermissionDeclaration` may carry `constraints`:
- `max_write_bytes`: Upper bound on single write/patch size.
- `allowed_modules`: Restrict memory operations to specific loaded module names (e.g., `["GameAssembly.dll"]`).
- `path_scope`: Restrict filesystem scopes to specific relative subdirectories.

---

## 4. Permission Grant Lifecycle (`permission-manager`)

1. **Manifest Validation:** `sdk/manifest` + JSON Schema validation runs first. Invalid or missing declarations reject the package immediately.
2. **Policy Evaluation:** `PermissionEvaluator::evaluate(manifest, trust_tier, policy_settings, stored_grant)` checks:
   - Are any declared scopes forbidden for this `TrustTier` under default-deny policy?
   - Is `compatibility.singleplayer_only == true`? (If `false`, deny immediately.)
   - Does any feature declare `mutates_memory: true` without `process.write_memory` in `permissions`? (If yes, deny as inconsistent.)
3. **Grant Binding & Invalidation:**
   - Stored user grants are keyed by `(trainer_id, version, manifest_sha256)` where `manifest_sha256` must be a valid 64-hex SHA-256 digest.
   - Any version bump or manifest edit invalidates prior grants and forces re-review.
4. **Immutable Runtime Snapshot:**
   - Upon approval, `permission-manager` constructs an immutable `GrantedPermissions` token containing both `granted_scopes` and per-scope `PermissionConstraints` (`max_write_bytes`, `allowed_modules`, `path_scope`), passed across `sdk/runtime-api` to `trainer-runtime`.
   - `trainer-runtime` re-checks `GrantedPermissions.allows(scope)` and enforces `constraints_for(scope)` before executing OS operations.
5. **Cross-Manifest Policy Enforcement (`nekod-core`):**
   - `nekod-core` verifies `GameManifest.compatibility_policy.trainer_policy != ProhibitedOnlineCompetitive` and checks `GameManifest.compatibility_policy.anti_cheat_detected` against `TrainerManifest.compatibility` before allowing session start.

---

## 5. Runtime Enforcement & Process Verification (`trainer-runtime`)

Before opening a handle or touching target memory:
1. **Anti-Cheat & Online Policy Check:**
   - Consult `GameManifest.compatibility_policy` and `TrainerManifest.compatibility`.
   - Check running processes/services for declared `anti_cheat_incompatible` markers; if detected, abort immediately with `RuntimeError::AntiCheatDetected`.
2. **Process Identity Verification:**
   - Verify process name, resolved full executable image path (against expected install root/binary name), and CPU architecture (`x86_64` vs `x86` vs `aarch64`).
   - Verify executable SHA-256 or store build ID if specified in `supported_versions` (or surface an explicit mismatch warning requiring user override for untested game patches).
3. **Least-Privilege OS Handle (Windows P2):**
   - Request only the Windows access mask corresponding to granted scopes (e.g. `PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ`, adding `PROCESS_VM_WRITE | PROCESS_VM_OPERATION` only when `process.write_memory` / `process.allocate_memory` is granted).
   - Never elevate privileges automatically (no UAC prompts, no `SeDebugPrivilege` escalation, no kernel drivers).
4. **Deterministic Patch Rollback:**
   - Every memory write appends a `PatchRecord { patch_id, feature_id, address, original_bytes, patched_bytes, timestamp }` to the session's `PatchLedger`.
   - Deactivating a feature or ending a session restores `original_bytes` in reverse chronological order (LIFO) while verifying the process is still alive and target bytes still match `patched_bytes` (or logging a conflict if overwritten).
5. **Audit Trail:**
   - Every permission decision, process attach/detach, feature activation, write failure, and rollback is recorded in structured `tracing` logs.
