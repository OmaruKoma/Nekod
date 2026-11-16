---
name: trainer-debugging
description: Diagnose trainer attachment, capability denials, PID verification, AOB/pointer resolution failures, and PatchLedger rollback conflicts read-only.
---

# Trainer Debugging Skill

Assists contributors in diagnosing runtime failures cleanly through structured diagnostics, permission evaluations, and read-only process discovery without bypassing runtime safety checks.

## Inputs
- Target `game_id` and `trainer_id`
- Failure symptom or `DiagnosticEvent` log messages

## Allowed MCP Tools (`crates/nekod-mcp`)
- `inspect_runtime_diagnostics` (read-only)
- `get_execution_logs` (read-only)
- `inspect_trainer_state` (read-only)
- `inspect_authorized_game_process` (read-only — requires `authorize_process_inspection: true`)
- `check_game_version` (read-only)

## Diagnostic Triage Rules
1. **Permission or Trust Errors:** Check `inspect_trainer_state` for `revoked: true`, `manifest_sha256` drift after an update, or missing explicit player confirmation (`RequiresConfirmation`).
2. **Elevation Mismatch (`ProcessElevatedAccessDenied`):** Verify whether the game runs elevated while Nekod runs as standard user (ADR-012 forbids auto-elevation). Instruct launching the game non-elevated.
3. **AOB / Pointer Failures:** Run `check_game_version` to check build drift, then test signatures offline against a sample module buffer using `nekod_trainer_sdk::test_aob_pattern_in_bytes`.
4. **Rollback Conflicts:** `PatchLedger` aborts restoration if memory bytes were altered externally after activation; inspect competing features or multi-operation collisions rather than forcing blind overwrites.

## Human Review Gate
- Never work around runtime verification errors by disabling `expected_original_bytes`, widening `allowed_modules` unnecessarily, or enabling `allow_third_party_native_modules`.
