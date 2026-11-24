import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Download,
  FolderOpen,
  Globe,
  Keyboard,
  Layers,
  MoreHorizontal,
  Package,
  Play,
  Power,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Share,
  Shield,
  ShieldAlert,
  SlidersHorizontal,
  Sparkles,
  Star,
  Trash2,
  Volume2,
  VolumeX,
  X,
  Zap,
} from "lucide-react";
import type {
  BridgeDeploymentStatus,
  CommunityTrainerView,
  CompanionBridgeCatalogEntry,
  CompatibilityRecord,
  CustomGamePathRecord,
  GameArtwork,
  GameEnvironmentProfile,
  LibraryGameView,
  LoaderProgressEvent,
  LocalNmodCandidate,
  ProcessIdentity,
  TrainerDetailView,
  TrainerFeature,
} from "../api/nekod";
import { translatePermission, useI18n } from "../i18n";
import {
  ENGINE_LABELS,
  findDuplicateHotkeys,
  formatHotkeyFromKeyboardEvent,
  gameMonogram,
  inferInstallDirFromInput,
  normalizePathForCompare,
  trustLabel,
  trustTagClass,
  type ActiveTrainerSession,
} from "../types/ui";

type FeatureGroupFilter = "all" | "toggle" | "value" | "action";

interface GameWorkbenchProps {
  selectedGame: LibraryGameView;
  selectedArtwork: GameArtwork | null;
  trainers: TrainerDetailView[];
  selectedTrainer: TrainerDetailView | null;
  activeSessions: Record<string, ActiveTrainerSession>;
  activeSessionList: ActiveTrainerSession[];
  activeGameKey: string | null;
  hasAnyActiveSession: boolean;
  isSelectedGameInjected: boolean;
  isSelectedTrainerInjected: boolean;
  hasAntiCheatMarkers: boolean;
  favoriteKeys: Set<string>;
  customPaths: CustomGamePathRecord[];
  runningProcs: ProcessIdentity[];
  selectedPid: number | null;
  gameEnv: GameEnvironmentProfile | null;
  bridgeStatus: BridgeDeploymentStatus | null;
  bridgeCatalog: CompanionBridgeCatalogEntry[];
  compatRecords: CompatibilityRecord[];
  communityTrainers: CommunityTrainerView[];
  uninstalledLocalNmods: LocalNmodCandidate[];
  profilesByTrainer: Record<string, Record<string, string>>;
  featureInputsByTrainer: Record<string, Record<string, string>>;
  applyingFeatures: Set<string>;
  busyAction: string | null;
  copiedPath: boolean;
  autoAttachEnabled: boolean;
  hudOverlayEnabled: boolean;
  soundEnabled: boolean;
  globalHotkeyCount: number;
  waitingLaunchGameKey: string | null;
  awaitingPermInject: string | null;
  confirmRemoveCustomGame: boolean;
  confirmDeleteTrainerId: string | null;
  profileJsonTrainerId: string | null;
  profileJsonBox: string;
  bridgeSourceInput: string;
  bridgeTargetInput: string;
  loaderProgress: LoaderProgressEvent | null;
  crossModHotkeyConflicts: string[];
  onSelectTrainer: (trainerId: string) => void;
  onSelectPid: (pid: number) => void;
  onToggleFavorite: (gameKey: string) => void;
  onCopyInstallPath: (path: string) => void;
  onSetConfirmRemoveCustomGame: (val: boolean) => void;
  onRemoveCustomGameEntry: (path: string) => void;
  onPlayAndInject: () => void;
  onCancelWaitingLaunch: () => void;
  onOneClickInject: (trainer: TrainerDetailView) => void;
  onUninject: (trainerId?: string) => void;
  onAutoDeployBridge: () => void;
  onManualDeployBridge: () => void;
  onUndeployBridge: () => void;
  onInstallBepInExLoader: () => void;
  onUndeployBepInExLoader: () => void;
  onQuickSetupUnityStarter: () => void;
  onQuickSetupUe4ssStarter: () => void;
  onInstallLocalNmod: (cand: LocalNmodCandidate) => void;
  onApprovePermissions: (trainer: TrainerDetailView, andInject: boolean) => void;
  onRevokePermissions: (trainer: TrainerDetailView) => void;
  onUninstallTrainer: (trainer: TrainerDetailView) => void;
  onSetConfirmDeleteTrainerId: (id: string | null) => void;
  onExportNmod: (trainer: TrainerDetailView) => void;
  onApplyFeature: (feature: TrainerFeature) => void;
  onFeatureInputChange: (trainerId: string, featureId: string, val: string) => void;
  onHotkeyChange: (trainerId: string, featureId: string, hk: string) => void;
  onApplyPreset: (
    trainer: TrainerDetailView,
    preset: "Story Mode" | "No Grind" | "Reset Defaults",
  ) => void;
  onSaveProfile: (trainer: TrainerDetailView) => void;
  onExportProfile: (trainer: TrainerDetailView) => void;
  onImportProfile: (trainer: TrainerDetailView) => void;
  onSetProfileJsonTrainerId: (id: string | null) => void;
  onSetProfileJsonBox: (val: string) => void;
  onSetBridgeSourceInput: (val: string) => void;
  onSetBridgeTargetInput: (val: string) => void;
  onScanProcesses: () => void;
  onRefreshDiagnostics: () => void;
  onSyncRegistries: () => void;
  onInstallCommunityTrainer: (repoId: string, trainerId: string) => void;
  onToggleAutoAttach: () => void;
  onToggleHudOverlay: () => void;
  onToggleSound: () => void;
}

