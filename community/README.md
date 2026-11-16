# L5 — Community Registry & Ecosystem (`community/`)

**Status:** Protocol defined in **P0**, Registry client, artifact verification & CI workflow templates active in **P3**

Nekod uses a hybrid community repository model:
- `Nekod` — Main desktop application, core, runtime, and SDK.
- `Nekod-Community` — Metadata index aggregating official and independent third-party trainer repositories via [`RegistryManifest`](../docs/specs/registry-manifest.md) entries.
- `Nekod-Official-Trainers` — Officially maintained trainers (`official` trust tier).

## Publishing Workflow for Independent Developers
1. Create a trainer using `sdk/trainer-sdk` or compatible tooling.
2. Generate `trainer-manifest.json` conforming to [`schemas/trainer-manifest.v1.schema.json`](../schemas/trainer-manifest.v1.schema.json).
3. Validate locally using `nekod-validator` (`tools/trainer-validator`).
4. Publish source and SHA-256-verified release artifacts in your own GitHub repository.
5. Register your repository metadata (`RegistryManifest`) with `Nekod-Community`.

## Governance Rules
- **Canonical Game IDs (U11):** `GameManifest.id` must be a unique lowercase kebab-case slug (e.g., `hollow-knight`, `skyrim-special-edition`) without storefront prefixes; `Nekod-Community` CI enforces global uniqueness across indexed games.
- **Trust Tiers:** Community-indexed entries default to `community_verified` only via `Nekod-Community` PR review + CI validation; upstream repositories cannot self-promote to `official`.

Developers retain ownership of their repositories; the community index serves as a discovery and metadata verification layer.
