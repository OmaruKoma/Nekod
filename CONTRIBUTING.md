# Contributing to Nekod

Thank you for helping build an open, player-first game trainer ecosystem!

Contributions fall into two primary tracks:
1. **Platform & Core Contributions (`Nekod`):** Desktop UI, Rust core, runtime engine, SDK, validator, documentation, and optional developer tooling.
2. **Community Trainer Contributions (`Nekod-Community` & Independent Repositories):** Creating and maintaining game trainers using the Nekod manifest standards.

---

## 1. Platform Contributions (This Repository)

### Ground Rules
- Read [`ARCHITECTURE.md`](ARCHITECTURE.md), [`ROADMAP.md`](ROADMAP.md), [`TECHNICAL_DECISIONS.md`](TECHNICAL_DECISIONS.md), and [`SECURITY.md`](SECURITY.md) before proposing major changes.
- Respect layer boundaries:
  - `apps/desktop` (L1) must remain presentation-only.
  - `crates/nekod-core` and L2 domain crates must not depend on `apps/desktop` or `crates/nekod-mcp`.
  - `sdk/` (L4) defines stable public contracts; never silently break versioned schemas or traits.
- Keep AI tooling (`crates/nekod-mcp`, `skills/`, `prompts/`) strictly optional for end-user builds.

### Development Checks (Rust Workspace)
```bash
cargo check --workspace
cargo test --workspace
```

---

## 2. Trainer Contributions (Community Ecosystem)

Developers do **not** need to hand over ownership of their trainers to Nekod:
1. Author your trainer (preferring `declarative_patch_v1` via `sdk/trainer-sdk`).
2. Generate `trainer-manifest.json` conforming to [`schemas/trainer-manifest.v1.schema.json`](schemas/trainer-manifest.v1.schema.json).
3. Validate locally using `nekod-validator` (`tools/trainer-validator`).
4. Publish your trainer source and release artifacts to your own public GitHub repository with SHA-256 checksums.
5. Submit a `RegistryManifest` entry PR to `Nekod-Community` (see [`community/README.md`](community/README.md)).

### Mandatory Policy Compliance
- Singleplayer / offline-capable scope only (`compatibility.singleplayer_only: true`).
- No anti-cheat bypasses or competitive online game modifications.
- All required permissions explicitly declared with plain-language `reason` strings for players.
- Human review and validation pass required before publication (even when assisted by AI tooling).
