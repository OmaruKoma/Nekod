import type {
  BridgeDeploymentStatus,
  BridgeUndeploySummary,
  CommunityTrainerView,
  CompanionBridgeCatalogEntry,
  CompatibilityRecord,
  CustomGamePathRecord,
  DiagnosticEvent,
  FeatureCommand,
  GameArtwork,
  GameEnvironmentProfile,
  GameProfile,
  GlobalHotkeyBinding,
  GlobalHotkeyEvent,
  LaunchGameResult,
  LibraryGameView,
  PermissionScope,
  ProcessIdentity,
  RegistrySourceState,
  RegistrySyncReport,
  RuntimeDiagnostics,
  SecurityPolicyConfig,
  TrainerDetailView,
  TrainerPermissionState,
  TrainerUpdateInfo,
  TrustTier,
} from "./nekod";

/**
 * In-memory browser-only preview adapter active exclusively under `vite` dev mode
 * when opened outside the Tauri WebView (`window.__TAURI_INTERNALS__` absent).
 */
export function createBrowserPreviewApi() {
  let games: LibraryGameView[] = [
    {
      library_key: "sample-singleplayer-rpg",
      trainer_count: 1,
      game: {
        matched_game_id: "sample-singleplayer-rpg",
        title: "Sample Singleplayer RPG",
        store_source: "steam",
        store_app_id: "999990",
        store_build_id: "1.2.0",
        install_dir: "C:\\Games\\Steam\\steamapps\\common\\SampleRPG",
        executable_path: "C:\\Games\\Steam\\steamapps\\common\\SampleRPG\\SampleRPG.exe",
        platform: "windows-x64",
      },
    },
    {
      library_key: "hollow-knight",
      trainer_count: 1,
      game: {
        matched_game_id: "hollow-knight",
        title: "Hollow Knight",
        store_source: "gog",
        store_app_id: "1308935151",
        store_build_id: "1.5.78.11833",
        install_dir: "C:\\GOG Games\\Hollow Knight",
        executable_path: "C:\\GOG Games\\Hollow Knight\\hollow_knight.exe",
        platform: "windows-x64",
      },
    },
    {
      library_key: "custom:indie-sandbox",
      trainer_count: 0,
      game: {
        matched_game_id: null,
        title: "Indie Sandbox Prototype",
        store_source: "custom_path",
        store_app_id: null,
        store_build_id: null,
        install_dir: "D:\\PortableGames\\IndieSandbox",
        executable_path: "D:\\PortableGames\\IndieSandbox\\Game.exe",
        platform: "windows-x64",
      },
    },
  ];

  const trainersByGame: Record<string, TrainerDetailView[]> = {
    "sample-singleplayer-rpg": [
      {
        trainer_id: "dev.example.sample-singleplayer-rpg",
        name: "Sample RPG Companion Trainer",
        version: "1.0.0",
        description: "Declarative AOB & scalar freeze reference trainer for Sample Singleplayer RPG.",
        authors: ["Nekod Examples"],
        source_repository: "https://github.com/nekod-org/Nekod",
        trust_tier: "community_verified",
        manifest_sha256: "4013a089f8359132f58aae28574cd521106733dc9422b7a92745f8a9c2b7b671",
        compatibility_status: "verified",
        compatibility_notes: "Verified against SampleRPG build 1.2.0",
        revoked: false,
        permissions: [
          {
            scope: "process.discover",
            reason: "Locate the running SampleRPG.exe process",
            required: true,
          },
          {
            scope: "process.read_memory",
            reason: "Scan AOB patterns and read current health/gold values",
            required: true,
          },
          {
            scope: "process.write_memory",
            reason: "Apply reversible patches for enabled cheats",
            required: true,
          },
        ],
        permission_state: {
          status: "requires_confirmation",
          missing_confirmations: [
            {
              scope: "process.write_memory",
              reason: "Apply reversible patches for enabled cheats",
              required: true,
            },
          ],
        },
        features: [
          {
            id: "infinite-health",
            name: "Infinite Health",
            description: "Prevents HP subtraction on player damage",
            category: "Survival",
            control_type: "toggle",
            default_hotkey: "F1",
            mutates_memory: true,
            supports_rollback: true,
            risk_level: "low",
          },
          {
            id: "set-gold",
            name: "Set Gold",
            description: "Locks gold currency to a custom integer value",
            category: "Inventory",
            control_type: "integer",
            default_hotkey: null,
            mutates_memory: true,
            supports_rollback: true,
            risk_level: "low",
          },
        ],
      },
    ],
    "hollow-knight": [
      {
        trainer_id: "org.nekod.hollow-knight-qol",
        name: "Hallownest QoL Trainer",
        version: "1.2.0",
        description: "Singleplayer practice utilities (Soul lock, Geo multiplier, invulnerability).",
        authors: ["Nekod Official Trainers"],
        source_repository: "https://github.com/nekod-org/Nekod-Official-Trainers",
        trust_tier: "official",
        manifest_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        compatibility_status: "verified",
        compatibility_notes: "Verified on GOG build 1.5.78.11833",
        revoked: false,
        permissions: [
          { scope: "process.discover", reason: "Locate hollow_knight.exe", required: true },
          { scope: "process.read_memory", reason: "Read player stats", required: true },
          { scope: "process.write_memory", reason: "Patch player state", required: true },
        ],
        permission_state: {
          status: "approved",
          granted_scopes: ["process.discover", "process.read_memory", "process.write_memory"],
        },
        features: [
          {
            id: "infinite-soul",
            name: "Infinite Soul",
            description: "Keeps Soul gauge full for practice runs",
            category: "Combat",
            control_type: "toggle",
            default_hotkey: "Ctrl+F2",
            mutates_memory: true,
            supports_rollback: true,
            risk_level: "low",
          },
        ],
      },
    ],
  };

  let customPaths: CustomGamePathRecord[] = [
    { path: "D:\\PortableGames\\IndieSandbox", label: "Portable Indie Folder" },
  ];
  let policy: SecurityPolicyConfig = {
    allow_third_party_native_modules: false,
    allow_unverified_sources: false,
  };
  let logs: DiagnosticEvent[] = [
    { level: "info", category: "preview", message: "Running in Vite browser preview mode (simulated IPC)." },
    { level: "info", category: "detection", message: "Discovered 3 sample library entries." },
  ];
  let registrySources: RegistrySourceState[] = [
    {
      source: {
        repository_id: "org.nekod.community-index",
        index_url: "https://raw.githubusercontent.com/nekod-org/Nekod-Community/main/registry-manifest.json",
        authoritative_trust_tier: "community_verified",
        enabled: true,
      },
      etag: "\"preview-etag-001\"",
      last_modified: null,
      last_synced_at: 1763265600,
      last_status: "synced",
      last_error: null,
    },
  ];
  const profiles = new Map<string, GameProfile>();
  let activeSession: RuntimeDiagnostics | null = null;
  let activeToggles = new Set<string>();

  const pushLog = (level: string, category: string, message: string) => {
    logs = [...logs, { level, category, message }];
  };

  const deployedBridges = new Map<string, string[]>();

  return {
    rescanGames: async (): Promise<LibraryGameView[]> => {
      pushLog("info", "detection", `Rescanned library (${games.length} entries)`);
      return [...games];
    },
    listLibraryGames: async (): Promise<LibraryGameView[]> => [...games],
    listTrainersForGame: async (
      gameId: string,
      _detectedBuildId: string | null,
    ): Promise<TrainerDetailView[]> => trainersByGame[gameId] ?? [],
    grantTrainerPermissions: async (
      trainerId: string,
      confirmedScopes: PermissionScope[],
    ): Promise<TrainerPermissionState> => {
      for (const list of Object.values(trainersByGame)) {
        const found = list.find((t) => t.trainer_id === trainerId);
        if (found) {
          found.permission_state = { status: "approved", granted_scopes: confirmedScopes };
          pushLog("info", "permissions", `Granted ${confirmedScopes.length} scope(s) to ${trainerId}`);
          return found.permission_state;
        }
      }
      throw new Error(`Trainer not found: ${trainerId}`);
    },
    revokeTrainerPermissions: async (trainerId: string): Promise<void> => {
      for (const list of Object.values(trainersByGame)) {
        const found = list.find((t) => t.trainer_id === trainerId);
        if (found) {
          found.permission_state = {
            status: "requires_confirmation",
            missing_confirmations: found.permissions,
          };
          pushLog("warn", "permissions", `Revoked permissions for ${trainerId}`);
        }
      }
    },
    ingestLocalTrainer: async (packageDir: string, trustTier: TrustTier): Promise<void> => {
      pushLog("info", "trainer", `Simulated ingest of '${packageDir}' (${trustTier})`);
    },
    registerGameManifest: async (rawJson: string): Promise<string> => {
      const parsed = JSON.parse(rawJson) as { id?: string };
      if (!parsed.id) throw new Error("Missing 'id' in GameManifest JSON");
      pushLog("info", "catalog", `Registered game manifest '${parsed.id}'`);
      return parsed.id;
    },
    addCustomGamePath: async (
      path: string,
      label: string | null,
    ): Promise<CustomGamePathRecord[]> => {
      customPaths = [...customPaths.filter((p) => p.path !== path), { path, label }];
      pushLog("info", "detection", `Added custom path '${path}'`);
      return customPaths;
    },
    removeCustomGamePath: async (path: string): Promise<CustomGamePathRecord[]> => {
      customPaths = customPaths.filter((p) => p.path !== path);
      games = games.filter((g) => g.game.install_dir !== path);
      pushLog("info", "detection", `Removed custom path '${path}'`);
      return customPaths;
    },
    listCustomGamePaths: async (): Promise<CustomGamePathRecord[]> => customPaths,
    getSecurityPolicy: async (): Promise<SecurityPolicyConfig> => policy,
    updateSecurityPolicy: async (next: SecurityPolicyConfig): Promise<void> => {
      policy = next;
      pushLog("info", "security", "Updated default-deny security policy in browser preview");
    },
    loadProfile: async (gameId: string, trainerId: string): Promise<GameProfile | null> =>
      profiles.get(`${gameId}:${trainerId}`) ?? null,
    saveProfile: async (profile: GameProfile): Promise<void> => {
      profiles.set(`${profile.game_id}:${profile.trainer_id}`, profile);
      pushLog("info", "profile", `Saved profile for ${profile.game_id}`);
    },
    exportProfileJson: async (gameId: string, trainerId: string): Promise<string> => {
      const existing = profiles.get(`${gameId}:${trainerId}`) ?? {
        game_id: gameId,
        trainer_id: trainerId,
        hotkeys: {},
        saved_feature_states: {},
        presets: {},
      };
      return JSON.stringify(existing, null, 2);
    },
    importProfileJson: async (rawJson: string): Promise<GameProfile> => {
      const parsed = JSON.parse(rawJson) as GameProfile;
      if (!parsed.game_id || !parsed.trainer_id) {
        throw new Error("Imported profile requires game_id and trainer_id");
      }
      profiles.set(`${parsed.game_id}:${parsed.trainer_id}`, parsed);
      pushLog("info", "profile", `Imported profile for ${parsed.game_id}`);
      return parsed;
    },
    getDiagnosticLogs: async (): Promise<DiagnosticEvent[]> => [...logs],
    discoverRunningProcesses: async (gameId: string): Promise<ProcessIdentity[]> => [
      {
        pid: 14820,
        process_name: gameId === "hollow-knight" ? "hollow_knight.exe" : "SampleRPG.exe",
        executable_path:
          gameId === "hollow-knight"
            ? "C:\\GOG Games\\Hollow Knight\\hollow_knight.exe"
            : "C:\\Games\\Steam\\steamapps\\common\\SampleRPG\\SampleRPG.exe",
        platform: "windows-x64",
        architecture: "x86_64",
        executable_sha256: null,
        detected_version: "1.2.0",
        process_creation_time_100ns: 133700000000000000,
      },
    ],
    startTrainerSession: async (
      _gameId: string,
      trainerId: string,
      targetProcess: ProcessIdentity,
    ): Promise<string> => {
      const sid = "preview-session-01";
      activeToggles.clear();
      activeSession = {
        session_id: sid,
        runtime_kind: "declarative_patch_v1 (preview)",
        state: "attached",
        active_patches: 0,
        rollback_failures: 0,
        rollback_conflicts: 0,
        active_freeze_features: 0,
        target_pid: targetProcess.pid,
        target_process_name: targetProcess.process_name,
        recent_events: [`Attached ${trainerId} to PID ${targetProcess.pid}`],
      };
      pushLog("info", "runtime", `Started preview session on PID ${targetProcess.pid}`);
      return sid;
    },
    applyTrainerFeature: async (_sessionId: string, command: FeatureCommand): Promise<void> => {
      if (!activeSession) throw new Error("No active session");
      if (command.value.type === "toggle") {
        if (command.value.value) activeToggles.add(command.feature_id);
        else activeToggles.delete(command.feature_id);
      }
      activeSession = {
        ...activeSession,
        active_patches: activeToggles.size,
        recent_events: [
          ...activeSession.recent_events,
          `Applied '${command.feature_id}' (${command.value.type})`,
        ],
      };
    },
    stopTrainerSession: async (_sessionId: string): Promise<RuntimeDiagnostics> => {
      const stopped: RuntimeDiagnostics = {
        ...(activeSession ?? {
          session_id: "preview-session-01",
          runtime_kind: "declarative_patch_v1 (preview)",
          state: "stopped",
          active_patches: 0,
          rollback_failures: 0,
          rollback_conflicts: 0,
          active_freeze_features: 0,
          target_pid: null,
          target_process_name: null,
          recent_events: [],
        }),
        state: "stopped",
        active_patches: 0,
        active_freeze_features: 0,
        recent_events: [
          ...(activeSession?.recent_events ?? []),
          "Rolled back active patches via PatchLedger (LIFO)",
        ],
      };
      activeToggles.clear();
      activeSession = null;
      pushLog("info", "runtime", "Stopped preview session and verified rollback");
      return stopped;
    },
    getSessionDiagnostics: async (_sessionId: string): Promise<RuntimeDiagnostics> => {
      if (!activeSession) throw new Error("No active session");
      return activeSession;
    },
    listRegistrySources: async (): Promise<RegistrySourceState[]> => registrySources,
    addThirdPartyRegistrySource: async (
      repositoryId: string,
      indexUrl: string,
      enabled: boolean,
    ): Promise<void> => {
      registrySources = [
        ...registrySources,
        {
          source: {
            repository_id: repositoryId,
            index_url: indexUrl,
            authoritative_trust_tier: "unverified",
            enabled,
          },
          etag: null,
          last_modified: null,
          last_synced_at: null,
          last_status: "pending",
          last_error: null,
        },
      ];
    },
    removeRegistrySource: async (repositoryId: string): Promise<void> => {
      registrySources = registrySources.filter((r) => r.source.repository_id !== repositoryId);
    },
    syncRegistrySources: async (): Promise<RegistrySyncReport> => ({
      sources_checked: registrySources.length,
      updated: 0,
      not_modified: registrySources.length,
      failed: 0,
      revoked_trainer_ids: [],
      errors: [],
    }),
    listCommunityTrainersForGame: async (gameId: string): Promise<CommunityTrainerView[]> => [
      {
        repository_id: "org.nekod.community-index",
        trainer_id: `dev.community.${gameId}-helper`,
        game_id: gameId,
        latest_version: "1.1.0",
        manifest_url: `https://example.org/trainers/${gameId}/trainer-manifest.json`,
        manifest_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        trust_tier: "community_verified",
        compatibility_status: "verified",
        last_verified_game_version: "1.2.0",
        install_state: { state: "not_installed" },
      },
    ],
    checkTrainerUpdates: async (): Promise<TrainerUpdateInfo[]> => [
      {
        repository_id: "org.nekod.community-index",
        trainer_id: "dev.example.sample-singleplayer-rpg",
        game_id: "sample-singleplayer-rpg",
        installed_version: "1.0.0",
        available_version: "1.1.0",
        trust_tier: "community_verified",
      },
    ],
    installOrUpdateTrainerFromRegistry: async (
      _repositoryId: string,
      trainerId: string,
    ): Promise<void> => {
      pushLog("info", "registry", `Simulated SHA-256 verified install for ${trainerId}`);
    },
    inspectGameEnvironment: async (installDir: string): Promise<GameEnvironmentProfile> => {
      const isHollow = installDir.toLowerCase().includes("hollow");
      return {
        install_dir: installDir,
        dir_exists: true,
        engine: isHollow ? "unity_mono" : "native_unknown",
        installed_loaders: isHollow
          ? [{ kind: "bep_in_ex_mono", entry_path: "BepInEx/core", detected_version: "5.4.22" }]
          : [],
        anti_cheat_files_present: [],
        recommended_adapters: isHollow
          ? ["unity_bridge_v1", "declarative_patch_v1"]
          : ["declarative_patch_v1", "ce_script_v1"],
      };
    },
    queryCompatibilityMatrix: async (gameId: string): Promise<CompatibilityRecord[]> => [
      {
        record_id: 1,
        game_id: gameId,
        game_version: "1.2.0",
        store_platform: "steam",
        runtime_adapter: "declarative_patch_v1",
        runtime_version: "1.0.0",
        mod_id: `dev.example.${gameId}`,
        mod_version: "1.0.0",
        status: "verified",
        evidence: "automated_runtime_test",
        notes: "Verified rollback & clean attach in preview matrix.",
        recorded_at: 1763265600,
      },
    ],
    getBridgeDeploymentStatus: async (
      gameKey: string,
      installDir: string,
    ): Promise<BridgeDeploymentStatus> => {
      const isHollow = installDir.toLowerCase().includes("hollow");
      const files = deployedBridges.get(gameKey) ?? [];
      const matched: CompanionBridgeCatalogEntry | null = isHollow
        ? {
            id: "nekod-unity-bridge-mono",
            display_name: "NekodUnityBridge (Mono · 零 UnityExplorer)",
            engine: "unity_mono",
            version: "0.2.0",
            target_relative_path: "BepInEx/plugins/NekodUnityBridge.Mono.dll",
            description: "独立 BepInEx 5 Mono 伴生桥接插件，无需 UnityExplorer。",
            ready_locally: true,
            source_kind: "workspace_dist",
            resolved_path: "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
            sha256: "e3b0c44298fc1c149afbf4c8996fb924",
            download_url: null,
          }
        : null;
      return {
        deployed: files.length > 0,
        engine: isHollow ? "unity_mono" : "native_unknown",
        active_files: files,
        suggested_source_file: matched?.resolved_path ?? null,
        suggested_target_relative_path: matched?.target_relative_path ?? null,
        matched_bridge: matched,
      };
    },
    listCompanionBridgeCatalog: async (): Promise<CompanionBridgeCatalogEntry[]> => [
      {
        id: "nekod-unity-bridge-mono",
        display_name: "NekodUnityBridge (Mono · 零 UnityExplorer)",
        engine: "unity_mono",
        version: "0.2.0",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.Mono.dll",
        description: "独立 BepInEx 5 Mono 伴生桥接插件，无需 UnityExplorer。",
        ready_locally: true,
        source_kind: "workspace_dist",
        resolved_path: "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
        sha256: "preview-mono-sha256",
        download_url: null,
      },
      {
        id: "nekod-unity-bridge-il2cpp",
        display_name: "NekodUnityBridge (IL2CPP · 零 UnityExplorer)",
        engine: "unity_il2cpp",
        version: "0.2.0",
        target_relative_path: "BepInEx/plugins/NekodUnityBridge.IL2CPP.dll",
        description: "独立 BepInEx 6 IL2CPP 伴生桥接插件，无需 UnityExplorer。",
        ready_locally: true,
        source_kind: "workspace_dist",
        resolved_path: "plugins/unity-nekod-bridge/dist/il2cpp/NekodUnityBridge.IL2CPP.dll",
        sha256: "preview-il2cpp-sha256",
        download_url: null,
      },
      {
        id: "nekod-reframework-bridge",
        display_name: "Nekod REFramework Lua Bridge",
        engine: "re_engine",
        version: "0.1.0",
        target_relative_path: "reframework/autorun/nekod_bridge.lua",
        description: "面向 RE Engine 的 REFramework Lua 伴生脚本。",
        ready_locally: true,
        source_kind: "workspace_dist",
        resolved_path: "plugins/reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua",
        sha256: "preview-ref-sha256",
        download_url: null,
      },
      {
        id: "nekod-ue4ss-bridge",
        display_name: "Nekod UE4SS Lua Bridge",
        engine: "unreal_engine",
        version: "0.1.0",
        target_relative_path: "ue4ss/Mods/NekodBridge/Scripts/main.lua",
        description: "面向 Unreal Engine 4/5 的 UE4SS Lua 伴生脚本。",
        ready_locally: true,
        source_kind: "workspace_dist",
        resolved_path: "plugins/ue4ss-nekod-bridge/Scripts/main.lua",
        sha256: "preview-ue4ss-sha256",
        download_url: null,
      },
      {
        id: "nekod-godot-bridge",
        display_name: "Nekod Godot GDScript Bridge",
        engine: "godot",
        version: "0.1.0",
        target_relative_path: "scripts/nekod_bridge.gd",
        description: "面向 Godot 单机游戏的本地 GDScript 桥接脚本。",
        ready_locally: true,
        source_kind: "workspace_dist",
        resolved_path: "plugins/godot-nekod-bridge/nekod_bridge.gd",
        sha256: "preview-godot-sha256",
        download_url: null,
      },
    ],
    autoDeployCompanionBridge: async (
      gameKey: string,
      installDir: string,
      _customSourceOverride: string | null = null,
    ): Promise<BridgeDeploymentStatus> => {
      const target = "BepInEx/plugins/NekodUnityBridge.Mono.dll";
      deployedBridges.set(gameKey, [target]);
      pushLog(
        "info",
        "deployment",
        `Auto-deployed companion bridge '${target}' for ${gameKey} (${installDir})`,
      );
      return {
        deployed: true,
        engine: "unity_mono",
        active_files: [target],
        suggested_source_file:
          "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
        suggested_target_relative_path: target,
        matched_bridge: {
          id: "nekod-unity-bridge-mono",
          display_name: "NekodUnityBridge (Mono · 零 UnityExplorer)",
          engine: "unity_mono",
          version: "0.2.0",
          target_relative_path: target,
          description: "独立 BepInEx 5 Mono 伴生桥接插件，无需 UnityExplorer。",
          ready_locally: true,
          source_kind: "workspace_dist",
          resolved_path:
            "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll",
          sha256: "preview-mono-sha256",
          download_url: null,
        },
      };
    },
    deployCompanionBridge: async (
      gameKey: string,
      installDir: string,
      _sourceFile: string,
      targetRelativePath: string,
    ): Promise<BridgeDeploymentStatus> => {
      const isHollow = installDir.toLowerCase().includes("hollow");
      const next = [targetRelativePath];
      deployedBridges.set(gameKey, next);
      pushLog("info", "deployment", `Staged companion bridge '${targetRelativePath}' for ${gameKey}`);
      return {
        deployed: true,
        engine: isHollow ? "unity_mono" : "native_unknown",
        active_files: next,
        suggested_source_file: isHollow
          ? "plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll"
          : null,
        suggested_target_relative_path: targetRelativePath,
        matched_bridge: null,
      };
    },
    undeployCompanionBridge: async (
      gameKey: string,
      _installDir: string,
    ): Promise<BridgeUndeploySummary> => {
      const prev = deployedBridges.get(gameKey) ?? [];
      deployedBridges.delete(gameKey);
      pushLog("info", "deployment", `Cleanly undeployed ${prev.length} bridge file(s) for ${gameKey}`);
      return {
        removed_count: prev.length,
        restored_originals: 0,
        conflicts: 0,
        conflict_paths: [],
      };
    },
    syncGlobalHotkeys: async (bindings: GlobalHotkeyBinding[]): Promise<number> => {
      pushLog("info", "hotkeys", `Synced ${bindings.length} global hotkey binding(s) (preview)`);
      return bindings.length;
    },
    getGameArtwork: async (
      _storeSource: string,
      storeAppId: string | null,
      installDir: string,
    ): Promise<GameArtwork> => {
      const isHollow =
        storeAppId === "367520" || installDir.toLowerCase().includes("hollow");
      const [c1, c2, label] = isHollow
        ? ["#1e1b4b", "#312e81", "HK"]
        : ["#0f172a", "#1e3a5f", "RPG"];
      const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='460' height='215'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0%' stop-color='${c1}'/><stop offset='100%' stop-color='${c2}'/></linearGradient></defs><rect width='460' height='215' fill='url(#g)'/><circle cx='380' cy='50' r='90' fill='rgba(79,140,255,0.22)'/><text x='28' y='132' fill='rgba(255,255,255,0.18)' font-family='Segoe UI,sans-serif' font-size='88' font-weight='800'>${label}</text></svg>`;
      const uri = `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
      return { thumbnail_data_uri: uri, header_data_uri: uri };
    },
    launchGame: async (
      storeSource: string,
      storeAppId: string | null,
      installDir: string,
      executablePath: string | null,
    ): Promise<LaunchGameResult> => {
      const target =
        storeSource === "steam" && storeAppId
          ? `steam://rungameid/${storeAppId}`
          : (executablePath ?? installDir);
      pushLog("info", "launcher", `Simulated game launch via ${target}`);
      return {
        method: storeSource === "steam" && storeAppId ? "steam_uri" : "direct_executable",
        target,
      };
    },
    onGlobalHotkey: async (
      _handler: (event: GlobalHotkeyEvent) => void,
    ): Promise<() => void> => {
      return () => {};
    },
  };
}
