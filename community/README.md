# L5 — Community Registry & Decentralized Mod Buckets (`community/`)

**Status:** Registry protocol & CI checks active in **P3**; evolving toward Homebrew/Scoop-style `.nmod` recipe buckets ([ADR-020](../TECHNICAL_DECISIONS.md#adr-020-homebrewscoop-style-decentralized-script-buckets--nmod-source-first-package-format))

Nekod operates as a decentralized **script & recipe bucket ecosystem** (inspired by Homebrew taps and Scoop buckets) rather than a central host mirroring closed-source `.exe` trainers or copyrighted assets:
- `Nekod` — Main desktop application, core, multi-engine runtime, and SDK.
- `Nekod-Community` — Metadata index aggregating official and independent GitHub **Mod Buckets** via [`RegistryManifest`](../docs/specs/registry-manifest.md) entries.
- `Nekod-Official-Trainers` — Officially maintained declarative/script packages (`official` trust tier).

## Core Distribution Principle (Source-First `.nmod` Buckets)
- **Readable Recipes, Not Opaque `.exe` Mirrors:** Community packages store human-readable `ModManifest` / `TrainerManifest` declarations, AOB/pointer/config rules, and lightweight scripts (Lua/JS), enabling Git line-by-line PR review, supply-chain auditability, and AI Mod Forge generation.
- **Pinned Upstream Dependencies:** External loaders (`BepInEx`, `UE4SS`, `REFramework`, etc.) are declared with pinned upstream coordinates + SHA-256 checksums + license metadata rather than illegally bundled binaries.

## Publishing Workflow for Independent Developers
1. Create a package using `sdk/trainer-sdk`, `nekod-validator import-ct`, or AI Mod Forge tooling, targeting `.nmod` source-first conventions.
2. Generate `trainer-manifest.json` (evolving to `mod-manifest.json` in V2-M1) conforming to versioned JSON Schemas.
3. Validate locally using `nekod-validator` (`tools/trainer-validator`).
4. Host the text recipe/script package and SHA-256-verified metadata in your own GitHub bucket repository.
5. Register your repository metadata (`RegistryManifest`) with `Nekod-Community`.

## Governance Rules
- **Canonical Game IDs (U11):** `GameManifest.id` must be a unique lowercase kebab-case slug (e.g., `hollow-knight`, `skyrim-special-edition`) without storefront prefixes; `Nekod-Community` CI enforces global uniqueness across indexed games.
- **Trust Tiers:** Community-indexed entries default to `community_verified` only via `Nekod-Community` PR review + CI validation; upstream repositories cannot self-promote to `official`.

Developers retain ownership of their repositories; the community index serves as a discovery and metadata verification layer.
