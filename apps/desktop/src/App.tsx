import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Activity, Check, X } from "lucide-react";
import {
  BridgeDeploymentStatus,
  CommunityTrainerView,
  CompanionBridgeCatalogEntry,
  CompatibilityRecord,
  CtImportUiReport,
  CustomGamePathRecord,
  DiagnosticEvent,
  FeatureCommand,
  GameArtwork,
  GameEnvironmentProfile,
  GameProfile,
  HudOverlayFeatureItem,
  HudOverlayState,
  LibraryGameView,
  LoaderProgressEvent,
  LocalNmodCandidate,
  ProcessIdentity,
  RegistrySourceState,
  SecurityPolicyConfig,
  TrainerDetailView,
  TrainerFeature,
  TrainerUpdateInfo,
  TrustTier,
  isBrowserPreview,
  nekodApi,
} from "./api/nekod";
import { GameSidebar } from "./components/GameSidebar";
import { GameWorkbench } from "./components/GameWorkbench";
import { ImportCenterView } from "./components/ImportCenterView";
import { LogsView } from "./components/LogsView";
import { AddGameModal, DropZoneOverlay } from "./components/Modals";
import { ModStoreView } from "./components/ModStoreView";
import { SettingsView } from "./components/SettingsView";
import { TitleBar } from "./components/TitleBar";
import { useI18n } from "./i18n";
import {
  AUTO_ATTACH_STORAGE_KEY,
  ENGINE_LABELS,
  FAV_STORAGE_KEY,
  HUD_OVERLAY_STORAGE_KEY,
  MINIMIZE_TO_TRAY_STORAGE_KEY,
  SOUND_STORAGE_KEY,
  formatOverlayFeatureItem,
  formatRuntimeError,
  inferInstallDirFromInput,
  loadStoredBool,
  loadStoredFavorites,
  normalizePathForCompare,
  type ActiveTrainerSession,
  type HudToast,
  type NavTab,
  type SidebarFilter,
} from "./types/ui";
import { soundFx } from "./utils/sound";

