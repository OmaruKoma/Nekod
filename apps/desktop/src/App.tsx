import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  ArrowLeft,
  Check,
  ChevronRight,
  Copy,
  Cpu,
  Download,
  FolderInput,
  FolderOpen,
  Globe,
  Keyboard,
  Layers,
  Package,
  Play,
  Power,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Settings,
  Shield,
  ShieldAlert,
  SlidersHorizontal,
  Star,
  Terminal,
  Trash2,
  Volume2,
  VolumeX,
  X,
  Zap,
} from "lucide-react";
import {
  BridgeDeploymentStatus,
  CommunityTrainerView,
  CompanionBridgeCatalogEntry,
  CompatibilityRecord,
  CustomGamePathRecord,
  DiagnosticEvent,
  FeatureCommand,
  GameArtwork,
  GameEnvironmentProfile,
  GameProfile,
  LibraryGameView,
  PermissionScope,
  ProcessIdentity,
  RegistrySourceState,
  RuntimeDiagnostics,
  SecurityPolicyConfig,
  TrainerDetailView,
  TrainerFeature,
  TrainerUpdateInfo,
  TrustTier,
  isBrowserPreview,
  nekodApi,
} from "./api/nekod";
import { soundFx } from "./utils/sound";

type NavTab = "workbench" | "community" | "import" | "settings" | "logs";
type SidebarFilter = "all" | "favorites" | "running";

interface HudToast {
  id: number;
  text: string;
  tone: "info" | "ok" | "warn";
}

const PERMISSION_LABELS: Record<PermissionScope, string> = {
  "process.discover": "发现运行中的游戏进程",
  "process.read_memory": "读取游戏内存数值",
  "process.write_memory": "写入游戏内存以启用模组功能",
  "process.allocate_memory": "在游戏进程中分配内存页",
  "process.suspend_resume": "打补丁时短暂停顿游戏线程",
  "filesystem.read_save_dir": "读取游戏存档目录",
  "filesystem.write_save_dir": "修改游戏存档目录文件",
  "runtime.native_module": "加载原生运行时模块（默认禁用）",
};

const ENGINE_LABELS: Record<GameEnvironmentProfile["engine"], string> = {
  unity_mono: "Unity (Mono)",
  unity_il2cpp: "Unity (IL2CPP)",
  unreal_engine: "Unreal Engine",
  re_engine: "RE Engine",
  godot: "Godot",
  native_unknown: "Native / Unknown",
};

const FAV_STORAGE_KEY = "nekod.favoriteGames.v1";
const SOUND_STORAGE_KEY = "nekod.soundFxEnabled.v1";
const AUTO_ATTACH_STORAGE_KEY = "nekod.autoAttachEnabled.v1";

