//! Address locator resolution (`module_offset`, `aob_pattern`, `pointer_chain`).

use nekod_manifest::BinaryArch;
use nekod_runtime_api::RuntimeError;
use nekod_trainer_sdk::{AddressLocator, parse_aob_pattern};

use crate::backend::{ModuleInfo, ProcessHandle};

const SCAN_CHUNK_SIZE: usize = 64 * 1024;

/// Finds a loaded module matching `module_name` (case-insensitive on Windows paths).
pub fn find_module<'a>(
    modules: &'a [ModuleInfo],
    module_name: &str,
) -> Result<&'a ModuleInfo, RuntimeError> {
    modules
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(module_name))
        .ok_or_else(|| {
            RuntimeError::AddressResolutionFailed(format!(
                "module '{module_name}' is not loaded in target process"
            ))
        })
}

/// Applies a signed byte offset with checked overflow handling.
pub fn checked_signed_offset(base: u64, offset: i64) -> Result<u64, RuntimeError> {
    let result = if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    };
    result.ok_or_else(|| {
        RuntimeError::AddressResolutionFailed(format!(
            "address arithmetic overflow: 0x{base:X} + ({offset})"
        ))
    })
}

/// Resolves any `AddressLocator` variant into an absolute virtual address in the target process.
pub fn resolve_locator(
    handle: &dyn ProcessHandle,
    arch: BinaryArch,
    locator: &AddressLocator,
) -> Result<u64, RuntimeError> {
    let modules = handle.list_modules()?;
    let module = find_module(&modules, locator.module())?;

    match locator {
        AddressLocator::ModuleOffset { offset, .. } => {
            if *offset >= module.size {
                return Err(RuntimeError::AddressResolutionFailed(format!(
                    "offset 0x{offset:X} exceeds size 0x{:X} of module '{}'",
                    module.size, module.name
                )));
            }
            module.base_address.checked_add(*offset).ok_or_else(|| {
                RuntimeError::AddressResolutionFailed("module offset overflow".to_owned())
            })
        }

        AddressLocator::AobPattern {
            pattern,
            match_index,
            offset,
            ..
        } => {
            let parsed = parse_aob_pattern(pattern)
                .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;
            let found = scan_module_aob(handle, module, &parsed, *match_index)?;
            checked_signed_offset(found, *offset)
        }

        AddressLocator::PointerChain {
            base_offset,
            offsets,
            ..
        } => {
            if *base_offset >= module.size {
                return Err(RuntimeError::AddressResolutionFailed(format!(
                    "pointer_chain base_offset 0x{base_offset:X} exceeds size 0x{:X} of '{}'",
                    module.size, module.name
                )));
            }
            let ptr_width = match arch {
                BinaryArch::X86 => 4usize,
                BinaryArch::X86_64 | BinaryArch::Aarch64 => 8usize,
            };

            let mut cursor = module
                .base_address
                .checked_add(*base_offset)
                .ok_or_else(|| {
                    RuntimeError::AddressResolutionFailed("pointer base overflow".to_owned())
                })?;

            // Each entry except the final displacement dereferences the pointer at `cursor`,
            // then adds the offset for the next hop. The final entry is added to obtain target.
            for (idx, step) in offsets.iter().enumerate() {
                cursor = read_pointer(handle, cursor, ptr_width).map_err(|e| {
                    RuntimeError::AddressResolutionFailed(format!(
                        "pointer_chain dereference step {idx} at 0x{cursor:X}: {e}"
                    ))
                })?;
                if cursor == 0 {
                    return Err(RuntimeError::AddressResolutionFailed(format!(
                        "pointer_chain encountered null pointer at step {idx}"
                    )));
                }
                cursor = checked_signed_offset(cursor, *step)?;
            }

            Ok(cursor)
        }
    }
}

fn read_pointer(
    handle: &dyn ProcessHandle,
    address: u64,
    ptr_width: usize,
) -> Result<u64, RuntimeError> {
    let bytes = handle.read_memory(address, ptr_width)?;
    match ptr_width {
        4 => {
            let raw: [u8; 4] = bytes.try_into().map_err(|_| {
                RuntimeError::AddressResolutionFailed("short 32-bit pointer read".to_owned())
            })?;
            Ok(u32::from_le_bytes(raw) as u64)
        }
        8 => {
            let raw: [u8; 8] = bytes.try_into().map_err(|_| {
                RuntimeError::AddressResolutionFailed("short 64-bit pointer read".to_owned())
            })?;
            Ok(u64::from_le_bytes(raw))
        }
        other => Err(RuntimeError::AddressResolutionFailed(format!(
            "unsupported pointer width {other}"
        ))),
    }
}

fn scan_module_aob(
    handle: &dyn ProcessHandle,
    module: &ModuleInfo,
    pattern: &[Option<u8>],
    target_match_index: u32,
) -> Result<u64, RuntimeError> {
    let regions = handle.list_readable_regions(module.base_address, module.size)?;
    let mod_end = module.base_address.saturating_add(module.size);
    let overlap = pattern.len().saturating_sub(1);
    let mut matches_seen: u32 = 0;

    for region in regions {
        if !region.is_committed || !region.is_readable {
            continue;
        }
        let scan_start = region.base_address.max(module.base_address);
        let scan_end = region.base_address.saturating_add(region.size).min(mod_end);
        if scan_end <= scan_start {
            continue;
        }

        let mut cursor = scan_start;
        while cursor < scan_end {
            let remaining = (scan_end - cursor) as usize;
            let to_read = remaining.min(SCAN_CHUNK_SIZE);
            if to_read < pattern.len() {
                break;
            }
            let buf = handle.read_memory(cursor, to_read)?;
            for rel in 0..=buf.len().saturating_sub(pattern.len()) {
                if pattern_matches(&buf[rel..rel + pattern.len()], pattern) {
                    if matches_seen == target_match_index {
                        return Ok(cursor + rel as u64);
                    }
                    matches_seen = matches_seen.saturating_add(1);
                }
            }
            if to_read == remaining {
                break;
            }
            cursor += (to_read - overlap) as u64;
        }
    }

    Err(RuntimeError::AddressResolutionFailed(format!(
        "AOB pattern in '{}' matched {matches_seen} times (wanted index {target_match_index})",
        module.name
    )))
}

fn pattern_matches(window: &[u8], pattern: &[Option<u8>]) -> bool {
    window
        .iter()
        .zip(pattern.iter())
        .all(|(byte, pat)| pat.is_none_or(|expected| *byte == expected))
}
