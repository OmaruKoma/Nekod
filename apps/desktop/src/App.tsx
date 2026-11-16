import { useEffect, useMemo, useState } from "react";
import {
  CommunityTrainerView,
  CustomGamePathRecord,
  DiagnosticEvent,
  FeatureCommand,
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
  nekodApi,
} from "./api/nekod";

type NavTab = "library" | "community" | "import" | "settings" | "logs";

const PERMISSION_LABELS: Record<PermissionScope, string> = {
  "process.discover": "Locate running game process",
  "process.read_memory": "Read game memory values",
  "process.write_memory": "Modify game memory for active cheats",
  "process.allocate_memory": "Allocate memory pages in game process",
  "process.suspend_resume": "Briefly pause game threads during patches",
  "filesystem.read_save_dir": "Read game save folder",
  "filesystem.write_save_dir": "Modify files in game save folder",
  "runtime.native_module": "Load native runtime library (disabled by default)",
};

function trustBadge(tier: TrustTier) {
  if (tier === "official") return <span className="badge official">Official</span>;
  if (tier === "community_verified") {
    return <span className="badge community">Community Verified</span>;
  }
  return <span className="badge unverified">Unverified</span>;
}

export default function App() {
  const [tab, setTab] = useState<NavTab>("library");
  const [games, setGames] = useState<LibraryGameView[]>([]);
  const [search, setSearch] = useState("");
  const [selectedGame, setSelectedGame] = useState<LibraryGameView | null>(null);
  const [trainers, setTrainers] = useState<TrainerDetailView[]>([]);
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

  // P2 Runtime session state
  const [runningProcs, setRunningProcs] = useState<ProcessIdentity[]>([]);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [activeTrainerId, setActiveTrainerId] = useState<string | null>(null);
  const [sessionDiag, setSessionDiag] = useState<RuntimeDiagnostics | null>(null);
  const [activeToggles, setActiveToggles] = useState<Record<string, boolean>>({});
  const [featureInputs, setFeatureInputs] = useState<Record<string, string>>({});

  // P3 Community ecosystem & profiles state
  const [registrySources, setRegistrySources] = useState<RegistrySourceState[]>([]);
  const [communityTrainers, setCommunityTrainers] = useState<CommunityTrainerView[]>([]);
  const [updates, setUpdates] = useState<TrainerUpdateInfo[]>([]);
  const [profilesByTrainer, setProfilesByTrainer] = useState<
    Record<string, Record<string, string>>
  >({});
  const [repoIdInput, setRepoIdInput] = useState("");
  const [repoUrlInput, setRepoUrlInput] = useState("");
  const [busyAction, setBusyAction] = useState<string | null>(null);

  async function refreshAll() {
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
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void refreshAll();
  }, []);

  async function handleRescan() {
    try {
      setError(null);
      const lib = await nekodApi.rescanGames();
      setGames(lib);
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(`Scanned library: found ${lib.length} installed game(s).`);
    } catch (e) {
      setError(String(e));
    }
  }

  async function scanRunningProcesses(gameId: string) {
    try {
      const procs = await nekodApi.discoverRunningProcesses(gameId);
      setRunningProcs(procs);
      setSelectedPid((prev) =>
        procs.some((p) => p.pid === prev) ? prev : (procs[0]?.pid ?? null),
      );
    } catch (e) {
      setError(String(e));
    }
  }

  async function openGame(entry: LibraryGameView) {
    setSelectedGame(entry);
    setStatusMsg(null);
    setError(null);
    setRunningProcs([]);
    setSelectedPid(null);
    if (entry.game.matched_game_id) {
      const gameId = entry.game.matched_game_id;
      try {
        const [list, procs, community] = await Promise.all([
          nekodApi.listTrainersForGame(gameId, entry.game.store_build_id),
          nekodApi.discoverRunningProcesses(gameId),
          nekodApi.listCommunityTrainersForGame(gameId),
        ]);
        setTrainers(list);
        setRunningProcs(procs);
        setSelectedPid(procs[0]?.pid ?? null);
        setCommunityTrainers(community);

        const loadedProfiles: Record<string, Record<string, string>> = {};
        await Promise.all(
          list.map(async (trainer) => {
            const prof = await nekodApi.loadProfile(gameId, trainer.trainer_id);
            const map: Record<string, string> = {};
            for (const f of trainer.features) {
              const saved = prof?.hotkeys[f.id];
              map[f.id] = saved ?? f.default_hotkey ?? "";
            }
            loadedProfiles[trainer.trainer_id] = map;
          }),
        );
        setProfilesByTrainer(loadedProfiles);
      } catch (e) {
        setError(String(e));
      }
    } else {
      setTrainers([]);
      setCommunityTrainers([]);
    }
  }

  async function handleStartSession(trainer: TrainerDetailView) {
    const gameId = selectedGame?.game.matched_game_id;
    if (!gameId) return;
    const proc = runningProcs.find((p) => p.pid === selectedPid) ?? runningProcs[0];
    if (!proc) {
      setError("Launch the game first and scan for its running process.");
      return;
    }
    try {
      setError(null);
      const sessionId = await nekodApi.startTrainerSession(
        gameId,
        trainer.trainer_id,
        proc,
      );
      setActiveSessionId(sessionId);
      setActiveTrainerId(trainer.trainer_id);
      setActiveToggles({});
      setSessionDiag(await nekodApi.getSessionDiagnostics(sessionId));
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(`Attached '${trainer.name}' to ${proc.process_name} (PID ${proc.pid}).`);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleStopSession() {
    if (!activeSessionId) return;
    try {
      setError(null);
      const diag = await nekodApi.stopTrainerSession(activeSessionId);
      setSessionDiag(diag);
      setActiveSessionId(null);
      setActiveTrainerId(null);
      setActiveToggles({});
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(
        `Session stopped cleanly (conflicts: ${diag.rollback_conflicts}, failures: ${diag.rollback_failures}).`,
      );
    } catch (e) {
      setError(String(e));
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
        setError(`Enter a valid number for '${feature.name}'.`);
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

  async function handleApprovePermissions(trainer: TrainerDetailView) {
    try {
      setError(null);
      const scopes = trainer.permissions.map((p) => p.scope);
      await nekodApi.grantTrainerPermissions(trainer.trainer_id, scopes);
      if (selectedGame?.game.matched_game_id) {
        setTrainers(
          await nekodApi.listTrainersForGame(
            selectedGame.game.matched_game_id,
            selectedGame.game.store_build_id,
          ),
        );
      }
      setLogs(await nekodApi.getDiagnosticLogs());
      setStatusMsg(`Permissions granted for ${trainer.name} v${trainer.version}.`);
    } catch (e) {
      setError(String(e));
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
      setStatusMsg("Validated and installed local trainer package.");
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
      setStatusMsg(`Registered game manifest '${gameId}'.`);
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
        `Synced ${report.sources_checked} source(s): ${report.updated} updated, ${report.not_modified} unchanged, ${report.failed} failed, ${report.revoked_trainer_ids.length} revoked.`,
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
      setStatusMsg("Added third-party repository as Unverified. Click Sync All Feeds to fetch its index.");
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
      setStatusMsg(`Removed repository '${repositoryId}'.`);
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
      await refreshAll();
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
        `Verified SHA-256 package and installed '${trainerId}'. Review and confirm its permissions before attaching.`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAction(null);
    }
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
      };
      await nekodApi.saveProfile(profile);
      setStatusMsg(`Saved hotkey profile for '${trainer.name}'.`);
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

  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="brand">
          <h1>Nekod</h1>
          <p>Modding games, together.</p>
        </div>

        <button
          className={`nav-btn ${tab === "library" ? "active" : ""}`}
          onClick={() => {
            setTab("library");
            setSelectedGame(null);
          }}
        >
          Game Library
        </button>
        <button
          className={`nav-btn ${tab === "community" ? "active" : ""}`}
          onClick={() => setTab("community")}
        >
          Community &amp; Updates{updates.length > 0 ? ` (${updates.length})` : ""}
        </button>
        <button
          className={`nav-btn ${tab === "import" ? "active" : ""}`}
          onClick={() => setTab("import")}
        >
          Local Packages
        </button>
        <button
          className={`nav-btn ${tab === "settings" ? "active" : ""}`}
          onClick={() => setTab("settings")}
        >
          Settings &amp; Security
        </button>
        <button
          className={`nav-btn ${tab === "logs" ? "active" : ""}`}
          onClick={async () => {
            setTab("logs");
            try {
              setLogs(await nekodApi.getDiagnosticLogs());
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          Diagnostics
        </button>
      </aside>

      <main className="content">
        {error && <div className="error-banner">{error}</div>}
        {statusMsg && <div className="banner">{statusMsg}</div>}

        {tab === "library" && !selectedGame && (
          <>
            <div className="header-row">
              <div>
                <h2 style={{ margin: 0 }}>Game Library</h2>
                <p className="muted" style={{ margin: "4px 0 0" }}>
                  Automatically discovered Steam games and registered local folders.
                </p>
              </div>
              <div className="row">
                <input
                  className="search-input"
                  placeholder="Search games..."
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                />
                <button className="btn" onClick={handleRescan}>
                  Rescan Library
                </button>
              </div>
            </div>

            {filteredGames.length === 0 ? (
              <div className="card">
                <p style={{ margin: 0 }}>
                  No installed games matched yet. Ensure Steam is installed or add a custom game
                  folder in Settings.
                </p>
              </div>
            ) : (
              <div className="grid">
                {filteredGames.map((item) => (
                  <div
                    key={item.library_key}
                    className="card game-card"
                    onClick={() => void openGame(item)}
                  >
                    <div className="row" style={{ justifyContent: "space-between" }}>
                      <span className="badge">{item.game.store_source.toUpperCase()}</span>
                      <span className="badge">{item.trainer_count} trainer(s)</span>
                    </div>
                    <h3 style={{ margin: "10px 0 6px" }}>{item.game.title}</h3>
                    <div className="muted">
                      {item.game.matched_game_id
                        ? `ID: ${item.game.matched_game_id}`
                        : "No Game Manifest matched"}
                    </div>
                    {item.game.store_build_id && (
                      <div className="muted">Build: {item.game.store_build_id}</div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </>
        )}

        {tab === "library" && selectedGame && (
          <div className="stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <button className="btn btn-secondary" onClick={() => setSelectedGame(null)}>
                &larr; Back to Library
              </button>
              <span className="badge">{selectedGame.game.platform}</span>
            </div>

            <div className="card">
              <h2 style={{ margin: "0 0 6px" }}>{selectedGame.game.title}</h2>
              <div className="muted">Install folder: {selectedGame.game.install_dir}</div>
              {selectedGame.game.executable_path && (
                <div className="muted">Executable: {selectedGame.game.executable_path}</div>
              )}
              {selectedGame.game.store_build_id && (
                <div className="muted">Steam Build ID: {selectedGame.game.store_build_id}</div>
              )}
            </div>

            {selectedGame.game.matched_game_id && (
              <div className="card stack">
                <div className="row" style={{ justifyContent: "space-between" }}>
                  <div>
                    <strong>Running Game Process</strong>
                    <div className="muted">
                      {runningProcs.length === 0
                        ? "No running game process detected yet. Start the game and rescan."
                        : `${runningProcs.length} matching process(es) discovered.`}
                    </div>
                  </div>
                  <div className="row">
                    {runningProcs.length > 0 && (
                      <select
                        className="search-input"
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
                    )}
                    <button
                      className="btn btn-secondary"
                      onClick={() =>
                        void scanRunningProcesses(selectedGame.game.matched_game_id!)
                      }
                      disabled={activeSessionId !== null}
                    >
                      Scan Process
                    </button>
                  </div>
                </div>

                {sessionDiag && (
                  <div className="card" style={{ background: "var(--surface-alt)" }}>
                    <div className="row" style={{ justifyContent: "space-between" }}>
                      <div className="row">
                        <strong>Session {sessionDiag.session_id}</strong>
                        <span className="badge">{sessionDiag.state}</span>
                        <span className="badge">
                          Active patches: {sessionDiag.active_patches}
                        </span>
                        <span className="badge">
                          Freezes: {sessionDiag.active_freeze_features}
                        </span>
                        {sessionDiag.rollback_conflicts > 0 && (
                          <span className="badge unverified">
                            Conflicts: {sessionDiag.rollback_conflicts}
                          </span>
                        )}
                        {sessionDiag.rollback_failures > 0 && (
                          <span className="badge unverified">
                            Failures: {sessionDiag.rollback_failures}
                          </span>
                        )}
                      </div>
                      <div className="row">
                        {activeSessionId && (
                          <>
                            <button
                              className="btn btn-secondary"
                              onClick={() => void handleRefreshDiagnostics()}
                            >
                              Refresh Status
                            </button>
                            <button
                              className="btn"
                              onClick={() => void handleStopSession()}
                            >
                              Stop &amp; Rollback Session
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                    {sessionDiag.recent_events.length > 0 && (
                      <div className="stack" style={{ marginTop: 8 }}>
                        {sessionDiag.recent_events.slice(-5).map((ev, idx) => (
                          <div key={idx} className="muted">
                            &bull; {ev}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {trainers.length === 0 ? (
              <div className="card">
                <p className="muted" style={{ margin: 0 }}>
                  No local trainers installed for this game yet. Use &ldquo;Local Packages&rdquo;
                  to register a Game Manifest and ingest a validated trainer package.
                </p>
              </div>
            ) : (
              trainers.map((t) => {
                const isThisTrainerAttached =
                  activeSessionId !== null && activeTrainerId === t.trainer_id;

                return (
                  <div key={t.trainer_id} className="card stack">
                    <div className="row" style={{ justifyContent: "space-between" }}>
                      <div className="row">
                        <h3 style={{ margin: 0 }}>{t.name}</h3>
                        <span className="badge">v{t.version}</span>
                        {trustBadge(t.trust_tier)}
                        <span className="badge">{t.compatibility_status}</span>
                        {t.revoked && (
                          <span className="badge unverified">Revoked by Registry</span>
                        )}
                      </div>
                      <div className="row">
                        <button
                          className="btn btn-secondary"
                          onClick={() => void handleSaveProfile(t)}
                        >
                          Save Profile
                        </button>
                        {t.permission_state.status === "approved" && !activeSessionId && (
                          <button
                            className="btn"
                            disabled={runningProcs.length === 0}
                            onClick={() => void handleStartSession(t)}
                          >
                            Attach Trainer
                          </button>
                        )}
                        {isThisTrainerAttached && (
                          <button
                            className="btn btn-secondary"
                            onClick={() => void handleStopSession()}
                          >
                            Detach &amp; Rollback
                          </button>
                        )}
                        {t.permission_state.status === "approved" && !activeSessionId && (
                          <button
                            className="btn btn-secondary"
                            onClick={() => void handleRevokePermissions(t)}
                          >
                            Revoke Permissions
                          </button>
                        )}
                      </div>
                    </div>

                    <p className="muted" style={{ margin: 0 }}>
                      {t.description}
                    </p>

                    <div className="card" style={{ background: "var(--surface-alt)" }}>
                      <strong>Permission Review</strong>
                      <div className="stack" style={{ marginTop: 8 }}>
                        {t.permissions.map((p) => (
                          <div key={p.scope} className="muted">
                            &bull; <strong>{PERMISSION_LABELS[p.scope]}</strong> ({p.scope}):{" "}
                            {p.reason}
                          </div>
                        ))}
                      </div>
                      <div style={{ marginTop: 10 }}>
                        {t.permission_state.status === "approved" && (
                          <span className="badge official">Permissions Approved</span>
                        )}
                        {t.permission_state.status === "requires_confirmation" && (
                          <button
                            className="btn"
                            onClick={() => void handleApprovePermissions(t)}
                          >
                            Confirm &amp; Grant Required Permissions
                          </button>
                        )}
                        {t.permission_state.status === "blocked" && (
                          <span className="badge unverified">
                            Blocked: {t.permission_state.reason}
                          </span>
                        )}
                      </div>
                    </div>

                    <div className="stack">
                      <strong>Features ({t.features.length})</strong>
                      {t.features.map((f) => {
                        const isOn = Boolean(activeToggles[f.id]);
                        return (
                          <div key={f.id} className="feature-row">
                            <div>
                              <div>
                                <strong>{f.name}</strong>{" "}
                                <span className="badge">{f.control_type}</span>
                              </div>
                              <div className="muted">{f.description}</div>
                            </div>
                            <div className="row">
                              <input
                                className="search-input"
                                style={{ width: 96 }}
                                placeholder="Hotkey"
                                aria-label={`Hotkey for ${f.name}`}
                                value={
                                  profilesByTrainer[t.trainer_id]?.[f.id] ??
                                  f.default_hotkey ??
                                  ""
                                }
                                onChange={(e) =>
                                  setProfilesByTrainer((prev) => ({
                                    ...prev,
                                    [t.trainer_id]: {
                                      ...(prev[t.trainer_id] ?? {}),
                                      [f.id]: e.target.value,
                                    },
                                  }))
                                }
                              />
                              {f.control_type === "toggle" && (
                                <button
                                  className={isOn ? "btn" : "btn btn-secondary"}
                                  disabled={!isThisTrainerAttached}
                                  onClick={() => void handleApplyFeature(f)}
                                >
                                  {isOn ? "Active (Turn Off)" : "Enable"}
                                </button>
                              )}
                              {f.control_type === "action" && (
                                <button
                                  className="btn btn-secondary"
                                  disabled={!isThisTrainerAttached}
                                  onClick={() => void handleApplyFeature(f)}
                                >
                                  Trigger
                                </button>
                              )}
                              {(f.control_type === "integer" ||
                                f.control_type === "float") && (
                                <>
                                  <input
                                    className="search-input"
                                    style={{ width: 110 }}
                                    type="number"
                                    step={f.control_type === "integer" ? "1" : "any"}
                                    placeholder="Value"
                                    disabled={!isThisTrainerAttached}
                                    value={featureInputs[f.id] ?? ""}
                                    onChange={(e) =>
                                      setFeatureInputs((prev) => ({
                                        ...prev,
                                        [f.id]: e.target.value,
                                      }))
                                    }
                                  />
                                  <button
                                    className="btn"
                                    disabled={!isThisTrainerAttached}
                                    onClick={() => void handleApplyFeature(f)}
                                  >
                                    Apply
                                  </button>
                                </>
                              )}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })
            )}

            {selectedGame.game.matched_game_id && (
              <div className="card stack">
                <div className="row" style={{ justifyContent: "space-between" }}>
                  <h3 style={{ margin: 0 }}>
                    Available Community Trainers ({communityTrainers.length})
                  </h3>
                  <button
                    className="btn btn-secondary"
                    disabled={busyAction === "sync"}
                    onClick={() => void handleSyncRegistries()}
                  >
                    {busyAction === "sync" ? "Syncing..." : "Sync Feeds"}
                  </button>
                </div>
                {communityTrainers.length === 0 ? (
                  <p className="muted" style={{ margin: 0 }}>
                    No indexed community trainers found for this game yet. Sync your registry feeds
                    in &ldquo;Community &amp; Updates&rdquo;.
                  </p>
                ) : (
                  <div className="stack">
                    {communityTrainers.map((ct) => (
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
                              <span className="badge">{ct.compatibility_status}</span>
                            )}
                          </div>
                          <div className="muted">
                            Repo: <code>{ct.repository_id}</code>
                            {ct.last_verified_game_version
                              ? ` · Verified build: ${ct.last_verified_game_version}`
                              : ""}
                          </div>
                        </div>
                        <div className="row">
                          {ct.install_state.state === "not_installed" && (
                            <button
                              className="btn"
                              disabled={busyAction === `install:${ct.trainer_id}`}
                              onClick={() =>
                                void handleInstallOrUpdateCommunity(
                                  ct.repository_id,
                                  ct.trainer_id,
                                )
                              }
                            >
                              {busyAction === `install:${ct.trainer_id}`
                                ? "Installing..."
                                : "Install"}
                            </button>
                          )}
                          {ct.install_state.state === "installed_up_to_date" && (
                            <span className="badge official">Installed (Up to date)</span>
                          )}
                          {ct.install_state.state === "update_available" && (
                            <button
                              className="btn"
                              disabled={busyAction === `install:${ct.trainer_id}`}
                              onClick={() =>
                                void handleInstallOrUpdateCommunity(
                                  ct.repository_id,
                                  ct.trainer_id,
                                )
                              }
                            >
                              Update to v{ct.install_state.latest_version}
                            </button>
                          )}
                          {ct.install_state.state === "revoked" && (
                            <span className="badge unverified">Revoked</span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {tab === "community" && (
          <div className="stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <h2 style={{ margin: 0 }}>Community Repositories &amp; Updates</h2>
              <button
                className="btn"
                disabled={busyAction === "sync"}
                onClick={() => void handleSyncRegistries()}
              >
                {busyAction === "sync" ? "Syncing..." : "Sync All Feeds"}
              </button>
            </div>

            <div className="card stack">
              <h3 style={{ margin: 0 }}>
                Available Trainer Updates ({updates.length})
              </h3>
              {updates.length === 0 ? (
                <p className="muted" style={{ margin: 0 }}>
                  All installed trainers match the latest synced registry versions.
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
                        Installed v{u.installed_version} &rarr; Available v{u.available_version} (
                        {u.repository_id})
                      </div>
                    </div>
                    <button
                      className="btn"
                      disabled={busyAction === `install:${u.trainer_id}`}
                      onClick={() =>
                        void handleInstallOrUpdateCommunity(
                          u.repository_id,
                          u.trainer_id,
                        )
                      }
                    >
                      {busyAction === `install:${u.trainer_id}`
                        ? "Updating..."
                        : `Update to v${u.available_version}`}
                    </button>
                  </div>
                ))
              )}
            </div>

            <div className="card stack">
              <h3 style={{ margin: 0 }}>
                Registered Repositories ({registrySources.length})
              </h3>
              {registrySources.length === 0 ? (
                <p className="muted" style={{ margin: 0 }}>
                  No community registry sources registered yet.
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
                        <div className="muted">ETag: <code>{s.etag}</code></div>
                      )}
                      {s.last_error && (
                        <div className="muted" style={{ color: "#f87171" }}>
                          {s.last_error}
                        </div>
                      )}
                    </div>
                    <button
                      className="btn btn-secondary"
                      onClick={() => void handleRemoveRepo(s.source.repository_id)}
                    >
                      Remove
                    </button>
                  </div>
                ))
              )}
            </div>

            <div className="card stack">
              <h3 style={{ margin: 0 }}>Add Third-Party Repository</h3>
              <p className="muted" style={{ margin: 0 }}>
                Add an HTTPS registry index feed (<code>registry-manifest.json</code>). All
                third-party repositories enter as <strong>Unverified</strong> and cannot
                self-promote to <strong>Official</strong>.
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
                  Add Repository
                </button>
              </div>
            </div>
          </div>
        )}

        {tab === "import" && (
          <div className="stack">
            <h2 style={{ margin: 0 }}>Local Packages &amp; Game Manifests</h2>
            <div className="card stack">
              <h3 style={{ margin: 0 }}>1. Register Game Manifest JSON</h3>
              <textarea
                className="text-input"
                rows={6}
                placeholder="Paste a validated game-manifest.json..."
                value={gameManifestJson}
                onChange={(e) => setGameManifestJson(e.target.value)}
              />
              <div>
                <button className="btn" onClick={() => void handleRegisterGameManifest()}>
                  Validate &amp; Register Game Manifest
                </button>
              </div>
            </div>

            <div className="card stack">
              <h3 style={{ margin: 0 }}>2. Ingest Local Trainer Folder</h3>
              <p className="muted" style={{ margin: 0 }}>
                Select a folder containing <code>trainer-manifest.json</code> and its verified
                artifacts. SHA-256 checksums and JSON Schema checks are enforced before import.
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
                  onChange={(e) => setTrainerTrust(e.target.value as TrustTier)}
                >
                  <option value="official">Official</option>
                  <option value="community_verified">Community Verified</option>
                  <option value="unverified">Unverified</option>
                </select>
                <button className="btn" onClick={() => void handleIngestTrainer()}>
                  Verify &amp; Import
                </button>
              </div>
            </div>
          </div>
        )}

        {tab === "settings" && (
          <div className="stack">
            <h2 style={{ margin: 0 }}>Settings &amp; Security Policy</h2>
            <div className="card stack">
              <h3 style={{ margin: 0 }}>Default-Deny Security Controls</h3>
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
                <span>Allow unverified local/third-party declarative trainers (with warnings)</span>
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
                <span>
                  Allow third-party native runtime modules (high risk; disabled by default)
                </span>
              </label>
            </div>

            <div className="card stack">
              <h3 style={{ margin: 0 }}>Custom Game Folders</h3>
              <div className="row">
                <input
                  className="text-input"
                  style={{ flex: 1 }}
                  placeholder="Folder path (e.g. D:\Games\HollowKnight)"
                  value={customDir}
                  onChange={(e) => setCustomDir(e.target.value)}
                />
                <input
                  className="text-input"
                  placeholder="Optional label"
                  value={customLabel}
                  onChange={(e) => setCustomLabel(e.target.value)}
                />
                <button className="btn" onClick={() => void handleAddCustomPath()}>
                  Add Folder
                </button>
              </div>

              {customPaths.map((cp) => (
                <div key={cp.path} className="feature-row">
                  <div>
                    <strong>{cp.label ?? cp.path}</strong>
                    <div className="muted">{cp.path}</div>
                  </div>
                  <button
                    className="btn btn-danger"
                    onClick={async () => {
                      try {
                        setError(null);
                        setCustomPaths(await nekodApi.removeCustomGamePath(cp.path));
                        await handleRescan();
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  >
                    Remove
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === "logs" && (
          <div className="stack">
            <div className="header-row">
              <h2 style={{ margin: 0 }}>Diagnostics &amp; Audit Logs</h2>
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
                Refresh
              </button>
            </div>
            <div className="card">
              {logs.length === 0 ? (
                <p className="muted" style={{ margin: 0 }}>
                  No diagnostic events recorded yet.
                </p>
              ) : (
                logs.map((entry, idx) => (
                  <div key={idx} className="log-line">
                    [{entry.level.toUpperCase()}] ({entry.category}) {entry.message}
                  </div>
                ))
              )}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
