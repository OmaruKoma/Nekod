# Trainer Manifest Specification (`1.0.0`)

- **Schema File:** [`schemas/trainer-manifest.v1.schema.json`](../../schemas/trainer-manifest.v1.schema.json)
- **Canonical `$id`:** `https://nekod.dev/schemas/v1/trainer-manifest.schema.json`
- **Rust Model:** [`sdk/manifest/src/trainer.rs`](../../sdk/manifest/src/trainer.rs)
- **Status:** Draft v1.0.0 (P0)

---

## 1. Purpose

The **Trainer Manifest** is the authoritative declaration of a trainer package's identity, provenance, target game compatibility, runtime requirements, explicit permissions, player-facing features, and artifact checksums.

Nekod **never** loads or executes a trainer whose manifest fails schema validation or policy checks.

---

## 2. Field Reference

### Top-Level Object

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `schema_version` | `string` (SemVer) | Yes | Must match `1.x.x` (e.g. `"1.0.0"`). |
| `id` | `string` (slug) | Yes | Namespaced trainer ID (`^[a-z0-9][a-z0-9._-]{2,127}$`), e.g. `"org.nekod.official.hollow-knight"`. |
| `version` | `string` (SemVer) | Yes | Trainer release version (e.g. `"1.0.0"`). |
| `name` | `string` | Yes | Player-facing trainer title. |
| `description` | `string` | Yes | Summary of trainer capabilities and caveats. |
| `authors` | `AuthorInfo[]` | Yes | Non-empty list of authors (`name`, optional `github`, optional `url`). |
| `license` | `string` | Yes | SPDX license identifier or expression (e.g. `"MIT"`, `"Apache-2.0"`). |
| `source_repository` | `string` (HTTPS URL) | Yes | Public source repository URL for transparency and auditability. |
| `homepage` | `string` (HTTPS URL) | No | Optional documentation or project homepage. |
| `target_game` | `TargetGameRef` | Yes | Target `game_id` (matching `GameManifest.id`), `game_name`, and optional `store_ids`. |
| `supported_versions` | `SupportedGameVersion[]` | Yes | Non-empty list of game builds/versions supported by this trainer. |
| `runtime` | `RuntimeRequirement` | Yes | Execution engine kind, API version, entrypoint, and supported platforms. |
| `permissions` | `PermissionDeclaration[]` | Yes | Non-empty list of explicit capability requests with user-facing reasons. |
| `features` | `TrainerFeature[]` | Yes | Non-empty list of player-facing cheats/toggles/actions. |
| `compatibility` | `TrainerCompatibility` | Yes | Singleplayer constraint, offline requirement, and anti-cheat conflicts. |
| `artifacts` | `TrainerArtifact[]` | Yes | Non-empty list of packaged files and their SHA-256 digests. |
| `extensions` | `object` | No | Optional non-executable metadata reserved for future extensions. |

### `SupportedGameVersion`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `platform` | `TargetPlatform` | Yes | OS/architecture target. |
| `version_pattern` | `string` | Yes | Exact version, build ID, or SemVer constraint supported. |
| `executable_sha256` | `string[]` | No | Verified game binary SHA-256 hashes (64 hex chars each). |
| `status` | `enum` | Yes | `"verified"`, `"experimental"`, `"broken"`, `"untested"`. |

### `RuntimeRequirement`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `kind` | `RuntimeKind` | Yes | `"declarative_patch_v1"`, `"wasm_sandbox_v1"`, `"native_provider_v1"`. |
| `api_version` | `string` (SemVer) | Yes | Target `sdk/runtime-api` version (e.g. `"1.0.0"`). |
| `min_nekod_version` | `string` (SemVer) | Yes | Minimum compatible Nekod client version. |
| `entrypoint` | `string` | Yes | Relative path inside the trainer package (no `..` or absolute paths). |
| `supported_platforms` | `TargetPlatform[]` | Yes | Non-empty unique list of platforms supported by this trainer. |

> **Security Rule:** `native_provider_v1` is treated as untrusted executable code and **disabled by default** for third-party/unverified trainers.

### `PermissionDeclaration`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `scope` | `PermissionScope` | Yes | One of: `process.discover`, `process.read_memory`, `process.write_memory`, `process.allocate_memory`, `process.suspend_resume`, `filesystem.read_save_dir`, `filesystem.write_save_dir`, `runtime.native_module`. |
| `reason` | `string` | Yes | Plain-language explanation shown to the player before consent. |
| `required` | `boolean` | Yes | Whether the trainer cannot start without this permission. |
| `constraints` | `PermissionConstraints` | No | Optional bounds (`max_write_bytes`, `allowed_modules`, `path_scope`). |

### `TrainerFeature`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `id` | `string` (slug) | Yes | Unique feature ID within the trainer. |
| `name` | `string` | Yes | Player-facing toggle/action label (e.g. `"Infinite Health"`). |
| `description` | `string` | Yes | Plain-language description of what the feature does and any side effects. |
| `category` | `enum` | Yes | `"player"`, `"inventory"`, `"world"`, `"time"`, `"economy"`, `"camera"`, `"other"`. |
| `control_type` | `enum` | Yes | `"toggle"`, `"action"`, `"integer"`, `"float"`. |
| `default_hotkey` | `string` | No | Default hotkey combo (e.g. `"F1"`, `"Ctrl+Shift+1"`). |
| `value_range` | `FeatureValueRange` | No | `{ min, max, step, default }` for `integer` or `float` controls. |
| `mutates_memory` | `boolean` | Yes | Whether activating this feature writes to process memory. |
| `supports_rollback` | `boolean` | Yes | Whether original state/bytes are restored on deactivation. |
| `risk_level` | `enum` | Yes | `"low"`, `"medium"`, `"high"`. |

### `TrainerCompatibility`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `singleplayer_only` | `boolean` | Yes | Must be `true` for standard Nekod acceptance. |
| `requires_offline_mode` | `boolean` | Yes | `true` for games with optional online modes requiring offline play. |
| `anti_cheat_incompatible` | `AntiCheatKind[]` | Yes | Known anti-cheat systems that block execution if active. |
| `last_verified_at` | `string` (date/RFC3339) | No | Optional ISO-8601 date/timestamp of last verification. |

### `TrainerArtifact`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `filename` | `string` | Yes | Relative path within the release package (no `..` or absolute roots). |
| `sha256` | `string` (64 hex) | Yes | Lowercase SHA-256 digest of the file. |
| `size_bytes` | `integer` (>= 1) | Yes | Exact byte size of the artifact. |
| `platform` | `TargetPlatform` | No | Optional platform restriction if artifact is platform-specific. |

---

## 3. Consistency Rules (Enforced by `trainer-validator` & `permission-manager`)

1. If any feature has `mutates_memory: true`, `permissions` must declare `process.write_memory`.
2. If `runtime.kind == "native_provider_v1"`, `permissions` must explicitly declare `runtime.native_module`.
3. `runtime.entrypoint` must match one of the entries in `artifacts[*].filename` (or be verified inside the declared bundle).
4. `compatibility.singleplayer_only` must be `true`; manifests declaring `false` are rejected by community CI and default client policy.
5. `source_repository` must use `https://`.
