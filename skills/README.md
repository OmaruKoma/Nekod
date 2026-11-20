# L6 — Agent Skills (`skills/`)

**Status:** Available (Phase P4 — Optional Developer Tooling)

Optional, model-independent Agent Skills for trainer contributors. End-user players never need Agent Skills, MCP servers, or AI clients.

Available skill modules:
- [`game-discovery`](game-discovery/SKILL.md) — Discover singleplayer games, assign canonical U11 kebab-case IDs, and validate `GameManifest` entries.
- [`trainer-development`](trainer-development/SKILL.md) — Author `declarative_patch_v1` trainers using `nekod-trainer-sdk` builders and minimal permissions.
- [`manifest-validation`](manifest-validation/SKILL.md) — Validate Draft 2020-12 manifests, relative paths, and SHA-256 package digests.
- [`trainer-debugging`](trainer-debugging/SKILL.md) — Diagnose capability denials, PID verification, AOB/pointer drift, and rollback conflicts read-only.
- [`compatibility-analysis`](compatibility-analysis/SKILL.md) — Check build patterns, offline requirements, and anti-cheat compatibility policies.
- [`trainer-publishing`](trainer-publishing/SKILL.md) — Prepare SHA-256-pinned `RegistryTrainerEntry` metadata for human-reviewed GitHub releases.

Each skill documents required inputs, MCP tool scoping (`crates/nekod-mcp`), CLI validation gates (`tools/trainer-validator`), singleplayer safety constraints, and mandatory human review before publication.

## Optional `.nmod` MCP Developer Workflow (`nekod-mcp`)

The optional `nekod-mcp` server (`23` stdio JSON-RPC tools) provides end-to-end `.nmod` v2 developer authoring helpers without changing the player runtime:

- `inspect_unity_assemblies` — Pure-Rust read-only ECMA-335 metadata inspector for Mono (`*_Data/Managed/*.dll`) and IL2CPP (`BepInEx/interop/*.dll`) assemblies (inheritance, subclasses, fields, properties, parameterless methods).
- `probe_unity_bridge` — Strictly read-only loopback probe (`ping`, `list_types`, `get_field`) against a running `NekodUnityBridge` PID (`authorize_process_inspection: true` required).
- `analyze_ct_file` — Static Cheat Engine `.CT` analyzer that extracts convertible AOB/byte patches and flags Lua/pointer-scan entries that require native/bridge implementation.
- `forge_nmod_package` — Source-first `.nmod` (`mod-manifest.json` + optional `game-manifest.json` + runtime entrypoint) generator supporting custom `features`, `entrypoint_payload`, and Mono/IL2CPP flavor selection (`authorize_write: true`).
- `pack_nmod_archive` — Validated `.nmod` ZIP packer (`recompute_checksums` + optional `install_to_local_nmods: true` into `<data_dir>/nmods/`).
- `verify_forged_mod_package` — Records `Experimental` (`ai_draft_unverified`) vs. `Verified` (`automated_runtime_test`) compatibility gates before player/community distribution.
