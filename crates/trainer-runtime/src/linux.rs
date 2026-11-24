//! Cross-platform Linux `/proc` & SteamOS Proton/Wine parsing utilities plus
//! optional Linux `/proc/<pid>/mem` backend implementation (ADR-017 / U8).

use nekod_runtime_api::ProcessIdentity;

use crate::{MemoryRegionInfo, ModuleInfo};

/// Parsed entry from `/proc/<pid>/maps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcMapEntry {
    pub region: MemoryRegionInfo,
    pub pathname: Option<String>,
}

/// Extracts process start timestamp (`starttime`, field #22 in clock ticks) from `/proc/<pid>/stat`.
/// Safely handles executable names in parentheses that themselves contain spaces or `)`.
pub fn parse_proc_stat_starttime(stat: &str) -> Option<u64> {
    let close_paren = stat.rfind(')')?;
    let after_comm = stat.get(close_paren + 1..)?.trim_start();
    // After ') ', field #3 is `state`, so `starttime` (field #22) is 0-based index 19.
    after_comm.split_whitespace().nth(19)?.parse::<u64>().ok()
}

/// Parses a single line of `/proc/<pid>/maps`.
pub fn parse_proc_maps_line(line: &str) -> Option<ProcMapEntry> {
    let mut parts = line.split_whitespace();
    let range = parts.next()?;
    let perms = parts.next()?;
    let _offset = parts.next()?;
    let _dev = parts.next()?;
    let _inode = parts.next()?;
    let pathname = parts.collect::<Vec<_>>().join(" ");

    let (start_hex, end_hex) = range.split_once('-')?;
    let start = u64::from_str_radix(start_hex, 16).ok()?;
    let end = u64::from_str_radix(end_hex, 16).ok()?;
    if end <= start || perms.len() < 3 {
        return None;
    }

    let bytes = perms.as_bytes();
    let is_readable = bytes[0] == b'r';
    let is_writable = bytes[1] == b'w';

    let clean_path = (!pathname.is_empty() && !pathname.starts_with('[')).then_some(pathname);

    Some(ProcMapEntry {
        region: MemoryRegionInfo {
            base_address: start,
            size: end - start,
            is_committed: true,
            is_readable,
            is_writable,
        },
        pathname: clean_path,
    })
}

/// Aggregates file-backed mappings from `/proc/<pid>/maps` into module base/size descriptors.
pub fn aggregate_proc_modules(maps_text: &str) -> Vec<ModuleInfo> {
    let mut modules: Vec<ModuleInfo> = Vec::new();
    for line in maps_text.lines() {
        let Some(entry) = parse_proc_maps_line(line) else {
            continue;
        };
        let Some(path) = entry.pathname else {
            continue;
        };
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        if name.is_empty() {
            continue;
        }
        let end = entry.region.base_address.saturating_add(entry.region.size);

        if let Some(existing) = modules.iter_mut().find(|m| m.name == name) {
            let current_end = existing.base_address.saturating_add(existing.size);
            if entry.region.base_address < existing.base_address {
                let new_end = current_end.max(end);
                existing.base_address = entry.region.base_address;
                existing.size = new_end.saturating_sub(existing.base_address);
            } else if end > current_end {
                existing.size = end.saturating_sub(existing.base_address);
            }
        } else {
            modules.push(ModuleInfo {
                name,
                base_address: entry.region.base_address,
                size: entry.region.size,
            });
        }
    }
    modules
}

/// Extracts a target Windows `.exe` basename from NUL-delimited `/proc/<pid>/cmdline`
/// arguments produced under SteamOS Proton or Wine.
pub fn extract_proton_windows_exe(cmdline_nul_bytes: &[u8]) -> Option<String> {
    for raw_arg in cmdline_nul_bytes.split(|b| *b == 0) {
        let Ok(arg) = std::str::from_utf8(raw_arg) else {
            continue;
        };
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            continue;
        }
        let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
        let lower = base.to_ascii_lowercase();
        if lower.ends_with(".exe")
            && !matches!(
                lower.as_str(),
                "wine64-preloader.exe" | "start.exe" | "explorer.exe" | "services.exe"
            )
        {
            return Some(base.to_owned());
        }
    }
    None
}

