---
name: game-discovery
description: Discover installed singleplayer games, assign U11 canonical kebab-case game IDs, and generate or inspect Game Manifest v1 entries safely.
---

# Game Discovery Skill

Optional developer skill for identifying target singleplayer games, defining canonical U11 slugs, and inspecting executable/store environment metadata without triggering prohibited multiplayer targets.

## Inputs
- Target game title, installation path or Steam AppID / GOG / Epic ID
- Host platform (`windows-x64`)

## Allowed MCP Tools (`crates/nekod-mcp`)
- `list_installed_games` (read-only)
- `get_game_info` (read-only)
- `inspect_game_environment` (read-only)

## Security & U11 Governance Rules
1. Canonical `id` MUST use strict lowercase kebab-case (`^[a-z0-9]+(?:-[a-z0-9]+)*$`) per ADR-010 (e.g., `hollow-knight`, `elden-ring`). Never embed Steam/Epic/GOG IDs inside `id`.
2. Store external IDs exclusively inside `store_ids`.
3. Check the community registry index before inventing a new slug so existing IDs are never duplicated or shadowed.
4. Refuse games with active online competitive anti-cheat enforcement (`trainer_policy: prohibited_online_competitive`).

## Workflow
1. Call `list_installed_games` or inspect the local game directory to collect executable filenames and store build IDs.
2. Call `get_game_info` if a manifest already exists locally; check `inspect_game_environment` to verify install path, binary presence, and `compatibility_policy`.
3. Draft `game-manifest.json` adhering to `schemas/game-manifest.v1.schema.json`.
4. Validate locally via CLI:
   ```bash
   cargo run -p nekod-validator -- game path/to/game-manifest.json
   ```

## Human Review Gate
- Confirm canonical U11 slug uniqueness against `Nekod-Community` and verify `compatibility_policy.trainer_policy` accurately reflects offline/singleplayer scope before committing.
