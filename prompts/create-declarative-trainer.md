# Prompt Template: Create Declarative Trainer

Use this prompt with any MCP-compatible coding assistant after installing `nekod-mcp` (and optionally loading the `trainer-development` skill):

> Help me create a policy-compliant `declarative_patch_v1` Nekod trainer package for `<GAME_NAME>` (`game_id`: `<KEBAB_CASE_SLUG>`).
>
> Requirements:
> 1. Inspect the game metadata first using `get_game_info` / `inspect_game_environment` (read-only). Refuse if the game is prohibited online competitive.
> 2. Scaffold an initial package in `<OUTPUT_DIR>` using `generate_trainer_template` (I explicitly authorize local file writes via `authorize_write: true`).
> 3. Use minimal permissions (`process.discover`, `process.read_memory`, `process.write_memory`) bounded by `allowed_modules` and `max_write_bytes`, with clear player-facing `reason` strings.
> 4. Ensure every `byte_patch` includes `expected_original_bytes` where static bytes are patched, and every `aob_pattern` includes concrete opcodes.
> 5. After any edits, verify the full package with `validate_trainer_manifest` (`package_dir`) and `validate_compatibility`, and summarize the exact diff for my manual review.