/// Checks if a Linux `ProcessIdentity` matches expected Windows/Linux executable names,
/// accounting for Proton wrapped command lines.
pub fn normalize_proton_process_name(identity: &mut ProcessIdentity, cmdline_nul_bytes: &[u8]) {
    if let Some(win_exe) = extract_proton_windows_exe(cmdline_nul_bytes) {
        identity.process_name = win_exe;
    }
}

#[cfg(target_os = "linux")]
pub mod backend {
    use std::fs::{self, File, OpenOptions};
    use std::os::unix::fs::FileExt;

    use nekod_manifest::{BinaryArch, TargetPlatform};
    use nekod_runtime_api::{ProcessIdentity, RuntimeError};

    use super::{
        aggregate_proc_modules, extract_proton_windows_exe, parse_proc_maps_line,
        parse_proc_stat_starttime,
    };
    use crate::{
        MemoryRegionInfo, ModuleInfo, ProcessAccessMode, ProcessHandle, ProcessMemoryBackend,
    };

    /// Linux/SteamOS `procfs` + `/proc/<pid>/mem` backend (no privilege escalation).
    #[derive(Debug, Default)]
    pub struct LinuxProcBackend;

    impl LinuxProcBackend {
        pub fn new() -> Self {
            Self
        }

        fn read_identity(pid: u32) -> Option<ProcessIdentity> {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let starttime = parse_proc_stat_starttime(&stat)?;
            let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            let exe_path = exe.to_string_lossy().into_owned();
            let mut process_name = exe
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_owned();
            if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline"))
                && let Some(proton_exe) = extract_proton_windows_exe(&cmdline)
            {
                process_name = proton_exe;
            }
            if process_name.is_empty() {
                return None;
            }

            Some(ProcessIdentity {
                pid,
                process_name,
                executable_path: exe_path,
                platform: TargetPlatform::LinuxX64,
                architecture: BinaryArch::X86_64,
                executable_sha256: None,
                detected_version: None,
                process_creation_time_100ns: Some(starttime),
            })
        }
    }

    struct LinuxProcHandle {
        pid: u32,
        mem: File,
        writable: bool,
    }

    impl ProcessHandle for LinuxProcHandle {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn verify_identity(&self, expected: &ProcessIdentity) -> Result<(), RuntimeError> {
            let current = LinuxProcBackend::read_identity(self.pid)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;
            if let (Some(exp_t), Some(cur_t)) = (
                expected.process_creation_time_100ns,
                current.process_creation_time_100ns,
            ) && exp_t != cur_t
            {
                return Err(RuntimeError::ProcessIdentityMismatch(format!(
                    "PID {} reused (expected starttime {}, observed {})",
                    self.pid, exp_t, cur_t
                )));
            }
            Ok(())
        }

        fn list_modules(&self) -> Result<Vec<ModuleInfo>, RuntimeError> {
            let maps = fs::read_to_string(format!("/proc/{}/maps", self.pid))
                .map_err(|_| RuntimeError::ProcessUnavailable(self.pid))?;
            Ok(aggregate_proc_modules(&maps))
        }

        fn list_readable_regions(
            &self,
            module_base: u64,
            module_size: u64,
        ) -> Result<Vec<MemoryRegionInfo>, RuntimeError> {
            let maps = fs::read_to_string(format!("/proc/{}/maps", self.pid))
                .map_err(|_| RuntimeError::ProcessUnavailable(self.pid))?;
            let module_end = module_base.saturating_add(module_size);
            Ok(maps
                .lines()
                .filter_map(parse_proc_maps_line)
                .map(|m| m.region)
                .filter(|r| {
                    r.is_readable
                        && r.base_address < module_end
                        && r.base_address.saturating_add(r.size) > module_base
                })
                .collect())
        }

