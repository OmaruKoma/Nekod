use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nekod_core::{NekodAppService, TrainerPermissionState};
use nekod_game_detection::{
    EpicDetector, GameDetector, GogDetector, match_games_against_manifests,
};
use nekod_manifest::{GameManifest, PermissionScope, TrustTier};
use nekod_profile_manager::GameProfile;
use nekod_registry_client::transport::mock::MockHttpTransport;
use nekod_runtime_api::{FeatureCommand, FeatureValue};
use nekod_trainer_runtime::backend::mock::MockMemoryBackend;
use nekod_trainer_runtime::{CompositeRuntimeRouter, DeclarativeRuntime};

#[test]
fn multi_store_example_package_ingestion_consent_profile_and_router_rollback_e2e() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nekod_e2e_{}_{}", std::process::id(), nanos));

    // 1. Multi-store discovery fixtures (Epic + GOG) matched against GameManifest.
    let epic_manifests = root.join("EpicManifests");
    let epic_game_dir = root.join("EpicGames").join("SampleRPG");
    let gog_game_dir = root.join("GogGames").join("SampleRPG");
    fs::create_dir_all(&epic_manifests).unwrap();
    fs::create_dir_all(&epic_game_dir).unwrap();
    fs::create_dir_all(&gog_game_dir).unwrap();
    fs::write(epic_game_dir.join("SampleRPG.exe"), b"MZ").unwrap();
    fs::write(gog_game_dir.join("SampleRPG.exe"), b"MZ").unwrap();

    fs::write(
        epic_manifests.join("sample.item"),
        serde_json::json!({
            "CatalogItemId": "epic-sample-id",
            "DisplayName": "Sample Singleplayer RPG (Epic)",
            "InstallLocation": epic_game_dir.to_string_lossy(),
            "LaunchExecutable": "SampleRPG.exe",
            "AppVersionString": "1.2.0"
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        gog_game_dir.join("goggame-1234567890.info"),
        serde_json::json!({
            "gameId": "1234567890",
            "name": "Sample Singleplayer RPG (GOG)",
            "version": "1.2.0",
            "playTasks": [{ "isPrimary": true, "path": "SampleRPG.exe" }]
        })
        .to_string(),
    )
    .unwrap();

    let mut game_manifest: GameManifest = serde_json::from_str(include_str!(
        "../../../schemas/examples/game-manifest.example.json"
    ))
    .unwrap();
    game_manifest.store_ids.epic_catalog_item_id = Some("epic-sample-id".to_owned());

    let mut detected = EpicDetector::from_manifests_dir(&epic_manifests)
        .discover_installed()
        .unwrap();
    detected.extend(
        GogDetector::from_install_dirs(vec![gog_game_dir])
            .discover_installed()
            .unwrap(),
    );
    match_games_against_manifests(&mut detected, std::slice::from_ref(&game_manifest));
    assert_eq!(detected.len(), 2);
    assert!(
        detected
            .iter()
            .all(|g| g.matched_game_id.as_deref() == Some("sample-singleplayer-rpg"))
    );

    // 2. Wire NekodAppService through CompositeRuntimeRouter with mock process bytes matching examples/sample-singleplayer-rpg.
    let mut exe_bytes = vec![0u8; 4096];
    exe_bytes[0x200..0x206].copy_from_slice(&[0x29, 0x73, 0x10, 0x8B, 0x43, 0x10]);

    let backend = Arc::new(MockMemoryBackend::new());
    let proc_img = MockMemoryBackend::sample_windows_process(7788, "SampleRPG.exe", exe_bytes);
    let proc_identity = proc_img.identity.clone();
    backend.insert_process(proc_img);

    let router = Arc::new(CompositeRuntimeRouter::with_declarative(Arc::new(
        DeclarativeRuntime::with_backend(backend.clone()),
    )));
    let service = NekodAppService::open_in_memory_with_transport(
        router,
        None,
        Arc::new(MockHttpTransport::new()),
        root.join("packages"),
    )
    .unwrap();

    let game_json = serde_json::to_string(&game_manifest).unwrap();
    service.register_game_manifest(&game_json).unwrap();

    // 3. Ingest the canonical repository fixture at examples/sample-singleplayer-rpg.
    let example_pkg_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("sample-singleplayer-rpg");
    let installed = service
        .ingest_local_trainer(&example_pkg_dir, TrustTier::CommunityVerified)
        .unwrap();

    // Default-deny before player confirmation.
    assert!(
        service
            .start_trainer_session(
                "sample-singleplayer-rpg",
                &installed.manifest.id,
                proc_identity.clone()
            )
            .is_err()
    );

    // Confirm declared scopes & verify UI view transitions to Approved.
    let scopes: BTreeSet<PermissionScope> = installed
        .manifest
        .permissions
        .iter()
        .map(|p| p.scope)
        .collect();
    let perm_state = service
        .grant_trainer_permissions(&installed.manifest.id, scopes)
        .unwrap();
    assert!(matches!(
        perm_state,
        TrainerPermissionState::Approved { .. }
    ));

    // Save and reload hotkey profile.
    let mut hotkeys = BTreeMap::new();
    hotkeys.insert("infinite-health".to_owned(), "Ctrl+F1".to_owned());
    service
        .save_profile(&GameProfile {
            game_id: "sample-singleplayer-rpg".to_owned(),
            trainer_id: installed.manifest.id.clone(),
            hotkeys,
            saved_feature_states: BTreeMap::new(),
        })
        .unwrap();
    let loaded_profile = service
        .load_profile("sample-singleplayer-rpg", &installed.manifest.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded_profile
            .hotkeys
            .get("infinite-health")
            .map(String::as_str),
        Some("Ctrl+F1")
    );

    // Attach, toggle infinite-health byte_patch, verify memory mutated & cleanly rolled back.
    let sid = service
        .start_trainer_session(
            "sample-singleplayer-rpg",
            &installed.manifest.id,
            proc_identity,
        )
        .unwrap();
    service
        .apply_trainer_feature(
            &sid,
            FeatureCommand {
                feature_id: "infinite-health".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();

    let patched = backend
        .with_process_mut(7788, |p| {
            p.pages[&0x1400_00000].bytes[0x200..0x203].to_vec()
        })
        .unwrap();
    assert_eq!(patched, vec![0x90, 0x90, 0x90]);

    let diag = service.stop_trainer_session(&sid).unwrap();
    assert_eq!(diag.active_patches, 0);
    assert_eq!(diag.rollback_conflicts, 0);

    let restored = backend
        .with_process_mut(7788, |p| {
            p.pages[&0x1400_00000].bytes[0x200..0x203].to_vec()
        })
        .unwrap();
    assert_eq!(restored, vec![0x29, 0x73, 0x10]);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn e2e_game_scan_manifest_validation_and_sqlite_restart_recovery() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nekod_e2e_persist_{}_{}",
        std::process::id(),
        nanos
    ));
    let data_dir = root.join("app_data");
    let custom_game_dir = root.join("CustomLibrary").join("SampleSingleplayerRPG");
    fs::create_dir_all(&custom_game_dir).unwrap();
    fs::write(custom_game_dir.join("SampleRPG.exe"), b"MZ").unwrap();

    let example_pkg_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("sample-singleplayer-rpg");

    // Session 1: persistent SQLite directory initialization, custom path + manifest registration + invalid rejections
    {
        let service = NekodAppService::open_in_dir(
            &data_dir,
            Arc::new(CompositeRuntimeRouter::with_declarative_default()),
        )
        .unwrap();

        // Reject invalid U11 slug game manifest & malformed JSON cleanly.
        assert!(service.register_game_manifest("{bad json").is_err());
        let bad_slug_manifest =
            include_str!("../../../schemas/examples/game-manifest.example.json")
                .replace("sample-singleplayer-rpg", "Steam_Sample_Game");
        assert!(service.register_game_manifest(&bad_slug_manifest).is_err());

        // Reject missing or prohibited trainer packages without mutating catalog.
        assert!(
            service
                .ingest_local_trainer(root.join("missing-pkg"), TrustTier::Unverified)
                .is_err()
        );

        // Register valid manifest & custom game folder.
        service
            .register_game_manifest(include_str!(
                "../../../schemas/examples/game-manifest.example.json"
            ))
            .unwrap();
        service
            .add_custom_game_path(
                &custom_game_dir.to_string_lossy(),
                Some("Portable Singleplayer RPG"),
            )
            .unwrap();

        let library = service.rescan_games().unwrap();
        assert!(
            library
                .iter()
                .any(|g| g.game.matched_game_id.as_deref() == Some("sample-singleplayer-rpg")),
            "expected custom folder to match registered GameManifest"
        );

        // Ingest package, grant permissions, and persist hotkey profile.
        let installed = service
            .ingest_local_trainer(&example_pkg_dir, TrustTier::CommunityVerified)
            .unwrap();
        let scopes: BTreeSet<PermissionScope> = installed
            .manifest
            .permissions
            .iter()
            .map(|p| p.scope)
            .collect();
        service
            .grant_trainer_permissions(&installed.manifest.id, scopes)
            .unwrap();

        let mut hotkeys = BTreeMap::new();
        hotkeys.insert("infinite-health".to_owned(), "Alt+1".to_owned());
        service
            .save_profile(&GameProfile {
                game_id: "sample-singleplayer-rpg".to_owned(),
                trainer_id: installed.manifest.id,
                hotkeys,
                saved_feature_states: BTreeMap::new(),
            })
            .unwrap();
    }

    // Session 2: Simulate full desktop application restart against same data_dir.
    {
        let reopened = NekodAppService::open_in_dir(
            &data_dir,
            Arc::new(CompositeRuntimeRouter::with_declarative_default()),
        )
        .unwrap();

        assert_eq!(reopened.list_custom_game_paths().unwrap().len(), 1);
        assert!(
            reopened
                .list_game_manifests()
                .unwrap()
                .iter()
                .any(|m| m.id == "sample-singleplayer-rpg")
        );

        let trainers = reopened
            .list_trainers_for_game("sample-singleplayer-rpg", Some("1.2.0"))
            .unwrap();
        assert_eq!(trainers.len(), 1);
        assert!(matches!(
            trainers[0].permission_state,
            TrainerPermissionState::Approved { .. }
        ));

        let restored_profile = reopened
            .load_profile("sample-singleplayer-rpg", &trainers[0].trainer_id)
            .unwrap()
            .expect("saved profile should survive SQLite restart");
        assert_eq!(
            restored_profile
                .hotkeys
                .get("infinite-health")
                .map(String::as_str),
            Some("Alt+1")
        );

        reopened
            .remove_custom_game_path(&custom_game_dir.to_string_lossy())
            .unwrap();
        assert!(reopened.list_custom_game_paths().unwrap().is_empty());
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn e2e_mod_lifecycle_sync_etag_outage_and_reconsent_recovery() {
    use nekod_manifest::TrainerManifest;
    use nekod_registry_client::RegistrySource;
    use nekod_registry_client::transport::mock::{MockHttpReply, MockHttpTransport};
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nekod_e2e_mod_{}_{}", std::process::id(), nanos));

    let index_url = "https://raw.githubusercontent.com/example-contributor/sample-rpg-trainer/main/registry.json";
    let payload_bytes =
        include_bytes!("../../../schemas/examples/declarative-patch.example.json").to_vec();
    let mut trainer_v1: TrainerManifest = serde_json::from_str(include_str!(
        "../../../schemas/examples/trainer-manifest.example.json"
    ))
    .unwrap();
    trainer_v1.artifacts[0].sha256 = sha256_hex(&payload_bytes);
    trainer_v1.artifacts[0].size_bytes = payload_bytes.len() as u64;
    let m_v1_bytes = serde_json::to_vec_pretty(&trainer_v1).unwrap();
    let m_v1_sha = sha256_hex(&m_v1_bytes);

    let mut trainer_v2 = trainer_v1.clone();
    trainer_v2.version = "1.1.0".parse().unwrap();
    let m_v2_bytes = serde_json::to_vec_pretty(&trainer_v2).unwrap();
    let m_v2_sha = sha256_hex(&m_v2_bytes);

    let m_v1_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer-manifest.json";
    let p_v1_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.0.0/trainer.json";
    let m_v2_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.1.0/trainer-manifest.json";
    let p_v2_url = "https://github.com/example-contributor/sample-rpg-trainer/releases/download/v1.1.0/trainer.json";

    let reg_v1 = include_str!("../../../schemas/examples/registry-manifest.example.json").replace(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        &m_v1_sha,
    );
    let reg_v2 = reg_v1
        .replace("\"1.0.0\"", "\"1.1.0\"")
        .replace("v1.0.0", "v1.1.0")
        .replace(&m_v1_sha, &m_v2_sha)
        .replacen("\"1.1.0\"", "\"1.0.0\"", 1);

    let transport = Arc::new(MockHttpTransport::new());
    // Sync 1: 200 OK -> Sync 2: 304 Not Modified -> Sync 3: 404 Upstream outage -> Sync 4: 200 OK v1.1.0
    transport.enqueue(
        index_url,
        MockHttpReply::Ok {
            body: reg_v1.as_bytes().to_vec(),
            etag: Some("\"etag-v1\"".to_owned()),
            last_modified: None,
        },
    );
    transport.enqueue(index_url, MockHttpReply::NotModified);
    transport.enqueue(
        index_url,
        MockHttpReply::Error(nekod_registry_client::RegistryError::UpstreamNotFound(
            index_url.to_owned(),
        )),
    );
    transport.enqueue(
        index_url,
        MockHttpReply::Ok {
            body: reg_v2.as_bytes().to_vec(),
            etag: Some("\"etag-v2\"".to_owned()),
            last_modified: None,
        },
    );
    transport.set_ok(m_v1_url, m_v1_bytes, None);
    transport.set_ok(p_v1_url, payload_bytes.clone(), None);
    transport.set_ok(m_v2_url, m_v2_bytes, None);
    transport.set_ok(p_v2_url, payload_bytes, None);

    let service = NekodAppService::open_in_memory_with_transport(
        Arc::new(CompositeRuntimeRouter::with_declarative_default()),
        None,
        transport,
        root.join("packages"),
    )
    .unwrap();
    service
        .add_configured_registry_source(RegistrySource {
            repository_id: "com.github.example-contributor.sample-trainers".to_owned(),
            index_url: index_url.to_owned(),
            authoritative_trust_tier: TrustTier::CommunityVerified,
            enabled: true,
        })
        .unwrap();

    // 1. Sync & install v1.0.0, then approve scopes.
    assert_eq!(service.sync_registry_sources().unwrap().updated, 1);
    service
        .install_or_update_trainer_from_registry(
            "com.github.example-contributor.sample-trainers",
            "dev.example.sample-singleplayer-rpg",
        )
        .unwrap();
    let scopes: BTreeSet<PermissionScope> = [
        PermissionScope::ProcessReadMemory,
        PermissionScope::ProcessWriteMemory,
    ]
    .into_iter()
    .collect();
    service
        .grant_trainer_permissions("dev.example.sample-singleplayer-rpg", scopes)
        .unwrap();

    // 2. Conditional 304 Not Modified preserves everything cleanly.
    let s2 = service.sync_registry_sources().unwrap();
    assert_eq!(s2.not_modified, 1);

    // 3. Upstream 404 outage records error without deleting previously cached discovery index.
    let s3 = service.sync_registry_sources().unwrap();
    assert_eq!(s3.failed, 1);
    assert_eq!(
        service
            .list_community_trainers_for_game("sample-singleplayer-rpg")
            .unwrap()
            .len(),
        1
    );

    // 4. Upstream recovers with v1.1.0 -> update available -> installing invalidates prior consent!
    assert_eq!(service.sync_registry_sources().unwrap().updated, 1);
    let updates = service.check_trainer_updates().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].available_version, "1.1.0");

    service
        .install_or_update_trainer_from_registry(
            "com.github.example-contributor.sample-trainers",
            "dev.example.sample-singleplayer-rpg",
        )
        .unwrap();
    let trainers = service
        .list_trainers_for_game("sample-singleplayer-rpg", None)
        .unwrap();
    assert_eq!(trainers[0].version, "1.1.0");
    assert!(matches!(
        trainers[0].permission_state,
        TrainerPermissionState::RequiresConfirmation { .. }
    ));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn e2e_runtime_failure_recovery_rollback_conflict_and_process_exit_reattach() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nekod_e2e_rec_{}_{}", std::process::id(), nanos));

    let mut exe_bytes = vec![0u8; 4096];
    exe_bytes[0x200..0x206].copy_from_slice(&[0x29, 0x73, 0x10, 0x8B, 0x43, 0x10]);

    let backend = Arc::new(MockMemoryBackend::new());
    let proc1 = MockMemoryBackend::sample_windows_process(8801, "SampleRPG.exe", exe_bytes.clone());
    let proc1_id = proc1.identity.clone();
    backend.insert_process(proc1);

    let router = Arc::new(CompositeRuntimeRouter::with_declarative(Arc::new(
        DeclarativeRuntime::with_backend(backend.clone()),
    )));
    let service = NekodAppService::open_in_memory_with_transport(
        router,
        None,
        Arc::new(MockHttpTransport::new()),
        root.join("packages"),
    )
    .unwrap();
    service
        .register_game_manifest(include_str!(
            "../../../schemas/examples/game-manifest.example.json"
        ))
        .unwrap();

    let pkg_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("sample-singleplayer-rpg");
    let installed = service
        .ingest_local_trainer(&pkg_dir, TrustTier::CommunityVerified)
        .unwrap();
    let scopes: BTreeSet<PermissionScope> = installed
        .manifest
        .permissions
        .iter()
        .map(|p| p.scope)
        .collect();
    service
        .grant_trainer_permissions(&installed.manifest.id, scopes)
        .unwrap();

    // Case A: External memory modification causes safe rollback conflict report without clobbering unknown bytes.
    let sid1 = service
        .start_trainer_session(
            "sample-singleplayer-rpg",
            &installed.manifest.id,
            proc1_id.clone(),
        )
        .unwrap();
    service
        .apply_trainer_feature(
            &sid1,
            FeatureCommand {
                feature_id: "infinite-health".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();

    // Simulate external mod / game self-modification overwriting patched bytes.
    backend
        .with_process_mut(8801, |p| {
            p.pages.get_mut(&0x1400_00000).unwrap().bytes[0x200..0x203]
                .copy_from_slice(&[0xCC, 0xCC, 0xCC]);
        })
        .unwrap();

    let diag1 = service.stop_trainer_session(&sid1).unwrap();
    assert_eq!(diag1.rollback_conflicts, 1);
    let current_bytes = backend
        .with_process_mut(8801, |p| {
            p.pages[&0x1400_00000].bytes[0x200..0x203].to_vec()
        })
        .unwrap();
    assert_eq!(
        current_bytes,
        vec![0xCC, 0xCC, 0xCC],
        "rollback must never blindly overwrite unexpectedly modified bytes"
    );

    // Case B: Mid-session game crash / termination fails closed and allows clean re-attach to relaunched PID.
    backend
        .with_process_mut(8801, |p| {
            p.pages.get_mut(&0x1400_00000).unwrap().bytes[0x200..0x203]
                .copy_from_slice(&[0x29, 0x73, 0x10]);
        })
        .unwrap();
    let sid2 = service
        .start_trainer_session("sample-singleplayer-rpg", &installed.manifest.id, proc1_id)
        .unwrap();
    backend.with_process_mut(8801, |p| p.alive = false).unwrap();

    assert!(
        service
            .apply_trainer_feature(
                &sid2,
                FeatureCommand {
                    feature_id: "infinite-health".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .is_err()
    );
    let _ = service.stop_trainer_session(&sid2).unwrap();

    // Relaunched game with new PID attaches and patches cleanly.
    let mut proc2 = MockMemoryBackend::sample_windows_process(9902, "SampleRPG.exe", exe_bytes);
    proc2.identity.process_creation_time_100ns = Some(222_222);
    let proc2_id = proc2.identity.clone();
    backend.insert_process(proc2);

    let sid3 = service
        .start_trainer_session("sample-singleplayer-rpg", &installed.manifest.id, proc2_id)
        .unwrap();
    service
        .apply_trainer_feature(
            &sid3,
            FeatureCommand {
                feature_id: "infinite-health".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
    let diag3 = service.stop_trainer_session(&sid3).unwrap();
    assert_eq!(diag3.rollback_conflicts, 0);
    assert_eq!(diag3.rollback_failures, 0);

    let _ = fs::remove_dir_all(&root);
}
