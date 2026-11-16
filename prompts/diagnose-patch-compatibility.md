# Prompt Template: Diagnose Patch & Version Compatibility

Use this prompt when an existing trainer fails to attach, find signatures, or roll back patches after a game update:

> Diagnose why trainer `<TRAINER_ID>` failed for `<GAME_ID>` without disabling any Nekod safety checks.
>
> Steps:
> 1. Read `inspect_trainer_state`, `inspect_runtime_diagnostics`, and `get_execution_logs` (all read-only).
> 2. Run `check_game_version` with the user's current `detected_build` and `validate_compatibility`.
> 3. Only if process matching needs verification, ask me for confirmation and pass `authorize_process_inspection: true` to `inspect_authorized_game_process`.
> 4. Distinguish between:
>    - Revocation or unconfirmed `(trainer_id, version, manifest_sha256)` permission grants
>    - Elevated target game (`ERROR_ACCESS_DENIED` / non-elevation policy)
>    - Game build AOB/pointer drift vs. `PatchLedger` rollback conflicts
> 5. Recommend targeted, minimal fixes to `supported_versions` or `declarative_patch_v1` locators.
