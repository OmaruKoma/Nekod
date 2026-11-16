# Declarative Patch Specification (`declarative_patch_v1` — `1.0.0`)

- **Schema File:** [`schemas/declarative-patch.v1.schema.json`](../../schemas/declarative-patch.v1.schema.json)
- **Canonical `$id`:** `https://nekod.dev/schemas/v1/declarative-patch.schema.json`
- **Rust Model:** [`sdk/trainer-sdk/src/lib.rs`](../../sdk/trainer-sdk/src/lib.rs)
- **Status:** Accepted v1.0.0 (P2)

---

## 1. Purpose

`declarative_patch_v1` defines Nekod's small, explicit, auditable instruction set for declarative memory trainers (`TrainerManifest.runtime.kind == "declarative_patch_v1"`).

By expressing trainer behavior as bounded declarative data rather than native DLLs or Turing-complete scripts, Nekod can statically validate payloads, enforce `PermissionConstraints` (`allowed_modules`, `max_write_bytes`), verify original memory bytes before patching, and guarantee deterministic LIFO rollback via `PatchLedger`.

---

## 2. Top-Level Structure

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `schema_version` | `string` (SemVer `1.x.x`) | Yes | Must match major version `1`. |
| `features` | `DeclarativeFeatureEntry[]` | Yes | Non-empty list mapping `feature_id` (matching `TrainerManifest.features[*].id`) to bounded operations. |

---

## 3. Address Locators (`AddressLocator`)

Every operation locates target memory through one of three bounded variants (`kind`):

1. **`module_offset`**
   - `module` (`string`): Loaded module name (e.g. `"SampleRPG.exe"` or `"GameAssembly.dll"`).
   - `offset` (`integer >= 0`): Byte offset from module base address.
2. **`aob_pattern`**
   - `module` (`string`): Module whose committed memory regions are scanned.
   - `pattern` (`string`): Space-separated hexadecimal bytes with optional `??` wildcards (1–128 bytes).
   - `match_index` (`integer >= 0`, default `0`): Zero-based match selector.
   - `offset` (`integer`, default `0`): Signed displacement from matched address.
3. **`pointer_chain`**
   - `module` (`string`): Base module name.
   - `base_offset` (`integer >= 0`): Offset from module base to initial pointer.
   - `offsets` (`integer[]`, length `1..=8`): Bounded pointer dereference chain (`read_ptr(addr) + offset`).

---

## 4. Operations (`DeclarativeOperation`)

1. **`byte_patch`**
   - `locator`: `AddressLocator` (`module_offset` or `aob_pattern`).
   - `expected_original_bytes` (`string`, optional): Hex byte string checked against current process memory *before* patching. Patch fails closed if bytes differ.
   - `patch_bytes` (`string`, 1–64 bytes): Hex byte string to write. Original bytes are saved in `PatchLedger`.
2. **`write_value`**
   - `locator`: `AddressLocator` (`module_offset`, `aob_pattern`, or `pointer_chain`).
   - `value_type`: `"i32" | "u32" | "i64" | "u64" | "f32" | "f64"`.
   - `value` (`number`, optional): Fixed value for `toggle`/`action` features; omitted when controlled dynamically by player `integer`/`float` sliders.
   - `freeze_interval_ms` (`integer`, optional, `50..=1000`): When set, re-applies the value periodically while active.

---

## 5. Security & Safety Invariants

- All module names must satisfy `PermissionConstraints.allowed_modules` if constrained.
- All write sizes must satisfy `PermissionConstraints.max_write_bytes` (and hard limit `<= 64` bytes per `byte_patch`).
- `process.read_memory` is required for AOB scanning, pointer dereferencing, and pre-patch verification.
- `process.write_memory` is required for applying any `byte_patch` or `write_value`.
- No executable code-cave allocation or arbitrary native jumps are permitted in `declarative_patch_v1`.
