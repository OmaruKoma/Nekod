---
name: trainer-development
description: Author policy-compliant declarative_patch_v1 trainers using nekod-trainer-sdk builders, bounded AOB/pointer locators, and explicit capability declarations.
---

# Trainer Development Skill

Guides contributors through authoring an auditable `declarative_patch_v1` trainer package (`trainer-manifest.json` + `trainer.json`) with minimal permissions and deterministic LIFO rollback support.

## Inputs
- Canonical target `game_id` (kebab-case) and main module filename (e.g., `Game.exe`)
- Verified singleplayer feature requirements (`byte_patch` or scalar `write_value`)
- Target output workspace directory

## Allowed MCP Tools (`crates/nekod-mcp`)
- `get_game_info` (read-only)
- `generate_trainer_template` (mutating — requires explicit `authorize_write: true`)
- `create_trainer_manifest` (mutating — requires explicit `authorize_write: true`)
- `validate_trainer_manifest` (read-only)

## Security & Policy Rules
1. Always set `compatibility.singleplayer_only: true`.
2. Use `runtime.kind: declarative_patch_v1`; do not scaffold `native_provider_v1` binaries by default.
3. Declare minimum required `permissions` (`process.discover`, `process.read_memory`, `process.write_memory`) constrained with `allowed_modules` and `max_write_bytes`, and provide plain-language `reason` strings for ordinary players.
4. Never include anti-cheat bypass, stealth injection, or non-deterministic wildcard-only AOB signatures.
5. Recompute `artifacts[*].sha256` and `size_bytes` via `nekod-trainer-sdk::build_trainer_package` after every payload edit.

## Workflow
1. Verify game metadata with `get_game_info`.
2. Scaffold the package with `generate_trainer_template` (`authorize_write: true`) or author directly via `TrainerManifestBuilder` and `DeclarativePatchBuilder`.
3. Replace placeholder AOB patterns, pointer chains (depth `1..=8`), and `expected_original_bytes` guards with verified reverse-engineered signatures. Test AOB patterns offline with `nekod_trainer_sdk::test_aob_pattern_in_bytes`.
4. Run full package validation:
   ```bash
   cargo run -p nekod-validator -- package path/to/trainer-dir
   ```

## Human Review Gate
- Inspect generated diffs in `trainer-manifest.json` and `trainer.json`, test every toggle's activation and rollback on a real singleplayer build, and confirm permission scopes match actual memory writes.
