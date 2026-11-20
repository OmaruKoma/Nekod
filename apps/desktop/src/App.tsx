import { useEffect, useMemo, useState } from "react";
import {
  BridgeDeploymentStatus,
  CommunityTrainerView,
  CompatibilityRecord,
  CustomGamePathRecord,
  DiagnosticEvent,
  FeatureCommand,
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

type NavTab = "workbench" | "community" | "import" | "settings" | "logs";

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

function trustBadge(tier: TrustTier) {
  if (tier === "official") return <span className="badge official">Official</span>;
  if (tier === "community_verified") {
    return <span className="badge community">Community Verified</span>;
  }
  return <span className="badge unverified">Unverified</span>;
}

export default function App() {
  const [tab, setTab] = useState<NavTab>("workbench");
  const [games, setGames] = useState<LibraryGameView[]>([]);
  const [search, setSearch] = useState("");
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

  // Import & custom folder form states
  const [customDir, setCustomDir] = useState("");
  const [customLabel, setCustomLabel] = useState("");
  const [trainerDir, setTrainerDir] = useState("");
  const [trainerTrust, setTrainerTrust] = useState<TrustTier>("community_verified");
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

  // Companion bridge deployment state (DeploymentLedger)
  const [bridgeStatus, setBridgeStatus] = useState<BridgeDeploymentStatus | null>(null);
  const [bridgeStatusByGame, setBridgeStatusByGame] = useState<
    Record<string, BridgeDeploymentStatus>
  >({});
  const [bridgeSourceInput, setBridgeSourceInput] = useState("");
  const [bridgeTargetInput, setBridgeTargetInput] = useState("");

  // Community ecosystem & profiles state
  const [registrySources, setRegistrySources] = useState<RegistrySourceState[]>([]);
  const [communityTrainers, setCommunityTrainers] = useState<CommunityTrainerView[]>([]);
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
  const [profileJsonTrainerId, setProfileJsonTrainerId] = useState<string | null>(null);
  const [profileJsonBox, setProfileJsonBox] = useState("");
  const [diagnosticBundle, setDiagnosticBundle] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);

  const selectedTrainer = useMemo(() => {
    if (trainers.length === 0) return null;
    return trainers.find((t) => t.trainer_id === selectedTrainerId) ?? trainers[0];
  }, [trainers, selectedTrainerId]);

  const isSelectedGameInjected =
    activeSessionId !== null &&
    selectedGame !== null &&
    activeGameKey === selectedGame.library_key;

  const isSelectedTrainerInjected =
    isSelectedGameInjected &&
    selectedTrainer !== null &&
    activeTrainerId === selectedTrainer.trainer_id;

  const hasAntiCheatMarkers = (gameEnv?.anti_cheat_files_present.length ?? 0) > 0;

  async function refreshAll(autoSelectFirst = false) {
    try {
      setError(null);
      const [lib, paths, sec, diag, sources, availUpdates] = await Promise.all([
        nekodApi.listLibraryGames(),
        nekodApi.listCustomGamePaths(),
        nekodApi.getSecurityPolicy(),
        nekodApi.getDiagnosticLogs(),
        nekodApi.listRegistrySources(),
        nekodApi.checkTrainerUpdates(),
      ]);
      setGames(lib);
      setCustomPaths(paths);
      setPolicy(sec);
      setLogs(diag);
      setRegistrySources(sources);
      setUpdates(availUpdates);
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

  async function handleRescan() {
    try {
      setBusyAction("rescan");
      setError(null);
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(`已重新扫描游戏库：发现 ${lib.length} 款已安装游戏。`);
      if (lib.length > 0 && !selectedGame) {
        await openGame(lib[0], false);
      } else if (selectedGame) {
        const refreshed = lib.find((g) => g.library_key === selectedGame.library_key);
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

  async function scanRunningProcesses(gameId: string): Promise<ProcessIdentity[]> {
    try {
      const procs = await nekodApi.discoverRunningProcesses(gameId);
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
      setBridgeSourceInput(status.suggested_source_file ?? "");
      setBridgeTargetInput(status.suggested_target_relative_path ?? "");
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
    setAwaitingPermInject(null);
    setRunningProcs([]);
    setSelectedPid(null);
    setGameEnv(null);
    setCompatRecords([]);
    setProfileJsonTrainerId(null);

    await Promise.all([
      (async () => {
        try {
          const env = await nekodApi.inspectGameEnvironment(entry.game.install_dir);
          setGameEnv(env);
        } catch {
          // Non-fatal if install directory is temporarily inaccessible
        }
      })(),
      refreshBridgeStatus(entry),
    ]);

    if (entry.game.matched_game_id) {
      const gameId = entry.game.matched_game_id;
      try {
        const [list, procs, community, matrix] = await Promise.all([
          nekodApi.listTrainersForGame(gameId, entry.game.store_build_id),
          nekodApi.discoverRunningProcesses(gameId),
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
        const loadedPresets: Record<string, Record<string, Record<string, unknown>>> = {};
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
    }
  }

  async function startSessionInternal(
    trainer: TrainerDetailView,
    overrideProcs?: ProcessIdentity[],
  ) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId || !selectedGame) return;
    if (hasAntiCheatMarkers) {
      setError(
        "检测到反作弊保护标记，根据安全策略拒绝注入该游戏进程（Nekod 仅支持单机/无反作弊环境）。",
      );
      return;
    }

    const procs = overrideProcs ?? runningProcs;
    const proc = procs.find((p) => p.pid === selectedPid) ?? procs[0];
    if (!proc) {
      setError("未检测到运行中的游戏进程。请先启动游戏，再点击「一键注入 Mod」。");
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
      setSessionDiag(await nekodApi.getSessionDiagnostics(sessionId));
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(
        `已成功注入「${trainer.name}」到 ${proc.process_name} (PID ${proc.pid})，现在可以直接开关下方模组功能。`,
      );
    } catch (e) {
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
      setError(
        "检测到反作弊保护标记，根据安全策略拒绝注入该游戏进程（Nekod 仅支持单机/无反作弊环境）。",
      );
      return;
    }
    if (trainer.permission_state.status === "blocked") {
      setError(`当前模组被安全策略阻止：${trainer.permission_state.reason}`);
      return;
    }
    if (trainer.permission_state.status === "requires_confirmation") {
      setAwaitingPermInject(trainer.trainer_id);
      setStatusMsg(
        `「${trainer.name}」需要您确认声明的运行权限，请在下方权限卡片点击「确认权限并立即注入」。`,
      );
      return;
    }

    let procs = runningProcs;
    if (procs.length === 0) {
      procs = await scanRunningProcesses(selectedGame.game.matched_game_id);
    }
    await startSessionInternal(trainer, procs);
  }

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
          procs = await scanRunningProcesses(selectedGame.game.matched_game_id);
        }
        await startSessionInternal(updatedTrainer, procs);
      } else {
        setStatusMsg(`已授权 ${trainer.name} v${trainer.version} 的声明权限。`);
      }
    } catch (e) {
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
      setStatusMsg(
        `已解除注入并按 LIFO 顺序完成内存回滚（冲突: ${diag.rollback_conflicts}，失败: ${diag.rollback_failures}）。`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
  }

  async function handleDeployBridge() {
    if (!selectedGame) return;
    const src = bridgeSourceInput.trim();
    const target = bridgeTargetInput.trim();
    if (!src || !target) {
      setError("请先在下方「伴生桥接部署配置」中填写源 DLL/脚本路径和目标相对路径。");
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
      try {
        setGameEnv(
          await nekodApi.inspectGameEnvironment(selectedGame.game.install_dir),
        );
      } catch {
        // Ignore env refresh error
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(
        `已通过 DeploymentLedger 安全部署伴生桥接文件到 ${target}（可随时一键解除）。`,
      );
    } catch (e) {
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
        // Ignore env refresh error
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(
        `已解除伴生桥接部署：移除 ${summary.removed_count} 个文件，还原 ${summary.restored_originals} 个原文件（冲突: ${summary.conflicts}）。`,
      );
    } catch (e) {
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

  async function handleApplyFeature(feature: TrainerFeature) {
    if (!activeSessionId) return;
    let command: FeatureCommand;
    if (feature.control_type === "toggle") {
      const next = !activeToggles[feature.id];
      command = { feature_id: feature.id, value: { type: "toggle", value: next } };
    } else if (feature.control_type === "action") {
      command = { feature_id: feature.id, value: { type: "action" } };
    } else {
      const raw = (featureInputs[feature.id] ?? "").trim();
      const num = Number(raw);
      if (!raw || !Number.isFinite(num)) {
        setError(`请为「${feature.name}」输入有效数字。`);
        return;
      }
      command =
        feature.control_type === "integer"
          ? { feature_id: feature.id, value: { type: "integer", value: Math.trunc(num) } }
          : { feature_id: feature.id, value: { type: "float", value: num } };
    }

    try {
      setError(null);
      await nekodApi.applyTrainerFeature(activeSessionId, command);
      if (feature.control_type === "toggle") {
        setActiveToggles((prev) => ({ ...prev, [feature.id]: !prev[feature.id] }));
      }
      const [diag, logList] = await Promise.all([
        nekodApi.getSessionDiagnostics(activeSessionId),
        nekodApi.getDiagnosticLogs(),
      ]);
      setSessionDiag(diag);
      setLogs(logList);
    } catch (e) {
      setError(String(e));
      if (activeSessionId) {
        try {
          setSessionDiag(await nekodApi.getSessionDiagnostics(activeSessionId));
        } catch {
          // Keep UI responsive if session faulted
        }
      }
    }
  }

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
      await nekodApi.installOrUpdateTrainerFromRegistry(repositoryId, trainerId);
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
        setProfilesByTrainer((prev) => ({ ...prev, [trainer.trainer_id]: defaults }));
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
          const enable = presetName === "Story Mode" ? f.risk_level === "low" : true;
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
        saved_feature_states: activeTrainerId === trainer.trainer_id ? activeToggles : {},
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
    if (!activeSessionId || !activeTrainerId) return;
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
        if (f.control_type !== "toggle" && f.control_type !== "action") return false;
        const bound =
          profilesByTrainer[trainer.trainer_id]?.[f.id] ?? f.default_hotkey ?? "";
        return bound.trim().toLowerCase() === combo;
      });
      if (match) {
        e.preventDefault();
        void handleApplyFeature(match);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeSessionId, activeTrainerId, trainers, profilesByTrainer, activeToggles]);

  const filteredGames = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return games;
    return games.filter(
      (g) =>
        g.game.title.toLowerCase().includes(q) ||
        (g.game.matched_game_id ?? "").toLowerCase().includes(q),
    );
  }, [games, search]);

  const activeProcessLabel = useMemo(() => {
    const proc = runningProcs.find((p) => p.pid === selectedPid) ?? runningProcs[0];
    return proc ? `${proc.process_name} (PID ${proc.pid})` : null;
  }, [runningProcs, selectedPid]);

  const bridgeAvailable = Boolean(
    bridgeStatus &&
      (bridgeStatus.deployed ||
        bridgeStatus.suggested_target_relative_path ||
        bridgeStatus.engine !== "native_unknown"),
  );

  return (
    <div className="workbench-shell">
      {/* ==================== LEFT PANE: 游戏列表 ==================== */}
      <aside className="game-sidebar">
        <div className="sidebar-top">
          <div className="brand-row">
            <div>
              <h1 className="brand-title">Nekod</h1>
              <p className="brand-sub">Modding games, together.</p>
            </div>
            <button
              className="btn btn-secondary btn-sm"
              disabled={busyAction === "rescan"}
              onClick={() => void handleRescan()}
              title="重新扫描 Steam 与自定义游戏目录"
            >
              {busyAction === "rescan" ? "扫描中..." : "刷新"}
            </button>
          </div>
          <input
            className="search-input sidebar-search"
            placeholder="搜索游戏列表..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>

        <div className="sidebar-section-header">
          <span>游戏列表 ({filteredGames.length})</span>
          {activeSessionId && (
            <span style={{ color: "var(--ok)", fontWeight: 700 }}>● 注入中</span>
          )}
        </div>

        <div className="game-list">
          {filteredGames.length === 0 ? (
            <div className="muted" style={{ padding: "12px 6px", lineHeight: 1.5 }}>
              未匹配到游戏。可在下方「设置」添加自定义游戏目录，或在「导入包」注册游戏清单。
            </div>
          ) : (
            filteredGames.map((item) => {
              const isSelected =
                tab === "workbench" &&
                selectedGame?.library_key === item.library_key;
              const isInjected =
                activeSessionId !== null && activeGameKey === item.library_key;
              const isBridgeDeployed = Boolean(
                bridgeStatusByGame[item.library_key]?.deployed,
              );

              return (
                <button
                  key={item.library_key}
                  className={`game-list-item ${isSelected ? "active" : ""}`}
                  onClick={() => void openGame(item, true)}
                >
                  <span
                    className={`game-status-dot ${
                      isInjected
                        ? "injected"
                        : isBridgeDeployed
                          ? "bridge-ready"
                          : ""
                    }`}
                    title={
                      isInjected
                        ? "已注入运行中"
                        : isBridgeDeployed
                          ? "已部署伴生桥接"
                          : "未注入"
                    }
                  />
                  <div className="game-list-meta">
                    <div className="game-list-title">{item.game.title}</div>
                    <div className="game-list-sub">
                      <span>{item.game.store_source.toUpperCase()}</span>
                      <span>&middot;</span>
                      <span>{item.trainer_count} 模组</span>
                      {isInjected && (
                        <>
                          <span>&middot;</span>
                          <span style={{ color: "#86efac", fontWeight: 600 }}>
                            已注入
                          </span>
                        </>
                      )}
                    </div>
                  </div>
                </button>
              );
            })
          )}
        </div>

        <nav className="bottom-nav" aria-label="Secondary navigation">
          <button
            className={`nav-pill span-2 ${tab === "workbench" ? "active" : ""}`}
            onClick={() => setTab("workbench")}
          >
            模组工作台
          </button>
          <button
            className={`nav-pill ${tab === "community" ? "active" : ""}`}
            onClick={() => setTab("community")}
          >
            社区源{updates.length > 0 ? ` (${updates.length})` : ""}
          </button>
          <button
            className={`nav-pill ${tab === "import" ? "active" : ""}`}
            onClick={() => setTab("import")}
          >
            本地导入
          </button>
          <button
            className={`nav-pill ${tab === "settings" ? "active" : ""}`}
            onClick={() => setTab("settings")}
          >
            设置安全
          </button>
          <button
            className={`nav-pill ${tab === "logs" ? "active" : ""}`}
            onClick={async () => {
              setTab("logs");
              try {
                setLogs(await nekodApi.getDiagnosticLogs());
              } catch (e) {
                setError(String(e));
              }
            }}
          >
            诊断日志
          </button>
        </nav>
      </aside>

      {/* ==================== RIGHT COLUMN ==================== */}
      <div className="right-pane">
        {(isBrowserPreview || error || statusMsg) && (
          <div className="top-banners">
            {isBrowserPreview && (
              <div className="banner">
                浏览器预览模式：当前展示模拟 IPC 数据；运行 Tauri 桌面端可连接真实 Windows x64 进程与 SQLite 账本。
              </div>
            )}
            {error && <div className="error-banner">{error}</div>}
            {statusMsg && <div className="banner">{statusMsg}</div>}
          </div>
        )}

        {/* ==================== WORKBENCH (Top-Right 游戏信息 + Bottom-Right 模组的功能) ==================== */}
        {tab === "workbench" && !selectedGame && (
          <div className="empty-state">
            <h2 style={{ margin: 0, color: "var(--text)" }}>选择一款游戏开始</h2>
            <p style={{ margin: 0, maxWidth: 420 }}>
              点击左侧「游戏列表」中的任意游戏，即可在上方查看游戏信息并一键注入 / 解除注入 Mod，在下方直接控制模组功能。
            </p>
            <button className="btn" onClick={() => void handleRescan()}>
              立即扫描游戏库
            </button>
          </div>
        )}

        {tab === "workbench" && selectedGame && (
          <>
            {/* ---------- TOP-RIGHT: 游戏信息 & 一键注入 / 解除注入 ---------- */}
            <header className="game-hero">
              <div className="game-hero-main">
                <div className="game-hero-info">
                  <div className="row">
                    <h2 className="game-hero-title">{selectedGame.game.title}</h2>
                    <span className="badge">
                      {selectedGame.game.store_source.toUpperCase()}
                    </span>
                    <span className="badge">{selectedGame.game.platform}</span>
                    {selectedGame.game.store_build_id && (
                      <span className="badge">
                        Build {selectedGame.game.store_build_id}
                      </span>
                    )}
                  </div>
                  <div
                    className="game-hero-path"
                    title={selectedGame.game.install_dir}
                  >
                    {selectedGame.game.install_dir}
                  </div>
                  <div className="row" style={{ marginTop: 8 }}>
                    {gameEnv && (
                      <span className="badge community">
                        引擎: {ENGINE_LABELS[gameEnv.engine]}
                      </span>
                    )}
                    {gameEnv?.installed_loaders.map((loader) => (
                      <span key={loader.entry_path} className="badge official">
                        加载器: {loader.kind}
                        {loader.detected_version
                          ? ` ${loader.detected_version}`
                          : ""}
                      </span>
                    ))}
                    {bridgeStatus?.deployed && (
                      <span className="status-pill bridge">
                        ● 伴生桥接已部署 ({bridgeStatus.active_files.length} 文件)
                      </span>
                    )}
                    {gameEnv?.anti_cheat_files_present.map((ac) => (
                      <span key={ac} className="badge unverified">
                        ⚠ 检测到反作弊: {ac}
                      </span>
                    ))}
                  </div>
                </div>

                {/* Primary Hero Controls: 一键注入 Mod / 解除注入 */}
                <div className="game-hero-actions">
                  <div className="hero-btn-row">
                    {!isSelectedGameInjected ? (
                      <button
                        className="btn-inject"
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
                        {busyAction === "inject"
                          ? "正在注入..."
                          : "⚡ 一键注入 Mod"}
                      </button>
                    ) : (
                      <button
                        className="btn-uninject"
                        disabled={busyAction === "uninject"}
                        onClick={() => void handleUninject()}
                      >
                        {busyAction === "uninject"
                          ? "正在回滚..."
                          : "⏏ 解除注入"}
                      </button>
                    )}

                    {bridgeAvailable &&
                      (bridgeStatus?.deployed ? (
                        <button
                          className="btn btn-danger"
                          disabled={busyAction === "undeploy-bridge"}
                          onClick={() => void handleUndeployBridge()}
                          title="按 DeploymentLedger 安全卸载游戏目录中的伴生桥接文件"
                        >
                          {busyAction === "undeploy-bridge"
                            ? "卸载中..."
                            : "🧹 解除桥接文件"}
                        </button>
                      ) : (
                        <button
                          className="btn btn-secondary"
                          disabled={
                            busyAction === "deploy-bridge" ||
                            hasAntiCheatMarkers ||
                            !bridgeSourceInput.trim() ||
                            !bridgeTargetInput.trim()
                          }
                          onClick={() => void handleDeployBridge()}
                          title="通过 DeploymentLedger 写入零 UnityExplorer 伴生桥接插件"
                        >
                          {busyAction === "deploy-bridge"
                            ? "部署中..."
                            : "📦 一键部署伴生桥接"}
                        </button>
                      ))}
                  </div>

                  <div className="row">
                    {isSelectedGameInjected && sessionDiag ? (
                      <>
                        <span className="status-pill live">
                          ● 已注入 {activeProcessLabel ?? sessionDiag.session_id} ·
                          补丁 {sessionDiag.active_patches} · 锁定{" "}
                          {sessionDiag.active_freeze_features}
                        </span>
                        <button
                          className="btn btn-secondary btn-sm"
                          onClick={() => void handleRefreshDiagnostics()}
                        >
                          刷新状态
                        </button>
                      </>
                    ) : activeSessionId && !isSelectedGameInjected ? (
                      <span className="status-pill">
                        另一款游戏正在注入中，请先解除当前会话
                        <button
                          className="btn btn-danger btn-sm"
                          style={{ marginLeft: 8 }}
                          onClick={() => void handleUninject()}
                        >
                          解除另一会话
                        </button>
                      </span>
                    ) : (
                      <span className="status-pill">
                        {selectedTrainer
                          ? `就绪 · 目标模组：${selectedTrainer.name} v${selectedTrainer.version}`
                          : "当前游戏暂无已安装模组"}
                      </span>
                    )}
                  </div>
                </div>
              </div>

              {/* Compact Process Selector Row inside Game Info Header */}
              {selectedGame.game.matched_game_id && (
                <div className="game-hero-bar">
                  <div className="row">
                    <span className="muted" style={{ fontWeight: 600 }}>
                      目标进程：
                    </span>
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
                        未发现运行中的游戏进程（启动游戏后点击扫描，或直接点一键注入自动扫描）
                      </span>
                    )}
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={() =>
                        void scanRunningProcesses(
                          selectedGame.game.matched_game_id!,
                        )
                      }
                      disabled={activeSessionId !== null}
                    >
                      扫描进程
                    </button>
                  </div>
                  <div className="row">
                    <span className="muted">
                      ID: <code>{selectedGame.game.matched_game_id}</code>
                    </span>
                  </div>
                </div>
              )}
            </header>

            {/* ---------- BOTTOM-RIGHT: 模组的功能 ---------- */}
            <section className="mod-workspace">
              {trainers.length === 0 ? (
                <div className="card stack">
                  <h3 style={{ margin: 0 }}>模组的功能</h3>
                  <p className="muted" style={{ margin: 0 }}>
                    当前游戏还没有安装本地模组。您可以展开下方「社区可用模组」一键安装，或切换到左侧底部「本地导入」导入本地模组包。
                  </p>
                </div>
              ) : (
                selectedTrainer && (
                  <>
                    {/* Mod Selector Tabs */}
                    <div className="row" style={{ justifyContent: "space-between" }}>
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
                              {isTabActive && (
                                <span
                                  className="game-status-dot injected"
                                  style={{ marginTop: 0 }}
                                />
                              )}
                              <span>{t.name}</span>
                              <span className="badge">v{t.version}</span>
                              {trustBadge(t.trust_tier)}
                            </button>
                          );
                        })}
                      </div>
                      <div className="row">
                        <span className="badge">
                          兼容性: {selectedTrainer.compatibility_status}
                        </span>
                        {selectedTrainer.revoked && (
                          <span className="badge unverified">已被社区源吊销</span>
                        )}
                      </div>
                    </div>

                    {/* Inline Permission Gate (Default-Deny preserved!) */}
                    {(selectedTrainer.permission_state.status ===
                      "requires_confirmation" ||
                      selectedTrainer.permission_state.status === "blocked" ||
                      awaitingPermInject === selectedTrainer.trainer_id) && (
                      <div className="permission-banner stack">
                        <div
                          className="row"
                          style={{ justifyContent: "space-between" }}
                        >
                          <strong>
                            🔒 注入前权限确认（Default-Deny 安全机制）
                          </strong>
                          {selectedTrainer.permission_state.status ===
                            "blocked" && (
                            <span className="badge unverified">
                              已阻止：{selectedTrainer.permission_state.reason}
                            </span>
                          )}
                        </div>
                        <div className="stack" style={{ gap: 6 }}>
                          {selectedTrainer.permissions.map((p) => (
                            <div key={p.scope} className="muted">
                              &bull; <strong>{PERMISSION_LABELS[p.scope]}</strong>{" "}
                              (<code>{p.scope}</code>)：{p.reason}
                            </div>
                          ))}
                        </div>
                        {selectedTrainer.permission_state.status ===
                          "requires_confirmation" && (
                          <div className="row">
                            <button
                              className="btn-inject"
                              style={{ padding: "8px 16px", fontSize: 13 }}
                              disabled={busyAction === "approve"}
                              onClick={() =>
                                void handleApprovePermissions(
                                  selectedTrainer,
                                  true,
                                )
                              }
                            >
                              ✓ 确认权限并立即注入
                            </button>
                            <button
                              className="btn btn-secondary"
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

                    {/* Feature Toolbar: Presets + Profile Save/Import */}
                    <div className="card stack">
                      <div
                        className="row"
                        style={{ justifyContent: "space-between" }}
                      >
                        <div>
                          <div className="row">
                            <h3 style={{ margin: 0 }}>
                              模组的功能 ({selectedTrainer.features.length})
                            </h3>
                            {findDuplicateHotkeys(
                              profilesByTrainer[selectedTrainer.trainer_id] ?? {},
                            ).length > 0 && (
                              <span className="badge unverified">
                                快捷键冲突：
                                {findDuplicateHotkeys(
                                  profilesByTrainer[selectedTrainer.trainer_id] ??
                                    {},
                                ).join(", ")}
                              </span>
                            )}
                          </div>
                          <div className="muted" style={{ marginTop: 4 }}>
                            {selectedTrainer.description}
                          </div>
                        </div>

                        <div className="row">
                          <button
                            className="btn btn-secondary btn-sm"
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
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              void handleApplyPreset(selectedTrainer, "No Grind")
                            }
                          >
                            No Grind
                          </button>
                          <button
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              void handleApplyPreset(
                                selectedTrainer,
                                "Reset Defaults",
                              )
                            }
                          >
                            重置默认
                          </button>
                          <button
                            className="btn btn-secondary btn-sm"
                            onClick={() => void handleSaveProfile(selectedTrainer)}
                          >
                            保存配置
                          </button>
                          <button
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              void handleExportProfile(selectedTrainer)
                            }
                          >
                            导入/导出 JSON
                          </button>
                        </div>
                      </div>

                      {!isSelectedTrainerInjected && (
                        <div className="banner" style={{ marginBottom: 0 }}>
                          当前尚未注入此模组。点击右上角「⚡ 一键注入 Mod」后即可实时开关或修改以下功能；注入后可随时点击「⏏ 解除注入」无痕回滚。
                        </div>
                      )}

                      {profileJsonTrainerId === selectedTrainer.trainer_id && (
                        <div
                          className="card stack"
                          style={{ background: "var(--surface-alt)" }}
                        >
                          <strong>
                            可移植配置 JSON ({selectedTrainer.trainer_id})
                          </strong>
                          <textarea
                            className="text-input"
                            rows={6}
                            value={profileJsonBox}
                            onChange={(e) => setProfileJsonBox(e.target.value)}
                          />
                          <div className="row">
                            <button
                              className="btn btn-sm"
                              onClick={() =>
                                void handleImportProfile(selectedTrainer)
                              }
                            >
                              导入 JSON 配置
                            </button>
                            <button
                              className="btn btn-secondary btn-sm"
                              onClick={() => setProfileJsonTrainerId(null)}
                            >
                              关闭
                            </button>
                          </div>
                        </div>
                      )}

                      {/* 2-Column Feature Control Grid */}
                      <div className="feature-grid">
                        {selectedTrainer.features.map((f) => {
                          const isOn =
                            isSelectedTrainerInjected &&
                            Boolean(activeToggles[f.id]);
                          return (
                            <div
                              key={f.id}
                              className={`feature-card ${isOn ? "enabled" : ""}`}
                            >
                              <div className="feature-card-top">
                                <div>
                                  <div className="row" style={{ gap: 6 }}>
                                    <strong>{f.name}</strong>
                                    <span className="badge">
                                      {f.control_type}
                                    </span>
                                  </div>
                                  <div
                                    className="muted"
                                    style={{ marginTop: 4 }}
                                  >
                                    {f.description}
                                  </div>
                                </div>
                              </div>

                              <div className="feature-card-controls">
                                <input
                                  className="search-input"
                                  style={{ width: 92, padding: "6px 8px" }}
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

                                <div className="row">
                                  {f.control_type === "toggle" && (
                                    <button
                                      className={
                                        isOn
                                          ? "btn btn-ok btn-sm"
                                          : "btn btn-secondary btn-sm"
                                      }
                                      disabled={!isSelectedTrainerInjected}
                                      onClick={() => void handleApplyFeature(f)}
                                    >
                                      {isOn ? "● 已开启（关闭）" : "开启"}
                                    </button>
                                  )}
                                  {f.control_type === "action" && (
                                    <button
                                      className="btn btn-secondary btn-sm"
                                      disabled={!isSelectedTrainerInjected}
                                      onClick={() => void handleApplyFeature(f)}
                                    >
                                      立即执行
                                    </button>
                                  )}
                                  {(f.control_type === "integer" ||
                                    f.control_type === "float") && (
                                    <>
                                      <input
                                        className="search-input"
                                        style={{
                                          width: 96,
                                          padding: "6px 8px",
                                        }}
                                        type="number"
                                        step={
                                          f.control_type === "integer"
                                            ? "1"
                                            : "any"
                                        }
                                        placeholder="数值"
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
                                        className="btn btn-sm"
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

              {/* Collapsible Drawers: Keep main feature canvas clean */}
              <details className="drawer">
                <summary>
                  <span>
                    🧩 伴生桥接部署配置 (DeploymentLedger ·{" "}
                    {bridgeStatus?.deployed ? "已部署" : "未部署"})
                  </span>
                  <span className="muted">展开 / 收起</span>
                </summary>
                <div className="drawer-body">
                  <div className="muted">
                    用于将无 UnityExplorer 依赖的 <code>NekodUnityBridge</code> /{" "}
                    <code>UE4SS</code> / <code>REFramework</code>{" "}
                    伴生插件安全部署到游戏子目录，并通过 SHA-256 账本记录，支持一键完整卸载还原。
                  </div>
                  <div className="row">
                    <input
                      className="text-input"
                      style={{ flex: 1 }}
                      placeholder="本地源插件文件路径（例如 plugins/unity-nekod-bridge/dist/mono/NekodUnityBridge.Mono.dll）"
                      value={bridgeSourceInput}
                      onChange={(e) => setBridgeSourceInput(e.target.value)}
                    />
                    <input
                      className="text-input"
                      style={{ flex: 1 }}
                      placeholder="游戏内目标相对路径（例如 BepInEx/plugins/NekodUnityBridge.Mono.dll）"
                      value={bridgeTargetInput}
                      onChange={(e) => setBridgeTargetInput(e.target.value)}
                    />
                  </div>
                  {bridgeStatus && bridgeStatus.active_files.length > 0 && (
                    <div className="muted">
                      当前账本跟踪的已部署文件：{" "}
                      {bridgeStatus.active_files.map((f) => (
                        <code key={f} style={{ marginRight: 8 }}>
                          {f}
                        </code>
                      ))}
                    </div>
                  )}
                  <div className="row">
                    <button
                      className="btn btn-sm"
                      disabled={
                        busyAction === "deploy-bridge" || hasAntiCheatMarkers
                      }
                      onClick={() => void handleDeployBridge()}
                    >
                      部署到游戏目录
                    </button>
                    {bridgeStatus?.deployed && (
                      <button
                        className="btn btn-danger btn-sm"
                        disabled={busyAction === "undeploy-bridge"}
                        onClick={() => void handleUndeployBridge()}
                      >
                        一键解除并清理账本文件
                      </button>
                    )}
                  </div>
                </div>
              </details>

              {selectedTrainer && (
                <details className="drawer">
                  <summary>
                    <span>
                      🔒 权限明细与撤销 ({selectedTrainer.permissions.length} 项权限)
                    </span>
                    <span className="muted">
                      {selectedTrainer.permission_state.status === "approved"
                        ? "已授权"
                        : selectedTrainer.permission_state.status}
                    </span>
                  </summary>
                  <div className="drawer-body">
                    {selectedTrainer.permissions.map((p) => (
                      <div key={p.scope} className="muted">
                        &bull; <strong>{PERMISSION_LABELS[p.scope]}</strong> (
                        <code>{p.scope}</code>)：{p.reason}
                      </div>
                    ))}
                    <div className="row">
                      {selectedTrainer.permission_state.status === "approved" &&
                        !activeSessionId && (
                          <button
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              void handleRevokePermissions(selectedTrainer)
                            }
                          >
                            撤销该模组授权
                          </button>
                        )}
                    </div>
                  </div>
                </details>
              )}

              {sessionDiag && sessionDiag.recent_events.length > 0 && (
                <details className="drawer" open={isSelectedGameInjected}>
                  <summary>
                    <span>
                      📜 实时注入会话事件 ({sessionDiag.session_id} ·{" "}
                      {sessionDiag.state})
                    </span>
                    <span className="muted">
                      冲突 {sessionDiag.rollback_conflicts} / 失败{" "}
                      {sessionDiag.rollback_failures}
                    </span>
                  </summary>
                  <div className="drawer-body">
                    {sessionDiag.recent_events.slice(-8).map((ev, idx) => (
                      <div key={idx} className="muted">
                        &bull; {ev}
                      </div>
                    ))}
                  </div>
                </details>
              )}

              {selectedGame.game.matched_game_id && (
                <details className="drawer">
                  <summary>
                    <span>📊 兼容矩阵 ({compatRecords.length})</span>
                    <span className="muted">展开 / 收起</span>
                  </summary>
                  <div className="drawer-body">
                    {compatRecords.length === 0 ? (
                      <div className="muted">
                        暂无已记录的版本兼容验证结果。
                      </div>
                    ) : (
                      compatRecords.map((rec) => (
                        <div key={rec.record_id} className="feature-row">
                          <div>
                            <div className="row">
                              <strong>{rec.mod_id}</strong>
                              <span className="badge">v{rec.mod_version}</span>
                              <span className="badge">{rec.runtime_adapter}</span>
                              <span className="badge">{rec.status}</span>
                            </div>
                            <div className="muted">
                              Build {rec.game_version} ({rec.store_platform}) &middot;{" "}
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
                    <span>
                      🌐 社区可用模组 ({communityTrainers.length})
                    </span>
                    <span className="muted">展开 / 收起</span>
                  </summary>
                  <div className="drawer-body">
                    <div className="row" style={{ justifyContent: "flex-end" }}>
                      <button
                        className="btn btn-secondary btn-sm"
                        disabled={busyAction === "sync"}
                        onClick={() => void handleSyncRegistries()}
                      >
                        {busyAction === "sync" ? "同步中..." : "同步社区源"}
                      </button>
                    </div>
                    {communityTrainers.length === 0 ? (
                      <div className="muted">
                        当前游戏在已同步社区源中暂无模组，可前往「社区源」添加仓库并同步。
                      </div>
                    ) : (
                      communityTrainers.map((ct) => (
                        <div
                          key={`${ct.repository_id}:${ct.trainer_id}`}
                          className="feature-row"
                        >
                          <div>
                            <div className="row">
                              <strong>{ct.trainer_id}</strong>
                              <span className="badge">v{ct.latest_version}</span>
                              {trustBadge(ct.trust_tier)}
                              {ct.compatibility_status && (
                                <span className="badge">
                                  {ct.compatibility_status}
                                </span>
                              )}
                            </div>
                            <div className="muted">
                              仓库：<code>{ct.repository_id}</code>
                              {ct.last_verified_game_version
                                ? ` · 验证版本：${ct.last_verified_game_version}`
                                : ""}
                            </div>
                          </div>
                          <div className="row">
                            {ct.install_state.state === "not_installed" && (
                              <button
                                className="btn btn-sm"
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
                                {busyAction === `install:${ct.trainer_id}`
                                  ? "安装中..."
                                  : "安装"}
                              </button>
                            )}
                            {ct.install_state.state ===
                              "installed_up_to_date" && (
                              <span className="badge official">已是最新</span>
                            )}
                            {ct.install_state.state === "update_available" && (
                              <button
                                className="btn btn-sm"
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
                              <span className="badge unverified">已吊销</span>
                            )}
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                </details>
              )}
            </section>
          </>
        )}

        {/* ==================== SECONDARY TABS ==================== */}
        {tab === "community" && (
          <div className="page-scroll">
            <div className="page-inner stack">
              <div className="row" style={{ justifyContent: "space-between" }}>
                <h2 style={{ margin: 0 }}>社区仓库与更新</h2>
                <button
                  className="btn"
                  disabled={busyAction === "sync"}
                  onClick={() => void handleSyncRegistries()}
                >
                  {busyAction === "sync" ? "同步中..." : "同步所有源"}
                </button>
              </div>

              <div className="card stack">
                <h3 style={{ margin: 0 }}>可用更新 ({updates.length})</h3>
                {updates.length === 0 ? (
                  <p className="muted" style={{ margin: 0 }}>
                    所有已安装模组均为最新版本。
                  </p>
                ) : (
                  updates.map((u) => (
                    <div
                      key={`${u.repository_id}:${u.trainer_id}`}
                      className="feature-row"
                    >
                      <div>
                        <div className="row">
                          <strong>{u.trainer_id}</strong>
                          <span className="badge">{u.game_id}</span>
                          {trustBadge(u.trust_tier)}
                        </div>
                        <div className="muted">
                          已安装 v{u.installed_version} &rarr; 可更新 v
                          {u.available_version} ({u.repository_id})
                        </div>
                      </div>
                      <button
                        className="btn btn-sm"
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

              <div className="card stack">
                <h3 style={{ margin: 0 }}>
                  已注册仓库 ({registrySources.length})
                </h3>
                {registrySources.length === 0 ? (
                  <p className="muted" style={{ margin: 0 }}>
                    尚未注册任何社区源。
                  </p>
                ) : (
                  registrySources.map((s) => (
                    <div key={s.source.repository_id} className="feature-row">
                      <div>
                        <div className="row">
                          <strong>{s.source.repository_id}</strong>
                          {trustBadge(s.source.authoritative_trust_tier)}
                          <span className="badge">{s.last_status}</span>
                        </div>
                        <div className="muted">{s.source.index_url}</div>
                        {s.etag && (
                          <div className="muted">
                            ETag: <code>{s.etag}</code>
                          </div>
                        )}
                        {s.last_error && (
                          <div className="muted" style={{ color: "#f87171" }}>
                            {s.last_error}
                          </div>
                        )}
                      </div>
                      <button
                        className="btn btn-secondary btn-sm"
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

              <div className="card stack">
                <h3 style={{ margin: 0 }}>添加第三方社区仓库</h3>
                <p className="muted" style={{ margin: 0 }}>
                  输入 HTTPS 协议的 <code>registry-manifest.json</code> 地址。所有第三方仓库默认归类为{" "}
                  <strong>Unverified</strong>，无法自我提升为 Official。
                </p>
                <div className="row">
                  <input
                    className="search-input"
                    style={{ width: 200 }}
                    placeholder="Repository ID (kebab-case)"
                    value={repoIdInput}
                    onChange={(e) => setRepoIdInput(e.target.value)}
                  />
                  <input
                    className="search-input"
                    style={{ flex: 1 }}
                    placeholder="https://example.org/nekod/registry-manifest.json"
                    value={repoUrlInput}
                    onChange={(e) => setRepoUrlInput(e.target.value)}
                  />
                  <button
                    className="btn"
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
            <div className="page-inner stack">
              <h2 style={{ margin: 0 }}>本地包与游戏清单导入</h2>
              <div className="card stack">
                <h3 style={{ margin: 0 }}>1. 注册 Game Manifest JSON</h3>
                <textarea
                  className="text-input"
                  rows={6}
                  placeholder="粘贴通过校验的 game-manifest.json..."
                  value={gameManifestJson}
                  onChange={(e) => setGameManifestJson(e.target.value)}
                />
                <div>
                  <button
                    className="btn"
                    onClick={() => void handleRegisterGameManifest()}
                  >
                    校验并注册游戏清单
                  </button>
                </div>
              </div>

              <div className="card stack">
                <h3 style={{ margin: 0 }}>2. 导入本地模组目录</h3>
                <p className="muted" style={{ margin: 0 }}>
                  指定包含 <code>trainer-manifest.json</code> 及对应构件的本地文件夹，导入前自动强制校验 JSON Schema 与 SHA-256。
                </p>
                <div className="row">
                  <input
                    className="text-input"
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
                    <option value="community_verified">
                      Community Verified
                    </option>
                    <option value="unverified">Unverified</option>
                  </select>
                  <button
                    className="btn"
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
            <div className="page-inner stack">
              <h2 style={{ margin: 0 }}>设置与安全策略</h2>
              <div className="card stack">
                <h3 style={{ margin: 0 }}>Default-Deny 安全控制</h3>
                <label className="row">
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
                  <span>允许未认证（Unverified）来源的声明式模组（附带风险提示）</span>
                </label>
                <label className="row">
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

              <div className="card stack">
                <h3 style={{ margin: 0 }}>自定义游戏目录</h3>
                <div className="row">
                  <input
                    className="text-input"
                    style={{ flex: 1 }}
                    placeholder="目录路径（如 D:\Games\HollowKnight）"
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
                    className="btn"
                    onClick={() => void handleAddCustomPath()}
                  >
                    添加目录
                  </button>
                </div>

                {customPaths.map((cp) => (
                  <div key={cp.path} className="feature-row">
                    <div>
                      <strong>{cp.label ?? cp.path}</strong>
                      <div className="muted">{cp.path}</div>
                    </div>
                    <button
                      className="btn btn-danger btn-sm"
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
            <div className="page-inner stack">
              <div className="header-row">
                <h2 style={{ margin: 0 }}>诊断与审计日志</h2>
                <div className="row">
                  <button
                    className="btn btn-secondary"
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
                    className="btn btn-secondary"
                    onClick={async () => {
                      try {
                        setLogs(await nekodApi.getDiagnosticLogs());
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  >
                    刷新日志
                  </button>
                </div>
              </div>
              {diagnosticBundle && (
                <div className="card stack">
                  <div
                    className="row"
                    style={{ justifyContent: "space-between" }}
                  >
                    <strong>Diagnostic Bundle (JSON)</strong>
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={() => setDiagnosticBundle(null)}
                    >
                      收起
                    </button>
                  </div>
                  <textarea
                    className="text-input"
                    rows={8}
                    readOnly
                    value={diagnosticBundle}
                  />
                </div>
              )}
              <div className="card">
                {logs.length === 0 ? (
                  <p className="muted" style={{ margin: 0 }}>
                    暂无诊断事件记录。
                  </p>
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
    </div>
  );
}
