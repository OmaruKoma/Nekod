//! Windows x64 `ProcessMemoryBackend` implementation using `windows-sys`.
//!
//! Strictly obeys ADR-012: least-privilege `OpenProcess` flags, no automatic
//! elevation or `SeDebugPrivilege` acquisition, explicit `ERROR_ACCESS_DENIED`
//! reporting, and `GetProcessTimes` + full image path PID-reuse protection.

use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;

use nekod_manifest::{BinaryArch, PermissionScope, TargetPlatform};
use nekod_runtime_api::{ProcessIdentity, RuntimeError};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_BAD_LENGTH, ERROR_INVALID_PARAMETER, FILETIME,
    GetLastError, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FlushInstructionCache, ReadProcessMemory, WriteProcessMemory,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
    PAGE_WRITECOPY, VirtualProtectEx, VirtualQueryEx,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, IsWow64Process, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
    QueryFullProcessImageNameW,
};

use crate::backend::{
    MemoryRegionInfo, ModuleInfo, ProcessAccessMode, ProcessHandle, ProcessMemoryBackend,
};

struct OwnedHandle(HANDLE);

// Win32 kernel process/snapshot HANDLEs are valid across threads within the owning process.
unsafe impl Send for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Real Windows x64 process & virtual memory backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct Win32ProcessBackend;

impl Win32ProcessBackend {
    pub fn new() -> Self {
        Self
    }
}

impl ProcessMemoryBackend for Win32ProcessBackend {
    fn enumerate_processes(&self) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            let err = unsafe { GetLastError() };
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS) failed with Win32 error {err}"
            )));
        }
        let _snap_guard = OwnedHandle(snap);

        let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        let mut results = Vec::new();
        let mut has_next = unsafe { Process32FirstW(snap, &mut entry) } != 0;
        while has_next {
            let pid = entry.th32ProcessID;
            if pid > 4 {
                let exe_name = utf16_slice_to_string(&entry.szExeFile);
                if let Some(identity) = inspect_process_identity_non_elevated(pid, &exe_name) {
                    results.push(identity);
                }
            }
            entry = unsafe { zeroed() };
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            has_next = unsafe { Process32NextW(snap, &mut entry) } != 0;
        }

        Ok(results)
    }

    fn open_process(
        &self,
        pid: u32,
        mode: ProcessAccessMode,
    ) -> Result<Box<dyn ProcessHandle>, RuntimeError> {
        let access = match mode {
            ProcessAccessMode::ReadOnly => PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            ProcessAccessMode::ReadWrite => {
                PROCESS_QUERY_LIMITED_INFORMATION
                    | PROCESS_VM_READ
                    | PROCESS_VM_WRITE
                    | PROCESS_VM_OPERATION
            }
        };

        let raw = unsafe { OpenProcess(access, 0, pid) };
        if raw.is_null() {
            let code = unsafe { GetLastError() };
            return Err(map_open_process_error(pid, code));
        }

        Ok(Box::new(Win32ProcessHandle {
            pid,
            mode,
            handle: OwnedHandle(raw),
        }))
    }
}

struct Win32ProcessHandle {
    pid: u32,
    mode: ProcessAccessMode,
    handle: OwnedHandle,
}

