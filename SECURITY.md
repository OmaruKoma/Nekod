# Nekod Security Policy

Security is foundational to Nekod. Game trainers interact with local processes and community-distributed files; therefore Nekod enforces **default-deny execution**, **explicit permissions**, **strict provenance**, and **singleplayer-only scope**.

---

## 1. Core Security Guarantees

1. **Default-Deny for Untrusted Code:**
   - Third-party executable modules (`native_provider_v1`) are treated as untrusted and **disabled by default**.
   - Unvalidated manifests (failing JSON Schema or semantic policy checks) are never loaded or executed.
2. **Explicit Permission Declarations & User Consent:**
   - Trainers must declare every required capability (`process.discover`, `process.read_memory`, `process.write_memory`, `process.allocate_memory`, `process.suspend_resume`, `filesystem.read_save_dir`, `filesystem.write_save_dir`, `runtime.native_module`).
   - Memory-write, allocation, save-directory write, and native-module scopes require explicit user confirmation before activation.
3. **Process & Game Version Verification:**
   - Before attaching, `trainer-runtime` verifies executable identity (process name, resolved path, architecture, and build/SHA-256 constraints when declared).
4. **No Automatic Privilege Elevation:**
   - Nekod never silently requests UAC elevation, installs kernel drivers, or spawns elevated helpers automatically.
5. **No Anti-Cheat Bypass or Online Cheating:**
   - Nekod explicitly prohibits anti-cheat bypass functionality and trainers targeting competitive online multiplayer games.
   - If a known anti-cheat or incompatible online mode is detected, runtime attachment is aborted immediately.
6. **Hash Verification Is Necessary, Not Sufficient:**
   - A valid SHA-256 digest verifies artifact integrity against tampering in transit, **not** author trustworthiness or runtime safety. Trust tiers, source repository provenance, and runtime permission enforcement remain mandatory.
7. **Deterministic Patch Rollback & Audit Logging:**
   - Every memory patch records original bytes in a `PatchLedger` for rollback on deactivation or session stop, and all runtime state changes are written to structured audit logs.

---

## 2. Trust Tiers

| Trust Tier | Meaning | Default Execution Policy |
| :--- | :--- | :--- |
| **`official`** | Maintained in `Nekod-Official-Trainers` with maintainer review & CI verification | Declarative trainers allowed with user permission grant; signed official providers follow policy |
| **`community_verified`** | Indexed in `Nekod-Community`, schema-validated, source-attributed, reviewed via community CI | Declarative trainers allowed with user permission grant; native modules **blocked by default** |
| **`unverified`** | Added directly by user from local disk or an unregistered third-party repository | Displayed with prominent unverified warning; declarative allowed only after explicit consent; native modules **blocked** unless explicitly overridden in advanced settings |

---

## 3. Prohibited Functionality

Contributions, manifests, registry entries, MCP tools, or skills that include any of the following will be rejected or revoked:
- Anti-cheat evasion, driver tampering, signature spoofing, or kernel-level bypasses.
- Trainers targeting competitive online multiplayer games or server-authoritative economies.
- Obfuscated binaries, arbitrary remote script downloaders, or undisclosed network/filesystem access.
- Path traversal (`..`) in manifest entrypoints or artifact paths, or non-HTTPS remote artifact URLs.

---

## 4. Reporting Vulnerabilities

If you discover a security vulnerability in Nekod Core, Runtime, Validator, or an official/indexed trainer:
- **Do not** open a public issue with exploit details or malicious payloads.
- Report privately to the project security maintainers (configure GitHub Private Vulnerability Reporting / security contact before public release).
- For malicious or compromised community trainers, report the `trainer_id` and `repository_id` so the registry entry can be marked `status: "revoked"`.

For the full permission taxonomy and enforcement architecture, see [`docs/security-and-permission-model.md`](docs/security-and-permission-model.md).
