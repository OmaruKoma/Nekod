//! OS process/memory backend traits plus deterministic mock implementation for tests.

use nekod_runtime_api::{ProcessIdentity, RuntimeError};

/// Minimum OS access rights requested for opening a target process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAccessMode {
    ReadOnly,
    ReadWrite,
}

/// Loaded module range inside a target process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    pub name: String,
    pub base_address: u64,
    pub size: u64,
}

/// Virtual memory region metadata returned by OS region queries (`VirtualQueryEx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegionInfo {
    pub base_address: u64,
    pub size: u64,
    pub is_committed: bool,
    pub is_readable: bool,
    pub is_writable: bool,
}

impl MemoryRegionInfo {
    /// Checks whether `[address, address + len)` lies completely within this region.
    pub fn contains_range(&self, address: u64, len: usize) -> bool {
        let Some(end) = address.checked_add(len as u64) else {
            return false;
        };
        let Some(region_end) = self.base_address.checked_add(self.size) else {
            return false;
        };
        address >= self.base_address && end <= region_end
    }
}

/// Scoped handle bound to a single target process.
pub trait ProcessHandle: Send {
    fn pid(&self) -> u32;

    /// Verifies that the process is still alive and still matches `expected` identity
    /// (PID, process creation timestamp, and executable path).
    fn verify_identity(&self, expected: &ProcessIdentity) -> Result<(), RuntimeError>;

    /// Lists loaded modules visible in the process.
    fn list_modules(&self) -> Result<Vec<ModuleInfo>, RuntimeError>;

    /// Lists committed readable memory regions inside `[module_base, module_base + module_size)`.
    fn list_readable_regions(
        &self,
        module_base: u64,
        module_size: u64,
    ) -> Result<Vec<MemoryRegionInfo>, RuntimeError>;

    /// Queries metadata for the memory region containing `address`.
    fn query_region(&self, address: u64) -> Result<MemoryRegionInfo, RuntimeError>;

    /// Reads `len` bytes starting at `address`.
    fn read_memory(&self, address: u64, len: usize) -> Result<Vec<u8>, RuntimeError>;

    /// Writes `bytes` starting at `address`. Fails unless opened in `ReadWrite` mode.
    fn write_memory(&self, address: u64, bytes: &[u8]) -> Result<(), RuntimeError>;
}

/// Abstraction over OS process enumeration and handle acquisition.
pub trait ProcessMemoryBackend: Send + Sync {
    /// Lists discoverable running processes without privilege elevation.
    fn enumerate_processes(&self) -> Result<Vec<ProcessIdentity>, RuntimeError>;

    /// Opens a target process with the minimum required access mode.
    fn open_process(
        &self,
        pid: u32,
        mode: ProcessAccessMode,
    ) -> Result<Box<dyn ProcessHandle>, RuntimeError>;
}

/// Deterministic in-memory process simulator used across L2/L3 tests.
pub mod mock {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use nekod_manifest::{BinaryArch, PermissionScope, TargetPlatform};

    use super::*;

    #[derive(Debug, Clone)]
    pub struct MockProcessImage {
        pub identity: ProcessIdentity,
        pub alive: bool,
        pub requires_elevation: bool,
        pub modules: Vec<ModuleInfo>,
        /// Contiguous simulated committed pages keyed by base address.
        pub pages: BTreeMap<u64, MockPage>,
    }

    #[derive(Debug, Clone)]
    pub struct MockPage {
        pub bytes: Vec<u8>,
        pub is_committed: bool,
        pub is_readable: bool,
        pub is_writable: bool,
    }

    #[derive(Debug, Default, Clone)]
    pub struct MockMemoryBackend {
        inner: Arc<Mutex<BTreeMap<u32, MockProcessImage>>>,
    }

    impl MockMemoryBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Inserts or replaces a simulated running process.
        pub fn insert_process(&self, image: MockProcessImage) {
            let pid = image.identity.pid;
            self.inner.lock().unwrap().insert(pid, image);
        }

        /// Simulates process termination or PID reuse.
        pub fn with_process_mut<F, R>(&self, pid: u32, f: F) -> Option<R>
        where
            F: FnOnce(&mut MockProcessImage) -> R,
        {
            self.inner.lock().unwrap().get_mut(&pid).map(f)
        }