        fn query_region(&self, address: u64) -> Result<MemoryRegionInfo, RuntimeError> {
            let maps = fs::read_to_string(format!("/proc/{}/maps", self.pid))
                .map_err(|_| RuntimeError::ProcessUnavailable(self.pid))?;
            maps.lines()
                .filter_map(parse_proc_maps_line)
                .map(|m| m.region)
                .find(|r| r.contains_range(address, 1))
                .ok_or_else(|| {
                    RuntimeError::AddressResolutionFailed(format!(
                        "address 0x{address:x} unmapped in PID {}",
                        self.pid
                    ))
                })
        }

        fn read_memory(&self, address: u64, len: usize) -> Result<Vec<u8>, RuntimeError> {
            let mut buf = vec![0u8; len];
            self.mem.read_exact_at(&mut buf, address).map_err(|e| {
                RuntimeError::AddressResolutionFailed(format!(
                    "failed to read {len} byte(s) at 0x{address:x}: {e}"
                ))
            })?;
            Ok(buf)
        }

        fn write_memory(&self, address: u64, bytes: &[u8]) -> Result<(), RuntimeError> {
            if !self.writable {
                return Err(RuntimeError::PermissionDenied(
                    nekod_manifest::PermissionScope::ProcessWriteMemory,
                ));
            }
            self.mem.write_all_at(bytes, address).map_err(|e| {
                RuntimeError::AddressResolutionFailed(format!(
                    "failed to write {} byte(s) at 0x{address:x}: {e}",
                    bytes.len()
                ))
            })
        }
    }

    impl ProcessMemoryBackend for LinuxProcBackend {
        fn enumerate_processes(&self) -> Result<Vec<ProcessIdentity>, RuntimeError> {
            let Ok(entries) = fs::read_dir("/proc") else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for entry in entries.flatten() {
                if let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|s| s.parse::<u32>().ok())
                    && let Some(id) = Self::read_identity(pid)
                {
                    out.push(id);
                }
            }
            Ok(out)
        }

        fn open_process(
            &self,
            pid: u32,
            mode: ProcessAccessMode,
        ) -> Result<Box<dyn ProcessHandle>, RuntimeError> {
            let writable = mode == ProcessAccessMode::ReadWrite;
            let mem = OpenOptions::new()
                .read(true)
                .write(writable)
                .open(format!("/proc/{pid}/mem"))
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        RuntimeError::ProcessElevatedAccessDenied
                    } else {
                        RuntimeError::ProcessUnavailable(pid)
                    }
                })?;
            Ok(Box::new(LinuxProcHandle { pid, mem, writable }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_starttime_even_with_spaces_in_comm() {
        let stat = "4210 (Hollow Knight) S 4000 4210 4210 0 -1 4194304 100 0 0 0 10 2 0 0 20 0 8 0 987654 123456 100";
        assert_eq!(parse_proc_stat_starttime(stat), Some(987654));
    }

    #[test]
    fn parses_proc_maps_and_aggregates_modules() {
        let maps = "\
00400000-00401000 r--p 00000000 08:01 12345 /home/deck/.local/share/Steam/steamapps/common/Game/Game.exe\n\
00401000-00405000 r-xp 00001000 08:01 12345 /home/deck/.local/share/Steam/steamapps/common/Game/Game.exe\n\
7fff1000-7fff2000 rw-p 00000000 00:00 0 [heap]\n";
        let mods = aggregate_proc_modules(maps);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "Game.exe");
        assert_eq!(mods[0].base_address, 0x0040_0000);
        assert_eq!(mods[0].size, 0x5000);
    }

    #[test]
    fn extracts_proton_target_windows_exe_skipping_preloader() {
        let cmdline = b"/usr/bin/proton\0waitforexitandrun\0/home/deck/.steam/root/proton/bin/wine64-preloader.exe\0Z:\\home\\deck\\Games\\SampleRPG.exe\0-fullscreen\0";
        assert_eq!(
            extract_proton_windows_exe(cmdline).as_deref(),
            Some("SampleRPG.exe")
        );
    }
}