impl ProcessHandle for Win32ProcessHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn verify_identity(&self, expected: &ProcessIdentity) -> Result<(), RuntimeError> {
        if expected.pid != self.pid {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "handle PID {} does not match expected PID {}",
                self.pid, expected.pid
            )));
        }

        if !is_process_still_active(self.handle.0) {
            return Err(RuntimeError::ProcessUnavailable(self.pid));
        }

        if let Some(expected_creation) = expected.process_creation_time_100ns {
            let actual_creation = query_process_creation_time_100ns(self.handle.0)?;
            if actual_creation != expected_creation {
                return Err(RuntimeError::ProcessIdentityMismatch(format!(
                    "PID {} reused: expected creation timestamp {expected_creation}, observed {actual_creation}",
                    self.pid
                )));
            }
        }

        let actual_path = query_full_image_name(self.handle.0)?;
        if !expected.executable_path.is_empty()
            && !actual_path.eq_ignore_ascii_case(&expected.executable_path)
        {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "process executable changed from '{}' to '{actual_path}'",
                expected.executable_path
            )));
        }

        Ok(())
    }

    fn list_modules(&self) -> Result<Vec<ModuleInfo>, RuntimeError> {
        let mut snap = INVALID_HANDLE_VALUE;
        for _ in 0..4 {
            snap = unsafe {
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid)
            };
            if snap != INVALID_HANDLE_VALUE && !snap.is_null() {
                break;
            }
            let err = unsafe { GetLastError() };
            if err != ERROR_BAD_LENGTH {
                return Err(map_open_process_error(self.pid, err));
            }
        }
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return Err(RuntimeError::AddressResolutionFailed(format!(
                "could not snapshot modules for pid {}",
                self.pid
            )));
        }
        let _snap_guard = OwnedHandle(snap);

        let mut me: MODULEENTRY32W = unsafe { zeroed() };
        me.dwSize = size_of::<MODULEENTRY32W>() as u32;
        let mut modules = Vec::new();
        let mut has_next = unsafe { Module32FirstW(snap, &mut me) } != 0;
        while has_next {
            let name = utf16_slice_to_string(&me.szModule);
            modules.push(ModuleInfo {
                name,
                base_address: me.modBaseAddr as u64,
                size: me.modBaseSize as u64,
            });
            me = unsafe { zeroed() };
            me.dwSize = size_of::<MODULEENTRY32W>() as u32;
            has_next = unsafe { Module32NextW(snap, &mut me) } != 0;
        }
        Ok(modules)
    }

    fn list_readable_regions(
        &self,
        module_base: u64,
        module_size: u64,
    ) -> Result<Vec<MemoryRegionInfo>, RuntimeError> {
        let mut regions = Vec::new();
        let mod_end = module_base.saturating_add(module_size);
        let mut cursor = module_base;

        while cursor < mod_end {
            let mbi = virtual_query(self.handle.0, cursor)?;
            let region_base = mbi.BaseAddress as u64;
            let region_size = mbi.RegionSize as u64;
            if region_size == 0 {
                break;
            }
            let info = mbi_to_region_info(&mbi);
            if info.is_committed && info.is_readable {
                regions.push(info);
            }
            let next = region_base.saturating_add(region_size);
            if next <= cursor {
                break;
            }
            cursor = next;
        }

        Ok(regions)
    }

    fn query_region(&self, address: u64) -> Result<MemoryRegionInfo, RuntimeError> {
        let mbi = virtual_query(self.handle.0, address)?;
        Ok(mbi_to_region_info(&mbi))
    }

    fn read_memory(&self, address: u64, len: usize) -> Result<Vec<u8>, RuntimeError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; len];
        let mut bytes_read: usize = 0;
        let ok = unsafe {
            ReadProcessMemory(
                self.handle.0,
                address as *const _,
                buf.as_mut_ptr().cast(),
                len,
                &mut bytes_read,
            )
        };
        if ok == 0 || bytes_read != len {
            let err = unsafe { GetLastError() };
            return Err(RuntimeError::AddressResolutionFailed(format!(
                "ReadProcessMemory(0x{address:X}, {len}) failed (read {bytes_read}, win32 {err})"
            )));
        }
        Ok(buf)
    }

    fn write_memory(&self, address: u64, bytes: &[u8]) -> Result<(), RuntimeError> {
        if self.mode != ProcessAccessMode::ReadWrite {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }
        if bytes.is_empty() {
            return Ok(());
        }

        let region = self.query_region(address)?;
        if !region.is_committed || !region.contains_range(address, bytes.len()) {
            return Err(RuntimeError::AddressResolutionFailed(format!(
                "target range 0x{address:X}..+{} is not inside a committed page",
                bytes.len()
            )));
        }

        let mut old_protect: u32 = 0;
        let needs_protect_flip = !region.is_writable;
        if needs_protect_flip {
            let flipped = unsafe {
                VirtualProtectEx(
                    self.handle.0,
                    address as *const _,
                    bytes.len(),
                    PAGE_EXECUTE_READWRITE,
                    &mut old_protect,
                )
            };
            if flipped == 0 {
                let err = unsafe { GetLastError() };
                if err == ERROR_ACCESS_DENIED {
                    return Err(RuntimeError::ProcessElevatedAccessDenied);
                }
                return Err(RuntimeError::AddressResolutionFailed(format!(
                    "VirtualProtectEx failed at 0x{address:X} (win32 {err})"
                )));
            }
        }

        let mut bytes_written: usize = 0;
        let write_ok = unsafe {
            WriteProcessMemory(
                self.handle.0,
                address as *const _,
                bytes.as_ptr().cast(),
                bytes.len(),
                &mut bytes_written,
            )
        };
        let write_err = unsafe { GetLastError() };

        if needs_protect_flip {
            let mut unused: u32 = 0;
            unsafe {
                VirtualProtectEx(
                    self.handle.0,
                    address as *const _,
                    bytes.len(),
                    old_protect,
                    &mut unused,
                );
            }
        }

        if write_ok == 0 || bytes_written != bytes.len() {
            if write_err == ERROR_ACCESS_DENIED {
                return Err(RuntimeError::ProcessElevatedAccessDenied);
            }
            return Err(RuntimeError::AddressResolutionFailed(format!(
                "WriteProcessMemory(0x{address:X}, {}) failed (wrote {bytes_written}, win32 {write_err})",
                bytes.len()
            )));
        }

        unsafe {
            FlushInstructionCache(self.handle.0, address as *const _, bytes.len());
        }

        Ok(())
    }
}