        /// Helper constructing a basic 64-bit Windows process image at `base = 0x1400_00000`.
        pub fn sample_windows_process(
            pid: u32,
            process_name: &str,
            module_bytes: Vec<u8>,
        ) -> MockProcessImage {
            let size = module_bytes.len().max(0x1000) as u64;
            let mut padded = module_bytes;
            if padded.len() < size as usize {
                padded.resize(size as usize, 0);
            }
            let base = 0x1400_00000;
            let mut pages = BTreeMap::new();
            pages.insert(
                base,
                MockPage {
                    bytes: padded,
                    is_committed: true,
                    is_readable: true,
                    is_writable: true,
                },
            );

            MockProcessImage {
                identity: ProcessIdentity {
                    pid,
                    process_name: process_name.to_owned(),
                    executable_path: format!(r"C:\Games\{process_name}"),
                    platform: TargetPlatform::WindowsX64,
                    architecture: BinaryArch::X86_64,
                    executable_sha256: None,
                    detected_version: Some("1.0.0".to_owned()),
                    process_creation_time_100ns: Some(133_500_000_000_000_000),
                },
                alive: true,
                requires_elevation: false,
                modules: vec![ModuleInfo {
                    name: process_name.to_owned(),
                    base_address: base,
                    size,
                }],
                pages,
            }
        }
    }

    struct MockHandle {
        pid: u32,
        mode: ProcessAccessMode,
        state: Arc<Mutex<BTreeMap<u32, MockProcessImage>>>,
    }

    impl ProcessHandle for MockHandle {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn verify_identity(&self, expected: &ProcessIdentity) -> Result<(), RuntimeError> {
            let guard = self.state.lock().unwrap();
            let proc_img = guard
                .get(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;

            if let (Some(exp_ts), Some(actual_ts)) = (
                expected.process_creation_time_100ns,
                proc_img.identity.process_creation_time_100ns,
            ) && exp_ts != actual_ts
            {
                return Err(RuntimeError::ProcessIdentityMismatch(format!(
                    "PID {} reused: expected creation time {exp_ts}, observed {actual_ts}",
                    self.pid
                )));
            }

            if !expected
                .executable_path
                .eq_ignore_ascii_case(&proc_img.identity.executable_path)
            {
                return Err(RuntimeError::ProcessIdentityMismatch(format!(
                    "executable path changed from '{}' to '{}'",
                    expected.executable_path, proc_img.identity.executable_path
                )));
            }

            Ok(())
        }

        fn list_modules(&self) -> Result<Vec<ModuleInfo>, RuntimeError> {
            let guard = self.state.lock().unwrap();
            let proc_img = guard
                .get(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;
            Ok(proc_img.modules.clone())
        }

        fn list_readable_regions(
            &self,
            module_base: u64,
            module_size: u64,
        ) -> Result<Vec<MemoryRegionInfo>, RuntimeError> {
            let guard = self.state.lock().unwrap();
            let proc_img = guard
                .get(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;
            let mod_end = module_base.saturating_add(module_size);
            Ok(proc_img
                .pages
                .iter()
                .filter_map(|(&base, page)| {
                    let page_end = base.saturating_add(page.bytes.len() as u64);
                    if page.is_committed
                        && page.is_readable
                        && page_end > module_base
                        && base < mod_end
                    {
                        Some(MemoryRegionInfo {
                            base_address: base,
                            size: page.bytes.len() as u64,
                            is_committed: page.is_committed,
                            is_readable: page.is_readable,
                            is_writable: page.is_writable,
                        })
                    } else {
                        None
                    }
                })
                .collect())
        }

        fn query_region(&self, address: u64) -> Result<MemoryRegionInfo, RuntimeError> {
            let guard = self.state.lock().unwrap();
            let proc_img = guard
                .get(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;

            for (&base, page) in &proc_img.pages {
                let end = base.saturating_add(page.bytes.len() as u64);
                if address >= base && address < end {
                    return Ok(MemoryRegionInfo {
                        base_address: base,
                        size: page.bytes.len() as u64,
                        is_committed: page.is_committed,
                        is_readable: page.is_readable,
                        is_writable: page.is_writable,
                    });
                }
            }
            Err(RuntimeError::AddressResolutionFailed(format!(
                "address 0x{address:X} is not mapped"
            )))
        }

        fn read_memory(&self, address: u64, len: usize) -> Result<Vec<u8>, RuntimeError> {
            let guard = self.state.lock().unwrap();
            let proc_img = guard
                .get(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;

            for (&base, page) in &proc_img.pages {
                let end = base.saturating_add(page.bytes.len() as u64);
                if page.is_committed
                    && page.is_readable
                    && address >= base
                    && address + len as u64 <= end
                {
                    let start = (address - base) as usize;
                    return Ok(page.bytes[start..start + len].to_vec());
                }
            }
            Err(RuntimeError::AddressResolutionFailed(format!(
                "cannot read {len} bytes at 0x{address:X}"
            )))
        }

        fn write_memory(&self, address: u64, bytes: &[u8]) -> Result<(), RuntimeError> {
            if self.mode != ProcessAccessMode::ReadWrite {
                return Err(RuntimeError::PermissionDenied(
                    PermissionScope::ProcessWriteMemory,
                ));
            }
            let mut guard = self.state.lock().unwrap();
            let proc_img = guard
                .get_mut(&self.pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(self.pid))?;

            for (&base, page) in &mut proc_img.pages {
                let end = base.saturating_add(page.bytes.len() as u64);
                if page.is_committed && address >= base && address + bytes.len() as u64 <= end {
                    let start = (address - base) as usize;
                    page.bytes[start..start + bytes.len()].copy_from_slice(bytes);
                    return Ok(());
                }
            }
            Err(RuntimeError::AddressResolutionFailed(format!(
                "cannot write {} bytes at 0x{address:X}",
                bytes.len()
            )))
        }
    }

    impl ProcessMemoryBackend for MockMemoryBackend {
        fn enumerate_processes(&self) -> Result<Vec<ProcessIdentity>, RuntimeError> {
            let guard = self.inner.lock().unwrap();
            Ok(guard
                .values()
                .filter(|p| p.alive && !p.requires_elevation)
                .map(|p| p.identity.clone())
                .collect())
        }

        fn open_process(
            &self,
            pid: u32,
            mode: ProcessAccessMode,
        ) -> Result<Box<dyn ProcessHandle>, RuntimeError> {
            let guard = self.inner.lock().unwrap();
            let proc_img = guard
                .get(&pid)
                .filter(|p| p.alive)
                .ok_or(RuntimeError::ProcessUnavailable(pid))?;
            if proc_img.requires_elevation {
                return Err(RuntimeError::ProcessElevatedAccessDenied);
            }
            Ok(Box::new(MockHandle {
                pid,
                mode,
                state: Arc::clone(&self.inner),
            }))
        }
    }
}
