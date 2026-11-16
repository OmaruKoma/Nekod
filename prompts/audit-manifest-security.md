# Prompt Template: Audit Manifest & Package Security

Use this prompt before installing, trusting, or merging a community trainer PR:

> Perform a strict security and protocol audit of the trainer package at `<PACKAGE_DIR>` (read-only).
>
> Checklist:
> 1. Run `validate_trainer_manifest` (`package_dir`) and `inspect_trainer` (`package_dir`) — do NOT write files or grant runtime permissions.
> 2. Verify `compatibility.singleplayer_only == true` and check `compatibility_policy` against the target `GameManifest` via `validate_compatibility`.
> 3. Audit every entry in `permissions[*]`:
>    - Are `allowed_modules` and `max_write_bytes` tightly scoped?
>    - Does `runtime.kind` avoid unapproved `native_provider_v1` modules?
>    - Are player-facing `reason` descriptions accurate?
> 4. Audit `trainer.json` for wildcard-heavy AOB patterns, missing `expected_original_bytes`, or excessive freeze frequencies outside `50..=1000` ms.
> 5. Report PASS / FAIL per check with exact findings and line/field citations.
