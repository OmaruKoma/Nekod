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
