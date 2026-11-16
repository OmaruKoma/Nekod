use serde::{Deserialize, Serialize};

/// Fine-grained capability scopes requested by a trainer manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PermissionScope {
    #[serde(rename = "process.discover")]
    ProcessDiscover,
    #[serde(rename = "process.read_memory")]
    ProcessReadMemory,
    #[serde(rename = "process.write_memory")]
    ProcessWriteMemory,
    #[serde(rename = "process.allocate_memory")]
    ProcessAllocateMemory,
    #[serde(rename = "process.suspend_resume")]
    ProcessSuspendResume,
    #[serde(rename = "filesystem.read_save_dir")]
    FilesystemReadSaveDir,
    #[serde(rename = "filesystem.write_save_dir")]
    FilesystemWriteSaveDir,
    #[serde(rename = "runtime.native_module")]
    RuntimeNativeModule,
}

impl PermissionScope {
    /// Returns `true` if granting this scope requires explicit player confirmation.
    pub const fn requires_explicit_confirmation(self) -> bool {
        matches!(
            self,
            Self::ProcessReadMemory
                | Self::ProcessWriteMemory
                | Self::ProcessAllocateMemory
                | Self::ProcessSuspendResume
                | Self::FilesystemReadSaveDir
                | Self::FilesystemWriteSaveDir
                | Self::RuntimeNativeModule
        )
    }

    /// Returns `true` if this scope mutates target process state, files, or loads native code.
    pub const fn is_elevated_risk(self) -> bool {
        matches!(
            self,
            Self::ProcessWriteMemory
                | Self::ProcessAllocateMemory
                | Self::FilesystemWriteSaveDir
                | Self::RuntimeNativeModule
        )
    }
}

/// Optional bounds narrowing a declared permission scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PermissionConstraints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_write_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_modules: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_scope: Option<Vec<String>>,
}

/// Explicit permission declaration in a `TrainerManifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDeclaration {
    pub scope: PermissionScope,
    pub reason: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<PermissionConstraints>,
}
