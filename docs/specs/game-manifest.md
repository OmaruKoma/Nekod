# Game Manifest Specification (`1.0.0`)

- **Schema File:** [`schemas/game-manifest.v1.schema.json`](../../schemas/game-manifest.v1.schema.json)
- **Canonical `$id`:** `https://nekod.dev/schemas/v1/game-manifest.schema.json`
- **Rust Model:** [`sdk/manifest/src/game.rs`](../../sdk/manifest/src/game.rs)
- **Status:** Draft v1.0.0 (P0)

---

## 1. Purpose

The **Game Manifest** provides a store-independent, canonical description of a game, its executables across platforms, its known build/version identifiers, and its multiplayer/anti-cheat policy metadata.

It separates *game identity* from *trainer implementations*, allowing multiple community trainers to target the same canonical `game_id` across Steam, GOG, Epic, or custom installations.

---

## 2. Versioning & Compatibility Rules

- `schema_version` uses Semantic Versioning (`MAJOR.MINOR.PATCH`).
- **Major version (`1.x.x`):** Breaking structural changes require incrementing major version and publishing a new schema file (`v2`).
- **Minor/Patch additions:** Additive optional fields inside `1.x.x` or inside the reserved `extensions` object must remain backward-compatible with `1.0.0` parsers.
- Top-level objects set `additionalProperties: false` to prevent silent typos; forward-compatible custom metadata belongs inside `extensions`.

---

## 3. Field Reference

### Top-Level Object

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `schema_version` | `string` (SemVer) | Yes | Must match `1.x.x` (e.g. `"1.0.0"`). |
| `id` | `string` (kebab-slug) | Yes | Globally unique canonical kebab-case game slug (`^[a-z0-9]+(?:-[a-z0-9]+)*$`, 2–128 chars), e.g. `"hollow-knight"` or `"skyrim-special-edition"`. Store prefixes (e.g. `steam:`) are prohibited here; use `store_ids`. |
| `name` | `string` | Yes | Official display title (1–200 chars). |
| `short_name` | `string` | No | Compact display name for narrow UI cards. |
| `platforms` | `TargetPlatform[]` | Yes | Non-empty unique list of supported platforms (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64`, `steamos-x64`). |
| `store_ids` | `StoreIdentifiers` | Yes | Known store identifiers used for automatic discovery. |
| `executables` | `ExecutableSignature[]` | Yes | Non-empty list of expected game binaries by platform. |
| `known_versions` | `KnownGameVersion[]` | Yes | Catalog of known builds/patches (may be empty `[]` for initial entry, populated over time). |
| `compatibility_policy` | `GameCompatibilityPolicy` | Yes | Multiplayer mode, known anti-cheat systems, and Nekod trainer policy. |
| `extensions` | `object` | No | Optional non-executable metadata reserved for future extensions. |

### `StoreIdentifiers`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `steam_app_id` | `integer` (>= 1) | No | Steam AppID (e.g. `367520`). |
| `epic_catalog_item_id` | `string` | No | Epic Games Store catalog item ID. |
| `gog_product_id` | `string` | No | GOG Galaxy product ID. |
| `xbox_product_id` | `string` | No | Microsoft / Xbox PC Store product ID. |
| `itch_id` | `string` | No | itch.io project ID or slug. |
| `custom_install_hints` | `string[]` | No | Optional relative marker files or folder hints for non-store installs. |

### `ExecutableSignature`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `platform` | `TargetPlatform` | Yes | OS/architecture target. |
| `relative_path` | `string` | Yes | Relative path from game install root (must not contain `..` or absolute roots). |
| `process_name` | `string` | Yes | Expected OS process name (e.g. `"hollow_knight.exe"`). |
| `architecture` | `BinaryArch` | Yes | `"x86_64"`, `"x86"`, or `"aarch64"`. |
| `product_name` | `string` | No | Optional PE/ELF version metadata hint. |
| `company_name` | `string` | No | Optional publisher/developer PE metadata hint. |

### `KnownGameVersion`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `version_id` | `string` | Yes | Canonical version/build token (e.g. `"1.5.78.11833"`). |
| `display_version` | `string` | Yes | Player-facing version label. |
| `platform` | `TargetPlatform` | Yes | Platform this build applies to. |
| `store_build_id` | `string` | No | Store build identifier (e.g. Steam `buildid` from `appmanifest_*.acf`). |
| `executable_sha256` | `string` (64 hex) | No | Lowercase SHA-256 of the main game executable for exact identity matching. |
| `notes` | `string` | No | Optional patch notes relevant to trainer compatibility. |

### `GameCompatibilityPolicy`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `multiplayer_mode` | `enum` | Yes | `"singleplayer_only"`, `"coop_offline_capable"`, `"online_multiplayer"`, `"competitive_online"`. |
| `anti_cheat_detected` | `AntiCheatKind[]` | Yes | List of anti-cheat systems shipped with the title (use `[]` if none; values: `"easy_anti_cheat"`, `"battleye"`, `"vanguard"`, `"vac"`, `"denuvo_anti_cheat"`, `"custom"`). |
| `trainer_policy` | `enum` | Yes | `"allowed_singleplayer"`, `"restricted_offline_only"`, `"prohibited_online_competitive"`. |

---

## 4. Validation & Security Invariants

1. `id` must be a strict lowercase kebab-case canonical slug (`^[a-z0-9]+(?:-[a-z0-9]+)*$`); distinct editions with different store IDs or executables must use distinct edition-suffixed slugs (e.g., `skyrim-special-edition`).
2. `relative_path` must be strictly relative and free of `..` / `.` segments, backslashes, drive prefixes, control characters, or Windows reserved device names.
3. Games marked `trainer_policy: "prohibited_online_competitive"` must be rejected by `nekod-core` and `trainer-runtime` if a trainer targets them.
4. Metadata fields (`notes`, `extensions`) must never be interpreted as executable scripts or shell commands.
