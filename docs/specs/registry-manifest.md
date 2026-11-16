# Registry Manifest Specification (`1.0.0`)

- **Schema File:** [`schemas/registry-manifest.v1.schema.json`](../../schemas/registry-manifest.v1.schema.json)
- **Canonical `$id`:** `https://nekod.dev/schemas/v1/registry-manifest.schema.json`
- **Rust Model:** [`sdk/manifest/src/registry.rs`](../../sdk/manifest/src/registry.rs)
- **Status:** Draft v1.0.0 (P0)

---

## 1. Purpose

The **Registry Manifest** enables Nekod's hybrid community ecosystem. It allows `Nekod-Community` to index both official repositories and independent third-party GitHub repositories without centralizing ownership or hosting third-party binaries.

---

## 2. Field Reference

### Top-Level Object

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `schema_version` | `string` (SemVer) | Yes | Must match `1.x.x` (e.g. `"1.0.0"`). |
| `repository_id` | `string` (slug) | Yes | Globally unique repository identifier (`^[a-z0-9][a-z0-9._-]{2,127}$`), e.g. `"com.github.nekod-org.official-trainers"`. |
| `name` | `string` | Yes | Human-readable repository title. |
| `description` | `string` | Yes | Summary of the repository's scope and maintained games. |
| `url` | `string` (HTTPS URL) | Yes | Canonical repository URL (e.g. `https://github.com/owner/repo`). |
| `maintainers` | `MaintainerInfo[]` | Yes | Non-empty list of repository maintainers (`name`, `github_handle`, optional `contact`). |
| `trust_tier` | `TrustTier` | Yes | Declared tier: `"official"`, `"community_verified"`, or `"unverified"`. |
| `channel` | `ReleaseChannel` | Yes | `"stable"`, `"beta"`, or `"nightly"`. |
| `feed_type` | `enum` | Yes | How clients discover releases: `"github_releases"` or `"static_index_json"`. |
| `trainers` | `RegistryTrainerEntry[]` | Yes | List of trainer entries published by this repository. |
| `updated_at` | `string` (RFC3339) | Yes | ISO-8601 UTC timestamp of last index update. |
| `extensions` | `object` | No | Optional non-executable metadata reserved for future extensions. |

### `RegistryTrainerEntry`

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `trainer_id` | `string` | Yes | Matches `TrainerManifest.id`. |
| `game_id` | `string` | Yes | Matches `GameManifest.id`. |
| `latest_version` | `string` (SemVer) | Yes | Latest published version of the trainer. |
| `manifest_url` | `string` (HTTPS URL) | Yes | Direct HTTPS URL to the immutable release `trainer-manifest.json`. |
| `manifest_sha256` | `string` (64 hex) | No | Expected SHA-256 digest of the referenced `trainer-manifest.json`. |
| `release_tag` | `string` | No | Git tag corresponding to `latest_version`. |
| `status` | `enum` | Yes | `"active"`, `"deprecated"`, `"archived"`, `"revoked"`. |
| `compatibility_summary` | `CompatibilitySummary` | No | Quick index metadata (`last_verified_game_version`, `status`). |

---

## 3. Trust & Provenance Rules

1. **Client-Side Trust Authority:** A third-party repository **cannot** self-promote to `official` or `community_verified` simply by setting `trust_tier` in its own JSON file. The Nekod client determines effective `TrustTier` from the **authoritative root source**:
   - Entries from the signed/pinned `Nekod-Official-Trainers` source -> `Official`.
   - Entries merged into the curated `Nekod-Community` index -> `CommunityVerified` (unless downgraded or revoked).
   - User-added arbitrary URLs or local files -> `Unverified`.
2. **Revocation:** If `status == "revoked"`, the client must refuse new installations and disable automatic activation of cached copies with a clear security notice.
3. **Transport Security:** `url` and `manifest_url` must use `https://`. Plain HTTP URLs are rejected.
