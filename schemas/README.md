# Nekod JSON Schemas (`v1`)

This directory contains the canonical, independently versioned JSON Schema (Draft 2020-12) contracts for Nekod L4 manifests.

| Schema | `$id` | Specification |
| :--- | :--- | :--- |
| [`game-manifest.v1.schema.json`](game-manifest.v1.schema.json) | `https://nekod.dev/schemas/v1/game-manifest.schema.json` | [`docs/specs/game-manifest.md`](../docs/specs/game-manifest.md) |
| [`trainer-manifest.v1.schema.json`](trainer-manifest.v1.schema.json) | `https://nekod.dev/schemas/v1/trainer-manifest.schema.json` | [`docs/specs/trainer-manifest.md`](../docs/specs/trainer-manifest.md) |
| [`registry-manifest.v1.schema.json`](registry-manifest.v1.schema.json) | `https://nekod.dev/schemas/v1/registry-manifest.schema.json` | [`docs/specs/registry-manifest.md`](../docs/specs/registry-manifest.md) |

## Design Conventions

1. **Self-Contained Portability:** Each `.v1.schema.json` file bundles its own `$defs` so offline tools, IDE validators, and CI scripts can validate manifests without external network resolution.
2. **Strict Core + Controlled Extensibility:** Core objects use `"additionalProperties": false` to catch typos and untrusted fields; forward-compatible non-executable metadata belongs in the optional `"extensions"` object.
3. **Offline Canonical `$id`s:** `$id` URIs are stable identifiers, not live HTTP endpoints required during validation.
4. **Examples:** Reference examples live in [`examples/`](examples/game-manifest.example.json) and are verified continuously by workspace tests.
