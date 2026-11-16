---
name: compatibility-analysis
description: Analyze trainer compatibility across game builds, executables, offline requirements, and incompatible anti-cheat policies.
---

# Compatibility Analysis Skill

Evaluates whether a trainer version matches an installed game's platform, build identifier, offline requirements, and singleplayer anti-cheat policy boundaries.

## Inputs
- `GameManifest` (or registered `game_id`) and `TrainerManifest` (or `trainer_id`)
- Optional detected `store_build_id` or executable SHA-256

## Allowed MCP Tools (`crates/nekod-mcp`)
- `check_game_version` (read-only)
- `validate_compatibility` (read-only)
- `inspect_game_environment` (read-only)

## Policy Rules
1. Games declaring `trainer_policy: prohibited_online_competitive` always reject all trainers.
2. Games declaring `trainer_policy: restricted_offline_only` reject any trainer without `compatibility.requires_offline_mode: true`.
3. Overlapping entries between `game.compatibility_policy.anti_cheat_detected` and `trainer.compatibility.anti_cheat_incompatible` require `requires_offline_mode: true` or reject fails-closed.
4. Unknown build patterns resolve conservatively to `Untested` rather than falsely claiming `Verified`.

## Workflow
1. Inspect detected game environment via `inspect_game_environment`.
2. Run `validate_compatibility` to confirm policy & target `game_id` alignment.
3. Run `check_game_version` with the installed `detected_build` to verify `SupportedGameVersion` matching (`Verified`, `Experimental`, `Broken`, or `Untested`).

## Human Review Gate
- Mark builds `verified` only after live testing activation + deactivation rollback against the exact binary build.