export function GameWorkbench({
  selectedGame,
  selectedArtwork,
  trainers,
  selectedTrainer,
  activeSessions,
  activeSessionList,
  hasAnyActiveSession,
  isSelectedGameInjected,
  isSelectedTrainerInjected,
  hasAntiCheatMarkers,
  favoriteKeys,
  customPaths,
  runningProcs,
  selectedPid,
  gameEnv,
  bridgeStatus,
  bridgeCatalog,
  compatRecords,
  communityTrainers,
  uninstalledLocalNmods,
  profilesByTrainer,
  featureInputsByTrainer,
  applyingFeatures,
  busyAction,
  copiedPath,
  autoAttachEnabled,
  hudOverlayEnabled,
  soundEnabled,
  globalHotkeyCount,
  waitingLaunchGameKey,
  awaitingPermInject,
  confirmRemoveCustomGame,
  confirmDeleteTrainerId,
  profileJsonTrainerId,
  profileJsonBox,
  bridgeSourceInput,
  bridgeTargetInput,
  loaderProgress,
  crossModHotkeyConflicts,
  onSelectTrainer,
  onSelectPid,
  onToggleFavorite,
  onCopyInstallPath,
  onSetConfirmRemoveCustomGame,
  onRemoveCustomGameEntry,
  onPlayAndInject,
  onCancelWaitingLaunch,
  onOneClickInject,
  onUninject,
  onAutoDeployBridge,
  onManualDeployBridge,
  onUndeployBridge,
  onInstallBepInExLoader,
  onUndeployBepInExLoader,
  onQuickSetupUnityStarter,
  onQuickSetupUe4ssStarter,
  onInstallLocalNmod,
  onApprovePermissions,
  onRevokePermissions,
  onUninstallTrainer,
  onSetConfirmDeleteTrainerId,
  onExportNmod,
  onApplyFeature,
  onFeatureInputChange,
  onHotkeyChange,
  onApplyPreset,
  onSaveProfile,
  onExportProfile,
  onImportProfile,
  onSetProfileJsonTrainerId,
  onSetProfileJsonBox,
  onSetBridgeSourceInput,
  onSetBridgeTargetInput,
  onScanProcesses,
  onRefreshDiagnostics,
  onSyncRegistries,
  onInstallCommunityTrainer,
  onToggleAutoAttach,
  onToggleHudOverlay,
  onToggleSound,
}: GameWorkbenchProps) {
  const { locale, m } = useI18n();
  const [featureQuery, setFeatureQuery] = useState("");
  const [featureFilter, setFeatureFilter] = useState<FeatureGroupFilter>("all");
  const [recordingFeatureKey, setRecordingFeatureKey] = useState<string | null>(
    null,
  );
  const [moreMenuOpen, setMoreMenuOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [descExpanded, setDescExpanded] = useState(false);
  const moreMenuRef = useRef<HTMLDivElement | null>(null);

  // Reset transient UI state when switching games or trainers
  useEffect(() => {
    setRecordingFeatureKey(null);
    setDescExpanded(false);
  }, [selectedTrainer?.trainer_id, selectedGame.library_key]);

  useEffect(() => {
    setMoreMenuOpen(false);
    setDetailsOpen(false);
  }, [selectedGame.library_key]);

  // Close More popover on outside click or Escape
  useEffect(() => {
    if (!moreMenuOpen) return;
    function onDocClick(e: MouseEvent) {
      if (
        moreMenuRef.current &&
        !moreMenuRef.current.contains(e.target as Node)
      ) {
        setMoreMenuOpen(false);
      }
    }
    function onDocKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setMoreMenuOpen(false);
      }
    }
    window.addEventListener("mousedown", onDocClick);
    window.addEventListener("keydown", onDocKey);
    return () => {
      window.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onDocKey);
    };
  }, [moreMenuOpen]);

  // Global key capture while recording a feature's custom hotkey pill
  useEffect(() => {
    if (!recordingFeatureKey || !selectedTrainer) return;
    const [recTrainerId, recFeatureId] = recordingFeatureKey.split("::");
    if (!recTrainerId || !recFeatureId) return;

    function handleCaptureKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecordingFeatureKey(null);
        return;
      }
      if (e.key === "Backspace" || e.key === "Delete") {
        onHotkeyChange(recTrainerId, recFeatureId, "");
        setRecordingFeatureKey(null);
        return;
      }
      const combo = formatHotkeyFromKeyboardEvent(e);
      if (!combo) return;
      onHotkeyChange(recTrainerId, recFeatureId, combo);
      setRecordingFeatureKey(null);
    }

    function handleBlurOrClickOutside() {
      setRecordingFeatureKey(null);
    }

    window.addEventListener("keydown", handleCaptureKeyDown, true);
    window.addEventListener("mousedown", handleBlurOrClickOutside, true);
    return () => {
      window.removeEventListener("keydown", handleCaptureKeyDown, true);
      window.removeEventListener("mousedown", handleBlurOrClickOutside, true);
    };
  }, [recordingFeatureKey, selectedTrainer, onHotkeyChange]);

  const selectedTrainerSession =
    selectedTrainer && isSelectedGameInjected
      ? (activeSessions[selectedTrainer.trainer_id] ?? null)
      : null;

  const sessionDiag =
    selectedTrainerSession?.diag ??
    (isSelectedGameInjected ? (activeSessionList[0]?.diag ?? null) : null);

  const activeToggles = selectedTrainerSession?.toggles ?? {};
  const featureInputs = selectedTrainer
    ? (featureInputsByTrainer[selectedTrainer.trainer_id] ?? {})
    : {};

  const bridgeAvailable = Boolean(
    bridgeStatus &&
      (bridgeStatus.deployed ||
        bridgeStatus.matched_bridge ||
        bridgeStatus.suggested_target_relative_path ||
        bridgeStatus.engine !== "native_unknown"),
  );

  const canLaunchSelectedGame = Boolean(
    (selectedGame.game.store_source === "steam" &&
      selectedGame.game.store_app_id) ||
      selectedGame.game.executable_path,
  );

  const coverUri =
    selectedArtwork?.thumbnail_data_uri ?? selectedArtwork?.header_data_uri ?? null;

  const compactCapsuleSummary = useMemo(() => {
    const parts: string[] = [selectedGame.game.store_source.toUpperCase()];
    if (gameEnv) {
      parts.push(ENGINE_LABELS[gameEnv.engine]);
    } else {
      parts.push(selectedGame.game.platform);
    }
    if (bridgeStatus?.deployed) {
      parts.push(m.workbench.bridgeDeployedBadge(bridgeStatus.active_files.length));
    }
    return parts.join(" · ");
  }, [selectedGame, gameEnv, bridgeStatus, m]);

  const mergedMetaItems = useMemo(() => {
    const items: { text: string; ok?: boolean }[] = [
      { text: selectedGame.game.store_source.toUpperCase() },
      { text: selectedGame.game.platform },
    ];
    if (selectedGame.game.store_build_id) {
      items.push({ text: `Build ${selectedGame.game.store_build_id}` });
    }
    if (gameEnv) {
      items.push({ text: ENGINE_LABELS[gameEnv.engine] });
      if (gameEnv.installed_loaders.length > 0) {
        const loadersText = gameEnv.installed_loaders
          .map((l) =>
            l.detected_version ? `${l.kind} ${l.detected_version}` : l.kind,
          )
          .join(", ");
        items.push({ text: loadersText });
      }
    }
    if (bridgeStatus?.deployed) {
      items.push({
        text: m.workbench.bridgeDeployedBadge(bridgeStatus.active_files.length),
        ok: true,
      });
    }
    return items;
  }, [selectedGame, gameEnv, bridgeStatus, m]);

  const filteredFeatures = useMemo(() => {
    if (!selectedTrainer) return [];
    const q = featureQuery.trim().toLowerCase();
    return selectedTrainer.features.filter((f) => {
      if (featureFilter === "toggle" && f.control_type !== "toggle") return false;
      if (featureFilter === "action" && f.control_type !== "action") return false;
      if (
        featureFilter === "value" &&
        f.control_type !== "integer" &&
        f.control_type !== "float"
      ) {
        return false;
      }
      if (!q) return true;
      return (
        f.name.toLowerCase().includes(q) ||
        f.description.toLowerCase().includes(q) ||
        f.category.toLowerCase().includes(q) ||
        f.id.toLowerCase().includes(q)
      );
    });
  }, [selectedTrainer, featureQuery, featureFilter]);

  const featureCounts = useMemo(() => {
    const list = selectedTrainer?.features ?? [];
    return {
      all: list.length,
      toggle: list.filter((f) => f.control_type === "toggle").length,
      value: list.filter(
        (f) => f.control_type === "integer" || f.control_type === "float",
      ).length,
      action: list.filter((f) => f.control_type === "action").length,
    };
  }, [selectedTrainer]);

  const matchedCustomEntry = useMemo(() => {
    if (selectedGame.game.store_source !== "custom_path") return null;
    return (
      customPaths.find(
        (cp) =>
          normalizePathForCompare(inferInstallDirFromInput(cp.path)) ===
            normalizePathForCompare(selectedGame.game.install_dir) ||
          normalizePathForCompare(cp.path) ===
            normalizePathForCompare(selectedGame.game.install_dir),
      ) ?? null
    );
  }, [selectedGame, customPaths]);

  const procDotClass = isSelectedGameInjected
    ? "status-dot ok"
    : runningProcs.length > 0
      ? "status-dot"
      : "status-dot idle";

  return (
    <div className="workbench-main">
      {/* ---------- Ultra-Compact 2-Row Game Header ---------- */}
      <header className="game-hero">
        {/* Row 1: Game Identity + Primary & Secondary CTA + More Popover */}
        <div className="game-hero-row1">
          <div className="game-hero-identity">
            <div className="hero-cover-badge" aria-hidden="true">
              {coverUri ? (
                <img
                  className="hero-cover-img"
                  src={coverUri}
                  alt={selectedGame.game.title}
                />
              ) : (
                <span className="hero-cover-mono">
                  {gameMonogram(selectedGame.game.title)}
                </span>
              )}
            </div>
            <h2 className="game-hero-title" title={selectedGame.game.title}>
              {selectedGame.game.title}
            </h2>
            <button
              type="button"
              className={`fav-btn ${
                favoriteKeys.has(selectedGame.library_key) ? "starred" : ""
              }`}
              onClick={() => onToggleFavorite(selectedGame.library_key)}
              title={
                favoriteKeys.has(selectedGame.library_key)
                  ? m.workbench.unfavorite
                  : m.workbench.favorite
              }
              aria-label={m.workbench.favorite}
            >
              <Star
                size={14}
                strokeWidth={1.5}
                fill={
                  favoriteKeys.has(selectedGame.library_key)
                    ? "currentColor"
                    : "none"
                }
              />
            </button>
          </div>

          <div className="hero-btn-row">
            {!isSelectedGameInjected ? (
              canLaunchSelectedGame ? (
                <>
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={
                      busyAction === "launch" ||
                      hasAntiCheatMarkers ||
                      (hasAnyActiveSession && !isSelectedGameInjected)
                    }
                    onClick={onPlayAndInject}
                    title={m.workbench.playAndInjectTitle}
                  >
                    <Play size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>
                      {busyAction === "launch"
                        ? m.workbench.launching
                        : m.workbench.playAndInject}
                    </span>
                  </button>

                  <button
                    type="button"
                    className="btn btn-outline btn-sm"
                    disabled={
                      !selectedTrainer ||
                      hasAntiCheatMarkers ||
                      busyAction === "inject" ||
                      (hasAnyActiveSession && !isSelectedGameInjected)
                    }
                    onClick={() =>
                      selectedTrainer && onOneClickInject(selectedTrainer)
                    }
                    title={m.workbench.oneClickInjectTitle}
                  >
                    <Zap size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>
                      {busyAction === "inject"
                        ? m.workbench.injecting
                        : m.workbench.oneClickInject}
                    </span>
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  disabled={
                    !selectedTrainer ||
                    hasAntiCheatMarkers ||
                    busyAction === "inject" ||
                    (hasAnyActiveSession && !isSelectedGameInjected)
                  }
                  onClick={() =>
                    selectedTrainer && onOneClickInject(selectedTrainer)
                  }
                  title={m.workbench.oneClickInjectTitle}
                >
                  <Zap size={13} strokeWidth={1.5} aria-hidden="true" />
                  <span>
                    {busyAction === "inject"
                      ? m.workbench.injecting
                      : m.workbench.oneClickInject}
                  </span>
                </button>
              )
            ) : (
              <>
                {selectedTrainer && !isSelectedTrainerInjected ? (
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={
                      hasAntiCheatMarkers ||
                      busyAction === "inject" ||
                      busyAction === "uninject"
                    }
                    onClick={() => onOneClickInject(selectedTrainer)}
                    title={m.workbench.stackInjectTitle}
                  >
                    <Zap size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>
                      {busyAction === "inject"
                        ? m.workbench.injecting
                        : m.workbench.stackInject}
                    </span>
                  </button>
                ) : (
                  selectedTrainer &&
                  isSelectedTrainerInjected && (
                    <button
                      type="button"
                      className="btn btn-danger-outline btn-sm"
                      disabled={busyAction === "uninject"}
                      onClick={() => onUninject(selectedTrainer.trainer_id)}
                      title={m.workbench.uninjectCurrentTitle}
                    >
                      <Power size={13} strokeWidth={1.5} aria-hidden="true" />
                      <span>
                        {busyAction === "uninject"
                          ? m.workbench.uninjecting
                          : activeSessionList.length > 1
                            ? m.workbench.uninjectCurrent
                            : m.workbench.uninjectSingle}
                      </span>
                    </button>
                  )
                )}
                {(activeSessionList.length > 1 || !selectedTrainer) && (
                  <button
                    type="button"
                    className="btn btn-danger-outline btn-sm"
                    disabled={busyAction === "uninject"}
                    onClick={() => onUninject()}
                    title={m.workbench.uninjectAllTitle}
                  >
                    <Power size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>
                      {busyAction === "uninject"
                        ? m.workbench.uninjecting
                        : m.workbench.uninjectAll(activeSessionList.length)}
                    </span>
                  </button>
                )}
              </>
            )}

            {/* More / Secondary Actions Popover */}
            <div className="hero-more-wrap" ref={moreMenuRef}>
              <button
                type="button"
                className={`icon-btn more-menu-trigger ${moreMenuOpen ? "active" : ""}`}
                onClick={() => setMoreMenuOpen((v) => !v)}
                title={m.workbench.moreOptionsTitle}
                aria-label={m.workbench.moreOptionsTitle}
                aria-expanded={moreMenuOpen}
              >
                <MoreHorizontal size={15} strokeWidth={1.5} />
                {(bridgeStatus?.bridge_update_available ||
                  autoAttachEnabled ||
                  hudOverlayEnabled) && <span className="more-dot" />}
              </button>

              {moreMenuOpen && (
                <div className="hero-more-popover" role="menu">
                  {bridgeAvailable && (
                    <>
                      <div className="popover-section-label">
                        {m.workbench.menuSectionBridge}
                      </div>
                      {bridgeStatus?.deployed ? (
                        <>
                          {bridgeStatus.bridge_update_available && (
                            <button
                              type="button"
                              className="popover-item accent"
                              disabled={
                                busyAction === "deploy-bridge" ||
                                isSelectedGameInjected ||
                                hasAntiCheatMarkers
                              }
                              onClick={() => {
                                setMoreMenuOpen(false);
                                onAutoDeployBridge();
                              }}
                              title={
                                isSelectedGameInjected
                                  ? m.workbench.updateBridgeInjectedTitle
                                  : m.workbench.updateBridgeReadyTitle
                              }
                            >
                              <Download size={14} strokeWidth={1.5} />
                              <span>
                                {busyAction === "deploy-bridge"
                                  ? m.workbench.updatingBridge
                                  : m.workbench.updateBridge}
                              </span>
                            </button>
                          )}
                          <button
                            type="button"
                            className="popover-item danger"
                            disabled={busyAction === "undeploy-bridge"}
                            onClick={() => {
                              setMoreMenuOpen(false);
                              onUndeployBridge();
                            }}
                          >
                            <Trash2 size={14} strokeWidth={1.5} />
                            <span>
                              {busyAction === "undeploy-bridge"
                                ? m.workbench.uninstallingBridge
                                : m.workbench.undeployBridge}
                            </span>
                          </button>
                        </>
                      ) : (
                        <button
                          type="button"
                          className="popover-item"
                          disabled={
                            busyAction === "deploy-bridge" ||
                            hasAntiCheatMarkers
                          }
                          onClick={() => {
                            setMoreMenuOpen(false);
                            onAutoDeployBridge();
                          }}
                        >
                          <Package size={14} strokeWidth={1.5} />
                          <span>
                            {busyAction === "deploy-bridge"
                              ? m.workbench.deployingBridge
                              : m.workbench.deployBridge}
                          </span>
                        </button>
                      )}
                      <div className="popover-divider" />
                    </>
                  )}

                  <div className="popover-section-label">
                    {m.workbench.menuSectionToggles}
                  </div>
                  <button
                    type="button"
                    className={`popover-item ${autoAttachEnabled ? "on" : ""}`}
                    onClick={onToggleAutoAttach}
                  >
                    <RefreshCw size={14} strokeWidth={1.5} />
                    <span>{m.workbench.autoInject}</span>
                    {autoAttachEnabled && (
                      <Check size={13} strokeWidth={1.5} className="popover-check" />
                    )}
                  </button>
                  <button
                    type="button"
                    className={`popover-item ${hudOverlayEnabled ? "on" : ""}`}
                    onClick={onToggleHudOverlay}
                  >
                    <Sparkles size={14} strokeWidth={1.5} />
                    <span>{m.workbench.hudOverlay}</span>
                    {hudOverlayEnabled && (
                      <Check size={13} strokeWidth={1.5} className="popover-check" />
                    )}
                  </button>
                  <button
                    type="button"
                    className={`popover-item ${soundEnabled ? "on" : ""}`}
                    onClick={onToggleSound}
                  >
                    {soundEnabled ? (
                      <Volume2 size={14} strokeWidth={1.5} />
                    ) : (
                      <VolumeX size={14} strokeWidth={1.5} />
                    )}
                    <span>{m.workbench.sound}</span>
                    {soundEnabled && (
                      <Check size={13} strokeWidth={1.5} className="popover-check" />
                    )}
                  </button>

                  {matchedCustomEntry && (
                    <>
                      <div className="popover-divider" />
                      <div className="popover-section-label">
                        {m.workbench.menuSectionGame}
                      </div>
                      {confirmRemoveCustomGame ? (
                        <div className="row" style={{ gap: 6, padding: "4px 6px" }}>
                          <button
                            type="button"
                            className="btn btn-danger-outline btn-sm"
                            style={{ flex: 1 }}
                            onClick={() => {
                              setMoreMenuOpen(false);
                              onRemoveCustomGameEntry(matchedCustomEntry.path);
                            }}
                          >
                            <Trash2 size={13} strokeWidth={1.5} />
                            <span>{m.workbench.confirmRemove}</span>
                          </button>
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => onSetConfirmRemoveCustomGame(false)}
                          >
                            {m.workbench.cancel}
                          </button>
                        </div>
                      ) : (
                        <button
                          type="button"
                          className="popover-item danger"
                          disabled={isSelectedGameInjected}
                          title={
                            isSelectedGameInjected
                              ? m.workbench.cannotRemoveWhileInjected
                              : m.workbench.removeCustomTitle
                          }
                          onClick={() => onSetConfirmRemoveCustomGame(true)}
                        >
                          <Trash2 size={14} strokeWidth={1.5} />
                          <span>{m.workbench.removeCustomBtn}</span>
                        </button>
                      )}
                    </>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Row 2: Merged Process Status + Borderless Env/Path Status Capsule */}
        <div className="game-hero-row2">
          <div className="proc-bar-left">
            <span className={procDotClass} />
            {runningProcs.length > 0 ? (
              <select
                className="select-input compact-select"
                value={selectedPid ?? ""}
                onChange={(e) => onSelectPid(Number(e.target.value))}
                disabled={hasAnyActiveSession}
              >
                {runningProcs.map((p) => (
                  <option key={p.pid} value={p.pid}>
                    {p.process_name} (PID {p.pid}, {p.architecture})
                  </option>
                ))}
              </select>
            ) : (
              <span className="muted">{m.workbench.idleHintShort}</span>
            )}
            <button
              type="button"
              className="icon-btn"
              onClick={onScanProcesses}
              disabled={hasAnyActiveSession}
              title={m.workbench.scanProcessesTitle}
              aria-label={m.workbench.scanProcessesTitle}
            >
              <RefreshCw size={13} strokeWidth={1.5} />
            </button>

            {waitingLaunchGameKey === selectedGame.library_key &&
              !isSelectedGameInjected && (
                <span className="row" style={{ gap: 5 }}>
                  <span className="muted">· {m.workbench.waitingProcessHint}</span>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={onCancelWaitingLaunch}
                  >
                    {m.workbench.cancel}
                  </button>
                </span>
              )}

            {isSelectedGameInjected && sessionDiag ? (
              <span className="proc-status-inline">
                {activeSessionList.length > 1 && (
                  <>
                    <span className="meta-sep">·</span>
                    <span>{activeSessionList.length} mods</span>
                  </>
                )}
                <span className="meta-sep">·</span>
                <span>{m.workbench.patchesCount(sessionDiag.active_patches)}</span>
                <span className="meta-sep">·</span>
                <span>
                  {m.workbench.freezesCount(sessionDiag.active_freeze_features)}
                </span>
                {globalHotkeyCount > 0 && (
                  <>
                    <span className="meta-sep">·</span>
                    <span style={{ color: "var(--brand)" }}>
                      {m.workbench.hotkeysCount(globalHotkeyCount)}
                    </span>
                  </>
                )}
                <button
                  type="button"
                  className="icon-btn"
                  onClick={onRefreshDiagnostics}
                  title={m.workbench.refreshDiagTitle}
                  aria-label={m.workbench.refreshDiagTitle}
                >
                  <RefreshCw size={12} strokeWidth={1.5} />
                </button>
              </span>
            ) : (
              hasAnyActiveSession &&
              !isSelectedGameInjected && (
                <span className="row" style={{ gap: 6 }}>
                  <span className="muted">· {m.workbench.otherGameRunning}</span>
                  <button
                    type="button"
                    className="btn btn-danger-outline btn-sm"
                    onClick={() => onUninject()}
                  >
                    {m.workbench.detachOtherSession}
                  </button>
                </span>
              )
            )}
          </div>

          <button
            type="button"
            className={`status-capsule ${detailsOpen ? "open" : ""} ${
              hasAntiCheatMarkers ? "warn" : ""
            }`}
            onClick={() => setDetailsOpen((v) => !v)}
            title={`${selectedGame.game.install_dir}\n${m.workbench.detailsToggleTitle}`}
          >
            {hasAntiCheatMarkers && (
              <AlertTriangle size={12} strokeWidth={1.5} />
            )}
            <span className="mono">{compactCapsuleSummary}</span>
            {detailsOpen ? (
              <ChevronDown size={13} strokeWidth={1.5} />
            ) : (
              <ChevronRight size={13} strokeWidth={1.5} />
            )}
          </button>
        </div>

        {/* Expandable Environment & Path Detail Strip (collapsed by default) */}
        {detailsOpen && (
          <div className="hero-details-strip">
            <div className="meta-line">
              {mergedMetaItems.map((item, idx) => (
                <span key={`${item.text}-${idx}`} className="row" style={{ gap: 6 }}>
                  {idx > 0 && <span className="meta-sep">·</span>}
                  <span className={item.ok ? "meta-ok" : undefined}>
                    {item.text}
                  </span>
                </span>
              ))}
              {hasAntiCheatMarkers && (
                <>
                  <span className="meta-sep">·</span>
                  <span className="meta-warn">
                    <AlertTriangle size={13} strokeWidth={1.5} />
                    <span>
                      {m.workbench.antiCheatPrefix}:{" "}
                      {gameEnv?.anti_cheat_files_present.join(", ")}
                    </span>
                  </span>
                </>
              )}
            </div>

            <button
              type="button"
              className="path-copy-btn"
              onClick={() => onCopyInstallPath(selectedGame.game.install_dir)}
              title={m.workbench.copyPathTitle}
            >
              <FolderOpen size={13} strokeWidth={1.5} aria-hidden="true" />
              <span className="path-copy-text">
                {selectedGame.game.install_dir}
              </span>
              {copiedPath ? (
                <Check
                  size={13}
                  strokeWidth={1.5}
                  className="path-copy-icon"
                  style={{ color: "var(--ok)", opacity: 1 }}
                />
              ) : (
                <Copy size={13} strokeWidth={1.5} className="path-copy-icon" />
              )}
            </button>
          </div>
        )}
      </header>

      {/* ---------- Bottom Mod Workspace ---------- */}
      <section className="mod-workspace">
        {bridgeStatus &&
          !bridgeStatus.loader_installed &&
          bridgeStatus.loader_warning &&
          !hasAntiCheatMarkers && (
            <div className="permission-banner stack">
              <div className="row" style={{ justifyContent: "space-between" }}>
                <div className="row" style={{ gap: 6 }}>
                  <AlertTriangle
                    size={15}
                    strokeWidth={1.5}
                    style={{ color: "var(--warn)" }}
                  />
                  <strong>
                    {bridgeStatus.engine === "unreal_engine"
                      ? m.workbench.missingUnrealLoader
                      : m.workbench.missingUnityLoader}
                  </strong>
                </div>
                <button
                  type="button"
                  className="btn btn-outline btn-sm"
                  disabled={busyAction === "install-loader"}
                  onClick={onInstallBepInExLoader}
                >
                  <Download size={14} strokeWidth={1.5} />
                  <span>
                    {busyAction === "install-loader"
                      ? m.workbench.installingLoader
                      : m.workbench.installLoaderBtn(
                          bridgeStatus.loader_display_name ??
                            (bridgeStatus.engine === "unreal_engine"
                              ? "RE-UE4SS"
                              : "BepInEx"),
                        )}
                  </span>
                </button>
              </div>
              <div className="muted">
                {busyAction === "install-loader" && loaderProgress ? (
                  <div className="stack" style={{ gap: 6 }}>
                    <span className="mono" style={{ color: "var(--text-primary)" }}>
                      {loaderProgress.message}
                    </span>
                    <div className="progress-track">
                      <div
                        className="progress-fill"
                        style={{
                          width: `${
                            loaderProgress.total > 0
                              ? Math.min(
                                  100,
                                  Math.round(
                                    (loaderProgress.current * 100) /
                                      loaderProgress.total,
                                  ),
                                )
                              : 28
                          }%`,
                        }}
                      />
                    </div>
                  </div>
                ) : (
                  bridgeStatus.loader_warning
                )}
              </div>
            </div>
          )}

        {uninstalledLocalNmods.length > 0 && (
          <div className="surface-panel stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <div className="row" style={{ gap: 6 }}>
                <Package size={15} strokeWidth={1.5} />
                <span style={{ fontWeight: 600 }}>
                  {m.workbench.localNmodFound}
                </span>
                <span className="muted mono">
                  ({uninstalledLocalNmods.length})
                </span>
              </div>
              <span className="tag">nmod_v1</span>
            </div>
            {uninstalledLocalNmods.map((cand) => {
              const isUpgrade = trainers.some(
                (t) => t.trainer_id === cand.mod_id,
              );
              return (
                <div key={`${cand.mod_id}:${cand.version}`} className="flat-row">
                  <div style={{ minWidth: 0 }}>
                    <div className="row">
                      <span style={{ fontWeight: 500 }}>{cand.name}</span>
                      <span className="mono muted">v{cand.version}</span>
                      <span className="tag">{cand.mod_kind}</span>
                      <span className="tag official">
                        {m.workbench.featuresCount(cand.feature_count)}
                      </span>
                      <span className="tag">{cand.runtime_kind}</span>
                    </div>
                    <div className="muted" style={{ marginTop: 2 }}>
                      {cand.description}
                    </div>
                    <div className="muted mono" style={{ marginTop: 2 }}>
                      {cand.mod_id} · {cand.source_format} · {cand.source_path}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={busyAction === `nmod:${cand.mod_id}`}
                    onClick={() => onInstallLocalNmod(cand)}
                  >
                    {busyAction === `nmod:${cand.mod_id}` ? (
                      <RefreshCw size={14} strokeWidth={1.5} className="spin" />
                    ) : (
                      <Package size={14} strokeWidth={1.5} />
                    )}
                    <span>
                      {busyAction === `nmod:${cand.mod_id}`
                        ? m.workbench.importing
                        : isUpgrade
                          ? m.workbench.updateToVersion(cand.version)
                          : m.workbench.importNmod}
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
        )}

        {trainers.length === 0 ? (
          <div className="surface-panel stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <div className="feature-heading">
                <SlidersHorizontal size={16} strokeWidth={1.5} />
                <h3>{m.workbench.modFeaturesTitle}</h3>
              </div>
              {gameEnv &&
                (gameEnv.engine === "unity_mono" ||
                  gameEnv.engine === "unity_il2cpp") &&
                !hasAntiCheatMarkers && (
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={busyAction === "quick-setup"}
                    onClick={onQuickSetupUnityStarter}
                  >
                    <Zap size={14} strokeWidth={1.5} />
                    <span>
                      {busyAction === "quick-setup"
                        ? m.workbench.initializing
                        : m.workbench.quickSetupUnity}
                    </span>
                  </button>
                )}
              {gameEnv &&
                gameEnv.engine === "unreal_engine" &&
                !hasAntiCheatMarkers && (
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={busyAction === "quick-setup"}
                    onClick={onQuickSetupUe4ssStarter}
                  >
                    <Zap size={14} strokeWidth={1.5} />
                    <span>
                      {busyAction === "quick-setup"
                        ? m.workbench.initializing
                        : m.workbench.quickSetupUe4ss}
                    </span>
                  </button>
                )}
            </div>
            <div className="muted">
              {gameEnv &&
              (gameEnv.engine === "unity_mono" ||
                gameEnv.engine === "unity_il2cpp")
                ? m.workbench.noModUnityHint
                : gameEnv && gameEnv.engine === "unreal_engine"
                  ? m.workbench.noModUnrealHint
                  : m.workbench.noModGenericHint}
            </div>
          </div>
        ) : (
          selectedTrainer && (
            <>
              {/* Mod Selector Pills + Export / Uninstall */}
              <div className="mod-tab-bar">
                <div className="mod-tabs">
                  {trainers.map((t) => {
                    const isTabSelected =
                      t.trainer_id === selectedTrainer.trainer_id;
                    const isTabActive =
                      isSelectedGameInjected &&
                      Boolean(activeSessions[t.trainer_id]);
                    return (
                      <button
                        type="button"
                        key={t.trainer_id}
                        className={`mod-tab ${isTabSelected ? "active" : ""}`}
                        onClick={() => {
                          onSelectTrainer(t.trainer_id);
                          onSetConfirmDeleteTrainerId(null);
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
                  <span className={trustTagClass(selectedTrainer.trust_tier)}>
                    {trustLabel(selectedTrainer.trust_tier, locale)}
                  </span>
                  <span className="tag">
                    compat:{selectedTrainer.compatibility_status}
                  </span>
                  {selectedTrainer.revoked && (
                    <span className="tag unverified">{m.workbench.revoked}</span>
                  )}

                  <button
                    type="button"
                    className="btn btn-outline btn-sm"
                    disabled={busyAction === "export-nmod"}
                    onClick={() => onExportNmod(selectedTrainer)}
                    title={m.workbench.exportNmodTitle}
                  >
                    <Share size={13} strokeWidth={1.5} />
                    <span>
                      {busyAction === "export-nmod"
                        ? m.workbench.exportingNmod
                        : m.workbench.exportNmod}
                    </span>
                  </button>

                  {confirmDeleteTrainerId === selectedTrainer.trainer_id ? (
                    <>
                      <button
                        type="button"
                        className="btn btn-danger-outline btn-sm"
                        disabled={
                          busyAction === "uninstall-trainer" ||
                          Boolean(activeSessions[selectedTrainer.trainer_id])
                        }
                        onClick={() => onUninstallTrainer(selectedTrainer)}
                      >
                        <Trash2 size={13} strokeWidth={1.5} />
                        <span>
                          {busyAction === "uninstall-trainer"
                            ? m.workbench.deletingMod
                            : m.workbench.confirmDeleteMod}
                        </span>
                      </button>
                      <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => onSetConfirmDeleteTrainerId(null)}
                      >
                        {m.workbench.cancel}
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-danger-outline btn-sm"
                      disabled={
                        busyAction === "uninstall-trainer" ||
                        Boolean(activeSessions[selectedTrainer.trainer_id])
                      }
                      title={
                        activeSessions[selectedTrainer.trainer_id]
                          ? m.workbench.cannotDeleteWhileInjected
                          : m.workbench.deleteModTitle
                      }
                      onClick={() =>
                        onSetConfirmDeleteTrainerId(selectedTrainer.trainer_id)
                      }
                    >
                      <Trash2 size={13} strokeWidth={1.5} />
                      <span>{m.workbench.deleteMod}</span>
                    </button>
                  )}
                </div>
              </div>

              {/* Inline Default-Deny Permission Gate */}
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
                      <strong>{m.workbench.permBannerTitle}</strong>
                    </div>
                    {selectedTrainer.permission_state.status === "blocked" && (
                      <span className="tag unverified">
                        {m.workbench.permBlockedPrefix}:{" "}
                        {selectedTrainer.permission_state.reason}
                      </span>
                    )}
                  </div>
                  <div className="stack" style={{ gap: 4 }}>
                    {selectedTrainer.permissions.map((p) => (
                      <div key={p.scope} className="muted">
                        <span style={{ color: "var(--text-primary)" }}>
                          {translatePermission(p.scope, locale)}
                        </span>{" "}
                        <span className="mono">({p.scope})</span>: {p.reason}
                      </div>
                    ))}
                  </div>
                  {selectedTrainer.permission_state.status ===
                    "requires_confirmation" && (
                    <div className="row">
                      <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={busyAction === "approve"}
                        onClick={() =>
                          onApprovePermissions(selectedTrainer, true)
                        }
                      >
                        <Check size={14} strokeWidth={1.5} />
                        <span>{m.workbench.approveAndInject}</span>
                      </button>
                      <button
                        type="button"
                        className="btn btn-outline btn-sm"
                        disabled={busyAction === "approve"}
                        onClick={() =>
                          onApprovePermissions(selectedTrainer, false)
                        }
                      >
                        {m.workbench.approveOnly}
                      </button>
                    </div>
                  )}
                </div>
              )}

              {/* Compact Mod Header + Clamped Single-Line Description + Unified Toolbar */}
              <div className="section-flat">
                <div className="feature-heading">
                  <h3>{m.workbench.modConsoleTitle}</h3>
                  <span className="muted mono">
                    ({filteredFeatures.length}/{selectedTrainer.features.length})
                  </span>
                  {selectedTrainer.description && (
                    <button
                      type="button"
                      className={`mod-desc-clamp ${descExpanded ? "expanded" : ""}`}
                      onClick={() => setDescExpanded((v) => !v)}
                      title={`${selectedTrainer.description} (${
                        descExpanded
                          ? m.workbench.collapseDescHint
                          : m.workbench.expandDescHint
                      })`}
                    >
                      {selectedTrainer.description}
                    </button>
                  )}
                  {findDuplicateHotkeys(
                    profilesByTrainer[selectedTrainer.trainer_id] ?? {},
                  ).length > 0 && (
                    <span className="tag unverified">
                      {m.workbench.hotkeyConflict(
                        findDuplicateHotkeys(
                          profilesByTrainer[selectedTrainer.trainer_id] ?? {},
                        ).join(", "),
                      )}
                    </span>
                  )}
                  {crossModHotkeyConflicts.length > 0 && (
                    <span className="tag unverified">
                      {m.workbench.crossModConflict(
                        crossModHotkeyConflicts.join(", "),
                      )}
                    </span>
                  )}
                </div>

                {/* Single Merged Strip: Filter Pills + Search + Presets/Profile Actions */}
                <div className="feature-unified-bar">
                  <div className="feature-filter-tabs">
                    <button
                      type="button"
                      className={`mini-tab ${featureFilter === "all" ? "active" : ""}`}
                      onClick={() => setFeatureFilter("all")}
                    >
                      {m.workbench.filterAll(featureCounts.all)}
                    </button>
                    <button
                      type="button"
                      className={`mini-tab ${featureFilter === "toggle" ? "active" : ""}`}
                      onClick={() => setFeatureFilter("toggle")}
                    >
                      {m.workbench.filterToggle(featureCounts.toggle)}
                    </button>
                    <button
                      type="button"
                      className={`mini-tab ${featureFilter === "value" ? "active" : ""}`}
                      onClick={() => setFeatureFilter("value")}
                    >
                      {m.workbench.filterValue(featureCounts.value)}
                    </button>
                    <button
                      type="button"
                      className={`mini-tab ${featureFilter === "action" ? "active" : ""}`}
                      onClick={() => setFeatureFilter("action")}
                    >
                      {m.workbench.filterAction(featureCounts.action)}
                    </button>
                  </div>

                  <div className="feature-unified-right">
                    <div className="feature-preset-group">
                      <button
                        type="button"
                        className="mini-tab"
                        onClick={() =>
                          onApplyPreset(selectedTrainer, "Story Mode")
                        }
                      >
                        Story Mode
                      </button>
                      <button
                        type="button"
                        className="mini-tab"
                        onClick={() => onApplyPreset(selectedTrainer, "No Grind")}
                      >
                        No Grind
                      </button>
                      <button
                        type="button"
                        className="mini-tab"
                        onClick={() =>
                          onApplyPreset(selectedTrainer, "Reset Defaults")
                        }
                        title={m.workbench.resetDefaultsTitle}
                      >
                        <RotateCcw size={12} strokeWidth={1.5} />
                        <span>{m.workbench.resetBtn}</span>
                      </button>
                      <button
                        type="button"
                        className="mini-tab"
                        onClick={() => onSaveProfile(selectedTrainer)}
                        title={m.workbench.saveProfileTitle}
                      >
                        <Save size={12} strokeWidth={1.5} />
                        <span>{m.workbench.saveProfileBtn}</span>
                      </button>
                      <button
                        type="button"
                        className="mini-tab"
                        onClick={() => onExportProfile(selectedTrainer)}
                      >
                        JSON
                      </button>
                    </div>

                    <div className="feature-search-wrap">
                      <Search
                        size={13}
                        strokeWidth={1.5}
                        className="feature-search-icon"
                      />
                      <input
                        className="feature-search-input"
                        placeholder={m.workbench.featureSearchPlaceholder}
                        value={featureQuery}
                        onChange={(e) => setFeatureQuery(e.target.value)}
                      />
                    </div>
                  </div>
                </div>

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
                        type="button"
                        className="icon-btn"
                        onClick={() => onSetProfileJsonTrainerId(null)}
                        aria-label={m.workbench.closeJsonEditorAria}
                      >
                        <X size={14} strokeWidth={1.5} />
                      </button>
                    </div>
                    <textarea
                      className="text-input mono"
                      rows={6}
                      value={profileJsonBox}
                      onChange={(e) => onSetProfileJsonBox(e.target.value)}
                    />
                    <div className="row">
                      <button
                        type="button"
                        className="btn btn-outline btn-sm"
                        onClick={() => onImportProfile(selectedTrainer)}
                      >
                        {m.workbench.importJsonBtn}
                      </button>
                    </div>
                  </div>
                )}

                {/* De-Cluttered Feature Grid Cards (no control-type tags) */}
                <div className="feature-grid">
                  {filteredFeatures.map((f) => {
                    const isOn =
                      isSelectedTrainerInjected && Boolean(activeToggles[f.id]);
                    const isApplying = applyingFeatures.has(
                      `${selectedTrainer.trainer_id}::${f.id}`,
                    );
                    const recKey = `${selectedTrainer.trainer_id}::${f.id}`;
                    const isRecording = recordingFeatureKey === recKey;
                    const currentHotkey =
                      profilesByTrainer[selectedTrainer.trainer_id]?.[f.id] ??
                      f.default_hotkey ??
                      "";

                    return (
                      <div
                        key={f.id}
                        className={`feature-cell ${isOn ? "enabled" : ""}`}
                      >
                        <div className="feature-cell-head">
                          <div className="feature-name">{f.name}</div>
                          <div className="feature-desc">{f.description}</div>
                        </div>

                        <div className="feature-cell-controls">
                          {(f.control_type === "toggle" ||
                            f.control_type === "action") && (
                            <button
                              type="button"
                              className={`hotkey-pill ${isRecording ? "recording" : ""} ${
                                currentHotkey ? "bound" : ""
                              }`}
                              onMouseDown={(e) => e.stopPropagation()}
                              onClick={(e) => {
                                e.stopPropagation();
                                setRecordingFeatureKey(isRecording ? null : recKey);
                              }}
                              title={m.workbench.recordHotkeyTitle}
                            >
                              <Keyboard size={12} strokeWidth={1.5} />
                              <span>
                                {isRecording
                                  ? m.workbench.recordingKey
                                  : currentHotkey || m.workbench.bindHotkey}
                              </span>
                            </button>
                          )}

                          <div className="row" style={{ gap: 6, marginLeft: "auto" }}>
                            {f.control_type === "toggle" && (
                              <button
                                type="button"
                                role="switch"
                                aria-checked={isOn}
                                className={`switch-btn ${isOn ? "on" : ""}`}
                                disabled={!isSelectedTrainerInjected || isApplying}
                                onClick={() => onApplyFeature(f)}
                              >
                                <span>
                                  {isApplying
                                    ? m.workbench.executing
                                    : isOn
                                      ? m.workbench.stateOn
                                      : m.workbench.stateOff}
                                </span>
                                <span className="switch-track">
                                  <span className="switch-thumb" />
                                </span>
                              </button>
                            )}
                            {f.control_type === "action" && (
                              <button
                                type="button"
                                className="btn btn-outline btn-sm"
                                disabled={!isSelectedTrainerInjected || isApplying}
                                onClick={() => onApplyFeature(f)}
                              >
                                {isApplying
                                  ? m.workbench.executing
                                  : m.workbench.triggerNow}
                              </button>
                            )}
                            {(f.control_type === "integer" ||
                              f.control_type === "float") && (
                              <>
                                <input
                                  className="text-input mono"
                                  style={{
                                    width: 84,
                                    height: 26,
                                    padding: "2px 8px",
                                  }}
                                  type="number"
                                  step={
                                    f.control_type === "integer" ? "1" : "any"
                                  }
                                  placeholder="0"
                                  disabled={
                                    !isSelectedTrainerInjected || isApplying
                                  }
                                  value={featureInputs[f.id] ?? ""}
                                  onChange={(e) =>
                                    onFeatureInputChange(
                                      selectedTrainer.trainer_id,
                                      f.id,
                                      e.target.value,
                                    )
                                  }
                                />
                                <button
                                  type="button"
                                  className="btn btn-outline btn-sm"
                                  disabled={
                                    !isSelectedTrainerInjected || isApplying
                                  }
                                  onClick={() => onApplyFeature(f)}
                                >
                                  {isApplying
                                    ? m.workbench.executing
                                    : m.workbench.applyValue}
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

        {/* Collapsible Drawers */}
        <div className="accordion-list">
          <details className="drawer">
            <summary>
              <span className="drawer-summary-left">
                <Package size={14} strokeWidth={1.5} aria-hidden="true" />
                <span>{m.workbench.bridgeDrawerTitle}</span>
              </span>
              <span className="row" style={{ gap: 6 }}>
                <span className="muted mono">
                  {bridgeStatus?.deployed
                    ? m.workbench.bridgeStatusDeployed
                    : bridgeStatus?.matched_bridge
                      ? bridgeStatus.matched_bridge.id
                      : m.workbench.bridgeStatusIdle}
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
                        {bridgeStatus.loader_installed ? "ready" : "missing"}
                      </span>
                    </div>
                    <div className="muted" style={{ marginTop: 2 }}>
                      {busyAction === "install-loader" && loaderProgress ? (
                        <span
                          className="mono"
                          style={{ color: "var(--text-primary)" }}
                        >
                          {loaderProgress.message}
                        </span>
                      ) : bridgeStatus.loader_installed ? (
                        bridgeStatus.engine === "unreal_engine"
                          ? m.workbench.ue4ssLoaderReady
                          : m.workbench.unityLoaderReady
                      ) : (
                        bridgeStatus.loader_warning ??
                        m.workbench.loaderMissingFallback
                      )}
                    </div>
                  </div>
                  <div className="row">
                    {!bridgeStatus.loader_installed ? (
                      (bridgeStatus.engine === "unity_mono" ||
                        bridgeStatus.engine === "unity_il2cpp" ||
                        bridgeStatus.engine === "unreal_engine") && (
                        <button
                          type="button"
                          className="btn btn-outline btn-sm"
                          disabled={
                            busyAction === "install-loader" ||
                            hasAntiCheatMarkers
                          }
                          onClick={onInstallBepInExLoader}
                        >
                          <Download size={14} strokeWidth={1.5} />
                          <span>
                            {busyAction === "install-loader"
                              ? m.workbench.installing
                              : bridgeStatus.engine === "unreal_engine"
                                ? m.workbench.installUe4ssBtn
                                : m.workbench.installLoaderSimple}
                          </span>
                        </button>
                      )
                    ) : (
                      bridgeStatus.loader_managed_by_nekod && (
                        <button
                          type="button"
                          className="btn btn-danger-outline btn-sm"
                          disabled={busyAction === "undeploy-loader"}
                          onClick={onUndeployBepInExLoader}
                        >
                          <Trash2 size={14} strokeWidth={1.5} />
                          <span>
                            {busyAction === "undeploy-loader"
                              ? m.workbench.uninstallingLoader
                              : m.workbench.uninstallLoader}
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
                        type="button"
                        className="btn btn-outline btn-sm"
                        disabled={
                          busyAction === "deploy-bridge" || hasAntiCheatMarkers
                        }
                        onClick={onAutoDeployBridge}
                      >
                        <Download size={14} strokeWidth={1.5} />
                        <span>{m.workbench.autoDeployOneClick}</span>
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="btn btn-danger-outline btn-sm"
                        disabled={busyAction === "undeploy-bridge"}
                        onClick={onUndeployBridge}
                      >
                        <Trash2 size={14} strokeWidth={1.5} />
                        <span>{m.workbench.undeployAndRestore}</span>
                      </button>
                    )}
                  </div>
                </div>
              ) : (
                <div className="muted">
                  {m.workbench.noBridgeNeeded(
                    bridgeStatus
                      ? ENGINE_LABELS[bridgeStatus.engine]
                      : m.app.unknownEngine,
                  )}
                </div>
              )}

              {bridgeStatus && bridgeStatus.active_files.length > 0 && (
                <div className="muted mono">
                  {m.workbench.ledgerFiles(bridgeStatus.active_files.join(", "))}
                </div>
              )}

              <details className="drawer">
                <summary>
                  <span className="drawer-summary-left">
                    <Layers size={14} strokeWidth={1.5} />
                    <span>
                      {m.workbench.allBuiltinBridges(bridgeCatalog.length)}
                    </span>
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
                            {ENGINE_LABELS[b.engine]} · v{b.version}
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
                  placeholder={m.workbench.manualSourcePlaceholder}
                  value={bridgeSourceInput}
                  onChange={(e) => onSetBridgeSourceInput(e.target.value)}
                />
                <input
                  className="text-input mono"
                  style={{ flex: 1 }}
                  placeholder={m.workbench.manualTargetPlaceholder}
                  value={bridgeTargetInput}
                  onChange={(e) => onSetBridgeTargetInput(e.target.value)}
                />
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  disabled={
                    busyAction === "deploy-bridge" || hasAntiCheatMarkers
                  }
                  onClick={onManualDeployBridge}
                >
                  {m.workbench.manualDeploy}
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
                    {m.workbench.permissionsDrawerTitle(
                      selectedTrainer.permissions.length,
                    )}
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
                      {translatePermission(p.scope, locale)}
                    </span>{" "}
                    <span className="mono">({p.scope})</span>: {p.reason}
                  </div>
                ))}
                {selectedTrainer.permission_state.status === "approved" &&
                  !activeSessions[selectedTrainer.trainer_id] && (
                    <div>
                      <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => onRevokePermissions(selectedTrainer)}
                      >
                        {m.workbench.revokeAuth}
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
                  <span>
                    {m.workbench.liveSessionEventsTitle(sessionDiag.session_id)}
                  </span>
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
                  <span>
                    {m.workbench.compatMatrixTitle(compatRecords.length)}
                  </span>
                </span>
                <ChevronRight
                  size={14}
                  strokeWidth={1.5}
                  className="drawer-chevron"
                />
              </summary>
              <div className="drawer-body">
                {compatRecords.length === 0 ? (
                  <div className="muted">{m.workbench.noCompatRecords}</div>
                ) : (
                  compatRecords.map((rec) => (
                    <div key={rec.record_id} className="flat-row">
                      <div>
                        <div className="row">
                          <span style={{ fontWeight: 500 }}>{rec.mod_id}</span>
                          <span className="mono muted">v{rec.mod_version}</span>
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
                  <span>
                    {m.workbench.communityModsTitle(communityTrainers.length)}
                  </span>
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
                    type="button"
                    className="btn btn-ghost btn-sm"
                    disabled={busyAction === "sync"}
                    onClick={onSyncRegistries}
                  >
                    <RefreshCw
                      size={14}
                      strokeWidth={1.5}
                      className={busyAction === "sync" ? "spin" : ""}
                    />
                    <span>
                      {busyAction === "sync"
                        ? m.workbench.syncing
                        : m.workbench.syncCommunity}
                    </span>
                  </button>
                </div>
                {communityTrainers.length === 0 ? (
                  <div className="muted">{m.workbench.noCommunityMods}</div>
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
                          <span className={trustTagClass(ct.trust_tier)}>
                            {trustLabel(ct.trust_tier, locale)}
                          </span>
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
                            type="button"
                            className="btn btn-outline btn-sm"
                            disabled={busyAction === `install:${ct.trainer_id}`}
                            onClick={() =>
                              onInstallCommunityTrainer(
                                ct.repository_id,
                                ct.trainer_id,
                              )
                            }
                          >
                            <Download size={14} strokeWidth={1.5} />
                            <span>
                              {busyAction === `install:${ct.trainer_id}`
                                ? m.workbench.installing
                                : m.workbench.install}
                            </span>
                          </button>
                        )}
                        {ct.install_state.state === "installed_up_to_date" && (
                          <span className="tag official">
                            {m.workbench.installed}
                          </span>
                        )}
                        {ct.install_state.state === "update_available" && (
                          <button
                            type="button"
                            className="btn btn-outline btn-sm"
                            disabled={busyAction === `install:${ct.trainer_id}`}
                            onClick={() =>
                              onInstallCommunityTrainer(
                                ct.repository_id,
                                ct.trainer_id,
                              )
                            }
                          >
                            {busyAction === `install:${ct.trainer_id}`
                              ? m.workbench.updating
                              : m.workbench.updateToVersion(
                                  ct.install_state.latest_version,
                                )}
                          </button>
                        )}
                        {ct.install_state.state === "revoked" && (
                          <span className="tag unverified">
                            {m.workbench.revoked}
                          </span>
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
    </div>
  );
}