function loadStoredFavorites(): Set<string> {
  try {
    const raw = localStorage.getItem(FAV_STORAGE_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw) as unknown;
    return Array.isArray(arr)
      ? new Set(arr.filter((v): v is string => typeof v === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function loadStoredBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    return raw === "true";
  } catch {
    return fallback;
  }
}

function gameMonogram(title: string): string {
  const words = title.trim().split(/\s+/).filter(Boolean);
  if (words.length >= 2) {
    return (words[0][0] + words[1][0]).toUpperCase();
  }
  return title.slice(0, 2).toUpperCase() || "GM";
}

function trustTag(tier: TrustTier) {
  if (tier === "official") return <span className="tag official">Official</span>;
  if (tier === "community_verified") {
    return <span className="tag community">Verified</span>;
  }
  return <span className="tag unverified">Unverified</span>;
}

export default function App() {
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

  // Sound FX, Auto-Attach, Global Hotkey count & HUD Toasts
  const [soundEnabled, setSoundEnabled] = useState<boolean>(() =>
    loadStoredBool(SOUND_STORAGE_KEY, true),
  );
  const [autoAttachEnabled, setAutoAttachEnabled] = useState<boolean>(() =>
    loadStoredBool(AUTO_ATTACH_STORAGE_KEY, false),
  );
  const [waitingLaunchGameKey, setWaitingLaunchGameKey] = useState<string | null>(
    null,
  );
  const [globalHotkeyCount, setGlobalHotkeyCount] = useState<number>(0);
  const [hudToasts, setHudToasts] = useState<HudToast[]>([]);

  // Import & custom folder form states
  const [customDir, setCustomDir] = useState("");
  const [customLabel, setCustomLabel] = useState("");
  const [trainerDir, setTrainerDir] = useState("");
  const [trainerTrust, setTrainerTrust] =
    useState<TrustTier>("community_verified");
  const [gameManifestJson, setGameManifestJson] = useState("");

  // Runtime session state (One-Click Inject / Uninject)
  const [runningProcs, setRunningProcs] = useState<ProcessIdentity[]>([]);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [activeGameKey, setActiveGameKey] = useState<string | null>(null);
  const [activeTrainerId, setActiveTrainerId] = useState<string | null>(null);
  const [sessionDiag, setSessionDiag] = useState<RuntimeDiagnostics | null>(null);
  const [activeToggles, setActiveToggles] = useState<Record<string, boolean>>({});
  const [featureInputs, setFeatureInputs] = useState<Record<string, string>>({});
  const [awaitingPermInject, setAwaitingPermInject] = useState<string | null>(null);

  // Built-in Companion Bridge Bucket & DeploymentLedger state
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

  // Community ecosystem & profiles state
  const [registrySources, setRegistrySources] = useState<RegistrySourceState[]>(
    [],
  );
  const [communityTrainers, setCommunityTrainers] = useState<
    CommunityTrainerView[]
  >([]);
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

  const selectedTrainer = useMemo(() => {
    if (trainers.length === 0) return null;
    return (
      trainers.find((t) => t.trainer_id === selectedTrainerId) ?? trainers[0]
    );
  }, [trainers, selectedTrainerId]);

  const isSelectedGameInjected =
    activeSessionId !== null &&
    selectedGame !== null &&
    activeGameKey === selectedGame.library_key;

  const isSelectedTrainerInjected =
    isSelectedGameInjected &&
    selectedTrainer !== null &&
    activeTrainerId === selectedTrainer.trainer_id;

  const hasAntiCheatMarkers =
    (gameEnv?.anti_cheat_files_present.length ?? 0) > 0;

  function pushHudToast(text: string, tone: HudToast["tone"] = "info") {
    const id = Date.now() + Math.floor(Math.random() * 1000);
    setHudToasts((prev) => [...prev.slice(-3), { id, text, tone }]);
    window.setTimeout(() => {
      setHudToasts((prev) => prev.filter((t) => t.id !== id));
    }, 3200);
  }

  async function handleCopyInstallPath(path: string) {
    try {
      await navigator.clipboard?.writeText(path);
      setCopiedPath(true);
      pushHudToast("已复制安装目录路径", "info");
      window.setTimeout(() => setCopiedPath(false), 1800);
    } catch {
      setStatusMsg(`安装路径: ${path}`);
    }
  }

  function toggleFavorite(gameKey: string) {
    setFavoriteKeys((prev) => {
      const next = new Set(prev);
      if (next.has(gameKey)) {
        next.delete(gameKey);
      } else {
        next.add(gameKey);
      }
      try {
        localStorage.setItem(FAV_STORAGE_KEY, JSON.stringify([...next]));
      } catch {
        // Ignore storage quota errors
      }
      return next;
    });
  }

  function toggleSoundEnabled() {
    setSoundEnabled((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(SOUND_STORAGE_KEY, String(next));
      } catch {
        // Ignore
      }
      if (next) soundFx.toggleOn();
      return next;
    });
  }

  function toggleAutoAttach() {
    setAutoAttachEnabled((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(AUTO_ATTACH_STORAGE_KEY, String(next));
      } catch {
        // Ignore
      }
      pushHudToast(
        next
          ? "已开启自动附着：检测到已授权游戏启动将自动注入"
          : "已关闭自动附着模式",
        "info",
      );
      return next;
    });
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
      const [lib, paths, sec, diag, sources, availUpdates, catalog] =
        await Promise.all([
          nekodApi.listLibraryGames(),
          nekodApi.listCustomGamePaths(),
          nekodApi.getSecurityPolicy(),
          nekodApi.getDiagnosticLogs(),
          nekodApi.listRegistrySources(),
          nekodApi.checkTrainerUpdates(),
          nekodApi.listCompanionBridgeCatalog(),
        ]);
      setGames(lib);
      setCustomPaths(paths);
      setPolicy(sec);
      setLogs(diag);
      setRegistrySources(sources);
      setUpdates(availUpdates);
      setBridgeCatalog(catalog);
      void loadArtworksForLibrary(lib);
      if (autoSelectFirst && lib.length > 0) {
        await openGame(lib[0], false);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void refreshAll(true);
  }, []);

  // Background lightweight check of which library games are currently running
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

  async function handleRescan() {
    try {
      setBusyAction("rescan");
      setError(null);
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      setLogs(await nekodApi.getDiagnosticLogs());
      void loadArtworksForLibrary(lib);
      setStatusMsg(`已重新扫描游戏库：发现 ${lib.length} 款已安装游戏。`);
      if (lib.length > 0 && !selectedGame) {
        await openGame(lib[0], false);
      } else if (selectedGame) {
        const refreshed = lib.find(
          (g) => g.library_key === selectedGame.library_key,
        );
        if (refreshed) {
          await openGame(refreshed, false);
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

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
    setRunningProcs([]);
    setSelectedPid(null);
    setGameEnv(null);
    setCompatRecords([]);
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
          if (activeGameKey === entry.library_key && activeTrainerId) {
            return activeTrainerId;
          }
          return list.some((t) => t.trainer_id === prev)
            ? prev
            : (list[0]?.trainer_id ?? null);
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
      setError(
        "检测到反作弊保护标记，根据安全策略拒绝注入该游戏进程（Nekod 仅支持单机/无反作弊环境）。",
      );
      return;
    }

    const procs = overrideProcs ?? runningProcs;
    const proc = procs.find((p) => p.pid === selectedPid) ?? procs[0];
    if (!proc) {
      if (!fromAutoAttach) {
        if (soundEnabled) soundFx.error();
        setError(
          "未检测到运行中的游戏进程。请先启动游戏（或直接点「启动游戏并注入」）。",
        );
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
      setActiveSessionId(sessionId);
      setActiveGameKey(selectedGame.library_key);
      setActiveTrainerId(trainer.trainer_id);
      setActiveToggles({});
      setAwaitingPermInject(null);
      setWaitingLaunchGameKey(null);
      setSessionDiag(await nekodApi.getSessionDiagnostics(sessionId));
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.injectSuccess();
      const msg = fromAutoAttach
        ? `已自动注入「${trainer.name}」 -> ${proc.process_name} (PID ${proc.pid})`
        : `已注入「${trainer.name}」 -> ${proc.process_name} (PID ${proc.pid})`;
      setStatusMsg(msg);
      pushHudToast(msg, "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleOneClickInject(trainer: TrainerDetailView) {
    if (!selectedGame?.game.matched_game_id) return;
    if (activeSessionId) {
      setError("当前已有活跃注入会话，请先点击「解除注入」后再切换其他模组或游戏。");
      return;
    }
    if (hasAntiCheatMarkers) {
      if (soundEnabled) soundFx.error();
      setError(
        "检测到反作弊保护标记，根据安全策略拒绝注入该游戏进程（Nekod 仅支持单机/无反作弊环境）。",
      );
      return;
    }
    if (trainer.permission_state.status === "blocked") {
      if (soundEnabled) soundFx.error();
      setError(`当前模组被安全策略阻止：${trainer.permission_state.reason}`);
      return;
    }
    if (trainer.permission_state.status === "requires_confirmation") {
      setAwaitingPermInject(trainer.trainer_id);
      setStatusMsg(
        `「${trainer.name}」需要确认声明的运行权限，请在下方点击「确认权限并立即注入」。`,
      );
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
      setError("检测到反作弊保护标记，无法对该游戏启用「启动游戏并注入」。");
      return;
    }
    try {
      setBusyAction("launch");
      setError(null);
      const launched = await nekodApi.launchGame(
        selectedGame.game.store_source,
        selectedGame.game.store_app_id,
        selectedGame.game.install_dir,
        selectedGame.game.executable_path,
      );
      if (!selectedTrainer) {
        setWaitingLaunchGameKey(null);
        if (soundEnabled) soundFx.toggleOn();
        const note = `已启动游戏 (${launched.target})。请先在下方点击「一键初始化通用 Unity 控制模组」（或导入模组）后再注入。`;
        setStatusMsg(note);
        pushHudToast(note, "info");
        return;
      }
      setWaitingLaunchGameKey(selectedGame.library_key);
      if (selectedTrainer.permission_state.status === "requires_confirmation") {
        setAwaitingPermInject(selectedTrainer.trainer_id);
      }
      if (soundEnabled) soundFx.toggleOn();
      const note = `已启动游戏 (${launched.target})，正在等待进程出现后自动注入...`;
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
    activeSessionId,
    hasAntiCheatMarkers,
    busyAction,
  });
  autoAttachRef.current = {
    selectedGame,
    selectedTrainer,
    activeSessionId,
    hasAntiCheatMarkers,
    busyAction,
  };

  useEffect(() => {
    const shouldWatch =
      (autoAttachEnabled ||
        (waitingLaunchGameKey !== null &&
          waitingLaunchGameKey === selectedGame?.library_key)) &&
      !activeSessionId &&
      Boolean(selectedGame?.game.matched_game_id);
    if (!shouldWatch) return;

    const timer = window.setInterval(async () => {
      const snap = autoAttachRef.current;
      if (
        !snap.selectedGame?.game.matched_game_id ||
        snap.activeSessionId ||
        snap.hasAntiCheatMarkers ||
        snap.busyAction
      ) {
        return;
      }
      const trainer = snap.selectedTrainer;
      if (!trainer || trainer.permission_state.status !== "approved") {
        // Strictly preserve Default-Deny: never auto-attach unapproved trainers!
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
    activeSessionId,
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
        setStatusMsg(`已授权 ${trainer.name} v${trainer.version} 的声明权限。`);
      }
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleUninject() {
    if (!activeSessionId) return;
    try {
      setBusyAction("uninject");
      setError(null);
      const diag = await nekodApi.stopTrainerSession(activeSessionId);
      setSessionDiag(diag);
      setActiveSessionId(null);
      setActiveGameKey(null);
      setActiveTrainerId(null);
      setActiveToggles({});
      setLogs(await nekodApi.getDiagnosticLogs());
      if (soundEnabled) soundFx.uninject();
      const msg = `已解除注入并按 LIFO 顺序完成回滚（冲突: ${diag.rollback_conflicts}，失败: ${diag.rollback_failures}）`;
      setStatusMsg(msg);
      pushHudToast(msg, "info");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

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
      const msg = `已自动识别 ${ENGINE_LABELS[next.engine]} 并部署伴生桥接 (${next.active_files.join(", ")})`;
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
      setError("请填写本地源插件路径与目标相对路径，或直接点击「一键自动部署」。");
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
      setStatusMsg(`已通过 DeploymentLedger 部署伴生桥接文件到 ${target}。`);
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
      const msg = `已解除伴生桥接：移除 ${summary.removed_count} 个文件，还原 ${summary.restored_originals} 个原文件（冲突: ${summary.conflicts}）`;
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
      const msg = `已安装 ${next.loader_display_name ?? "BepInEx"} 引导加载器。若游戏正在运行，请重启一次游戏即可自动加载桥接插件。`;
      setStatusMsg(msg);
      pushHudToast("BepInEx 加载器安装完成（重启游戏后生效）", "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
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
      const msg = `已卸载 BepInEx 加载器：移除 ${summary.removed_count} 个文件，还原 ${summary.restored_originals} 个原文件（冲突: ${summary.conflicts}）`;
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
      const msg = `已初始化通用 Unity 控制模组 (${res.trainer_id})！请先确认运行权限，然后点击「一键注入 Mod」。`;
      setStatusMsg(msg);
      pushHudToast("通用 Unity 控制模组初始化完成", "ok");
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleRefreshDiagnostics() {
    if (!activeSessionId) return;
    try {
      setSessionDiag(await nekodApi.getSessionDiagnostics(activeSessionId));
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleApplyFeature(
    feature: TrainerFeature,
    triggeredHotkey?: string,
  ) {
    if (!activeSessionId) return;
    let command: FeatureCommand;
    let nextToggleVal = false;
    if (feature.control_type === "toggle") {
      nextToggleVal = !activeToggles[feature.id];
      command = {
        feature_id: feature.id,
        value: { type: "toggle", value: nextToggleVal },
      };
    } else if (feature.control_type === "action") {
      command = { feature_id: feature.id, value: { type: "action" } };
    } else {
      const raw = (featureInputs[feature.id] ?? "").trim();
      const num = Number(raw);
      if (!raw || !Number.isFinite(num)) {
        if (soundEnabled) soundFx.error();
        setError(`请为「${feature.name}」输入有效数字。`);
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

    try {
      setError(null);
      await nekodApi.applyTrainerFeature(activeSessionId, command);
      if (feature.control_type === "toggle") {
        setActiveToggles((prev) => ({ ...prev, [feature.id]: nextToggleVal }));
        if (soundEnabled) {
          if (nextToggleVal) soundFx.toggleOn();
          else soundFx.toggleOff();
        }
        const suffix = triggeredHotkey ? ` (${triggeredHotkey})` : "";
        pushHudToast(
          `${feature.name} ${nextToggleVal ? "已开启" : "已关闭"}${suffix}`,
          nextToggleVal ? "ok" : "info",
        );
      } else {
        if (soundEnabled) soundFx.toggleOn();
        const suffix = triggeredHotkey ? ` (${triggeredHotkey})` : "";
        pushHudToast(`已执行：${feature.name}${suffix}`, "ok");
      }
      const [diag, logList] = await Promise.all([
        nekodApi.getSessionDiagnostics(activeSessionId),
        nekodApi.getDiagnosticLogs(),
      ]);
      setSessionDiag(diag);
      setLogs(logList);
    } catch (e) {
      if (soundEnabled) soundFx.error();
      setError(String(e));
      if (activeSessionId) {
        try {
          setSessionDiag(await nekodApi.getSessionDiagnostics(activeSessionId));
        } catch {
          // Keep UI responsive
        }
      }
    }
  }

  useEffect(() => {
    if (!activeSessionId || !activeTrainerId) {
      void nekodApi.syncGlobalHotkeys([]).then(setGlobalHotkeyCount);
      return;
    }
    const trainer = trainers.find((t) => t.trainer_id === activeTrainerId);
    if (!trainer) {
      void nekodApi.syncGlobalHotkeys([]).then(setGlobalHotkeyCount);
      return;
    }
    const bindings = trainer.features
      .filter((f) => f.control_type === "toggle" || f.control_type === "action")
      .map((f) => ({
        feature_id: f.id,
        hotkey: (
          profilesByTrainer[trainer.trainer_id]?.[f.id] ??
          f.default_hotkey ??
          ""
        ).trim(),
      }))
      .filter((b) => b.hotkey.length > 0);
    void nekodApi.syncGlobalHotkeys(bindings).then(setGlobalHotkeyCount);
  }, [activeSessionId, activeTrainerId, trainers, profilesByTrainer]);

  const globalHotkeyHandlerRef = useRef<
    (featureId: string, hotkey: string) => void
  >(() => {});
  globalHotkeyHandlerRef.current = (featureId: string, hotkey: string) => {
    if (!activeSessionId || !activeTrainerId) return;
    const trainer = trainers.find((t) => t.trainer_id === activeTrainerId);
    const feat = trainer?.features.find((f) => f.id === featureId);
    if (feat) {
      void handleApplyFeature(feat, hotkey);
    }
  };

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void nekodApi
      .onGlobalHotkey((ev) => {
        globalHotkeyHandlerRef.current(ev.feature_id, ev.hotkey);
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      if (unlisten) unlisten();
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
      setStatusMsg(`已撤销「${trainer.name}」的运行权限。`);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleAddCustomPath() {
    if (!customDir.trim()) return;
    try {
      setError(null);
      const updated = await nekodApi.addCustomGamePath(
        customDir.trim(),
        customLabel.trim() || null,
      );
      setCustomPaths(updated);
      setCustomDir("");
      setCustomLabel("");
      await handleRescan();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleIngestTrainer() {
    if (!trainerDir.trim()) return;
    try {
      setError(null);
      await nekodApi.ingestLocalTrainer(trainerDir.trim(), trainerTrust);
      setTrainerDir("");
      await handleRescan();
      setStatusMsg("已完成校验并导入本地模组包。");
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
      setStatusMsg(`已注册游戏清单 '${gameId}'。`);
    } catch (e) {
      setError(String(e));
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
      const [sources, availUpdates, diag] = await Promise.all([
        nekodApi.listRegistrySources(),
        nekodApi.checkTrainerUpdates(),
        nekodApi.getDiagnosticLogs(),
      ]);
      setRegistrySources(sources);
      setUpdates(availUpdates);
      setLogs(diag);
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
        `同步完成（检查 ${report.sources_checked} 源）：${report.updated} 更新，${report.not_modified} 无变化，${report.failed} 失败，${report.revoked_trainer_ids.length} 已吊销。`,
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
      setStatusMsg("已添加第三方社区源（默认 Unverified），请点击同步获取索引。");
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
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(`已移除社区源 '${repositoryId}'。`);
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
        `已完成 SHA-256 校验并安装 '${trainerId}'，注入前请确认其运行权限。`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  function findDuplicateHotkeys(hotkeys: Record<string, string>): string[] {
    const counts = new Map<string, number>();
    for (const raw of Object.values(hotkeys)) {
      const norm = raw.trim().toLowerCase();
      if (!norm) continue;
      counts.set(norm, (counts.get(norm) ?? 0) + 1);
    }
    return [...counts.entries()].filter(([, c]) => c > 1).map(([k]) => k);
  }

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
          activeTrainerId === trainer.trainer_id ? activeToggles : {},
        presets: profilePresets[trainer.trainer_id] ?? {},
      };
      await nekodApi.saveProfile(profile);
      setStatusMsg(`已保存「${trainer.name}」的快捷键与预设配置。`);
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
        if (activeSessionId && activeTrainerId === trainer.trainer_id) {
          for (const f of trainer.features) {
            if (f.control_type === "toggle" && activeToggles[f.id]) {
              await nekodApi.applyTrainerFeature(activeSessionId, {
                feature_id: f.id,
                value: { type: "toggle", value: false },
              });
            }
          }
          setActiveToggles({});
        }
        setStatusMsg(`已重置「${trainer.name}」的默认快捷键与开关状态。`);
        return;
      }

      const presetState: Record<string, unknown> = {};
      for (const f of trainer.features) {
        if (f.control_type === "toggle") {
          const enable =
            presetName === "Story Mode" ? f.risk_level === "low" : true;
          presetState[f.id] = { type: "toggle", value: enable };
          if (activeSessionId && activeTrainerId === trainer.trainer_id) {
            await nekodApi.applyTrainerFeature(activeSessionId, {
              feature_id: f.id,
              value: { type: "toggle", value: enable },
            });
            setActiveToggles((prev) => ({ ...prev, [f.id]: enable }));
          }
        }
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
        saved_feature_states:
          activeTrainerId === trainer.trainer_id ? activeToggles : {},
        presets: nextTrainerPresets,
      });
      setStatusMsg(`已应用并保存预设「${presetName}」。`);
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
      setStatusMsg(`已导出「${trainer.name}」的可移植 JSON 配置。`);
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
      setStatusMsg(`已导入 '${imported.trainer_id}' 的 JSON 配置。`);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    if (!activeSessionId || !activeTrainerId || !isBrowserPreview) return;
    const trainer = trainers.find((t) => t.trainer_id === activeTrainerId);
    if (!trainer) return;

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

      const match = trainer?.features.find((f) => {
        if (f.control_type !== "toggle" && f.control_type !== "action")
          return false;
        const bound =
          profilesByTrainer[trainer.trainer_id]?.[f.id] ??
          f.default_hotkey ??
          "";
        return bound.trim().toLowerCase() === combo;
      });
      if (match) {
        e.preventDefault();
        void handleApplyFeature(match, parts.join("+"));
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeSessionId, activeTrainerId, trainers, profilesByTrainer, activeToggles]);

  const filteredGames = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = games.filter((g) => {
      if (sidebarFilter === "favorites" && !favoriteKeys.has(g.library_key)) {
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
        if (activeSessionId && activeGameKey === item.library_key) return 0;
        if (runningGameKeys.has(item.library_key)) return 1;
        if (favoriteKeys.has(item.library_key)) return 2;
        return 3;
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
    activeSessionId,
    activeGameKey,
  ]);

  const activeProcessLabel = useMemo(() => {
    const proc =
      runningProcs.find((p) => p.pid === selectedPid) ?? runningProcs[0];
    return proc ? `${proc.process_name} (PID ${proc.pid})` : null;
  }, [runningProcs, selectedPid]);

  const bridgeAvailable = Boolean(
    bridgeStatus &&
      (bridgeStatus.deployed ||
        bridgeStatus.matched_bridge ||
        bridgeStatus.suggested_target_relative_path ||
        bridgeStatus.engine !== "native_unknown"),
  );

  const selectedArtwork = selectedGame
    ? artworkByGame[selectedGame.library_key]
    : null;

  const canLaunchSelectedGame = Boolean(
    selectedGame &&
      ((selectedGame.game.store_source === "steam" &&
        selectedGame.game.store_app_id) ||
        selectedGame.game.executable_path),
  );

  const mergedMetaItems = useMemo(() => {
    if (!selectedGame) return [];
    const items: string[] = [
      selectedGame.game.store_source.toUpperCase(),
      selectedGame.game.platform,
    ];
    if (selectedGame.game.store_build_id) {
      items.push(`Build ${selectedGame.game.store_build_id}`);
    }
    if (gameEnv) {
      items.push(ENGINE_LABELS[gameEnv.engine]);
      if (gameEnv.installed_loaders.length > 0) {
        const loadersText = gameEnv.installed_loaders
          .map((l) =>
            l.detected_version ? `${l.kind} ${l.detected_version}` : l.kind,
          )
          .join(", ");
        items.push(loadersText);
      }
    }
    if (bridgeStatus?.deployed) {
      items.push(`桥接已部署 (${bridgeStatus.active_files.length})`);
    }
    return items;
  }, [selectedGame, gameEnv, bridgeStatus]);

  function switchSecondaryTab(next: Exclude<NavTab, "workbench">) {
    if (tab === next && selectedGame) {
      setTab("workbench");
    } else {
      setTab(next);
    }
  }

  return (
    <div className="workbench-shell">
      {/* ==================== LEFT SIDEBAR ==================== */}
      <aside className="game-sidebar">
        <div className="sidebar-top">
          <div className="brand-row">
            <div className="brand-group">
              <h1 className="brand-title">Nekod</h1>
              <span className="brand-tag">v0.1</span>
            </div>
            <button
              className="icon-btn"
              disabled={busyAction === "rescan"}
              onClick={() => void handleRescan()}
              title="重新扫描游戏目录"
              aria-label="重新扫描游戏目录"
            >
              <RefreshCw
                size={14}
                strokeWidth={1.5}
                className={busyAction === "rescan" ? "spin" : ""}
                aria-hidden="true"
              />
            </button>
          </div>

          <div className="search-wrap">
            <Search
              size={14}
              strokeWidth={1.5}
              className="search-icon"
              aria-hidden="true"
            />
            <input
              className="sidebar-search"
              placeholder="筛选游戏..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
        </div>

        <div className="segmented-wrap">
          <div className="segmented-control" role="tablist" aria-label="游戏列表筛选">
            <button
              className={`segmented-btn ${sidebarFilter === "all" ? "active" : ""}`}
              onClick={() => setSidebarFilter("all")}
            >
              <span>全部</span>
              <span className="seg-count">{games.length}</span>
            </button>
            <button
              className={`segmented-btn ${sidebarFilter === "favorites" ? "active" : ""}`}
              onClick={() => setSidebarFilter("favorites")}
            >
              <span>收藏</span>
              <span className="seg-count">{favoriteKeys.size}</span>
            </button>
            <button
              className={`segmented-btn ${sidebarFilter === "running" ? "active" : ""}`}
              onClick={() => setSidebarFilter("running")}
            >
              <span>运行</span>
              <span className="seg-count">{runningGameKeys.size}</span>
            </button>
          </div>
        </div>

        <div className="sidebar-section-header">
          <span>游戏列表 ({filteredGames.length})</span>
          {activeSessionId ? (
            <span className="sidebar-live-label">
              <span className="status-dot ok" />
              <span>已注入</span>
            </span>
          ) : null}
        </div>

        <div className="game-list">
          {filteredGames.length === 0 ? (
            <div className="muted" style={{ padding: "10px 6px" }}>
              当前筛选下无匹配游戏。可切换至「全部」或在设置中添加游戏目录。
            </div>
          ) : (
            filteredGames.map((item) => {
              const isSelected =
                tab === "workbench" &&
                selectedGame?.library_key === item.library_key;
              const isInjected =
                activeSessionId !== null && activeGameKey === item.library_key;
              const isRunning = runningGameKeys.has(item.library_key);
              const isBridgeDeployed = Boolean(
                bridgeStatusByGame[item.library_key]?.deployed,
              );
              const isFav = favoriteKeys.has(item.library_key);
              const thumb = artworkByGame[item.library_key]?.thumbnail_data_uri;

              return (
                <div
                  key={item.library_key}
                  className={`game-list-item ${isSelected ? "active" : ""} ${
                    isInjected ? "is-injected" : ""
                  }`}
                  role="button"
                  tabIndex={0}
                  onClick={() => void openGame(item, true)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      void openGame(item, true);
                    }
                  }}
                >
                  <div className="game-thumb-wrap">
                    {thumb ? (
                      <img
                        className="game-thumb-img"
                        src={thumb}
                        alt={item.game.title}
                      />
                    ) : (
                      <div className="game-thumb-monogram">
                        {gameMonogram(item.game.title)}
                      </div>
                    )}
                    <span
                      className={`game-thumb-dot ${
                        isInjected
                          ? "injected"
                          : isRunning
                            ? "running"
                            : isBridgeDeployed
                              ? "bridge-ready"
                              : ""
                      }`}
                    />
                  </div>
                  <div className="game-list-meta">
                    <div className="game-list-title">{item.game.title}</div>
                    <div className="game-list-sub">
                      <span>{item.game.store_source.toUpperCase()}</span>
                      <span>·</span>
                      <span>{item.trainer_count} mod</span>
                      {isInjected && (
                        <>
                          <span>·</span>
                          <span className="game-list-state-ok">injected</span>
                        </>
                      )}
                      {!isInjected && isRunning && (
                        <>
                          <span>·</span>
                          <span className="game-list-state-run">running</span>
                        </>
                      )}
                    </div>
                  </div>
                  <button
                    className={`fav-btn ${isFav ? "starred" : ""}`}
                    title={isFav ? "取消收藏" : "收藏置顶"}
                    aria-label={isFav ? "取消收藏" : "收藏置顶"}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleFavorite(item.library_key);
                    }}
                  >
                    <Star
                      size={14}
                      strokeWidth={1.5}
                      fill={isFav ? "currentColor" : "none"}
                      aria-hidden="true"
                    />
                  </button>
                </div>
              );
            })
          )}
        </div>

        {/* Bottom Single-Column Compact Vertical Navigation */}
        <nav className="sidebar-nav" aria-label="Secondary navigation">
          <button
            className={`sidebar-nav-item ${tab === "community" ? "active" : ""}`}
            onClick={() => switchSecondaryTab("community")}
          >
            <Globe size={15} strokeWidth={1.5} aria-hidden="true" />
            <span>社区源</span>
            {updates.length > 0 && (
              <span className="sidebar-nav-badge">{updates.length}</span>
            )}
          </button>
          <button
            className={`sidebar-nav-item ${tab === "import" ? "active" : ""}`}
            onClick={() => switchSecondaryTab("import")}
          >
            <FolderInput size={15} strokeWidth={1.5} aria-hidden="true" />
            <span>本地导入</span>
          </button>
          <button
            className={`sidebar-nav-item ${tab === "logs" ? "active" : ""}`}
            onClick={async () => {
              if (tab === "logs" && selectedGame) {
                setTab("workbench");
                return;
              }
              setTab("logs");
              try {
                setLogs(await nekodApi.getDiagnosticLogs());
              } catch (e) {
                setError(String(e));
              }
            }}
          >
            <Terminal size={15} strokeWidth={1.5} aria-hidden="true" />
            <span>诊断日志</span>
          </button>
          <button
            className={`sidebar-nav-item ${tab === "settings" ? "active" : ""}`}
            onClick={() => switchSecondaryTab("settings")}
          >
            <Settings size={15} strokeWidth={1.5} aria-hidden="true" />
            <span>设置安全</span>
          </button>
        </nav>
      </aside>

      {/* ==================== RIGHT COLUMN ==================== */}
      <div className="right-pane">
        {(isBrowserPreview || error || statusMsg) && (
          <div className="top-banners">
            {isBrowserPreview && (
              <div className="banner">
                <span>
                  浏览器预览模式：当前展示模拟 IPC 数据；运行 Tauri 桌面端可连接真实 Windows 进程、Steam 封面与全局热键。
                </span>
              </div>
            )}
            {error && (
              <div className="error-banner">
                <span>{error}</span>
                <button
                  className="icon-btn"
                  onClick={() => setError(null)}
                  aria-label="关闭错误提示"
                >
                  <X size={14} strokeWidth={1.5} />
                </button>
              </div>
            )}
            {statusMsg && (
              <div className="banner">
                <span>{statusMsg}</span>
                <button
                  className="icon-btn"
                  onClick={() => setStatusMsg(null)}
                  aria-label="关闭状态提示"
                >
                  <X size={14} strokeWidth={1.5} />
                </button>
              </div>
            )}
          </div>
        )}

        {/* ==================== WORKBENCH ==================== */}
        {tab === "workbench" && !selectedGame && (
          <div className="empty-state">
            <SlidersHorizontal size={20} strokeWidth={1.5} aria-hidden="true" />
            <div style={{ color: "var(--text-primary)", fontWeight: 500 }}>
              从左侧选择一款游戏
            </div>
            <div className="muted" style={{ maxWidth: 400 }}>
              选定游戏后可在顶部一键启动、注入或卸载模组，并在下方管理具体功能与伴生桥接。
            </div>
            <button className="btn btn-primary" onClick={() => void handleRescan()}>
              <RefreshCw size={14} strokeWidth={1.5} aria-hidden="true" />
              <span>扫描本地游戏库</span>
            </button>
          </div>
        )}

        {tab === "workbench" && selectedGame && (
          <>
            {/* ---------- TOP-RIGHT: Hero Header ---------- */}
            <header className="game-hero">
              {selectedArtwork?.header_data_uri ? (
                <div
                  className="game-hero-bg"
                  style={{
                    backgroundImage: `url("${selectedArtwork.header_data_uri}")`,
                  }}
                />
              ) : (
                <div className="game-hero-fallback" />
              )}
              <div className="game-hero-scrim" />

              <div className="game-hero-main">
                <div className="game-hero-info">
                  <div className="game-hero-title-row">
                    <h2 className="game-hero-title">{selectedGame.game.title}</h2>
                    <button
                      className={`fav-btn ${
                        favoriteKeys.has(selectedGame.library_key) ? "starred" : ""
                      }`}
                      onClick={() => toggleFavorite(selectedGame.library_key)}
                      title={
                        favoriteKeys.has(selectedGame.library_key)
                          ? "取消收藏"
                          : "收藏置顶"
                      }
                      aria-label="切换收藏"
                    >
                      <Star
                        size={15}
                        strokeWidth={1.5}
                        fill={
                          favoriteKeys.has(selectedGame.library_key)
                            ? "currentColor"
                            : "none"
                        }
                      />
                    </button>
                  </div>

                  {/* Single merged borderless monospace metadata row */}
                  <div className="meta-line">
                    {mergedMetaItems.map((item, idx) => (
                      <span key={`${item}-${idx}`} className="row" style={{ gap: 6 }}>
                        {idx > 0 && <span className="meta-sep">·</span>}
                        <span
                          className={
                            item.startsWith("桥接已部署") ? "meta-ok" : undefined
                          }
                        >
                          {item}
                        </span>
                      </span>
                    ))}
                    {hasAntiCheatMarkers && (
                      <>
                        <span className="meta-sep">·</span>
                        <span className="meta-warn">
                          <AlertTriangle size={14} strokeWidth={1.5} />
                          <span>
                            反作弊: {gameEnv?.anti_cheat_files_present.join(", ")}
                          </span>
                        </span>
                      </>
                    )}
                  </div>

                  {/* Standalone muted install directory with hover highlight & click-to-copy */}
                  <div>
                    <button
                      className="path-copy-btn"
                      onClick={() =>
                        void handleCopyInstallPath(selectedGame.game.install_dir)
                      }
                      title="点击复制安装目录路径"
                    >
                      <FolderOpen size={14} strokeWidth={1.5} aria-hidden="true" />
                      <span className="path-copy-text">
                        {selectedGame.game.install_dir}
                      </span>
                      {copiedPath ? (
                        <Check
                          size={14}
                          strokeWidth={1.5}
                          className="path-copy-icon"
                          style={{ color: "var(--ok)", opacity: 1 }}
                        />
                      ) : (
                        <Copy
                          size={14}
                          strokeWidth={1.5}
                          className="path-copy-icon"
                        />
                      )}
                    </button>
                  </div>
                </div>

                {/* Converged Button Hierarchy */}
                <div className="game-hero-actions">
                  <div className="hero-btn-row">
                    {!isSelectedGameInjected ? (
                      canLaunchSelectedGame ? (
                        <>
                          <button
                            className="btn btn-primary"
                            disabled={
                              busyAction === "launch" ||
                              hasAntiCheatMarkers ||
                              (activeSessionId !== null && !isSelectedGameInjected)
                            }
                            onClick={() => void handlePlayAndInject()}
                            title="启动游戏并在检测到进程后自动注入已授权 Mod"
                          >
                            <Play size={14} strokeWidth={1.5} aria-hidden="true" />
                            <span>
                              {busyAction === "launch"
                                ? "启动中..."
                                : "启动游戏并注入"}
                            </span>
                          </button>

                          <button
                            className="btn btn-outline"
                            disabled={
                              !selectedTrainer ||
                              hasAntiCheatMarkers ||
                              busyAction === "inject" ||
                              (activeSessionId !== null && !isSelectedGameInjected)
                            }
                            onClick={() =>
                              selectedTrainer &&
                              void handleOneClickInject(selectedTrainer)
                            }
                          >
                            <Zap size={14} strokeWidth={1.5} aria-hidden="true" />
                            <span>
                              {busyAction === "inject"
                                ? "注入中..."
                                : "一键注入 Mod"}
                            </span>
                          </button>
                        </>
                      ) : (
                        <button
                          className="btn btn-primary"
                          disabled={
                            !selectedTrainer ||
                            hasAntiCheatMarkers ||
                            busyAction === "inject" ||
                            (activeSessionId !== null && !isSelectedGameInjected)
                          }
                          onClick={() =>
                            selectedTrainer &&
                            void handleOneClickInject(selectedTrainer)
                          }
                        >
                          <Zap size={14} strokeWidth={1.5} aria-hidden="true" />
                          <span>
                            {busyAction === "inject"
                              ? "注入中..."
                              : "一键注入 Mod"}
                          </span>
                        </button>
                      )
                    ) : (
                      <button
                        className="btn btn-danger-outline"
                        disabled={busyAction === "uninject"}
                        onClick={() => void handleUninject()}
                      >
                        <Power size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>
                          {busyAction === "uninject" ? "回滚中..." : "解除注入"}
                        </span>
                      </button>
                    )}

                    {bridgeAvailable &&
                      (bridgeStatus?.deployed ? (
                        <button
                          className="btn btn-danger-outline"
                          disabled={busyAction === "undeploy-bridge"}
                          onClick={() => void handleUndeployBridge()}
                          title="按 DeploymentLedger 卸载游戏目录中的伴生桥接文件"
                        >
                          <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
                          <span>
                            {busyAction === "undeploy-bridge"
                              ? "卸载中..."
                              : "解除桥接"}
                          </span>
                        </button>
                      ) : (
                        <button
                          className="btn btn-ghost"
                          disabled={
                            busyAction === "deploy-bridge" || hasAntiCheatMarkers
                          }
                          onClick={() => void handleAutoDeployBridge(null)}
                          title="自动识别引擎并一键部署伴生桥接"
                        >
                          <Package size={14} strokeWidth={1.5} aria-hidden="true" />
                          <span>
                            {busyAction === "deploy-bridge"
                              ? "部署中..."
                              : "部署伴生桥接"}
                          </span>
                        </button>
                      ))}
                  </div>

                  {/* Secondary Icon Toggles */}
                  <div className="hero-secondary-row">
                    <button
                      className={`icon-toggle ${autoAttachEnabled ? "on" : ""}`}
                      onClick={toggleAutoAttach}
                      title="检测到已授权游戏进程启动时自动注入"
                    >
                      <RefreshCw size={14} strokeWidth={1.5} aria-hidden="true" />
                      <span>自动注入</span>
                    </button>
                    <button
                      className={`icon-toggle ${soundEnabled ? "on" : ""}`}
                      onClick={toggleSoundEnabled}
                      title="开关与注入提示音效"
                    >
                      {soundEnabled ? (
                        <Volume2 size={14} strokeWidth={1.5} aria-hidden="true" />
                      ) : (
                        <VolumeX size={14} strokeWidth={1.5} aria-hidden="true" />
                      )}
                      <span>提示音</span>
                    </button>
                    {isSelectedGameInjected && globalHotkeyCount > 0 && (
                      <span className="hotkey-status-text">
                        <Keyboard size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>{globalHotkeyCount} hotkeys</span>
                      </span>
                    )}
                  </div>
                </div>
              </div>

              {/* Compact Process Bar */}
              <div className="game-hero-bar">
                <div className="proc-bar-left">
                  <Cpu size={14} strokeWidth={1.5} aria-hidden="true" />
                  {runningProcs.length > 0 ? (
                    <select
                      className="select-input"
                      value={selectedPid ?? ""}
                      onChange={(e) => setSelectedPid(Number(e.target.value))}
                      disabled={activeSessionId !== null}
                    >
                      {runningProcs.map((p) => (
                        <option key={p.pid} value={p.pid}>
                          {p.process_name} (PID {p.pid}, {p.architecture})
                        </option>
                      ))}
                    </select>
                  ) : (
                    <span className="muted">
                      未检测到进程（可点击「启动游戏并注入」或手动启动游戏）
                    </span>
                  )}
                  <button
                    className="icon-btn"
                    onClick={() =>
                      void scanRunningProcesses(
                        selectedGame.game.matched_game_id,
                        selectedGame.game.install_dir,
                      )
                    }
                    disabled={activeSessionId !== null}
                    title="扫描运行中的游戏进程"
                    aria-label="扫描运行中的游戏进程"
                  >
                    <RefreshCw size={14} strokeWidth={1.5} />
                  </button>
                </div>

                <div className="proc-status-inline">
                  {waitingLaunchGameKey === selectedGame.library_key &&
                    !isSelectedGameInjected && (
                      <span className="row" style={{ gap: 6 }}>
                        <span className="status-dot" />
                        <span>等待进程出现后自动注入...</span>
                        <button
                          className="btn btn-ghost btn-sm"
                          onClick={() => setWaitingLaunchGameKey(null)}
                        >
                          取消
                        </button>
                      </span>
                    )}
                  {isSelectedGameInjected && sessionDiag ? (
                    <span className="row" style={{ gap: 8 }}>
                      <span className="status-dot ok" />
                      <span style={{ color: "var(--text-primary)" }}>
                        {activeProcessLabel ?? sessionDiag.session_id}
                      </span>
                      <span className="meta-sep">·</span>
                      <span>patches {sessionDiag.active_patches}</span>
                      <span className="meta-sep">·</span>
                      <span>freezes {sessionDiag.active_freeze_features}</span>
                      <button
                        className="icon-btn"
                        onClick={() => void handleRefreshDiagnostics()}
                        title="刷新会话状态"
                        aria-label="刷新会话状态"
                      >
                        <RefreshCw size={14} strokeWidth={1.5} />
                      </button>
                    </span>
                  ) : activeSessionId && !isSelectedGameInjected ? (
                    <span className="row" style={{ gap: 8 }}>
                      <span className="muted">另一款游戏正在注入中</span>
                      <button
                        className="btn btn-danger-outline btn-sm"
                        onClick={() => void handleUninject()}
                      >
                        解除另一会话
                      </button>
                    </span>
                  ) : (
                    selectedGame.game.matched_game_id && (
                      <span className="muted mono">
                        {selectedGame.game.matched_game_id}
                      </span>
                    )
                  )}
                </div>
              </div>
            </header>

            {/* ---------- BOTTOM-RIGHT: Mod Workspace ---------- */}
            <section className="mod-workspace">
              {bridgeStatus &&
                !bridgeStatus.loader_installed &&
                bridgeStatus.loader_warning &&
                !hasAntiCheatMarkers && (
                  <div
                    className="permission-banner stack"
                    style={{ marginTop: 14, marginBottom: 4 }}
                  >
                    <div
                      className="row"
                      style={{ justifyContent: "space-between" }}
                    >
                      <div className="row" style={{ gap: 6 }}>
                        <AlertTriangle
                          size={15}
                          strokeWidth={1.5}
                          style={{ color: "var(--warn)" }}
                        />
                        <strong>缺少 Unity 引导加载器</strong>
                      </div>
                      <button
                        className="btn btn-outline btn-sm"
                        disabled={busyAction === "install-loader"}
                        onClick={() => void handleInstallBepInExLoader()}
                      >
                        <Download size={14} strokeWidth={1.5} />
                        <span>
                          {busyAction === "install-loader"
                            ? "下载并校验安装中..."
                            : `安装 ${bridgeStatus.loader_display_name ?? "BepInEx"}`}
                        </span>
                      </button>
                    </div>
                    <div className="muted">{bridgeStatus.loader_warning}</div>
                  </div>
                )}

              {trainers.length === 0 ? (
                <div className="section-flat" style={{ paddingTop: 18 }}>
                  <div
                    className="row"
                    style={{ justifyContent: "space-between" }}
                  >
                    <div className="feature-heading">
                      <h3>模组功能</h3>
                    </div>
                    {gameEnv &&
                      (gameEnv.engine === "unity_mono" ||
                        gameEnv.engine === "unity_il2cpp") &&
                      !hasAntiCheatMarkers && (
                        <button
                          className="btn btn-primary btn-sm"
                          disabled={busyAction === "quick-setup"}
                          onClick={() => void handleQuickSetupUnityStarter()}
                        >
                          <Zap size={14} strokeWidth={1.5} />
                          <span>
                            {busyAction === "quick-setup"
                              ? "初始化中..."
                              : "一键初始化通用 Unity 控制模组"}
                          </span>
                        </button>
                      )}
                  </div>
                  <div className="muted">
                    {gameEnv &&
                    (gameEnv.engine === "unity_mono" ||
                      gameEnv.engine === "unity_il2cpp")
                      ? "检测到当前为 Unity 引擎游戏，尚未绑定专用模组。可点击右上角「一键初始化通用 Unity 控制模组」，自动配置暂停时间 (F8)、全局变速 (0.1x-8.0x) 与主线程资源清理（零 UnityExplorer 依赖）。"
                      : "当前游戏尚未安装本地模组。可展开下方「社区可用模组」一键安装，或切换至「本地导入」导入本地模组包。"}
                  </div>
                </div>
              ) : (
                selectedTrainer && (
                  <>
                    {/* Underline Mod Selector Tabs */}
                    <div className="mod-tab-bar">
                      <div className="mod-tabs">
                        {trainers.map((t) => {
                          const isTabSelected =
                            t.trainer_id === selectedTrainer.trainer_id;
                          const isTabActive =
                            isSelectedGameInjected &&
                            activeTrainerId === t.trainer_id;
                          return (
                            <button
                              key={t.trainer_id}
                              className={`mod-tab ${isTabSelected ? "active" : ""}`}
                              onClick={() => {
                                setSelectedTrainerId(t.trainer_id);
                                setAwaitingPermInject(null);
                              }}
                            >
                              {isTabActive && <span className="status-dot ok" />}
                              <span>{t.name}</span>
                              <span className="muted mono">v{t.version}</span>
                            </button>
                          );
                        })}
                      </div>

                      <div className="mod-meta-inline">
                        {trustTag(selectedTrainer.trust_tier)}
                        <span>compat:{selectedTrainer.compatibility_status}</span>
                        {selectedTrainer.revoked && (
                          <span className="tag unverified">revoked</span>
                        )}
                      </div>
                    </div>

                    {/* Inline Permission Gate (Default-Deny preserved) */}
                    {(selectedTrainer.permission_state.status ===
                      "requires_confirmation" ||
                      selectedTrainer.permission_state.status === "blocked" ||
                      awaitingPermInject === selectedTrainer.trainer_id) && (
                      <div className="permission-banner stack">
                        <div
                          className="row"
                          style={{ justifyContent: "space-between" }}
                        >
                          <div className="row" style={{ gap: 6 }}>
                            <ShieldAlert
                              size={15}
                              strokeWidth={1.5}
                              style={{ color: "var(--warn)" }}
                            />
                            <strong>注入前权限确认 (Default-Deny)</strong>
                          </div>
                          {selectedTrainer.permission_state.status ===
                            "blocked" && (
                            <span className="tag unverified">
                              已阻止：{selectedTrainer.permission_state.reason}
                            </span>
                          )}
                        </div>
                        <div className="stack" style={{ gap: 4 }}>
                          {selectedTrainer.permissions.map((p) => (
                            <div key={p.scope} className="muted">
                              <span style={{ color: "var(--text-primary)" }}>
                                {PERMISSION_LABELS[p.scope]}
                              </span>{" "}
                              <span className="mono">({p.scope})</span>：{p.reason}
                            </div>
                          ))}
                        </div>
                        {selectedTrainer.permission_state.status ===
                          "requires_confirmation" && (
                          <div className="row">
                            <button
                              className="btn btn-primary btn-sm"
                              disabled={busyAction === "approve"}
                              onClick={() =>
                                void handleApprovePermissions(
                                  selectedTrainer,
                                  true,
                                )
                              }
                            >
                              <Check size={14} strokeWidth={1.5} />
                              <span>确认权限并立即注入</span>
                            </button>
                            <button
                              className="btn btn-outline btn-sm"
                              disabled={busyAction === "approve"}
                              onClick={() =>
                                void handleApprovePermissions(
                                  selectedTrainer,
                                  false,
                                )
                              }
                            >
                              仅授权暂不注入
                            </button>
                          </div>
                        )}
                      </div>
                    )}

                    {/* Flat Feature Section */}
                    <div className="section-flat">
                      <div className="feature-toolbar">
                        <div>
                          <div className="feature-heading">
                            <h3>模组功能</h3>
                            <span className="muted mono">
                              ({selectedTrainer.features.length})
                            </span>
                            {findDuplicateHotkeys(
                              profilesByTrainer[selectedTrainer.trainer_id] ?? {},
                            ).length > 0 && (
                              <span className="tag unverified">
                                快捷键冲突:{" "}
                                {findDuplicateHotkeys(
                                  profilesByTrainer[selectedTrainer.trainer_id] ??
                                    {},
                                ).join(", ")}
                              </span>
                            )}
                          </div>
                          <div className="muted" style={{ marginTop: 2 }}>
                            {selectedTrainer.description}
                          </div>
                        </div>

                        <div className="row" style={{ gap: 6 }}>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              void handleApplyPreset(
                                selectedTrainer,
                                "Story Mode",
                              )
                            }
                          >
                            Story Mode
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              void handleApplyPreset(selectedTrainer, "No Grind")
                            }
                          >
                            No Grind
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              void handleApplyPreset(
                                selectedTrainer,
                                "Reset Defaults",
                              )
                            }
                            title="恢复默认开关与热键"
                          >
                            <RotateCcw size={14} strokeWidth={1.5} />
                            <span>重置</span>
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() => void handleSaveProfile(selectedTrainer)}
                            title="保存当前配置"
                          >
                            <Save size={14} strokeWidth={1.5} />
                            <span>保存</span>
                          </button>
                          <button
                            className="btn btn-ghost btn-sm"
                            onClick={() =>
                              void handleExportProfile(selectedTrainer)
                            }
                          >
                            <span>JSON</span>
                          </button>
                        </div>
                      </div>

                      {!isSelectedTrainerInjected && (
                        <div className="muted" style={{ padding: "2px 0" }}>
                          点击右上角「启动游戏并注入」或「一键注入 Mod」后即可切换开关并激活全局快捷键。
                        </div>
                      )}

                      {profileJsonTrainerId === selectedTrainer.trainer_id && (
                        <div className="surface-panel stack">
                          <div
                            className="row"
                            style={{ justifyContent: "space-between" }}
                          >
                            <span className="mono">
                              profile.json ({selectedTrainer.trainer_id})
                            </span>
                            <button
                              className="icon-btn"
                              onClick={() => setProfileJsonTrainerId(null)}
                              aria-label="关闭 JSON 编辑器"
                            >
                              <X size={14} strokeWidth={1.5} />
                            </button>
                          </div>
                          <textarea
                            className="text-input mono"
                            rows={6}
                            value={profileJsonBox}
                            onChange={(e) => setProfileJsonBox(e.target.value)}
                          />
                          <div className="row">
                            <button
                              className="btn btn-outline btn-sm"
                              onClick={() =>
                                void handleImportProfile(selectedTrainer)
                              }
                            >
                              导入 JSON 配置
                            </button>
                          </div>
                        </div>
                      )}

                      {/* Flat Hairline-Separated Feature Grid */}
                      <div className="feature-grid">
                        {selectedTrainer.features.map((f) => {
                          const isOn =
                            isSelectedTrainerInjected &&
                            Boolean(activeToggles[f.id]);
                          return (
                            <div
                              key={f.id}
                              className={`feature-cell ${isOn ? "enabled" : ""}`}
                            >
                              <div className="feature-cell-head">
                                <div>
                                  <div className="feature-title-row">
                                    <span className="feature-name">{f.name}</span>
                                    <span className="feature-type-tag">
                                      {f.control_type}
                                    </span>
                                  </div>
                                  <div className="feature-desc">
                                    {f.description}
                                  </div>
                                </div>
                              </div>

                              <div className="feature-cell-controls">
                                <input
                                  className="hotkey-input"
                                  placeholder="热键"
                                  aria-label={`Hotkey for ${f.name}`}
                                  value={
                                    profilesByTrainer[selectedTrainer.trainer_id]?.[
                                      f.id
                                    ] ??
                                    f.default_hotkey ??
                                    ""
                                  }
                                  onChange={(e) =>
                                    setProfilesByTrainer((prev) => ({
                                      ...prev,
                                      [selectedTrainer.trainer_id]: {
                                        ...(prev[selectedTrainer.trainer_id] ??
                                          {}),
                                        [f.id]: e.target.value,
                                      },
                                    }))
                                  }
                                />

                                <div className="row" style={{ gap: 6 }}>
                                  {f.control_type === "toggle" && (
                                    <button
                                      type="button"
                                      role="switch"
                                      aria-checked={isOn}
                                      className={`switch-btn ${isOn ? "on" : ""}`}
                                      disabled={!isSelectedTrainerInjected}
                                      onClick={() => void handleApplyFeature(f)}
                                    >
                                      <span>{isOn ? "开启" : "关闭"}</span>
                                      <span className="switch-track">
                                        <span className="switch-thumb" />
                                      </span>
                                    </button>
                                  )}
                                  {f.control_type === "action" && (
                                    <button
                                      className="btn btn-outline btn-sm"
                                      disabled={!isSelectedTrainerInjected}
                                      onClick={() => void handleApplyFeature(f)}
                                    >
                                      执行
                                    </button>
                                  )}
                                  {(f.control_type === "integer" ||
                                    f.control_type === "float") && (
                                    <>
                                      <input
                                        className="text-input mono"
                                        style={{
                                          width: 88,
                                          height: 26,
                                          padding: "2px 7px",
                                        }}
                                        type="number"
                                        step={
                                          f.control_type === "integer"
                                            ? "1"
                                            : "any"
                                        }
                                        placeholder="0"
                                        disabled={!isSelectedTrainerInjected}
                                        value={featureInputs[f.id] ?? ""}
                                        onChange={(e) =>
                                          setFeatureInputs((prev) => ({
                                            ...prev,
                                            [f.id]: e.target.value,
                                          }))
                                        }
                                      />
                                      <button
                                        className="btn btn-outline btn-sm"
                                        disabled={!isSelectedTrainerInjected}
                                        onClick={() =>
                                          void handleApplyFeature(f)
                                        }
                                      >
                                        应用
                                      </button>
                                    </>
                                  )}
                                </div>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  </>
                )
              )}

              {/* Lightweight 1px-Separated Accordions */}
              <div className="accordion-list">
                <details className="drawer">
                  <summary>
                    <span className="drawer-summary-left">
                      <Package size={14} strokeWidth={1.5} aria-hidden="true" />
                      <span>内置伴生桥接资源桶 (DeploymentLedger)</span>
                    </span>
                    <span className="row" style={{ gap: 6 }}>
                      <span className="muted mono">
                        {bridgeStatus?.deployed
                          ? "deployed"
                          : bridgeStatus?.matched_bridge
                            ? bridgeStatus.matched_bridge.id
                            : "idle"}
                      </span>
                      <ChevronRight
                        size={14}
                        strokeWidth={1.5}
                        className="drawer-chevron"
                      />
                    </span>
                  </summary>
                  <div className="drawer-body">
                    {bridgeStatus?.loader_display_name && (
                      <div className="flat-row">
                        <div>
                          <div className="row">
                            <span style={{ fontWeight: 500 }}>
                              {bridgeStatus.loader_display_name}
                            </span>
                            <span
                              className={`tag ${
                                bridgeStatus.loader_installed
                                  ? "official"
                                  : "unverified"
                              }`}
                            >
                              {bridgeStatus.loader_installed
                                ? "ready"
                                : "missing"}
                            </span>
                          </div>
                          <div className="muted" style={{ marginTop: 2 }}>
                            {bridgeStatus.loader_installed
                              ? "引导加载器已就绪，重启或运行游戏时将自动加载 NekodUnityBridge。"
                              : "缺少引导加载器（winhttp.dll / BepInEx/core），可通过 DeploymentLedger 一键安装与无痕卸载。"}
                          </div>
                        </div>
                        <div className="row">
                          {!bridgeStatus.loader_installed ? (
                            <button
                              className="btn btn-outline btn-sm"
                              disabled={
                                busyAction === "install-loader" ||
                                hasAntiCheatMarkers
                              }
                              onClick={() => void handleInstallBepInExLoader()}
                            >
                              <Download size={14} strokeWidth={1.5} />
                              <span>
                                {busyAction === "install-loader"
                                  ? "安装中..."
                                  : "安装加载器"}
                              </span>
                            </button>
                          ) : (
                            bridgeStatus.loader_managed_by_nekod && (
                              <button
                                className="btn btn-danger-outline btn-sm"
                                disabled={busyAction === "undeploy-loader"}
                                onClick={() =>
                                  void handleUndeployBepInExLoader()
                                }
                              >
                                <Trash2 size={14} strokeWidth={1.5} />
                                <span>
                                  {busyAction === "undeploy-loader"
                                    ? "卸载中..."
                                    : "卸载加载器"}
                                </span>
                              </button>
                            )
                          )}
                        </div>
                      </div>
                    )}

                    {bridgeStatus?.matched_bridge ? (
                      <div className="flat-row">
                        <div>
                          <div className="row">
                            <span style={{ fontWeight: 500 }}>
                              {bridgeStatus.matched_bridge.display_name}
                            </span>
                            <span className="mono muted">
                              v{bridgeStatus.matched_bridge.version}
                            </span>
                            <span
                              className={`tag ${
                                bridgeStatus.matched_bridge.ready_locally
                                  ? "official"
                                  : "unverified"
                              }`}
                            >
                              {bridgeStatus.matched_bridge.ready_locally
                                ? bridgeStatus.matched_bridge.source_kind
                                : "build_required"}
                            </span>
                          </div>
                          <div className="muted mono" style={{ marginTop: 3 }}>
                            {bridgeStatus.matched_bridge.target_relative_path}
                            {bridgeStatus.matched_bridge.sha256
                              ? ` · sha256:${bridgeStatus.matched_bridge.sha256.slice(0, 12)}`
                              : ""}
                          </div>
                        </div>
                        <div className="row">
                          {!bridgeStatus.deployed ? (
                            <button
                              className="btn btn-outline btn-sm"
                              disabled={
                                busyAction === "deploy-bridge" ||
                                hasAntiCheatMarkers
                              }
                              onClick={() => void handleAutoDeployBridge(null)}
                            >
                              <Download size={14} strokeWidth={1.5} />
                              <span>一键自动部署</span>
                            </button>
                          ) : (
                            <button
                              className="btn btn-danger-outline btn-sm"
                              disabled={busyAction === "undeploy-bridge"}
                              onClick={() => void handleUndeployBridge()}
                            >
                              <Trash2 size={14} strokeWidth={1.5} />
                              <span>卸载并还原</span>
                            </button>
                          )}
                        </div>
                      </div>
                    ) : (
                      <div className="muted">
                        当前游戏引擎（
                        {bridgeStatus
                          ? ENGINE_LABELS[bridgeStatus.engine]
                          : "未知"}
                        ）无需伴生桥接，或可在下方手动指定自定义插件路径。
                      </div>
                    )}

                    {bridgeStatus && bridgeStatus.active_files.length > 0 && (
                      <div className="muted mono">
                        账本记录文件: {bridgeStatus.active_files.join(", ")}
                      </div>
                    )}

                    <details className="drawer">
                      <summary>
                        <span className="drawer-summary-left">
                          <Layers size={14} strokeWidth={1.5} />
                          <span>全部内置伴生桥接清单 ({bridgeCatalog.length})</span>
                        </span>
                        <ChevronRight
                          size={14}
                          strokeWidth={1.5}
                          className="drawer-chevron"
                        />
                      </summary>
                      <div className="drawer-body" style={{ paddingLeft: 16 }}>
                        {bridgeCatalog.map((b) => (
                          <div key={b.id} className="flat-row">
                            <div>
                              <div className="row">
                                <span style={{ fontWeight: 500 }}>
                                  {b.display_name}
                                </span>
                                <span className="mono muted">
                                  {ENGINE_LABELS[b.engine]}
                                </span>
                                <span
                                  className={`tag ${
                                    b.ready_locally ? "official" : "unverified"
                                  }`}
                                >
                                  {b.ready_locally ? "ready" : "missing"}
                                </span>
                              </div>
                              <div className="muted mono" style={{ marginTop: 2 }}>
                                {b.target_relative_path}
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    </details>

                    <div className="row">
                      <input
                        className="text-input mono"
                        style={{ flex: 1 }}
                        placeholder="手动源路径（可选）"
                        value={bridgeSourceInput}
                        onChange={(e) => setBridgeSourceInput(e.target.value)}
                      />
                      <input
                        className="text-input mono"
                        style={{ flex: 1 }}
                        placeholder="游戏内目标相对路径"
                        value={bridgeTargetInput}
                        onChange={(e) => setBridgeTargetInput(e.target.value)}
                      />
                      <button
                        className="btn btn-ghost btn-sm"
                        disabled={
                          busyAction === "deploy-bridge" || hasAntiCheatMarkers
                        }
                        onClick={() => void handleManualDeployBridge()}
                      >
                        手动部署
                      </button>
                    </div>
                  </div>
                </details>

                {selectedTrainer && (
                  <details className="drawer">
                    <summary>
                      <span className="drawer-summary-left">
                        <Shield size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>
                          权限明细与撤销 ({selectedTrainer.permissions.length})
                        </span>
                      </span>
                      <span className="row" style={{ gap: 6 }}>
                        <span className="muted mono">
                          {selectedTrainer.permission_state.status}
                        </span>
                        <ChevronRight
                          size={14}
                          strokeWidth={1.5}
                          className="drawer-chevron"
                        />
                      </span>
                    </summary>
                    <div className="drawer-body">
                      {selectedTrainer.permissions.map((p) => (
                        <div key={p.scope} className="muted">
                          <span style={{ color: "var(--text-primary)" }}>
                            {PERMISSION_LABELS[p.scope]}
                          </span>{" "}
                          <span className="mono">({p.scope})</span>：{p.reason}
                        </div>
                      ))}
                      {selectedTrainer.permission_state.status === "approved" &&
                        !activeSessionId && (
                          <div>
                            <button
                              className="btn btn-ghost btn-sm"
                              onClick={() =>
                                void handleRevokePermissions(selectedTrainer)
                              }
                            >
                              撤销该模组授权
                            </button>
                          </div>
                        )}
                    </div>
                  </details>
                )}

                {sessionDiag && sessionDiag.recent_events.length > 0 && (
                  <details className="drawer" open={isSelectedGameInjected}>
                    <summary>
                      <span className="drawer-summary-left">
                        <Activity size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>实时注入会话事件 ({sessionDiag.session_id})</span>
                      </span>
                      <span className="row" style={{ gap: 6 }}>
                        <span className="muted mono">
                          {sessionDiag.state} · conflicts{" "}
                          {sessionDiag.rollback_conflicts}
                        </span>
                        <ChevronRight
                          size={14}
                          strokeWidth={1.5}
                          className="drawer-chevron"
                        />
                      </span>
                    </summary>
                    <div className="drawer-body">
                      {sessionDiag.recent_events.slice(-8).map((ev, idx) => (
                        <div key={idx} className="muted mono">
                          {ev}
                        </div>
                      ))}
                    </div>
                  </details>
                )}

                {selectedGame.game.matched_game_id && (
                  <details className="drawer">
                    <summary>
                      <span className="drawer-summary-left">
                        <Layers size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>兼容矩阵 ({compatRecords.length})</span>
                      </span>
                      <ChevronRight
                        size={14}
                        strokeWidth={1.5}
                        className="drawer-chevron"
                      />
                    </summary>
                    <div className="drawer-body">
                      {compatRecords.length === 0 ? (
                        <div className="muted">暂无已记录的版本兼容验证结果。</div>
                      ) : (
                        compatRecords.map((rec) => (
                          <div key={rec.record_id} className="flat-row">
                            <div>
                              <div className="row">
                                <span style={{ fontWeight: 500 }}>{rec.mod_id}</span>
                                <span className="mono muted">
                                  v{rec.mod_version}
                                </span>
                                <span className="tag">{rec.runtime_adapter}</span>
                                <span className="tag">{rec.status}</span>
                              </div>
                              <div className="muted mono" style={{ marginTop: 2 }}>
                                Build {rec.game_version} ({rec.store_platform}) ·{" "}
                                {rec.evidence}
                                {rec.notes ? ` · ${rec.notes}` : ""}
                              </div>
                            </div>
                          </div>
                        ))
                      )}
                    </div>
                  </details>
                )}

                {selectedGame.game.matched_game_id && (
                  <details className="drawer">
                    <summary>
                      <span className="drawer-summary-left">
                        <Globe size={14} strokeWidth={1.5} aria-hidden="true" />
                        <span>社区可用模组 ({communityTrainers.length})</span>
                      </span>
                      <ChevronRight
                        size={14}
                        strokeWidth={1.5}
                        className="drawer-chevron"
                      />
                    </summary>
                    <div className="drawer-body">
                      <div className="row" style={{ justifyContent: "flex-end" }}>
                        <button
                          className="btn btn-ghost btn-sm"
                          disabled={busyAction === "sync"}
                          onClick={() => void handleSyncRegistries()}
                        >
                          <RefreshCw
                            size={14}
                            strokeWidth={1.5}
                            className={busyAction === "sync" ? "spin" : ""}
                          />
                          <span>
                            {busyAction === "sync" ? "同步中..." : "同步社区源"}
                          </span>
                        </button>
                      </div>
                      {communityTrainers.length === 0 ? (
                        <div className="muted">
                          当前游戏在已同步社区源中暂无模组，可前往左侧「社区源」添加仓库并同步。
                        </div>
                      ) : (
                        communityTrainers.map((ct) => (
                          <div
                            key={`${ct.repository_id}:${ct.trainer_id}`}
                            className="flat-row"
                          >
                            <div>
                              <div className="row">
                                <span style={{ fontWeight: 500 }}>
                                  {ct.trainer_id}
                                </span>
                                <span className="mono muted">
                                  v{ct.latest_version}
                                </span>
                                {trustTag(ct.trust_tier)}
                                {ct.compatibility_status && (
                                  <span className="tag">
                                    {ct.compatibility_status}
                                  </span>
                                )}
                              </div>
                              <div className="muted mono" style={{ marginTop: 2 }}>
                                {ct.repository_id}
                                {ct.last_verified_game_version
                                  ? ` · verified ${ct.last_verified_game_version}`
                                  : ""}
                              </div>
                            </div>
                            <div className="row">
                              {ct.install_state.state === "not_installed" && (
                                <button
                                  className="btn btn-outline btn-sm"
                                  disabled={
                                    busyAction === `install:${ct.trainer_id}`
                                  }
                                  onClick={() =>
                                    void handleInstallOrUpdateCommunity(
                                      ct.repository_id,
                                      ct.trainer_id,
                                    )
                                  }
                                >
                                  <Download size={14} strokeWidth={1.5} />
                                  <span>
                                    {busyAction === `install:${ct.trainer_id}`
                                      ? "安装中..."
                                      : "安装"}
                                  </span>
                                </button>
                              )}
                              {ct.install_state.state ===
                                "installed_up_to_date" && (
                                <span className="tag official">installed</span>
                              )}
                              {ct.install_state.state === "update_available" && (
                                <button
                                  className="btn btn-outline btn-sm"
                                  disabled={
                                    busyAction === `install:${ct.trainer_id}`
                                  }
                                  onClick={() =>
                                    void handleInstallOrUpdateCommunity(
                                      ct.repository_id,
                                      ct.trainer_id,
                                    )
                                  }
                                >
                                  更新至 v{ct.install_state.latest_version}
                                </button>
                              )}
                              {ct.install_state.state === "revoked" && (
                                <span className="tag unverified">revoked</span>
                              )}
                            </div>
                          </div>
                        ))
                      )}
                    </div>
                  </details>
                )}
              </div>
            </section>
          </>
        )}

        {/* ==================== SECONDARY TABS ==================== */}
        {tab === "community" && (
          <div className="page-scroll">
            <div className="page-inner stack" style={{ gap: 16 }}>
              <div className="header-row" style={{ marginBottom: 0 }}>
                <div className="row">
                  {selectedGame && (
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={() => setTab("workbench")}
                    >
                      <ArrowLeft size={14} strokeWidth={1.5} />
                      <span>工作台</span>
                    </button>
                  )}
                  <h2 style={{ margin: 0, fontSize: 16, fontWeight: 600 }}>
                    社区仓库与更新
                  </h2>
                </div>
                <button
                  className="btn btn-primary btn-sm"
                  disabled={busyAction === "sync"}
                  onClick={() => void handleSyncRegistries()}
                >
                  <RefreshCw
                    size={14}
                    strokeWidth={1.5}
                    className={busyAction === "sync" ? "spin" : ""}
                  />
                  <span>{busyAction === "sync" ? "同步中..." : "同步所有源"}</span>
                </button>
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>
                  可用更新 <span className="muted mono">({updates.length})</span>
                </div>
                {updates.length === 0 ? (
                  <div className="muted">所有已安装模组均为最新版本。</div>
                ) : (
                  updates.map((u) => (
                    <div
                      key={`${u.repository_id}:${u.trainer_id}`}
                      className="flat-row"
                    >
                      <div>
                        <div className="row">
                          <span style={{ fontWeight: 500 }}>{u.trainer_id}</span>
                          <span className="tag">{u.game_id}</span>
                          {trustTag(u.trust_tier)}
                        </div>
                        <div className="muted mono" style={{ marginTop: 2 }}>
                          v{u.installed_version} -&gt; v{u.available_version} (
                          {u.repository_id})
                        </div>
                      </div>
                      <button
                        className="btn btn-outline btn-sm"
                        disabled={busyAction === `install:${u.trainer_id}`}
                        onClick={() =>
                          void handleInstallOrUpdateCommunity(
                            u.repository_id,
                            u.trainer_id,
                          )
                        }
                      >
                        {busyAction === `install:${u.trainer_id}`
                          ? "更新中..."
                          : `更新至 v${u.available_version}`}
                      </button>
                    </div>
                  ))
                )}
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>
                  已注册仓库{" "}
                  <span className="muted mono">({registrySources.length})</span>
                </div>
                {registrySources.length === 0 ? (
                  <div className="muted">尚未注册任何社区源。</div>
                ) : (
                  registrySources.map((s) => (
                    <div key={s.source.repository_id} className="flat-row">
                      <div>
                        <div className="row">
                          <span style={{ fontWeight: 500 }}>
                            {s.source.repository_id}
                          </span>
                          {trustTag(s.source.authoritative_trust_tier)}
                          <span className="tag">{s.last_status}</span>
                        </div>
                        <div className="muted mono" style={{ marginTop: 2 }}>
                          {s.source.index_url}
                        </div>
                        {s.etag && (
                          <div className="muted mono">ETag: {s.etag}</div>
                        )}
                        {s.last_error && (
                          <div className="mono" style={{ color: "#fca5a5" }}>
                            {s.last_error}
                          </div>
                        )}
                      </div>
                      <button
                        className="btn btn-ghost btn-sm"
                        onClick={() =>
                          void handleRemoveRepo(s.source.repository_id)
                        }
                      >
                        移除
                      </button>
                    </div>
                  ))
                )}
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>添加第三方社区仓库</div>
                <div className="muted">
                  输入 HTTPS 协议的 <span className="mono">registry-manifest.json</span>{" "}
                  地址。第三方仓库默认归类为 Unverified。
                </div>
                <div className="row">
                  <input
                    className="search-input mono"
                    style={{ width: 200 }}
                    placeholder="repository-id"
                    value={repoIdInput}
                    onChange={(e) => setRepoIdInput(e.target.value)}
                  />
                  <input
                    className="search-input mono"
                    style={{ flex: 1 }}
                    placeholder="https://example.org/nekod/registry-manifest.json"
                    value={repoUrlInput}
                    onChange={(e) => setRepoUrlInput(e.target.value)}
                  />
                  <button
                    className="btn btn-outline"
                    disabled={busyAction === "add-repo"}
                    onClick={() => void handleAddThirdPartyRepo()}
                  >
                    添加仓库
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {tab === "import" && (
          <div className="page-scroll">
            <div className="page-inner stack" style={{ gap: 16 }}>
              <div className="header-row" style={{ marginBottom: 0 }}>
                <div className="row">
                  {selectedGame && (
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={() => setTab("workbench")}
                    >
                      <ArrowLeft size={14} strokeWidth={1.5} />
                      <span>工作台</span>
                    </button>
                  )}
                  <h2 style={{ margin: 0, fontSize: 16, fontWeight: 600 }}>
                    本地包与游戏清单导入
                  </h2>
                </div>
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>1. 注册 Game Manifest JSON</div>
                <textarea
                  className="text-input mono"
                  rows={6}
                  placeholder="粘贴通过校验的 game-manifest.json..."
                  value={gameManifestJson}
                  onChange={(e) => setGameManifestJson(e.target.value)}
                />
                <div>
                  <button
                    className="btn btn-primary"
                    onClick={() => void handleRegisterGameManifest()}
                  >
                    校验并注册游戏清单
                  </button>
                </div>
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>2. 导入本地模组目录</div>
                <div className="muted">
                  指定包含 <span className="mono">trainer-manifest.json</span>{" "}
                  及对应构件的本地目录，导入前强制校验 JSON Schema 与 SHA-256。
                </div>
                <div className="row">
                  <input
                    className="text-input mono"
                    style={{ flex: 1 }}
                    placeholder="C:\Trainers\my-trainer-package"
                    value={trainerDir}
                    onChange={(e) => setTrainerDir(e.target.value)}
                  />
                  <select
                    className="select-input"
                    value={trainerTrust}
                    onChange={(e) =>
                      setTrainerTrust(e.target.value as TrustTier)
                    }
                  >
                    <option value="official">Official</option>
                    <option value="community_verified">Community Verified</option>
                    <option value="unverified">Unverified</option>
                  </select>
                  <button
                    className="btn btn-outline"
                    onClick={() => void handleIngestTrainer()}
                  >
                    校验并导入
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {tab === "settings" && (
          <div className="page-scroll">
            <div className="page-inner stack" style={{ gap: 16 }}>
              <div className="header-row" style={{ marginBottom: 0 }}>
                <div className="row">
                  {selectedGame && (
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={() => setTab("workbench")}
                    >
                      <ArrowLeft size={14} strokeWidth={1.5} />
                      <span>工作台</span>
                    </button>
                  )}
                  <h2 style={{ margin: 0, fontSize: 16, fontWeight: 600 }}>
                    设置与安全策略
                  </h2>
                </div>
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>Default-Deny 安全控制</div>
                <label className="row" style={{ cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={policy.allow_unverified_sources}
                    onChange={(e) =>
                      void handleTogglePolicy({
                        ...policy,
                        allow_unverified_sources: e.target.checked,
                      })
                    }
                  />
                  <span>
                    允许未认证（Unverified）来源的声明式模组（附带风险提示）
                  </span>
                </label>
                <label className="row" style={{ cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={policy.allow_third_party_native_modules}
                    onChange={(e) =>
                      void handleTogglePolicy({
                        ...policy,
                        allow_third_party_native_modules: e.target.checked,
                      })
                    }
                  />
                  <span>允许第三方原生运行时模块（高风险，默认关闭）</span>
                </label>
              </div>

              <div className="surface-panel stack">
                <div style={{ fontWeight: 600 }}>自定义游戏目录</div>
                <div className="row">
                  <input
                    className="text-input mono"
                    style={{ flex: 1 }}
                    placeholder="D:\Games\HollowKnight"
                    value={customDir}
                    onChange={(e) => setCustomDir(e.target.value)}
                  />
                  <input
                    className="text-input"
                    placeholder="可选显示标签"
                    value={customLabel}
                    onChange={(e) => setCustomLabel(e.target.value)}
                  />
                  <button
                    className="btn btn-outline"
                    onClick={() => void handleAddCustomPath()}
                  >
                    添加目录
                  </button>
                </div>

                {customPaths.map((cp) => (
                  <div key={cp.path} className="flat-row">
                    <div>
                      <div style={{ fontWeight: 500 }}>{cp.label ?? cp.path}</div>
                      <div className="muted mono">{cp.path}</div>
                    </div>
                    <button
                      className="btn btn-danger-outline btn-sm"
                      onClick={async () => {
                        try {
                          setError(null);
                          setCustomPaths(
                            await nekodApi.removeCustomGamePath(cp.path),
                          );
                          await handleRescan();
                        } catch (e) {
                          setError(String(e));
                        }
                      }}
                    >
                      移除
                    </button>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {tab === "logs" && (
          <div className="page-scroll">
            <div className="page-inner stack" style={{ gap: 16 }}>
              <div className="header-row" style={{ marginBottom: 0 }}>
                <div className="row">
                  {selectedGame && (
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={() => setTab("workbench")}
                    >
                      <ArrowLeft size={14} strokeWidth={1.5} />
                      <span>工作台</span>
                    </button>
                  )}
                  <h2 style={{ margin: 0, fontSize: 16, fontWeight: 600 }}>
                    诊断与审计日志
                  </h2>
                </div>
                <div className="row">
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => {
                      const summary = JSON.stringify(
                        {
                          generated_at: new Date().toISOString(),
                          security_policy: policy,
                          active_session: sessionDiag,
                          logs,
                        },
                        null,
                        2,
                      );
                      setDiagnosticBundle(summary);
                      setStatusMsg("已生成可审计诊断包 JSON。");
                    }}
                  >
                    导出诊断包
                  </button>
                  <button
                    className="btn btn-outline btn-sm"
                    onClick={async () => {
                      try {
                        setLogs(await nekodApi.getDiagnosticLogs());
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  >
                    <RefreshCw size={14} strokeWidth={1.5} />
                    <span>刷新</span>
                  </button>
                </div>
              </div>

              {diagnosticBundle && (
                <div className="surface-panel stack">
                  <div
                    className="row"
                    style={{ justifyContent: "space-between" }}
                  >
                    <span className="mono">diagnostic-bundle.json</span>
                    <button
                      className="icon-btn"
                      onClick={() => setDiagnosticBundle(null)}
                      aria-label="收起诊断包"
                    >
                      <X size={14} strokeWidth={1.5} />
                    </button>
                  </div>
                  <textarea
                    className="text-input mono"
                    rows={8}
                    readOnly
                    value={diagnosticBundle}
                  />
                </div>
              )}

              <div className="surface-panel" style={{ padding: "4px 10px" }}>
                {logs.length === 0 ? (
                  <div className="muted" style={{ padding: 10 }}>
                    暂无诊断事件记录。
                  </div>
                ) : (
                  logs.map((entry, idx) => (
                    <div key={idx} className="log-line">
                      [{entry.level.toUpperCase()}] ({entry.category}){" "}
                      {entry.message}
                    </div>
                  ))
                )}
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Floating HUD Toast Overlay */}
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