export default function App() {
  const { locale, m } = useI18n();
  const mRef = useRef(m);
  mRef.current = m;
  const localeRef = useRef(locale);
  localeRef.current = locale;

  const [appVersion, setAppVersion] = useState("1.0.0");
  const [tab, setTab] = useState<NavTab>("workbench");
  const [games, setGames] = useState<LibraryGameView[]>([]);
  const [search, setSearch] = useState("");
  const [sidebarFilter, setSidebarFilter] = useState<SidebarFilter>("all");
  const [favoriteKeys, setFavoriteKeys] = useState<Set<string>>(() =>
    loadStoredFavorites(),
  );
  const [artworkByGame, setArtworkByGame] = useState<Record<string, GameArtwork>>(
    {},
  );
  const [runningGameKeys, setRunningGameKeys] = useState<Set<string>>(new Set());

  const [selectedGame, setSelectedGame] = useState<LibraryGameView | null>(null);
  const [trainers, setTrainers] = useState<TrainerDetailView[]>([]);
  const [selectedTrainerId, setSelectedTrainerId] = useState<string | null>(null);
  const [customPaths, setCustomPaths] = useState<CustomGamePathRecord[]>([]);
  const [policy, setPolicy] = useState<SecurityPolicyConfig>({
    allow_third_party_native_modules: false,
    allow_unverified_sources: false,
  });
  const [logs, setLogs] = useState<DiagnosticEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [statusMsg, setStatusMsg] = useState<string | null>(null);
  const [copiedPath, setCopiedPath] = useState(false);

  // Desktop toggles
  const [soundEnabled, setSoundEnabled] = useState<boolean>(() =>
    loadStoredBool(SOUND_STORAGE_KEY, true),
  );
  const [autoAttachEnabled, setAutoAttachEnabled] = useState<boolean>(() =>
    loadStoredBool(AUTO_ATTACH_STORAGE_KEY, false),
  );
  const [hudOverlayEnabled, setHudOverlayEnabled] = useState<boolean>(() =>
    loadStoredBool(HUD_OVERLAY_STORAGE_KEY, false),
  );
  const [minimizeToTray, setMinimizeToTray] = useState<boolean>(() =>
    loadStoredBool(MINIMIZE_TO_TRAY_STORAGE_KEY, true),
  );
  const [waitingLaunchGameKey, setWaitingLaunchGameKey] = useState<string | null>(
    null,
  );
  const [globalHotkeyCount, setGlobalHotkeyCount] = useState<number>(0);
  const [hudToasts, setHudToasts] = useState<HudToast[]>([]);

  // Drag-and-drop overlay state
  const [dragOverlayVisible, setDragOverlayVisible] = useState(false);
  const [dragOverlayPaths, setDragOverlayPaths] = useState<string[]>([]);

  // Import & custom folder form states
  const [customDir, setCustomDir] = useState("");
  const [customLabel, setCustomLabel] = useState("");
  const [showAddGameModal, setShowAddGameModal] = useState(false);
  const [confirmRemoveCustomGame, setConfirmRemoveCustomGame] = useState(false);
  const [confirmDeleteTrainerId, setConfirmDeleteTrainerId] = useState<
    string | null
  >(null);
  const [trainerDir, setTrainerDir] = useState("");
  const [trainerTrust, setTrainerTrust] =
    useState<TrustTier>("community_verified");
  const [gameManifestJson, setGameManifestJson] = useState("");
  const [ctPath, setCtPath] = useState("");
  const [ctTargetGameKey, setCtTargetGameKey] = useState("");
  const [ctCustomGameId, setCtCustomGameId] = useState("");
  const [ctCustomGameName, setCtCustomGameName] = useState("");
  const [ctDefaultModule, setCtDefaultModule] = useState("");
  const [ctTrust, setCtTrust] = useState<TrustTier>("unverified");
  const [ctReport, setCtReport] = useState<CtImportUiReport | null>(null);

  // Runtime session state (Multi-Mod Same-Game Inject / Uninject)
  const [runningProcs, setRunningProcs] = useState<ProcessIdentity[]>([]);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [activeSessions, setActiveSessions] = useState<
    Record<string, ActiveTrainerSession>
  >({});
  const activeSessionsRef = useRef<Record<string, ActiveTrainerSession>>({});
  activeSessionsRef.current = activeSessions;
  const [featureInputsByTrainer, setFeatureInputsByTrainer] = useState<
    Record<string, Record<string, string>>
  >({});
  const featureInputsByTrainerRef = useRef<Record<string, Record<string, string>>>(
    {},
  );
  featureInputsByTrainerRef.current = featureInputsByTrainer;
  const applyingFeaturesRef = useRef<Set<string>>(new Set());
  const [applyingFeatures, setApplyingFeatures] = useState<Set<string>>(
    () => new Set(),
  );
  const [awaitingPermInject, setAwaitingPermInject] = useState<string | null>(null);

  // Companion Bridge Bucket & DeploymentLedger state
  const [bridgeCatalog, setBridgeCatalog] = useState<CompanionBridgeCatalogEntry[]>(
    [],
  );
  const [bridgeStatus, setBridgeStatus] = useState<BridgeDeploymentStatus | null>(
    null,
  );
  const [bridgeStatusByGame, setBridgeStatusByGame] = useState<
    Record<string, BridgeDeploymentStatus>
  >({});
  const [bridgeSourceInput, setBridgeSourceInput] = useState("");
  const [bridgeTargetInput, setBridgeTargetInput] = useState("");
  const [loaderProgress, setLoaderProgress] =
    useState<LoaderProgressEvent | null>(null);

  // Community store & profiles state
  const [registrySources, setRegistrySources] = useState<RegistrySourceState[]>(
    [],
  );
  const [communityTrainers, setCommunityTrainers] = useState<
    CommunityTrainerView[]
  >([]);
  const [allCommunityTrainers, setAllCommunityTrainers] = useState<
    CommunityTrainerView[]
  >([]);
  const [localNmods, setLocalNmods] = useState<LocalNmodCandidate[]>([]);
  const [updates, setUpdates] = useState<TrainerUpdateInfo[]>([]);
  const [profilesByTrainer, setProfilesByTrainer] = useState<
    Record<string, Record<string, string>>
  >({});
  const [repoIdInput, setRepoIdInput] = useState("");
  const [repoUrlInput, setRepoUrlInput] = useState("");
  const [gameEnv, setGameEnv] = useState<GameEnvironmentProfile | null>(null);
  const [compatRecords, setCompatRecords] = useState<CompatibilityRecord[]>([]);
  const [profilePresets, setProfilePresets] = useState<
    Record<string, Record<string, Record<string, unknown>>>
  >({});
  const [profileJsonTrainerId, setProfileJsonTrainerId] = useState<string | null>(
    null,
  );
  const [profileJsonBox, setProfileJsonBox] = useState("");
  const [diagnosticBundle, setDiagnosticBundle] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const toastSeqRef = useRef(0);
  const lastHudPayloadRef = useRef<string | null>(null);
  const hudSyncedOnceRef = useRef<boolean>(false);

  const selectedTrainer = useMemo(() => {
    if (trainers.length === 0) return null;
    return (
      trainers.find((t) => t.trainer_id === selectedTrainerId) ?? trainers[0]
    );
  }, [trainers, selectedTrainerId]);

  const uninstalledLocalNmods = useMemo(
    () => localNmods.filter((m) => !m.already_installed),
    [localNmods],
  );

  const activeSessionList = useMemo(
    () => Object.values(activeSessions),
    [activeSessions],
  );
  const activeGameKey = activeSessionList[0]?.gameKey ?? null;
  const activeGameEntry = useMemo(
    () =>
      activeGameKey
        ? (games.find((g) => g.library_key === activeGameKey) ?? null)
        : null,
    [games, activeGameKey],
  );
  const hasAnyActiveSession = activeSessionList.length > 0;

  const isSelectedGameInjected =
    hasAnyActiveSession &&
    selectedGame !== null &&
    activeGameKey === selectedGame.library_key;

  const selectedTrainerSession =
    selectedTrainer && isSelectedGameInjected
      ? (activeSessions[selectedTrainer.trainer_id] ?? null)
      : null;

  const sessionDiag =
    selectedTrainerSession?.diag ??
    (isSelectedGameInjected ? (activeSessionList[0]?.diag ?? null) : null);

  const hasAntiCheatMarkers =
    (gameEnv?.anti_cheat_files_present.length ?? 0) > 0;

  const pushHudToast = useCallback(
    (text: string, tone: HudToast["tone"] = "info") => {
      const id = ++toastSeqRef.current;
      setHudToasts((prev) => [...prev.slice(-3), { id, text, tone }]);
      window.setTimeout(() => {
        setHudToasts((prev) => prev.filter((t) => t.id !== id));
      }, 3200);
    },
    [],
  );

  async function handleCopyInstallPath(path: string) {
    try {
      await navigator.clipboard?.writeText(path);
      setCopiedPath(true);
      pushHudToast(mRef.current.app.copiedPathToast, "info");
      window.setTimeout(() => setCopiedPath(false), 1800);
    } catch {
      setStatusMsg(mRef.current.app.installPathStatus(path));
    }
  }

  function toggleFavorite(gameKey: string) {
    const next = new Set(favoriteKeys);
    if (next.has(gameKey)) {
      next.delete(gameKey);
    } else {
      next.add(gameKey);
    }
    setFavoriteKeys(next);
    try {
      localStorage.setItem(FAV_STORAGE_KEY, JSON.stringify([...next]));
    } catch {
      // Ignore storage quota errors
    }
  }

  function toggleSoundEnabled() {
    const next = !soundEnabled;
    setSoundEnabled(next);
    try {
      localStorage.setItem(SOUND_STORAGE_KEY, String(next));
    } catch {
      // Ignore
    }
    if (next) soundFx.toggleOn();
  }

  function toggleAutoAttach() {
    const next = !autoAttachEnabled;
    setAutoAttachEnabled(next);
    try {
      localStorage.setItem(AUTO_ATTACH_STORAGE_KEY, String(next));
    } catch {
      // Ignore
    }
    pushHudToast(
      next ? mRef.current.app.autoAttachOn : mRef.current.app.autoAttachOff,
      "info",
    );
  }

  function toggleHudOverlay() {
    const next = !hudOverlayEnabled;
    setHudOverlayEnabled(next);
    try {
      localStorage.setItem(HUD_OVERLAY_STORAGE_KEY, String(next));
    } catch {
      // Ignore
    }
    pushHudToast(
      next ? mRef.current.app.hudOverlayOn : mRef.current.app.hudOverlayOff,
      "info",
    );
  }

  function toggleMinimizeToTray() {
    const next = !minimizeToTray;
    setMinimizeToTray(next);
    try {
      localStorage.setItem(MINIMIZE_TO_TRAY_STORAGE_KEY, String(next));
    } catch {
      // Ignore
    }
    void nekodApi.setMinimizeToTray(next).catch(() => {});
    pushHudToast(next ? mRef.current.app.trayOn : mRef.current.app.trayOff, "info");
  }

  async function loadArtworksForLibrary(lib: LibraryGameView[]) {
    const results = await Promise.all(
      lib.map(async (entry) => {
        try {
          const art = await nekodApi.getGameArtwork(
            entry.game.store_source,
            entry.game.store_app_id,
            entry.game.install_dir,
          );
          return [entry.library_key, art] as const;
        } catch {
          return [
            entry.library_key,
            { thumbnail_data_uri: null, header_data_uri: null },
          ] as const;
        }
      }),
    );
    setArtworkByGame((prev) => {
      const merged = { ...prev };
      for (const [k, v] of results) {
        merged[k] = v;
      }
      return merged;
    });
  }

  async function refreshAll(autoSelectFirst = false) {
    try {
      setError(null);
      const [
        lib,
        paths,
        sec,
        diag,
        sources,
        availUpdates,
        catalog,
        storeCatalog,
      ] = await Promise.all([
        nekodApi.listLibraryGames(),
        nekodApi.listCustomGamePaths(),
        nekodApi.getSecurityPolicy(),
        nekodApi.getDiagnosticLogs(),
        nekodApi.listRegistrySources(),
        nekodApi.checkTrainerUpdates(),
        nekodApi.listCompanionBridgeCatalog(),
        nekodApi.listAllCommunityTrainers().catch(() => []),
      ]);
      setGames(lib);
      setCustomPaths(paths);
      setPolicy(sec);
      setLogs(diag);
      setRegistrySources(sources);
      setUpdates(availUpdates);
      setBridgeCatalog(catalog);
      setAllCommunityTrainers(storeCatalog);
      void loadArtworksForLibrary(lib);
      if (autoSelectFirst && lib.length > 0) {
        await openGame(lib[0], false);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void nekodApi
      .getAppVersion()
      .then((ver) => {
        if (ver?.trim()) setAppVersion(ver.trim());
      })
      .catch(() => {});
    void nekodApi.setMinimizeToTray(minimizeToTray).catch(() => {});
    void refreshAll(true);
  }, []);

  // Background lightweight poll of running processes across game library
  useEffect(() => {
    let cancelled = false;
    async function pollRunningLibraryGames() {
      if (games.length === 0) return;
      const running = new Set<string>();
      await Promise.all(
        games.map(async (g) => {
          try {
            const procs = await nekodApi.discoverRunningProcesses(
              g.game.matched_game_id,
              g.game.install_dir,
            );
            if (procs.length > 0) {
              running.add(g.library_key);
            }
          } catch {
            // Ignore transient poll errors
          }
        }),
      );
      if (!cancelled) {
        setRunningGameKeys(running);
      }
    }

    void pollRunningLibraryGames();
    const timer = window.setInterval(() => {
      void pollRunningLibraryGames();
    }, 6000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [games]);

  const handleRescan = useCallback(
    async (showSummaryStatus = true): Promise<LibraryGameView[]> => {
      try {
        setBusyAction("rescan");
        setError(null);
        const lib = await nekodApi.rescanGames();
        setGames(lib);
        setLogs(await nekodApi.getDiagnosticLogs());
        void loadArtworksForLibrary(lib);
        if (showSummaryStatus) {
          setStatusMsg(mRef.current.app.rescanSummary(lib.length));
        }
        if (lib.length > 0 && !selectedGame) {
          await openGame(lib[0], false);
        } else if (selectedGame) {
          const refreshed = lib.find(
            (g) => g.library_key === selectedGame.library_key,
          );
          if (refreshed) {
            await openGame(refreshed, false);
          } else if (lib.length > 0) {
            await openGame(lib[0], false);
          } else {
            setSelectedGame(null);
            setTrainers([]);
            setSelectedTrainerId(null);
          }
        }
        return lib;
      } catch (e) {
        setError(String(e));
        return [];
      } finally {
        setBusyAction(null);
      }
    },
    [selectedGame],
  );

  // Tauri native drag-and-drop listener (.nmod, .zip, .ct, or package directory)
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void nekodApi
      .onDragDropFiles((ev) => {
        if (ev.type === "enter") {
          setDragOverlayPaths(ev.paths ?? []);
          setDragOverlayVisible(true);
        } else if (ev.type === "leave") {
          setDragOverlayVisible(false);
          setDragOverlayPaths([]);
        } else if (ev.type === "drop") {
          setDragOverlayVisible(false);
          setDragOverlayPaths([]);
          const paths = ev.paths ?? [];
          if (paths.length === 0) return;
          const first = paths[0];
          const lower = first.toLowerCase();
          if (lower.endsWith(".ct") || lower.endsWith(".xml")) {
            setCtPath(first);
            setTab("import");
            pushHudToast(mRef.current.app.ctDropToast, "info");
            return;
          }
          void (async () => {
            try {
              setBusyAction("drop-import");
              setError(null);
              await nekodApi.ingestLocalTrainer(first, "community_verified");
              await handleRescan(false);
              if (soundEnabled) soundFx.injectSuccess();
              pushHudToast(mRef.current.app.dropImportOkToast, "ok");
              setStatusMsg(mRef.current.app.dropImportStatus(first));
            } catch (e) {
              if (soundEnabled) soundFx.error();
              setError(String(e));
            } finally {
              setBusyAction(null);
            }
          })();
        }
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, [handleRescan, pushHudToast, soundEnabled]);

  async function scanRunningProcesses(
    gameId: string | null,
    installDir?: string | null,
  ): Promise<ProcessIdentity[]> {
    try {
      const procs = await nekodApi.discoverRunningProcesses(gameId, installDir);
      setRunningProcs(procs);
      setSelectedPid((prev) =>
        procs.some((p) => p.pid === prev) ? prev : (procs[0]?.pid ?? null),
      );
      return procs;
    } catch (e) {
      setError(String(e));
      return [];
    }
  }

  async function refreshBridgeStatus(entry: LibraryGameView) {
    try {
      const status = await nekodApi.getBridgeDeploymentStatus(
        entry.library_key,
        entry.game.install_dir,
      );
      setBridgeStatus(status);
      setBridgeStatusByGame((prev) => ({ ...prev, [entry.library_key]: status }));
      setBridgeSourceInput(
        status.matched_bridge?.resolved_path ?? status.suggested_source_file ?? "",
      );
      setBridgeTargetInput(
        status.matched_bridge?.target_relative_path ??
          status.suggested_target_relative_path ??
          "",
      );
    } catch {
      setBridgeStatus(null);
    }
  }

  async function openGame(entry: LibraryGameView, switchToWorkbench = true) {
    if (switchToWorkbench) {
      setTab("workbench");
    }
    setSelectedGame(entry);
    setStatusMsg(null);
    setError(null);
    setCopiedPath(false);
    setAwaitingPermInject(null);
    setConfirmDeleteTrainerId(null);
    setConfirmRemoveCustomGame(false);
    setRunningProcs([]);
    setSelectedPid(null);
    setGameEnv(null);
    setCompatRecords([]);
    setLocalNmods([]);
    setProfileJsonTrainerId(null);

    await Promise.all([
      (async () => {
        try {
          const env = await nekodApi.inspectGameEnvironment(
            entry.game.install_dir,
          );
          setGameEnv(env);
        } catch {
          // Non-fatal
        }
      })(),
      (async () => {
        try {
          const cands = await nekodApi.discoverLocalNmods({
            gameId: entry.game.matched_game_id,
            steamAppId: entry.game.store_app_id,
            installDir: entry.game.install_dir,
            executablePath: entry.game.executable_path,
          });
          setLocalNmods(cands);
        } catch {
          // Non-fatal
        }
      })(),
      refreshBridgeStatus(entry),
    ]);

    if (entry.game.matched_game_id) {
      const gameId = entry.game.matched_game_id;
      try {
        const [list, procs, community, matrix] = await Promise.all([
          nekodApi.listTrainersForGame(gameId, entry.game.store_build_id),
          nekodApi.discoverRunningProcesses(gameId, entry.game.install_dir),
          nekodApi.listCommunityTrainersForGame(gameId),
          nekodApi.queryCompatibilityMatrix(gameId, null, null),
        ]);
        setTrainers(list);
        setSelectedTrainerId((prev) => {
          if (prev && list.some((t) => t.trainer_id === prev)) {
            return prev;
          }
          if (activeGameKey === entry.library_key) {
            const firstActive = list.find((t) =>
              Boolean(activeSessionsRef.current[t.trainer_id]),
            );
            if (firstActive) {
              return firstActive.trainer_id;
            }
          }
          return list[0]?.trainer_id ?? null;
        });
        setRunningProcs(procs);
        setSelectedPid(procs[0]?.pid ?? null);
        setCommunityTrainers(community);
        setCompatRecords(matrix);

        const loadedProfiles: Record<string, Record<string, string>> = {};
        const loadedPresets: Record<
          string,
          Record<string, Record<string, unknown>>
        > = {};
        await Promise.all(
          list.map(async (trainer) => {
            const prof = await nekodApi.loadProfile(gameId, trainer.trainer_id);
            const map: Record<string, string> = {};
            for (const f of trainer.features) {
              const saved = prof?.hotkeys[f.id];
              map[f.id] = saved ?? f.default_hotkey ?? "";
            }
            loadedProfiles[trainer.trainer_id] = map;
            loadedPresets[trainer.trainer_id] = prof?.presets ?? {};
          }),
        );
        setProfilesByTrainer(loadedProfiles);
        setProfilePresets(loadedPresets);
      } catch (e) {
        setError(String(e));
      }
    } else {
      setTrainers([]);
      setSelectedTrainerId(null);
      setCommunityTrainers([]);
      setCompatRecords([]);
      try {
        const procs = await nekodApi.discoverRunningProcesses(
          null,
          entry.game.install_dir,
        );
        setRunningProcs(procs);
        setSelectedPid(procs[0]?.pid ?? null);
      } catch {
        // Non-fatal
      }
    }
  }

  async function startSessionInternal(
    trainer: TrainerDetailView,
    overrideProcs?: ProcessIdentity[],
    fromAutoAttach = false,
  ) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId || !selectedGame) return;
    if (hasAntiCheatMarkers) {
      if (soundEnabled) soundFx.error();
      setError(mRef.current.app.antiCheatRejectInject);
      return;
    }

    const procs = overrideProcs ?? runningProcs;
    const proc = procs.find((p) => p.pid === selectedPid) ?? procs[0];
    if (!proc) {
      if (!fromAutoAttach) {
        if (soundEnabled) soundFx.error();
        setError(mRef.current.app.noProcessError);
      }
      return;
    }

    try {
      setBusyAction("inject");
      setError(null);
      const sessionId = await nekodApi.startTrainerSession(
        gameId,
        trainer.trainer_id,
        proc,
      );
      const diag = await nekodApi.getSessionDiagnostics(sessionId);
      const gameKey = selectedGame.library_key;
      const trainerId = trainer.trainer_id;
      setActiveSessions((prev) => ({
        ...prev,
        [trainerId]: {
          sessionId,
          gameKey,
          trainerId,
          trainerName: trainer.name,
          targetPid: proc.pid,
          features: trainer.features,
          diag,
          toggles: {},
          triggeredActions: {},
        },
      }));
      setAwaitingPermInject(null);
      setWaitingLaunchGameKey(null);
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      const msg = fromAutoAttach
        ? mRef.current.app.autoInjectSuccess(
            trainer.name,
            proc.process_name,
            proc.pid,
          )
        : mRef.current.app.injectSuccess(
            trainer.name,
            proc.process_name,
            proc.pid,
          );
      setStatusMsg(msg);
      pushHudToast(msg, "ok");
    } catch (e) {
      const errText = String(e);
      const lower = errText.toLowerCase();
      const isTransientBridgeStartup =
        fromAutoAttach &&
        (lower.includes("not ready") ||
          lower.includes("discovery record") ||
          lower.includes("connection refused") ||
          lower.includes("10061") ||
          lower.includes("timed out") ||
          lower.includes("10060"));
      if (isTransientBridgeStartup) {
        setStatusMsg(
          mRef.current.app.waitingBridgeReady(proc.process_name, proc.pid),
        );
      } else {
        if (soundEnabled) soundFx.error();
        setError(errText);
      }
    } finally {
      setBusyAction(null);
    }
  }

  async function handleOneClickInject(trainer: TrainerDetailView) {
    if (!selectedGame?.game.matched_game_id) return;
    if (hasAnyActiveSession && !isSelectedGameInjected) {
      setError(mRef.current.app.otherGameSessionActive);
      return;
    }
    if (activeSessions[trainer.trainer_id]) {
      setStatusMsg(mRef.current.app.trainerAlreadyRunning(trainer.name));
      return;
    }
    if (hasAntiCheatMarkers) {
      if (soundEnabled) soundFx.error();
      setError(mRef.current.app.antiCheatRejectInject);
      return;
    }
    if (trainer.permission_state.status === "blocked") {
      if (soundEnabled) soundFx.error();
      setError(
        mRef.current.app.trainerBlocked(trainer.permission_state.reason ?? ""),
      );
      return;
    }
    if (trainer.permission_state.status === "requires_confirmation") {
      setAwaitingPermInject(trainer.trainer_id);
      setStatusMsg(mRef.current.app.requiresConfirmationStatus(trainer.name));
      return;
    }

    let procs = runningProcs;
    if (procs.length === 0) {
      procs = await scanRunningProcesses(
        selectedGame.game.matched_game_id,
        selectedGame.game.install_dir,
      );
    }
    await startSessionInternal(trainer, procs, false);
  }

  async function handlePlayAndInject() {
    if (!selectedGame) return;
    if (hasAntiCheatMarkers) {
      if (soundEnabled) soundFx.error();
      setError(mRef.current.app.antiCheatRejectLaunch);
      return;
    }
    try {
      setBusyAction("launch");
      setError(null);
      if (
        bridgeStatus?.deployed &&
        bridgeStatus.bridge_update_available &&
        !isSelectedGameInjected
      ) {
        let liveProcs = runningProcs;
        try {
          liveProcs = await nekodApi.discoverRunningProcesses(
            selectedGame.game.matched_game_id,
            selectedGame.game.install_dir,
          );
        } catch {
          // fallback
        }
        if (liveProcs.length === 0) {
          const updatedBridge = await nekodApi.autoDeployCompanionBridge(
            selectedGame.library_key,
            selectedGame.game.install_dir,
            null,
          );
          setBridgeStatus(updatedBridge);
          setBridgeStatusByGame((prev) => ({
            ...prev,
            [selectedGame.library_key]: updatedBridge,
          }));
          pushHudToast(mRef.current.app.bridgeAutoUpdatedToast, "ok");
        }
      }
      const launched = await nekodApi.launchGame(
        selectedGame.game.store_source,
        selectedGame.game.store_app_id,
        selectedGame.game.install_dir,
        selectedGame.game.executable_path,
      );
      if (!selectedTrainer) {
        setWaitingLaunchGameKey(null);
        if (soundEnabled) soundFx.toggleOn();
        const isUnityEngine =
          gameEnv?.engine === "unity_mono" ||
          gameEnv?.engine === "unity_il2cpp";
        const note = isUnityEngine
          ? mRef.current.app.launchedNeedUnityStarter(launched.target)
          : gameEnv?.engine === "unreal_engine"
            ? mRef.current.app.launchedNeedUe4ssStarter(launched.target)
            : mRef.current.app.launchedNeedMod(launched.target);
        setStatusMsg(note);
        pushHudToast(note, "info");
        return;
      }
      setWaitingLaunchGameKey(selectedGame.library_key);
      if (selectedTrainer.permission_state.status === "requires_confirmation") {
        setAwaitingPermInject(selectedTrainer.trainer_id);
      }
      if (soundEnabled) soundFx.toggleOn();
      const note = mRef.current.app.launchedWaitingInject(launched.target);
      setStatusMsg(note);
      pushHudToast(note, "info");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  const autoAttachRef = useRef({
    selectedGame,
    selectedTrainer,
    hasAnyActiveSession,
    hasAntiCheatMarkers,
    busyAction,
  });
  autoAttachRef.current = {
    selectedGame,
    selectedTrainer,
    hasAnyActiveSession,
    hasAntiCheatMarkers,
    busyAction,
  };

  useEffect(() => {
    const shouldWatch =
      (autoAttachEnabled ||
        (waitingLaunchGameKey !== null &&
          waitingLaunchGameKey === selectedGame?.library_key)) &&
      !hasAnyActiveSession &&
      Boolean(selectedGame?.game.matched_game_id);
    if (!shouldWatch) return;

    const timer = window.setInterval(async () => {
      const snap = autoAttachRef.current;
      if (
        !snap.selectedGame?.game.matched_game_id ||
        snap.hasAnyActiveSession ||
        snap.hasAntiCheatMarkers ||
        snap.busyAction
      ) {
        return;
      }
      const trainer = snap.selectedTrainer;
      if (!trainer || trainer.permission_state.status !== "approved") {
        return;
      }
      const procs = await scanRunningProcesses(
        snap.selectedGame.game.matched_game_id,
        snap.selectedGame.game.install_dir,
      );
      if (procs.length > 0) {
        await startSessionInternal(trainer, procs, true);
      }
    }, 2500);

    return () => window.clearInterval(timer);
  }, [
    autoAttachEnabled,
    waitingLaunchGameKey,
    selectedGame?.library_key,
    selectedGame?.game.matched_game_id,
    hasAnyActiveSession,
  ]);

  async function handleApprovePermissions(
    trainer: TrainerDetailView,
    andInject = false,
  ) {
    try {
      setBusyAction("approve");
      setError(null);
      const scopes = trainer.permissions.map((p) => p.scope);
      await nekodApi.grantTrainerPermissions(trainer.trainer_id, scopes);
      let updatedTrainer = trainer;
      if (selectedGame?.game.matched_game_id) {
        const refreshed = await nekodApi.listTrainersForGame(
          selectedGame.game.matched_game_id,
          selectedGame.game.store_build_id,
        );
        setTrainers(refreshed);
        updatedTrainer =
          refreshed.find((t) => t.trainer_id === trainer.trainer_id) ?? trainer;
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      setAwaitingPermInject(null);
      if (andInject && selectedGame?.game.matched_game_id) {
        let procs = runningProcs;
        if (procs.length === 0) {
          procs = await scanRunningProcesses(
            selectedGame.game.matched_game_id,
            selectedGame.game.install_dir,
          );
        }
        await startSessionInternal(updatedTrainer, procs, false);
      } else {
        setStatusMsg(
          mRef.current.app.permissionsApprovedStatus(
            trainer.name,
            trainer.version,
          ),
        );
      }
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleUninject(targetTrainerId?: string) {
    const targets = targetTrainerId
      ? activeSessions[targetTrainerId]
        ? [activeSessions[targetTrainerId]]
        : []
      : [...activeSessionList].reverse();
    if (targets.length === 0) return;

    try {
      setBusyAction("uninject");
      setError(null);
      let totalConflicts = 0;
      let totalFailures = 0;
      let firstError: unknown = null;
      const stoppedIds = new Set<string>();

      for (const target of targets) {
        try {
          const diag = await nekodApi.stopTrainerSession(target.sessionId);
          totalConflicts += diag.rollback_conflicts;
          totalFailures += diag.rollback_failures;
          stoppedIds.add(target.trainerId);
        } catch (err) {
          stoppedIds.add(target.trainerId);
          if (firstError === null) firstError = err;
        }
      }

      setActiveSessions((prev) => {
        const next = { ...prev };
        for (const id of stoppedIds) {
          delete next[id];
        }
        return next;
      });
      if (firstError !== null) {
        throw firstError;
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.uninject();
      const msg =
        targets.length === 1
          ? mRef.current.app.uninjectSingleStatus(
              targets[0].trainerName,
              totalConflicts,
              totalFailures,
            )
          : mRef.current.app.uninjectMultiStatus(
              targets.length,
              totalConflicts,
              totalFailures,
            );
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      setActiveSessions((prev) => {
        if (!targetTrainerId) return {};
        const next = { ...prev };
        delete next[targetTrainerId];
        return next;
      });
      if (soundEnabled) soundFx.error();
      setError(formatRuntimeError(String(e), localeRef.current));
    } finally {
      setBusyAction(null);
    }
  }

  // Watchdog: detach all active sessions when target game exits
  useEffect(() => {
    if (!hasAnyActiveSession || !activeGameKey) return;
    const activeGame = games.find((g) => g.library_key === activeGameKey);
    if (!activeGame) return;

    let cancelled = false;
    let detaching = false;
    const targetPid = activeSessionList[0]?.targetPid ?? null;

    const timer = window.setInterval(async () => {
      if (cancelled || detaching) return;
      try {
        const procs = await nekodApi.discoverRunningProcesses(
          activeGame.game.matched_game_id,
          activeGame.game.install_dir,
        );
        if (cancelled) return;
        const stillAlive =
          procs.length > 0 &&
          (targetPid === null || procs.some((p) => p.pid === targetPid));
        if (!stillAlive) {
          detaching = true;
          const liveSessions = Object.values(activeSessionsRef.current).reverse();
          for (const s of liveSessions) {
            await nekodApi.stopTrainerSession(s.sessionId).catch(() => null);
          }
          if (cancelled) return;
          setActiveSessions({});
          setRunningProcs([]);
          setSelectedPid(null);
          try {
            setLogs(await nekodApi.getDiagnosticLogs());
          } catch {
            // Non-fatal
          }
          if (soundEnabled) soundFx.uninject();
          const msg = mRef.current.app.gameExitedStatus(activeGame.game.title);
          setStatusMsg(msg);
          pushHudToast(msg, "info");
        }
      } catch {
        // Ignore transient process scan failures
      }
    }, 3000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [
    hasAnyActiveSession,
    activeGameKey,
    activeSessionList[0]?.targetPid,
    games,
    soundEnabled,
    pushHudToast,
  ]);

  async function handleAutoDeployBridge(customOverride: string | null = null) {
    if (!selectedGame) return;
    try {
      setBusyAction("deploy-bridge");
      setError(null);
      const next = await nekodApi.autoDeployCompanionBridge(
        selectedGame.library_key,
        selectedGame.game.install_dir,
        customOverride,
      );
      setBridgeStatus(next);
      setBridgeStatusByGame((prev) => ({
        ...prev,
        [selectedGame.library_key]: next,
      }));
      try {
        setGameEnv(
          await nekodApi.inspectGameEnvironment(selectedGame.game.install_dir),
        );
      } catch {
        // Non-fatal
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      const msg = mRef.current.app.bridgeAutoDeployed(
        ENGINE_LABELS[next.engine],
        next.active_files.join(", "),
      );
      setStatusMsg(msg);
      pushHudToast(msg, "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleManualDeployBridge() {
    if (!selectedGame) return;
    const src = bridgeSourceInput.trim();
    const target = bridgeTargetInput.trim();
    if (!src || !target) {
      setError(mRef.current.app.manualBridgeInputError);
      return;
    }
    try {
      setBusyAction("deploy-bridge");
      setError(null);
      const next = await nekodApi.deployCompanionBridge(
        selectedGame.library_key,
        selectedGame.game.install_dir,
        src,
        target,
      );
      setBridgeStatus(next);
      setBridgeStatusByGame((prev) => ({
        ...prev,
        [selectedGame.library_key]: next,
      }));
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      setStatusMsg(mRef.current.app.bridgeManualDeployed(target));
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleUndeployBridge() {
    if (!selectedGame) return;
    try {
      setBusyAction("undeploy-bridge");
      setError(null);
      const summary = await nekodApi.undeployCompanionBridge(
        selectedGame.library_key,
        selectedGame.game.install_dir,
      );
      await refreshBridgeStatus(selectedGame);
      try {
        setGameEnv(
          await nekodApi.inspectGameEnvironment(selectedGame.game.install_dir),
        );
      } catch {
        // Non-fatal
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.uninject();
      const msg = mRef.current.app.bridgeUndeployed(
        summary.removed_count,
        summary.restored_originals,
        summary.conflicts,
      );
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleInstallBepInExLoader() {
    if (!selectedGame) return;
    try {
      setBusyAction("install-loader");
      setLoaderProgress({
        stage: "starting",
        current: 0,
        total: 0,
        message: mRef.current.app.loaderInitProgress,
      });
      setError(null);
      const next = await nekodApi.installBepInExLoader(
        selectedGame.library_key,
        selectedGame.game.install_dir,
      );
      setBridgeStatus(next);
      setBridgeStatusByGame((prev) => ({
        ...prev,
        [selectedGame.library_key]: next,
      }));
      try {
        setGameEnv(
          await nekodApi.inspectGameEnvironment(selectedGame.game.install_dir),
        );
      } catch {
        // Non-fatal
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      const loaderLabel =
        next.loader_display_name ??
        (next.engine === "unreal_engine" ? "RE-UE4SS" : "BepInEx");
      const shortLabel =
        next.engine === "unreal_engine" ? "RE-UE4SS" : "BepInEx";
      const msg = !next.deployed
        ? mRef.current.app.loaderInstalledNeedBridge(loaderLabel)
        : mRef.current.app.loaderInstalledNeedRestart(loaderLabel);
      setStatusMsg(msg);
      pushHudToast(mRef.current.app.loaderInstalledToast(shortLabel), "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
      setLoaderProgress(null);
    }
  }

  async function handleUndeployBepInExLoader() {
    if (!selectedGame) return;
    try {
      setBusyAction("undeploy-loader");
      setError(null);
      const summary = await nekodApi.undeployBepInExLoader(
        selectedGame.library_key,
        selectedGame.game.install_dir,
      );
      await refreshBridgeStatus(selectedGame);
      try {
        setGameEnv(
          await nekodApi.inspectGameEnvironment(selectedGame.game.install_dir),
        );
      } catch {
        // Non-fatal
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.uninject();
      const shortLabel =
        bridgeStatus?.engine === "unreal_engine" ? "RE-UE4SS" : "BepInEx";
      const msg = mRef.current.app.loaderUndeployed(
        shortLabel,
        summary.removed_count,
        summary.restored_originals,
        summary.conflicts,
      );
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleQuickSetupUnityStarter() {
    if (!selectedGame) return;
    try {
      setBusyAction("quick-setup");
      setError(null);
      const res = await nekodApi.quickSetupUnityStarter(
        selectedGame.game.title,
        selectedGame.game.store_source,
        selectedGame.game.store_app_id,
        selectedGame.game.store_build_id,
        selectedGame.game.install_dir,
      );
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      const refreshed =
        lib.find((g) => g.library_key === selectedGame.library_key) ??
        lib.find((g) => g.game.matched_game_id === res.game_id);
      if (refreshed) {
        await openGame(refreshed, false);
      }
      setSelectedTrainerId(res.trainer_id);
      setAwaitingPermInject(res.trainer_id);
      if (soundEnabled) soundFx.injectSuccess();
      const msg = mRef.current.app.unityStarterDoneStatus(res.trainer_id);
      setStatusMsg(msg);
      pushHudToast(mRef.current.app.unityStarterDoneToast, "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleQuickSetupUe4ssStarter() {
    if (!selectedGame) return;
    try {
      setBusyAction("quick-setup");
      setError(null);
      const res = await nekodApi.quickSetupUe4ssStarter(
        selectedGame.game.title,
        selectedGame.game.store_source,
        selectedGame.game.store_app_id,
        selectedGame.game.store_build_id,
        selectedGame.game.install_dir,
      );
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      const refreshed =
        lib.find((g) => g.library_key === selectedGame.library_key) ??
        lib.find((g) => g.game.matched_game_id === res.game_id);
      if (refreshed) {
        await openGame(refreshed, false);
      }
      setSelectedTrainerId(res.trainer_id);
      setAwaitingPermInject(res.trainer_id);
      if (soundEnabled) soundFx.injectSuccess();
      const msg = mRef.current.app.ue4ssStarterDoneStatus(res.trainer_id);
      setStatusMsg(msg);
      pushHudToast(mRef.current.app.ue4ssStarterDoneToast, "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleInstallLocalNmod(candidate: LocalNmodCandidate) {
    if (!selectedGame) return;
    try {
      setBusyAction(`nmod:${candidate.mod_id}`);
      setError(null);
      await nekodApi.ingestLocalTrainer(
        candidate.source_path,
        "community_verified",
      );
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      const refreshed =
        lib.find((g) => g.library_key === selectedGame.library_key) ??
        lib.find((g) => g.game.matched_game_id === candidate.target_game_id);
      if (refreshed) {
        await openGame(refreshed, false);
      }
      setSelectedTrainerId(candidate.mod_id);
      setAwaitingPermInject(candidate.mod_id);
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      const msg = mRef.current.app.nmodInstalledStatus(
        candidate.name,
        candidate.feature_count,
      );
      setStatusMsg(msg);
      pushHudToast(mRef.current.app.nmodInstalledToast(candidate.name), "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleRefreshDiagnostics() {
    if (!hasAnyActiveSession) return;
    const targets = selectedTrainerSession
      ? [selectedTrainerSession]
      : activeSessionList;
    try {
      const updated = await Promise.all(
        targets.map(async (s) => [
          s.trainerId,
          await nekodApi.getSessionDiagnostics(s.sessionId),
        ] as const),
      );
      setActiveSessions((prev) => {
        const next = { ...prev };
        for (const [id, diag] of updated) {
          if (next[id]) {
            next[id] = { ...next[id], diag };
          }
        }
        return next;
      });
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleApplyFeature(
    feature: TrainerFeature,
    triggeredHotkey?: string,
    explicitTrainerId?: string,
  ) {
    const trainerId = explicitTrainerId ?? selectedTrainer?.trainer_id;
    const targetSession = trainerId ? activeSessionsRef.current[trainerId] : null;
    if (!trainerId || !targetSession) return;
    const lockKey = `${trainerId}::${feature.id}`;
    if (applyingFeaturesRef.current.has(lockKey)) return;

    let command: FeatureCommand;
    let nextToggleVal = false;
    if (feature.control_type === "toggle") {
      nextToggleVal = !targetSession.toggles[feature.id];
      command = {
        feature_id: feature.id,
        value: { type: "toggle", value: nextToggleVal },
      };
    } else if (feature.control_type === "action") {
      command = { feature_id: feature.id, value: { type: "action" } };
    } else {
      const raw = (
        featureInputsByTrainerRef.current[trainerId]?.[feature.id] ?? ""
      ).trim();
      const num = Number(raw);
      if (!raw || !Number.isFinite(num)) {
        if (soundEnabled) soundFx.error();
        setError(mRef.current.app.invalidNumberError(feature.name));
        return;
      }
      command =
        feature.control_type === "integer"
          ? {
              feature_id: feature.id,
              value: { type: "integer", value: Math.trunc(num) },
            }
          : { feature_id: feature.id, value: { type: "float", value: num } };
    }

    applyingFeaturesRef.current.add(lockKey);
    setApplyingFeatures(new Set(applyingFeaturesRef.current));

    try {
      setError(null);
      await nekodApi.applyTrainerFeature(targetSession.sessionId, command);
      const [diag, logList] = await Promise.all([
        nekodApi.getSessionDiagnostics(targetSession.sessionId),
        nekodApi.getDiagnosticLogs(),
      ]);
      setActiveSessions((prev) => {
        const cur = prev[trainerId];
        if (!cur || cur.sessionId !== targetSession.sessionId) return prev;
        return {
          ...prev,
          [trainerId]: {
            ...cur,
            toggles:
              feature.control_type === "toggle"
                ? { ...cur.toggles, [feature.id]: nextToggleVal }
                : cur.toggles,
            triggeredActions:
              feature.control_type === "action"
                ? { ...cur.triggeredActions, [feature.id]: true }
                : cur.triggeredActions,
            diag,
          },
        };
      });
      setLogs(logList);
      if (feature.control_type === "toggle") {
        if (soundEnabled) {
          if (nextToggleVal) soundFx.toggleOn();
          else soundFx.toggleOff();
        }
        const suffix = triggeredHotkey ? ` (${triggeredHotkey})` : "";
        pushHudToast(
          nextToggleVal
            ? mRef.current.app.featureToggledOn(feature.name, suffix)
            : mRef.current.app.featureToggledOff(feature.name, suffix),
          nextToggleVal ? "ok" : "info",
        );
      } else {
        if (soundEnabled) soundFx.toggleOn();
        const suffix = triggeredHotkey ? ` (${triggeredHotkey})` : "";
        pushHudToast(
          mRef.current.app.featureTriggered(feature.name, suffix),
          "ok",
        );
      }
    } catch (e) {
      if (soundEnabled) soundFx.error();
      const rawErr = String(e);
      const lower = rawErr.toLowerCase();
      setError(formatRuntimeError(rawErr, localeRef.current));
      if (
        !lower.includes("timed out") &&
        !lower.includes("dispatch timeout") &&
        !lower.includes("busy")
      ) {
        try {
          const diag = await nekodApi.getSessionDiagnostics(
            targetSession.sessionId,
          );
          setActiveSessions((prev) => {
            const cur = prev[trainerId];
            if (!cur || cur.sessionId !== targetSession.sessionId) return prev;
            return { ...prev, [trainerId]: { ...cur, diag } };
          });
        } catch {
          // Keep UI responsive
        }
      }
    } finally {
      applyingFeaturesRef.current.delete(lockKey);
      setApplyingFeatures(new Set(applyingFeaturesRef.current));
    }
  }

  const orderedActiveSessions = useMemo(() => {
    if (!selectedTrainerId) return activeSessionList;
    const chosen = activeSessions[selectedTrainerId];
    if (!chosen) return activeSessionList;
    return [
      chosen,
      ...activeSessionList.filter((s) => s.trainerId !== selectedTrainerId),
    ];
  }, [activeSessionList, activeSessions, selectedTrainerId]);

  useEffect(() => {
    if (orderedActiveSessions.length === 0) {
      void nekodApi.syncGlobalHotkeys([]).then(setGlobalHotkeyCount);
      return;
    }
    const seenKeys = new Set<string>();
    const bindings: { feature_id: string; hotkey: string }[] = [];
    for (const session of orderedActiveSessions) {
      for (const f of session.features) {
        if (f.control_type !== "toggle" && f.control_type !== "action") continue;
        const hk = (
          profilesByTrainer[session.trainerId]?.[f.id] ??
          f.default_hotkey ??
          ""
        ).trim();
        if (!hk) continue;
        const norm = hk.toLowerCase();
        if (seenKeys.has(norm)) continue;
        seenKeys.add(norm);
        bindings.push({
          feature_id: `${session.trainerId}::${f.id}`,
          hotkey: hk,
        });
      }
    }
    void nekodApi.syncGlobalHotkeys(bindings).then(setGlobalHotkeyCount);
  }, [orderedActiveSessions, profilesByTrainer]);

  const globalHotkeyHandlerRef = useRef<
    (featureId: string, hotkey: string) => void
  >(() => {});
  globalHotkeyHandlerRef.current = (compositeId: string, hotkey: string) => {
    const sepIdx = compositeId.indexOf("::");
    const trainerId =
      sepIdx > 0 ? compositeId.slice(0, sepIdx) : (selectedTrainerId ?? undefined);
    const featureId = sepIdx > 0 ? compositeId.slice(sepIdx + 2) : compositeId;
    const session = trainerId ? activeSessionsRef.current[trainerId] : undefined;
    const feat = session?.features.find((f) => f.id === featureId);
    if (session && feat) {
      void handleApplyFeature(feat, hotkey, session.trainerId);
    }
  };

  useEffect(() => {
    let unlistenHotkey: (() => void) | null = null;
    let unlistenLoader: (() => void) | null = null;
    let unlistenTrayHud: (() => void) | null = null;
    void nekodApi
      .onGlobalHotkey((ev) => {
        globalHotkeyHandlerRef.current(ev.feature_id, ev.hotkey);
      })
      .then((fn) => {
        unlistenHotkey = fn;
      });
    void nekodApi
      .onLoaderProgress((ev) => {
        setLoaderProgress(ev);
      })
      .then((fn) => {
        unlistenLoader = fn;
      });
    void nekodApi
      .onTrayHudToggled((enabled) => {
        setHudOverlayEnabled(enabled);
        try {
          localStorage.setItem(HUD_OVERLAY_STORAGE_KEY, String(enabled));
        } catch {
          // Ignore
        }
      })
      .then((fn) => {
        unlistenTrayHud = fn;
      });
    return () => {
      if (unlistenHotkey) unlistenHotkey();
      if (unlistenLoader) unlistenLoader();
      if (unlistenTrayHud) unlistenTrayHud();
    };
  }, []);

  async function handleRevokePermissions(trainer: TrainerDetailView) {
    try {
      await nekodApi.revokeTrainerPermissions(trainer.trainer_id);
      if (selectedGame?.game.matched_game_id) {
        setTrainers(
          await nekodApi.listTrainersForGame(
            selectedGame.game.matched_game_id,
            selectedGame.game.store_build_id,
          ),
        );
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(mRef.current.app.permissionsRevoked(trainer.name));
    } catch (e) {
      setError(String(e));
    }
  }

  async function pickPathInto(
    mode: "folder" | "file",
    title: string,
    onPicked: (path: string) => void,
    extensions?: string[],
  ) {
    try {
      const picked = await nekodApi.pickLocalPath(mode, title, extensions ?? null);
      if (picked) {
        onPicked(picked);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleAddCustomPath(switchToNew = false) {
    const trimmed = customDir.trim().replace(/^"+|"+$/g, "");
    if (!trimmed) return;
    try {
      setBusyAction("add-game");
      setError(null);
      const updated = await nekodApi.addCustomGamePath(
        trimmed,
        customLabel.trim() || null,
      );
      setCustomPaths(updated);
      setCustomDir("");
      setCustomLabel("");
      setShowAddGameModal(false);

      const lib = await handleRescan(false);
      const targetDir = normalizePathForCompare(inferInstallDirFromInput(trimmed));
      const matched =
        lib.find(
          (g) =>
            g.game.store_source === "custom_path" &&
            normalizePathForCompare(g.game.install_dir) === targetDir,
        ) ??
        lib.find((g) => normalizePathForCompare(g.game.install_dir) === targetDir);

      if (matched && switchToNew) {
        await openGame(matched, true);
      }
      const msg = matched
        ? mRef.current.app.customGameImported(matched.game.title)
        : mRef.current.app.customGameAdded;
      setStatusMsg(msg);
      pushHudToast(msg, "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleRemoveCustomGameEntry(entryPath: string) {
    try {
      setError(null);
      setConfirmRemoveCustomGame(false);
      const updated = await nekodApi.removeCustomGamePath(entryPath);
      setCustomPaths(updated);
      await handleRescan(false);
      setTab("workbench");
      const msg = mRef.current.app.customGameRemoved;
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleUninstallTrainer(trainer: TrainerDetailView) {
    if (activeSessions[trainer.trainer_id]) {
      setError(mRef.current.app.uninstallStopFirstError);
      return;
    }
    try {
      setBusyAction("uninstall-trainer");
      setError(null);
      const res = await nekodApi.uninstallTrainer(trainer.trainer_id);
      setConfirmDeleteTrainerId(null);
      if (awaitingPermInject === trainer.trainer_id) {
        setAwaitingPermInject(null);
      }
      await handleRescan(false);
      const [availUpdates, diagLogs, storeCatalog] = await Promise.all([
        nekodApi.checkTrainerUpdates(),
        nekodApi.getDiagnosticLogs(),
        nekodApi.listAllCommunityTrainers().catch(() => []),
      ]);
      setUpdates(availUpdates);
      setLogs(diagLogs);
      setAllCommunityTrainers(storeCatalog);
      if (soundEnabled) soundFx.toggleOff();
      const msg = mRef.current.app.trainerUninstalled(res.name);
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleExportNmod(trainer: TrainerDetailView) {
    try {
      setBusyAction("export-nmod");
      setError(null);
      const saved = await nekodApi.exportTrainerNmod(trainer.trainer_id, null);
      if (saved) {
        setStatusMsg(mRef.current.app.nmodExportedStatus(saved));
        pushHudToast(mRef.current.app.nmodExportedToast, "ok");
      }
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleIngestTrainer() {
    if (!trainerDir.trim()) return;
    try {
      setError(null);
      await nekodApi.ingestLocalTrainer(trainerDir.trim(), trainerTrust);
      setTrainerDir("");
      await handleRescan();
      setStatusMsg(mRef.current.app.localTrainerIngested);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRegisterGameManifest() {
    if (!gameManifestJson.trim()) return;
    try {
      setError(null);
      const gameId = await nekodApi.registerGameManifest(gameManifestJson);
      setGameManifestJson("");
      await handleRescan();
      setStatusMsg(mRef.current.app.gameManifestRegistered(gameId));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleImportCtFile() {
    if (!ctPath.trim()) return;
    const effectiveKey =
      ctTargetGameKey ||
      selectedGame?.library_key ||
      games[0]?.library_key ||
      "__custom__";
    const targetGame =
      effectiveKey === "__custom__"
        ? null
        : (games.find((g) => g.library_key === effectiveKey) ?? null);
    const exeBase =
      targetGame?.game.executable_path?.split(/[/\\]/).pop()?.trim() || null;
    const gameName =
      targetGame?.game.title ??
      (ctCustomGameName.trim() || ctCustomGameId.trim() || "Custom Game");
    const gameId =
      targetGame?.game.matched_game_id ?? (ctCustomGameId.trim() || null);
    const defaultModule = ctDefaultModule.trim() || exeBase;

    try {
      setBusyAction("import-ct");
      setError(null);
      const report = await nekodApi.importCtFile({
        ctPath: ctPath.trim(),
        gameId,
        gameName,
        defaultModule,
        installDir: targetGame?.game.install_dir ?? null,
        storeSource: targetGame?.game.store_source ?? null,
        storeAppId: targetGame?.game.store_app_id ?? null,
        storeBuildId: targetGame?.game.store_build_id ?? null,
        trustTier: ctTrust,
      });
      setCtReport(report);
      setStatusMsg(report.message);
      if (report.imported) {
        const [lib, diag] = await Promise.all([
          nekodApi.rescanGames(),
          nekodApi.getDiagnosticLogs(),
        ]);
        setGames(lib);
        setLogs(diag);
        if (selectedGame?.game.matched_game_id === report.game_id) {
          setTrainers(
            await nekodApi.listTrainersForGame(
              report.game_id,
              selectedGame.game.store_build_id,
            ),
          );
        }
        if (soundEnabled) soundFx.injectSuccess();
        pushHudToast(
          mRef.current.app.ctImportedToast(report.converted_features.length),
          "ok",
        );
      } else {
        setLogs(await nekodApi.getDiagnosticLogs());
        if (soundEnabled) soundFx.error();
        pushHudToast(
          mRef.current.app.ctSkippedToast(report.skipped_entries.length),
          "warn",
        );
      }
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleTogglePolicy(next: SecurityPolicyConfig) {
    try {
      setError(null);
      await nekodApi.updateSecurityPolicy(next);
      setPolicy(next);
      setLogs(await nekodApi.getDiagnosticLogs());
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSyncRegistries() {
    try {
      setBusyAction("sync");
      setError(null);
      const report = await nekodApi.syncRegistrySources();
      const [sources, availUpdates, diag, storeCatalog] = await Promise.all([
        nekodApi.listRegistrySources(),
        nekodApi.checkTrainerUpdates(),
        nekodApi.getDiagnosticLogs(),
        nekodApi.listAllCommunityTrainers().catch(() => []),
      ]);
      setRegistrySources(sources);
      setUpdates(availUpdates);
      setLogs(diag);
      setAllCommunityTrainers(storeCatalog);
      if (selectedGame?.game.matched_game_id) {
        const gameId = selectedGame.game.matched_game_id;
        const [list, community] = await Promise.all([
          nekodApi.listTrainersForGame(gameId, selectedGame.game.store_build_id),
          nekodApi.listCommunityTrainersForGame(gameId),
        ]);
        setTrainers(list);
        setCommunityTrainers(community);
      }
      setStatusMsg(
        mRef.current.app.syncComplete(
          report.sources_checked,
          report.updated,
          report.not_modified,
          report.failed,
          report.revoked_trainer_ids.length,
        ),
      );
      if (report.errors.length > 0) {
        setError(report.errors.join(" | "));
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleAddThirdPartyRepo() {
    if (!repoIdInput.trim() || !repoUrlInput.trim()) return;
    try {
      setBusyAction("add-repo");
      setError(null);
      await nekodApi.addThirdPartyRegistrySource(
        repoIdInput.trim(),
        repoUrlInput.trim(),
        true,
      );
      setRepoIdInput("");
      setRepoUrlInput("");
      const [sources, diag] = await Promise.all([
        nekodApi.listRegistrySources(),
        nekodApi.getDiagnosticLogs(),
      ]);
      setRegistrySources(sources);
      setLogs(diag);
      setStatusMsg(mRef.current.app.repoAdded);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleRemoveRepo(repositoryId: string) {
    try {
      setError(null);
      await nekodApi.removeRegistrySource(repositoryId);
      setRegistrySources(await nekodApi.listRegistrySources());
      setUpdates(await nekodApi.checkTrainerUpdates());
      setAllCommunityTrainers(
        await nekodApi.listAllCommunityTrainers().catch(() => []),
      );
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(mRef.current.app.repoRemoved(repositoryId));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleInstallOrUpdateCommunity(
    repositoryId: string,
    trainerId: string,
  ) {
    try {
      setBusyAction(`install:${trainerId}`);
      setError(null);
      await nekodApi.installOrUpdateTrainerFromRegistry(
        repositoryId,
        trainerId,
      );
      await refreshAll(false);
      await handleRescan(false);
      if (selectedGame?.game.matched_game_id) {
        const gameId = selectedGame.game.matched_game_id;
        const [list, community] = await Promise.all([
          nekodApi.listTrainersForGame(gameId, selectedGame.game.store_build_id),
          nekodApi.listCommunityTrainersForGame(gameId),
        ]);
        setTrainers(list);
        setCommunityTrainers(community);
      }
      if (soundEnabled) soundFx.injectSuccess();
      setStatusMsg(mRef.current.app.communityInstalledStatus(trainerId));
      pushHudToast(mRef.current.app.communityInstalledToast(trainerId), "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  const handleHotkeyChange = useCallback(
    (trainerId: string, featureId: string, hk: string) => {
      setProfilesByTrainer((prev) => {
        const nextTrainerMap = {
          ...(prev[trainerId] ?? {}),
          [featureId]: hk,
        };
        const gameId = selectedGame?.game.matched_game_id;
        if (gameId) {
          void nekodApi
            .saveProfile({
              game_id: gameId,
              trainer_id: trainerId,
              hotkeys: nextTrainerMap,
              saved_feature_states:
                activeSessionsRef.current[trainerId]?.toggles ?? {},
              presets: profilePresets[trainerId] ?? {},
            })
            .catch(() => {});
        }
        return {
          ...prev,
          [trainerId]: nextTrainerMap,
        };
      });
      pushHudToast(
        hk
          ? mRef.current.app.hotkeyBoundToast(hk)
          : mRef.current.app.hotkeyClearedToast,
        "info",
      );
    },
    [selectedGame, profilePresets, pushHudToast],
  );

  const crossModHotkeyConflicts = useMemo(() => {
    if (!isSelectedGameInjected || activeSessionList.length <= 1) return [];
    const seen = new Map<string, string>();
    const conflicts = new Set<string>();
    for (const session of activeSessionList) {
      for (const f of session.features) {
        if (f.control_type !== "toggle" && f.control_type !== "action") continue;
        const hk = (
          profilesByTrainer[session.trainerId]?.[f.id] ??
          f.default_hotkey ??
          ""
        ).trim();
        if (!hk) continue;
        const norm = hk.toLowerCase();
        const prevOwner = seen.get(norm);
        if (prevOwner && prevOwner !== session.trainerId) {
          conflicts.add(hk.toUpperCase());
        } else {
          seen.set(norm, session.trainerId);
        }
      }
    }
    return [...conflicts];
  }, [isSelectedGameInjected, activeSessionList, profilesByTrainer]);

  async function handleSaveProfile(trainer: TrainerDetailView) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId) return;
    try {
      setError(null);
      const hotkeys = profilesByTrainer[trainer.trainer_id] ?? {};
      const profile: GameProfile = {
        game_id: gameId,
        trainer_id: trainer.trainer_id,
        hotkeys,
        saved_feature_states:
          activeSessions[trainer.trainer_id]?.toggles ?? {},
        presets: profilePresets[trainer.trainer_id] ?? {},
      };
      await nekodApi.saveProfile(profile);
      setStatusMsg(mRef.current.app.profileSavedStatus(trainer.name));
      pushHudToast(mRef.current.app.profileSavedToast, "ok");
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleApplyPreset(
    trainer: TrainerDetailView,
    presetName: "Story Mode" | "No Grind" | "Reset Defaults",
  ) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId) return;
    const session = activeSessions[trainer.trainer_id];
    try {
      setError(null);
      if (presetName === "Reset Defaults") {
        const defaults: Record<string, string> = {};
        for (const f of trainer.features) {
          defaults[f.id] = f.default_hotkey ?? "";
        }
        setProfilesByTrainer((prev) => ({
          ...prev,
          [trainer.trainer_id]: defaults,
        }));
        if (session) {
          for (const f of trainer.features) {
            if (f.control_type === "toggle" && session.toggles[f.id]) {
              await nekodApi.applyTrainerFeature(session.sessionId, {
                feature_id: f.id,
                value: { type: "toggle", value: false },
              });
            }
          }
          setActiveSessions((prev) => {
            const cur = prev[trainer.trainer_id];
            if (!cur) return prev;
            return {
              ...prev,
              [trainer.trainer_id]: {
                ...cur,
                toggles: {},
                triggeredActions: {},
              },
            };
          });
        }
        setStatusMsg(mRef.current.app.profileResetStatus(trainer.name));
        return;
      }

      const presetState: Record<string, unknown> = {};
      const nextToggles: Record<string, boolean> = {
        ...(session?.toggles ?? {}),
      };
      for (const f of trainer.features) {
        if (f.control_type === "toggle") {
          const enable =
            presetName === "Story Mode" ? f.risk_level === "low" : true;
          presetState[f.id] = { type: "toggle", value: enable };
          if (session) {
            await nekodApi.applyTrainerFeature(session.sessionId, {
              feature_id: f.id,
              value: { type: "toggle", value: enable },
            });
            nextToggles[f.id] = enable;
          }
        }
      }
      if (session) {
        setActiveSessions((prev) => {
          const cur = prev[trainer.trainer_id];
          if (!cur) return prev;
          return {
            ...prev,
            [trainer.trainer_id]: { ...cur, toggles: nextToggles },
          };
        });
      }
      const nextTrainerPresets = {
        ...(profilePresets[trainer.trainer_id] ?? {}),
        [presetName]: presetState,
      };
      setProfilePresets((prev) => ({
        ...prev,
        [trainer.trainer_id]: nextTrainerPresets,
      }));
      await nekodApi.saveProfile({
        game_id: gameId,
        trainer_id: trainer.trainer_id,
        hotkeys: profilesByTrainer[trainer.trainer_id] ?? {},
        saved_feature_states: session ? nextToggles : {},
        presets: nextTrainerPresets,
      });
      setStatusMsg(mRef.current.app.presetAppliedStatus(presetName));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleExportProfile(trainer: TrainerDetailView) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId) return;
    try {
      setError(null);
      const json = await nekodApi.exportProfileJson(gameId, trainer.trainer_id);
      setProfileJsonTrainerId(trainer.trainer_id);
      setProfileJsonBox(json);
      setStatusMsg(mRef.current.app.profileExportedStatus(trainer.name));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleImportProfile(trainer: TrainerDetailView) {
    if (!profileJsonBox.trim()) return;
    try {
      setError(null);
      const imported = await nekodApi.importProfileJson(profileJsonBox);
      setProfilesByTrainer((prev) => ({
        ...prev,
        [trainer.trainer_id]: imported.hotkeys,
      }));
      setProfilePresets((prev) => ({
        ...prev,
        [trainer.trainer_id]: imported.presets ?? {},
      }));
      setStatusMsg(mRef.current.app.profileImportedStatus(imported.trainer_id));
    } catch (e) {
      setError(String(e));
    }
  }

  // Browser preview fallback hotkeys
  useEffect(() => {
    if (orderedActiveSessions.length === 0 || !isBrowserPreview) return;

    function onKeyDown(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT")
      ) {
        return;
      }
      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      const keyLabel = e.key.length === 1 ? e.key.toUpperCase() : e.key;
      parts.push(keyLabel);
      const combo = parts.join("+").toLowerCase();

      for (const session of orderedActiveSessions) {
        const match = session.features.find((f) => {
          if (f.control_type !== "toggle" && f.control_type !== "action")
            return false;
          const bound =
            profilesByTrainer[session.trainerId]?.[f.id] ??
            f.default_hotkey ??
            "";
          return bound.trim().toLowerCase() === combo;
        });
        if (match) {
          e.preventDefault();
          void handleApplyFeature(match, parts.join("+"), session.trainerId);
          return;
        }
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [orderedActiveSessions, profilesByTrainer]);

  // Real-time RGB HUD overlay sync
  useEffect(() => {
    const gameTitle =
      activeGameEntry?.game.title ??
      selectedGame?.game.title ??
      orderedActiveSessions[0]?.trainerName ??
      selectedTrainer?.name ??
      "Nekod";
    const engineKind = gameEnv?.engine;
    const shortEngine =
      engineKind === "unreal_engine"
        ? "UE4SS v0.7"
        : engineKind === "unity_il2cpp"
          ? "IL2CPP"
          : engineKind === "unity_mono"
            ? "MONO"
            : engineKind === "re_engine"
              ? "RE"
              : engineKind === "godot"
                ? "GODOT"
                : "NEKOD";
    const bridgeTag =
      activeSessionList.length > 1
        ? `${shortEngine} · ${activeSessionList.length}M`
        : shortEngine;

    const items: HudOverlayFeatureItem[] = [];
    for (const session of activeSessionList) {
      const hotkeysMap = profilesByTrainer[session.trainerId] ?? {};
      const inputsMap = featureInputsByTrainer[session.trainerId] ?? {};
      for (const f of session.features) {
        const hk = hotkeysMap[f.id] ?? f.default_hotkey ?? null;
        const compositeId = `${session.trainerId}:${f.id}`;
        if (f.control_type === "toggle") {
          if (session.toggles[f.id]) {
            items.push(
              formatOverlayFeatureItem(f, undefined, hk, false, compositeId),
            );
          }
        } else if (f.control_type === "float" || f.control_type === "integer") {
          const curStr = inputsMap[f.id];
          const curNum = curStr !== undefined ? Number(curStr) : NaN;
          const defNum = f.control_type === "float" ? 1.0 : 0;
          if (!Number.isNaN(curNum) && Math.abs(curNum - defNum) > 0.001) {
            items.push(
              formatOverlayFeatureItem(f, curStr, hk, false, compositeId),
            );
          }
        } else if (f.control_type === "action") {
          if (session.triggeredActions[f.id]) {
            items.push(
              formatOverlayFeatureItem(f, undefined, hk, true, compositeId),
            );
          }
        }
      }
    }

    const activePid =
      activeSessionList[0]?.targetPid ??
      runningProcs[0]?.pid ??
      selectedPid ??
      null;
    const payload: HudOverlayState = {
      enabled: hudOverlayEnabled,
      game_title: gameTitle,
      bridge_tag: bridgeTag,
      target_pid: activePid,
      attached: hasAnyActiveSession,
      features: items,
    };

    if (!hudOverlayEnabled && !hudSyncedOnceRef.current) {
      return;
    }

    const serialized = JSON.stringify(payload);
    if (serialized === lastHudPayloadRef.current) {
      return;
    }

    const flush = () => {
      hudSyncedOnceRef.current = true;
      lastHudPayloadRef.current = serialized;
      void nekodApi.syncHudOverlayState(payload).catch(() => {});
    };

    if (!hudOverlayEnabled) {
      flush();
      return;
    }

    const timer = window.setTimeout(flush, 120);
    return () => window.clearTimeout(timer);
  }, [
    hudOverlayEnabled,
    hasAnyActiveSession,
    activeGameEntry,
    activeSessionList,
    orderedActiveSessions,
    selectedTrainer,
    selectedGame,
    gameEnv,
    runningProcs,
    selectedPid,
    featureInputsByTrainer,
    profilesByTrainer,
  ]);

  const filteredGames = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = games.filter((g) => {
      if (sidebarFilter === "favorites" && !favoriteKeys.has(g.library_key)) {
        return false;
      }
      if (sidebarFilter === "modded" && g.trainer_count === 0) {
        return false;
      }
      if (
        sidebarFilter === "running" &&
        !runningGameKeys.has(g.library_key) &&
        activeGameKey !== g.library_key
      ) {
        return false;
      }
      if (!q) return true;
      return (
        g.game.title.toLowerCase().includes(q) ||
        (g.game.matched_game_id ?? "").toLowerCase().includes(q)
      );
    });

    return [...list].sort((a, b) => {
      const rank = (item: LibraryGameView) => {
        if (hasAnyActiveSession && activeGameKey === item.library_key) return 0;
        if (runningGameKeys.has(item.library_key)) return 1;
        if (favoriteKeys.has(item.library_key)) return 2;
        if (item.trainer_count > 0) return 3;
        return 4;
      };
      const diff = rank(a) - rank(b);
      if (diff !== 0) return diff;
      return a.game.title.localeCompare(b.game.title);
    });
  }, [
    games,
    search,
    sidebarFilter,
    favoriteKeys,
    runningGameKeys,
    hasAnyActiveSession,
    activeGameKey,
  ]);

  function handleSelectTopTab(next: NavTab) {
    setTab(next);
    if (next === "community") {
      void nekodApi
        .listAllCommunityTrainers()
        .then(setAllCommunityTrainers)
        .catch(() => {});
    } else if (next === "logs") {
      void nekodApi
        .getDiagnosticLogs()
        .then(setLogs)
        .catch((e) => setError(String(e)));
    }
  }

  return (
    <div className="desktop-frame">
      <TitleBar
        appVersion={appVersion}
        tab={tab}
        updatesCount={updates.length}
        activeSessionCount={activeSessionList.length}
        activeGameTitle={activeGameEntry?.game.title ?? null}
        hudOverlayEnabled={hudOverlayEnabled}
        soundEnabled={soundEnabled}
        minimizeToTray={minimizeToTray}
        onSelectTab={handleSelectTopTab}
        onToggleHudOverlay={toggleHudOverlay}
        onToggleSound={toggleSoundEnabled}
        onOpenActiveGame={() => {
          if (activeGameEntry) {
            void openGame(activeGameEntry, true);
          }
        }}
      />

      <div className="desktop-content">
        <div className="workbench-shell">
          <GameSidebar
            games={games}
            filteredGames={filteredGames}
            selectedGame={selectedGame}
            search={search}
            sidebarFilter={sidebarFilter}
            favoriteKeys={favoriteKeys}
            runningGameKeys={runningGameKeys}
            activeGameKey={activeGameKey}
            activeSessionCount={activeSessionList.length}
            artworkByGame={artworkByGame}
            bridgeStatusByGame={bridgeStatusByGame}
            busyAction={busyAction}
            onSearchChange={setSearch}
            onFilterChange={setSidebarFilter}
            onOpenAddGame={() => {
              setError(null);
              setShowAddGameModal(true);
            }}
            onRescan={() => void handleRescan()}
            onSelectGame={(entry) => void openGame(entry, true)}
            onToggleFavorite={toggleFavorite}
          />

          <div className="right-pane">
            {(isBrowserPreview || error || statusMsg) && (
              <div className="top-banners">
                {isBrowserPreview && (
                  <div className="banner">
                    <span>{m.app.browserPreviewBanner}</span>
                  </div>
                )}
                {error && (
                  <div className="error-banner">
                    <span>{error}</span>
                    <button
                      type="button"
                      className="icon-btn"
                      onClick={() => setError(null)}
                      aria-label={m.app.closeErrorAria}
                    >
                      <X size={14} strokeWidth={1.5} />
                    </button>
                  </div>
                )}
                {statusMsg && (
                  <div className="banner">
                    <span>{statusMsg}</span>
                    <button
                      type="button"
                      className="icon-btn"
                      onClick={() => setStatusMsg(null)}
                      aria-label={m.app.closeStatusAria}
                    >
                      <X size={14} strokeWidth={1.5} />
                    </button>
                  </div>
                )}
              </div>
            )}

            {tab === "workbench" &&
              (selectedGame ? (
                <GameWorkbench
                  selectedGame={selectedGame}
                  selectedArtwork={artworkByGame[selectedGame.library_key] ?? null}
                  trainers={trainers}
                  selectedTrainer={selectedTrainer}
                  activeSessions={activeSessions}
                  activeSessionList={activeSessionList}
                  activeGameKey={activeGameKey}
                  hasAnyActiveSession={hasAnyActiveSession}
                  isSelectedGameInjected={isSelectedGameInjected}
                  isSelectedTrainerInjected={selectedTrainerSession !== null}
                  hasAntiCheatMarkers={hasAntiCheatMarkers}
                  favoriteKeys={favoriteKeys}
                  customPaths={customPaths}
                  runningProcs={runningProcs}
                  selectedPid={selectedPid}
                  gameEnv={gameEnv}
                  bridgeStatus={bridgeStatus}
                  bridgeCatalog={bridgeCatalog}
                  compatRecords={compatRecords}
                  communityTrainers={communityTrainers}
                  uninstalledLocalNmods={uninstalledLocalNmods}
                  profilesByTrainer={profilesByTrainer}
                  featureInputsByTrainer={featureInputsByTrainer}
                  applyingFeatures={applyingFeatures}
                  busyAction={busyAction}
                  copiedPath={copiedPath}
                  autoAttachEnabled={autoAttachEnabled}
                  hudOverlayEnabled={hudOverlayEnabled}
                  soundEnabled={soundEnabled}
                  globalHotkeyCount={globalHotkeyCount}
                  waitingLaunchGameKey={waitingLaunchGameKey}
                  awaitingPermInject={awaitingPermInject}
                  confirmRemoveCustomGame={confirmRemoveCustomGame}
                  confirmDeleteTrainerId={confirmDeleteTrainerId}
                  profileJsonTrainerId={profileJsonTrainerId}
                  profileJsonBox={profileJsonBox}
                  bridgeSourceInput={bridgeSourceInput}
                  bridgeTargetInput={bridgeTargetInput}
                  loaderProgress={loaderProgress}
                  crossModHotkeyConflicts={crossModHotkeyConflicts}
                  onSelectTrainer={(id) => {
                    setSelectedTrainerId(id);
                    setAwaitingPermInject(null);
                  }}
                  onSelectPid={setSelectedPid}
                  onToggleFavorite={toggleFavorite}
                  onCopyInstallPath={(p) => void handleCopyInstallPath(p)}
                  onSetConfirmRemoveCustomGame={setConfirmRemoveCustomGame}
                  onRemoveCustomGameEntry={(p) =>
                    void handleRemoveCustomGameEntry(p)
                  }
                  onPlayAndInject={() => void handlePlayAndInject()}
                  onCancelWaitingLaunch={() => setWaitingLaunchGameKey(null)}
                  onOneClickInject={(t) => void handleOneClickInject(t)}
                  onUninject={(tid) => void handleUninject(tid)}
                  onAutoDeployBridge={() => void handleAutoDeployBridge(null)}
                  onManualDeployBridge={() => void handleManualDeployBridge()}
                  onUndeployBridge={() => void handleUndeployBridge()}
                  onInstallBepInExLoader={() => void handleInstallBepInExLoader()}
                  onUndeployBepInExLoader={() =>
                    void handleUndeployBepInExLoader()
                  }
                  onQuickSetupUnityStarter={() =>
                    void handleQuickSetupUnityStarter()
                  }
                  onQuickSetupUe4ssStarter={() =>
                    void handleQuickSetupUe4ssStarter()
                  }
                  onInstallLocalNmod={(c) => void handleInstallLocalNmod(c)}
                  onApprovePermissions={(t, inj) =>
                    void handleApprovePermissions(t, inj)
                  }
                  onRevokePermissions={(t) => void handleRevokePermissions(t)}
                  onUninstallTrainer={(t) => void handleUninstallTrainer(t)}
                  onSetConfirmDeleteTrainerId={setConfirmDeleteTrainerId}
                  onExportNmod={(t) => void handleExportNmod(t)}
                  onApplyFeature={(f) => void handleApplyFeature(f)}
                  onFeatureInputChange={(tid, fid, val) => {
                    setFeatureInputsByTrainer((prev) => ({
                      ...prev,
                      [tid]: {
                        ...(prev[tid] ?? {}),
                        [fid]: val,
                      },
                    }));
                  }}
                  onHotkeyChange={handleHotkeyChange}
                  onApplyPreset={(t, p) => void handleApplyPreset(t, p)}
                  onSaveProfile={(t) => void handleSaveProfile(t)}
                  onExportProfile={(t) => void handleExportProfile(t)}
                  onImportProfile={(t) => void handleImportProfile(t)}
                  onSetProfileJsonTrainerId={setProfileJsonTrainerId}
                  onSetProfileJsonBox={setProfileJsonBox}
                  onSetBridgeSourceInput={setBridgeSourceInput}
                  onSetBridgeTargetInput={setBridgeTargetInput}
                  onScanProcesses={() =>
                    void scanRunningProcesses(
                      selectedGame.game.matched_game_id,
                      selectedGame.game.install_dir,
                    )
                  }
                  onRefreshDiagnostics={() => void handleRefreshDiagnostics()}
                  onSyncRegistries={() => void handleSyncRegistries()}
                  onInstallCommunityTrainer={(r, id) =>
                    void handleInstallOrUpdateCommunity(r, id)
                  }
                  onToggleAutoAttach={toggleAutoAttach}
                  onToggleHudOverlay={toggleHudOverlay}
                  onToggleSound={toggleSoundEnabled}
                />
              ) : (
                <div className="empty-state">
                  <div className="muted" style={{ maxWidth: 420 }}>
                    {m.workbench.selectLeftPrompt}
                  </div>
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() => handleSelectTopTab("community")}
                  >
                    {m.workbench.openStoreBtn}
                  </button>
                </div>
              ))}

            {tab === "community" && (
              <ModStoreView
                allCommunityTrainers={allCommunityTrainers}
                registrySources={registrySources}
                updates={updates}
                repoIdInput={repoIdInput}
                repoUrlInput={repoUrlInput}
                busyAction={busyAction}
                onRepoIdChange={setRepoIdInput}
                onRepoUrlChange={setRepoUrlInput}
                onSyncRegistries={() => void handleSyncRegistries()}
                onInstallOrUpdate={(r, id) =>
                  void handleInstallOrUpdateCommunity(r, id)
                }
                onAddThirdPartyRepo={() => void handleAddThirdPartyRepo()}
                onRemoveRepo={(r) => void handleRemoveRepo(r)}
              />
            )}

            {tab === "import" && (
              <ImportCenterView
                hasSelectedGame={Boolean(selectedGame)}
                games={games}
                selectedGame={selectedGame}
                customDir={customDir}
                customLabel={customLabel}
                trainerDir={trainerDir}
                trainerTrust={trainerTrust}
                gameManifestJson={gameManifestJson}
                ctPath={ctPath}
                ctTargetGameKey={ctTargetGameKey}
                ctCustomGameId={ctCustomGameId}
                ctCustomGameName={ctCustomGameName}
                ctDefaultModule={ctDefaultModule}
                ctTrust={ctTrust}
                ctReport={ctReport}
                busyAction={busyAction}
                onBackToWorkbench={() => setTab("workbench")}
                onCustomDirChange={setCustomDir}
                onCustomLabelChange={setCustomLabel}
                onPickCustomFolder={() =>
                  void pickPathInto(
                    "folder",
                    m.app.pickFolderTitle,
                    setCustomDir,
                  )
                }
                onPickCustomExe={() =>
                  void pickPathInto("file", m.app.pickExeTitle, setCustomDir, [
                    "exe",
                  ])
                }
                onSubmitCustomGame={() => void handleAddCustomPath(true)}
                onTrainerDirChange={setTrainerDir}
                onTrainerTrustChange={setTrainerTrust}
                onPickNmodFile={() =>
                  void pickPathInto("file", m.app.pickNmodTitle, setTrainerDir, [
                    "nmod",
                    "zip",
                  ])
                }
                onPickNmodFolder={() =>
                  void pickPathInto(
                    "folder",
                    m.app.pickNmodFolderTitle,
                    setTrainerDir,
                  )
                }
                onIngestTrainer={() => void handleIngestTrainer()}
                onGameManifestJsonChange={setGameManifestJson}
                onRegisterGameManifest={() => void handleRegisterGameManifest()}
                onCtPathChange={setCtPath}
                onPickCtFile={() =>
                  void pickPathInto("file", m.app.pickCtTitle, setCtPath, [
                    "ct",
                    "xml",
                  ])
                }
                onCtTargetGameKeyChange={setCtTargetGameKey}
                onCtCustomGameIdChange={setCtCustomGameId}
                onCtCustomGameNameChange={setCtCustomGameName}
                onCtDefaultModuleChange={setCtDefaultModule}
                onCtTrustChange={setCtTrust}
                onImportCtFile={() => void handleImportCtFile()}
                onOpenCtImportedGame={(gid, tid) => {
                  const target = games.find(
                    (g) => g.game.matched_game_id === gid,
                  );
                  if (target) {
                    void openGame(target, false).then(() => {
                      setSelectedTrainerId(tid);
                      setTab("workbench");
                    });
                  } else {
                    setTab("workbench");
                  }
                }}
              />
            )}

            {tab === "settings" && (
              <SettingsView
                hasSelectedGame={Boolean(selectedGame)}
                appVersion={appVersion}
                policy={policy}
                minimizeToTray={minimizeToTray}
                autoAttachEnabled={autoAttachEnabled}
                hudOverlayEnabled={hudOverlayEnabled}
                soundEnabled={soundEnabled}
                customPaths={customPaths}
                customDir={customDir}
                customLabel={customLabel}
                bridgeCatalog={bridgeCatalog}
                busyAction={busyAction}
                onBackToWorkbench={() => setTab("workbench")}
                onTogglePolicy={(next) => void handleTogglePolicy(next)}
                onToggleMinimizeToTray={toggleMinimizeToTray}
                onToggleAutoAttach={toggleAutoAttach}
                onToggleHudOverlay={toggleHudOverlay}
                onToggleSound={toggleSoundEnabled}
                onCustomDirChange={setCustomDir}
                onCustomLabelChange={setCustomLabel}
                onPickCustomFolder={() =>
                  void pickPathInto(
                    "folder",
                    m.app.pickFolderTitle,
                    setCustomDir,
                  )
                }
                onPickCustomExe={() =>
                  void pickPathInto("file", m.app.pickExeTitle, setCustomDir, [
                    "exe",
                  ])
                }
                onAddCustomPath={() => void handleAddCustomPath(false)}
                onRemoveCustomPath={(p) => void handleRemoveCustomGameEntry(p)}
              />
            )}

            {tab === "logs" && (
              <LogsView
                hasSelectedGame={Boolean(selectedGame)}
                logs={logs}
                diagnosticBundle={diagnosticBundle}
                onBackToWorkbench={() => setTab("workbench")}
                onExportBundle={() => {
                  const summary = JSON.stringify(
                    {
                      generated_at: new Date().toISOString(),
                      app_version: appVersion,
                      security_policy: policy,
                      active_session: sessionDiag,
                      active_sessions: activeSessionList.map((s) => ({
                        trainer_id: s.trainerId,
                        session_id: s.sessionId,
                        diag: s.diag,
                      })),
                      logs,
                    },
                    null,
                    2,
                  );
                  setDiagnosticBundle(summary);
                  setStatusMsg(m.app.bundleGeneratedStatus);
                }}
                onCloseBundle={() => setDiagnosticBundle(null)}
                onRefreshLogs={() => {
                  void nekodApi
                    .getDiagnosticLogs()
                    .then(setLogs)
                    .catch((e) => setError(String(e)));
                }}
              />
            )}
          </div>
        </div>
      </div>

      <AddGameModal
        open={showAddGameModal}
        customDir={customDir}
        customLabel={customLabel}
        busyAction={busyAction}
        onCustomDirChange={setCustomDir}
        onCustomLabelChange={setCustomLabel}
        onPickFolder={() =>
          void pickPathInto("folder", m.app.pickFolderTitle, setCustomDir)
        }
        onPickExe={() =>
          void pickPathInto("file", m.app.pickExeTitle, setCustomDir, ["exe"])
        }
        onSubmit={() => void handleAddCustomPath(true)}
        onClose={() => setShowAddGameModal(false)}
      />

      <DropZoneOverlay visible={dragOverlayVisible} paths={dragOverlayPaths} />

      {hudToasts.length > 0 && (
        <div className="hud-toast-stack" aria-live="polite">
          {hudToasts.map((t) => (
            <div key={t.id} className={`hud-toast ${t.tone}`}>
              {t.tone === "ok" ? (
                <Check
                  size={14}
                  strokeWidth={1.5}
                  style={{ color: "var(--ok)" }}
                />
              ) : (
                <Activity
                  size={14}
                  strokeWidth={1.5}
                  style={{ color: "var(--text-secondary)" }}
                />
              )}
              <span>{t.text}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
