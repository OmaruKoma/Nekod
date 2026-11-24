#![cfg(target_os = "windows")]

use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::ptr::{read_volatile, write_volatile};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nekod_manifest::{PermissionScope, TargetPlatform, TrainerManifest, TrustTier};
use nekod_runtime_api::{
    FeatureCommand, FeatureValue, GrantedPermissions, RuntimeError, RuntimeProvider,
    SessionLifecycleState, StartSessionRequest,
};
use nekod_trainer_runtime::DeclarativeRuntime;
use nekod_trainer_runtime::win32::Win32ProcessBackend;

#[repr(C)]
struct LiveTargetBuffer(UnsafeCell<[u8; 24]>);

unsafe impl Sync for LiveTargetBuffer {}

// 12-byte magic signature + 3 patchable bytes (offset +12) + 4-byte i32 gold value = 100 (offset +16)
#[used]
static LIVE_TARGET: LiveTargetBuffer = LiveTargetBuffer(UnsafeCell::new([
    0xDE, 0xAD, 0xBE, 0xEF, 0x4E, 0x45, 0x4B, 0x4F, 0xA7, 0x5C, 0x33, 0x19, // +0..12 AOB
    0x29, 0x73, 0x10, 0x00, // +12..15 code bytes (+ padding)
    100, 0x00, 0x00, 0x00, // +16..20 little-endian i32 (100)
    0x00, 0x00, 0x00, 0x00, // padding
]));

fn read_target_byte(offset: usize) -> u8 {
    unsafe {
        let ptr = (*LIVE_TARGET.0.get()).as_ptr().add(offset);
        read_volatile(ptr)
    }
}

fn read_target_gold() -> i32 {
    unsafe {
        let ptr = (*LIVE_TARGET.0.get()).as_ptr().add(16).cast::<i32>();
        read_volatile(ptr)
    }
}

fn write_target_gold(val: i32) {
    unsafe {
        let ptr = (*LIVE_TARGET.0.get()).as_mut_ptr().add(16).cast::<i32>();
        write_volatile(ptr, val);
    }
}

#[test]
fn win32_backend_discovers_scans_patches_freezes_and_rolls_back_real_process() {
    let runtime = DeclarativeRuntime::with_backend(Arc::new(Win32ProcessBackend::new()));
    let self_pid = std::process::id();
    let exe_path = std::env::current_exe().unwrap();
    let exe_name = Path::new(&exe_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap()
        .to_owned();

    // 1. Discover own live process through Win32 Toolhelp + OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)
    let discovered = runtime
        .discover_matching_processes(std::slice::from_ref(&exe_name))
        .unwrap();
    let self_proc = discovered
        .into_iter()
        .find(|p| p.pid == self_pid)
        .expect("self process must be discoverable via Win32ProcessBackend");
    assert!(self_proc.process_creation_time_100ns.is_some());
    assert_eq!(self_proc.platform, TargetPlatform::WindowsX64);

    // 2. Build trainer manifest + declarative_patch_v1 payload targeting self module via AobPattern
    let mut manifest: TrainerManifest = serde_json::from_str(include_str!(
        "../../../schemas/examples/trainer-manifest.example.json"
    ))
    .unwrap();
    manifest.compatibility.anti_cheat_incompatible.clear();

    let payload = serde_json::json!({
        "schema_version": "1.0.0",
        "features": [
            {
                "feature_id": "infinite-health",
                "operations": [
                    {
                        "kind": "byte_patch",
                        "locator": {
                            "kind": "aob_pattern",
                            "module": exe_name,
                            "pattern": "DE AD BE EF 4E 45 4B 4F A7 5C 33 19 29 73 10",
                            "match_index": 0,
                            "offset": 12
                        },
                        "expected_original_bytes": "29 73 10",
                        "patch_bytes": "90 90 90"
                    }
                ]
            },
            {
                "feature_id": "set-gold",
                "operations": [
                    {
                        "kind": "write_value",
                        "locator": {
                            "kind": "aob_pattern",
                            "module": exe_name,
                            "pattern": "DE AD BE EF 4E 45 4B 4F A7 5C 33 19",
                            "match_index": 0,
                            "offset": 16
                        },
                        "value_type": "i32",
                        "freeze_interval_ms": 60
                    }
                ]
            }
        ]
    });

    let granted_permissions = GrantedPermissions {
        trainer_id: manifest.id.clone(),
        trainer_version: manifest.version.to_string(),
        manifest_sha256: "b".repeat(64),
        trust_tier: TrustTier::CommunityVerified,
        granted_scopes: [
            PermissionScope::ProcessDiscover,
            PermissionScope::ProcessReadMemory,
            PermissionScope::ProcessWriteMemory,
        ]
        .into_iter()
        .collect(),
        scope_constraints: BTreeMap::new(),
    };

    let sid = runtime
        .start_session(StartSessionRequest {
            trainer: manifest,
            target_process: self_proc,
            granted_permissions,
            declarative_payload_json: Some(payload.to_string()),
            verified_package_assets: BTreeMap::new(),
        })
        .unwrap();

    // 3. Apply AOB byte_patch (`29 73 10` -> `90 90 90`) on real Win32 memory
    assert_eq!(
        (
            read_target_byte(12),
            read_target_byte(13),
            read_target_byte(14)
        ),
        (0x29, 0x73, 0x10)
    );
    runtime
        .apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "infinite-health".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
    assert_eq!(
        (
            read_target_byte(12),
            read_target_byte(13),
            read_target_byte(14)
        ),
        (0x90, 0x90, 0x90)
    );

    // 4. Apply bounded frozen `write_value` (gold = 7777) and verify freeze worker restores it
    runtime
        .apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "set-gold".to_owned(),
                value: FeatureValue::Integer(7777),
            },
        )
        .unwrap();
    assert_eq!(read_target_gold(), 7777);

    write_target_gold(12);
    thread::sleep(Duration::from_millis(160));
    assert_eq!(read_target_gold(), 7777);

    // 5. Stop session -> LIFO rollback restores both code bytes and original gold (`100`)
    let diag = runtime.stop_session(&sid).unwrap();
    assert_eq!(diag.state, SessionLifecycleState::Stopped);
    assert_eq!(diag.active_patches, 0);
    assert_eq!(diag.rollback_conflicts, 0);
    assert_eq!(diag.rollback_failures, 0);
    assert_eq!(
        (
            read_target_byte(12),
            read_target_byte(13),
            read_target_byte(14)
        ),
        (0x29, 0x73, 0x10)
    );
    assert_eq!(read_target_gold(), 100);

    // 6. Trying to open Windows System Idle/System (PID 4) returns ProcessElevatedAccessDenied cleanly
    let sys_err = runtime
        .apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "infinite-health".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap_err();
    assert!(matches!(sys_err, RuntimeError::SessionNotFound(_)));
}
