---
name: manifest-validation
description: Validate Game, Trainer, Registry, and declarative patch manifests plus unpacked package directories using Draft 2020-12 JSON Schemas and security policy checks.
---

# Manifest Validation Skill

Performs deterministic schema, semantic, path-containment, and SHA-256 verification before any trainer package or registry index update is ingested or published.

## Inputs
- Path to a manifest JSON file (`game`, `trainer`, `registry`, `patch`) or trainer package directory

## Allowed MCP Tools (`crates/nekod-mcp`)
- `validate_trainer_manifest` (read-only)
- `validate_repository` (read-only)
- `inspect_trainer` (read-only)

## Validation Checklist
1. JSON Schema Draft 2020-12 compliance (`additionalProperties: false` outside `extensions`).
2. U11 canonical slug syntax (`game_id`) and namespaced `trainer_id`.
3. Relative path safety (`runtime.entrypoint`, `artifacts[*].filename` — no `..`, absolute paths, or drive prefixes).
4. Exact byte size and lowercase SHA-256 match for all declared `artifacts[*]`, including `runtime.entrypoint`.
5. `declarative_patch_v1` cross-check: every payload feature matches a `TrainerManifest.features[*].id`, and dynamic `write_value` inputs pair with `integer` or `float` controls.
6. Explicit `singleplayer_only: true` and valid HTTPS `source_repository` / `manifest_url` URLs.

## CLI Gate
```bash
cargo run -p nekod-validator -- package path/to/trainer-dir
cargo run -p nekod-validator -- registry path/to/registry-manifest.json
```

## Human Review Gate
- Review any semantic or policy warnings manually; never relax schema strictness or weaken default-deny permission rules to force validation to pass.