fn inspect_process_identity_non_elevated(pid: u32, fallback_name: &str) -> Option<ProcessIdentity> {
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        // Gracefully skip protected / elevated / exited processes during discovery.
        return None;
    }
    let handle = OwnedHandle(raw);
    if !is_process_still_active(handle.0) {
        return None;
    }

    let executable_path = query_full_image_name(handle.0).ok()?;
    let process_creation_time_100ns = query_process_creation_time_100ns(handle.0).ok();
    let architecture = query_process_arch(handle.0);

    let process_name = std::path::Path::new(&executable_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| fallback_name.to_owned());

    Some(ProcessIdentity {
        pid,
        process_name,
        executable_path,
        platform: TargetPlatform::WindowsX64,
        architecture,
        executable_sha256: None,
        detected_version: None,
        process_creation_time_100ns,
    })
}

fn is_process_still_active(handle: HANDLE) -> bool {
    let mut exit_code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    ok != 0 && exit_code == STILL_ACTIVE as u32
}

fn query_process_creation_time_100ns(handle: HANDLE) -> Result<u64, RuntimeError> {
    let mut creation: FILETIME = unsafe { zeroed() };
    let mut exit_t: FILETIME = unsafe { zeroed() };
    let mut kernel_t: FILETIME = unsafe { zeroed() };
    let mut user_t: FILETIME = unsafe { zeroed() };
    let ok = unsafe {
        GetProcessTimes(
            handle,
            &mut creation,
            &mut exit_t,
            &mut kernel_t,
            &mut user_t,
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(RuntimeError::ProcessIdentityMismatch(format!(
            "GetProcessTimes failed (win32 {err})"
        )));
    }
    Ok(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

fn query_full_image_name(handle: HANDLE) -> Result<String, RuntimeError> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(RuntimeError::ProcessIdentityMismatch(format!(
            "QueryFullProcessImageNameW failed (win32 {err})"
        )));
    }
    Ok(OsString::from_wide(&buf[..len as usize])
        .to_string_lossy()
        .into_owned())
}

fn query_process_arch(handle: HANDLE) -> BinaryArch {
    let mut is_wow64: i32 = 0;
    let ok = unsafe { IsWow64Process(handle, &mut is_wow64) };
    if ok != 0 && is_wow64 != 0 {
        BinaryArch::X86
    } else {
        BinaryArch::X86_64
    }
}

fn virtual_query(handle: HANDLE, address: u64) -> Result<MEMORY_BASIC_INFORMATION, RuntimeError> {
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
    let ret = unsafe {
        VirtualQueryEx(
            handle,
            address as *const _,
            &mut mbi,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if ret == 0 {
        let err = unsafe { GetLastError() };
        return Err(RuntimeError::AddressResolutionFailed(format!(
            "VirtualQueryEx(0x{address:X}) failed (win32 {err})"
        )));
    }
    Ok(mbi)
}

fn mbi_to_region_info(mbi: &MEMORY_BASIC_INFORMATION) -> MemoryRegionInfo {
    let base_prot = mbi.Protect & 0xFF;
    let is_guard_or_noaccess =
        (mbi.Protect & PAGE_GUARD) != 0 || base_prot == PAGE_NOACCESS || base_prot == 0;
    let is_committed = mbi.State == MEM_COMMIT;
    let is_readable = is_committed
        && !is_guard_or_noaccess
        && matches!(
            base_prot,
            PAGE_READONLY
                | PAGE_READWRITE
                | PAGE_WRITECOPY
                | PAGE_EXECUTE_READ
                | PAGE_EXECUTE_READWRITE
                | PAGE_EXECUTE_WRITECOPY
        );
    let is_writable = is_committed
        && !is_guard_or_noaccess
        && matches!(
            base_prot,
            PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
        );

    MemoryRegionInfo {
        base_address: mbi.BaseAddress as u64,
        size: mbi.RegionSize as u64,
        is_committed,
        is_readable,
        is_writable,
    }
}

fn map_open_process_error(pid: u32, code: u32) -> RuntimeError {
    if code == ERROR_ACCESS_DENIED {
        RuntimeError::ProcessElevatedAccessDenied
    } else if code == ERROR_INVALID_PARAMETER {
        RuntimeError::ProcessUnavailable(pid)
    } else {
        RuntimeError::ProcessIdentityMismatch(format!(
            "OpenProcess({pid}) failed with Win32 error {code}"
        ))
    }
}

fn utf16_slice_to_string(slice: &[u16]) -> String {
    let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    OsString::from_wide(&slice[..len])
        .to_string_lossy()
        .into_owned()
}
