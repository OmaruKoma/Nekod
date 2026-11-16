use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced when validating manifest invariants beyond JSON Schema syntax.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ManifestError {
    #[error("unsupported schema_version '{0}': expected major version 1")]
    UnsupportedSchemaVersion(String),

    #[error("invalid identifier '{0}': {1}")]
    InvalidIdentifier(String, &'static str),

    #[error(
        "invalid relative path '{0}': must be non-empty, relative, and free of traversal or reserved names"
    )]
    InvalidRelativePath(String),

    #[error("invalid HTTPS URL '{0}'")]
    InvalidHttpsUrl(String),

    #[error("invalid SHA-256 hex digest '{0}'")]
    InvalidSha256(String),

    #[error("policy violation: {0}")]
    PolicyViolation(String),

    #[error("inconsistent manifest: {0}")]
    InconsistentManifest(String),
}

/// Supported OS + CPU architecture targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetPlatform {
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "windows-arm64")]
    WindowsArm64,
    #[serde(rename = "linux-x64")]
    LinuxX64,
    #[serde(rename = "linux-arm64")]
    LinuxArm64,
    #[serde(rename = "steamos-x64")]
    SteamOsX64,
}

/// CPU instruction set architecture of a game binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryArch {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "x86")]
    X86,
    #[serde(rename = "aarch64")]
    Aarch64,
}

/// Known anti-cheat systems tracked for safety and compatibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiCheatKind {
    EasyAntiCheat,
    Battleye,
    Vanguard,
    Vac,
    DenuvoAntiCheat,
    Custom,
}

/// Trust tier associated with a trainer source or repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    Unverified,
    CommunityVerified,
    Official,
}

/// Compatibility verification status for a trainer against a game version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Verified,
    Experimental,
    Broken,
    Untested,
}

/// Ensures `schema_version` has major version `1`.
pub fn validate_schema_v1(version: &Version) -> Result<(), ManifestError> {
    if version.major != crate::SUPPORTED_SCHEMA_MAJOR {
        return Err(ManifestError::UnsupportedSchemaVersion(version.to_string()));
    }
    Ok(())
}

/// Checks strict lowercase kebab-case canonical slug (`^[a-z0-9]+(?:-[a-z0-9]+)*$`, length 2..=128).
pub fn is_valid_kebab_slug(id: &str) -> bool {
    if !(2..=128).contains(&id.len()) {
        return false;
    }
    let bytes = id.as_bytes();
    if !is_lower_alnum(bytes[0]) || !is_lower_alnum(bytes[bytes.len() - 1]) {
        return false;
    }
    let mut prev_hyphen = false;
    for &b in bytes {
        if b == b'-' {
            if prev_hyphen {
                return false;
            }
            prev_hyphen = true;
        } else if is_lower_alnum(b) {
            prev_hyphen = false;
        } else {
            return false;
        }
    }
    true
}

/// Checks namespaced identifier (`trainer_id` / `repository_id`: `[a-z0-9][a-z0-9._-]{2,127}` without `..`).
pub fn is_valid_namespaced_id(id: &str) -> bool {
    if !(3..=128).contains(&id.len()) || id.contains("..") {
        return false;
    }
    let bytes = id.as_bytes();
    if !is_lower_alnum(bytes[0]) || !is_lower_alnum(bytes[bytes.len() - 1]) {
        return false;
    }
    bytes
        .iter()
        .all(|&b| is_lower_alnum(b) || matches!(b, b'.' | b'_' | b'-'))
}

/// Checks feature identifier (`^[a-z0-9][a-z0-9_-]{1,63}$`).
pub fn is_valid_feature_id(id: &str) -> bool {
    if !(2..=64).contains(&id.len()) {
        return false;
    }
    let bytes = id.as_bytes();
    if !is_lower_alnum(bytes[0]) {
        return false;
    }
    bytes
        .iter()
        .all(|&b| is_lower_alnum(b) || matches!(b, b'_' | b'-'))
}

const fn is_lower_alnum(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'0'..=b'9')
}

fn is_windows_reserved_stem(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Validates that a relative path is safe across POSIX and Windows filesystems.
pub fn validate_relative_path(path: &str) -> Result<(), ManifestError> {
    if path.is_empty()
        || path.len() > 260
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path.as_bytes().get(1) == Some(&b':')
    {
        return Err(ManifestError::InvalidRelativePath(path.to_owned()));
    }
    for segment in path.split('/') {
        if segment.is_empty()
            || segment == ".."
            || segment == "."
            || segment.ends_with(' ')
            || segment.ends_with('.')
            || is_windows_reserved_stem(segment)
        {
            return Err(ManifestError::InvalidRelativePath(path.to_owned()));
        }
    }
    Ok(())
}

/// Validates a public HTTPS URL, rejecting userinfo (`@`) and loopback/localhost authorities.
pub fn validate_https_url(url: &str) -> Result<(), ManifestError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| ManifestError::InvalidHttpsUrl(url.to_owned()))?;
    if rest.is_empty()
        || rest.starts_with('/')
        || rest.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ManifestError::InvalidHttpsUrl(url.to_owned()));
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(ManifestError::InvalidHttpsUrl(url.to_owned()));
    }
    let host = authority
        .strip_prefix('[')
        .and_then(|s| s.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(""));

    let host_lower = host.to_ascii_lowercase();
    if host_lower.is_empty()
        || host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.starts_with("127.")
        || host_lower == "::1"
        || host_lower == "0.0.0.0"
    {
        return Err(ManifestError::InvalidHttpsUrl(url.to_owned()));
    }
    Ok(())
}

/// Checks whether a string is a 64-character lowercase hexadecimal SHA-256 digest.
pub fn is_valid_sha256_hex(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
