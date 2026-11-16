---
name: trainer-publishing
description: Prepare verified SHA-256 release summaries and RegistryManifest v1 entries for independent GitHub repositories and Nekod-Community submissions.
---

# Trainer Publishing Skill

Prepares reproducible release metadata and CI-verified manifests for publishing a trainer on GitHub and indexing it in a `RegistryManifest` without automated or silent publishing.

## Inputs
- Validated trainer package directory (`package_dir`)
- Immutable HTTPS GitHub Release asset URL (`manifest_url`) and git tag (`tag`, e.g., `v1.0.0`)

## Allowed MCP Tools (`crates/nekod-mcp`)
- `validate_trainer_manifest` (read-only)
- `prepare_trainer_release` (mutating — requires explicit `authorize_write: true`)
- `validate_repository` (read-only)
- `inspect_registry_entry` (read-only)

## Security & Distribution Policy (ADR-014, ADR-015, ADR-016)
1. AI tools NEVER automatically push git tags, publish GitHub releases, submit community PRs, or alter `TrustTier` authority.
2. Release `manifest_url` and `download_archive_url` must use immutable HTTPS release assets (e.g., `/releases/download/vX.Y.Z/...`).
3. `manifest_sha256` MUST match the exact bytes served at `manifest_url`.
4. New third-party submissions default to `unverified`; `community_verified` requires meeting `community/README.md` CI and review criteria.

## Workflow
1. Validate package directory via `validate_trainer_manifest` (`package_dir`) or `nekod-validator package`.
2. Run `prepare_trainer_release` with `authorize_write: true` to emit `release-entry-summary.json` containing canonical `manifest_sha256`.
3. Verify CI workflows (`community/github-actions-trainer-ci.yml` / `community/github-actions-registry-ci.yml`) pass on the repository.
4. Manually upload release artifacts and open the `Nekod-Community` PR with release notes.

## Human Review Gate
- Human maintainer verifies exact uploaded release tag digest against `release-entry-summary.json` before submitting or merging any PR.
