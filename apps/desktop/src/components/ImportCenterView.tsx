import {
  ArrowLeft,
  Check,
  FileCode2,
  FolderOpen,
  Package,
  Play,
  Plus,
  RefreshCw,
  ShieldAlert,
  UploadCloud,
} from "lucide-react";
import type {
  CtImportUiReport,
  LibraryGameView,
  TrustTier,
} from "../api/nekod";
import { useI18n } from "../i18n";

interface ImportCenterViewProps {
  hasSelectedGame: boolean;
  games: LibraryGameView[];
  selectedGame: LibraryGameView | null;
  customDir: string;
  customLabel: string;
  trainerDir: string;
  trainerTrust: TrustTier;
  gameManifestJson: string;
  ctPath: string;
  ctTargetGameKey: string;
  ctCustomGameId: string;
  ctCustomGameName: string;
  ctDefaultModule: string;
  ctTrust: TrustTier;
  ctReport: CtImportUiReport | null;
  busyAction: string | null;
  onBackToWorkbench: () => void;
  onCustomDirChange: (v: string) => void;
  onCustomLabelChange: (v: string) => void;
  onPickCustomFolder: () => void;
  onPickCustomExe: () => void;
  onSubmitCustomGame: () => void;
  onTrainerDirChange: (v: string) => void;
  onTrainerTrustChange: (v: TrustTier) => void;
  onPickNmodFile: () => void;
  onPickNmodFolder: () => void;
  onIngestTrainer: () => void;
  onGameManifestJsonChange: (v: string) => void;
  onRegisterGameManifest: () => void;
  onCtPathChange: (v: string) => void;
  onPickCtFile: () => void;
  onCtTargetGameKeyChange: (v: string) => void;
  onCtCustomGameIdChange: (v: string) => void;
  onCtCustomGameNameChange: (v: string) => void;
  onCtDefaultModuleChange: (v: string) => void;
  onCtTrustChange: (v: TrustTier) => void;
  onImportCtFile: () => void;
  onOpenCtImportedGame: (gameId: string, trainerId: string) => void;
}

export function ImportCenterView({
  hasSelectedGame,
  games,
  selectedGame,
  customDir,
  customLabel,
  trainerDir,
  trainerTrust,
  gameManifestJson,
  ctPath,
  ctTargetGameKey,
  ctCustomGameId,
  ctCustomGameName,
  ctDefaultModule,
  ctTrust,
  ctReport,
  busyAction,
  onBackToWorkbench,
  onCustomDirChange,
  onCustomLabelChange,
  onPickCustomFolder,
  onPickCustomExe,
  onSubmitCustomGame,
  onTrainerDirChange,
  onTrainerTrustChange,
  onPickNmodFile,
  onPickNmodFolder,
  onIngestTrainer,
  onGameManifestJsonChange,
  onRegisterGameManifest,
  onCtPathChange,
  onPickCtFile,
  onCtTargetGameKeyChange,
  onCtCustomGameIdChange,
  onCtCustomGameNameChange,
  onCtDefaultModuleChange,
  onCtTrustChange,
  onImportCtFile,
  onOpenCtImportedGame,
}: ImportCenterViewProps) {
  const { m } = useI18n();
  const effectiveCtGameKey =
    ctTargetGameKey ||
    selectedGame?.library_key ||
    games[0]?.library_key ||
    "__custom__";
  const isCustomCtTarget = effectiveCtGameKey === "__custom__";

  return (
    <div className="page-scroll">
      <div className="page-inner stack" style={{ gap: 16 }}>
        <div className="header-row" style={{ marginBottom: 0 }}>
          <div className="row">
            {hasSelectedGame && (
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={onBackToWorkbench}
              >
                <ArrowLeft size={14} strokeWidth={1.5} />
                <span>{m.importCenter.backToWorkbench}</span>
              </button>
            )}
            <h2 style={{ margin: 0, fontSize: 17, fontWeight: 700 }}>
              {m.importCenter.title}
            </h2>
          </div>
          <span className="pill-badge accent">
            <UploadCloud size={12} strokeWidth={1.5} />
            <span>{m.importCenter.dropHintPill}</span>
          </span>
        </div>

        {/* 0. Add Local Game */}
        <div className="surface-panel stack">
          <div style={{ fontWeight: 600 }}>{m.importCenter.sec0Title}</div>
          <div className="muted">
            {m.importCenter.sec0DescPrefix} <span className="mono">.exe</span>
            {m.importCenter.sec0DescSuffix}
          </div>
          <div className="stack">
            <div className="row">
              <input
                className="text-input mono"
                style={{ flex: 1, minWidth: 220 }}
                placeholder={m.importCenter.pathPlaceholder}
                value={customDir}
                onChange={(e) => onCustomDirChange(e.target.value)}
              />
              <button
                type="button"
                className="btn btn-outline btn-sm"
                onClick={onPickCustomFolder}
              >
                <FolderOpen size={14} strokeWidth={1.5} />
                <span>{m.importCenter.chooseFolder}</span>
              </button>
              <button
                type="button"
                className="btn btn-outline btn-sm"
                onClick={onPickCustomExe}
              >
                <Play size={14} strokeWidth={1.5} />
                <span>{m.importCenter.chooseExe}</span>
              </button>
            </div>
            <div className="row">
              <input
                className="text-input"
                style={{ flex: 1 }}
                placeholder={m.importCenter.labelPlaceholder}
                value={customLabel}
                onChange={(e) => onCustomLabelChange(e.target.value)}
              />
              <button
                type="button"
                className="btn btn-primary"
                disabled={!customDir.trim() || busyAction === "add-game"}
                onClick={onSubmitCustomGame}
              >
                <Plus size={14} strokeWidth={1.5} />
                <span>
                  {busyAction === "add-game"
                    ? m.importCenter.importing
                    : m.importCenter.addToLibrary}
                </span>
              </button>
            </div>
          </div>
        </div>

        {/* 1. Import Local .nmod */}
        <div className="surface-panel stack">
          <div style={{ fontWeight: 600 }}>
            {m.importCenter.sec1TitlePrefix}{" "}
            <span className="mono">.nmod</span>{" "}
            {m.importCenter.sec1TitleMid}{" "}
            <span className="mono">.nmod</span>{" "}
            {m.importCenter.sec1TitleSuffix}
          </div>
          <div className="muted">
            {m.importCenter.sec1DescPrefix}{" "}
            <span className="mono">.nmod</span>{" "}
            {m.importCenter.sec1DescSuffix}
          </div>
          <div className="row">
            <input
              className="text-input mono"
              style={{ flex: 1, minWidth: 220 }}
              placeholder={m.importCenter.nmodPlaceholder}
              value={trainerDir}
              onChange={(e) => onTrainerDirChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickNmodFile}
            >
              <Package size={14} strokeWidth={1.5} />
              <span>{m.importCenter.chooseNmod}</span>
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickNmodFolder}
            >
              <FolderOpen size={14} strokeWidth={1.5} />
              <span>{m.importCenter.chooseFolder}</span>
            </button>
            <select
              className="select-input"
              value={trainerTrust}
              onChange={(e) =>
                onTrainerTrustChange(e.target.value as TrustTier)
              }
            >
              <option value="official">Official</option>
              <option value="community_verified">Community Verified</option>
              <option value="unverified">Unverified</option>
            </select>
            <button
              type="button"
              className="btn btn-primary"
              disabled={!trainerDir.trim()}
              onClick={onIngestTrainer}
            >
              {m.importCenter.verifyAndImport}
            </button>
          </div>
        </div>

        {/* 2. Import Cheat Engine .CT */}
        <div className="surface-panel stack">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div style={{ fontWeight: 600 }}>
              {m.importCenter.sec2TitlePrefix}
              <span className="mono">.CT</span> {m.importCenter.sec2TitleSuffix}
            </div>
            <span className="tag">{m.importCenter.sec2Badge}</span>
          </div>
          <div className="muted">
            {m.importCenter.sec2Desc1}{" "}
            <span className="mono">aobscanmodule + db</span>{" "}
            {m.importCenter.sec2Desc2} <span className="mono">{"{$lua}"}</span>,{" "}
            <span className="mono">alloc()</span> {m.importCenter.sec2Desc3}
          </div>

          <div className="row">
            <select
              className="select-input"
              style={{ flex: 1, minWidth: 220 }}
              value={effectiveCtGameKey}
              onChange={(e) => onCtTargetGameKeyChange(e.target.value)}
            >
              {games.map((g) => (
                <option key={g.library_key} value={g.library_key}>
                  {g.game.title} (
                  {g.game.matched_game_id ?? g.game.store_source})
                </option>
              ))}
              <option value="__custom__">{m.importCenter.customTargetOption}</option>
            </select>

            <select
              className="select-input"
              value={ctTrust}
              onChange={(e) => onCtTrustChange(e.target.value as TrustTier)}
            >
              <option value="unverified">
                {m.importCenter.trustUnverifiedOpt}
              </option>
              <option value="community_verified">
                {m.importCenter.trustCommunityOpt}
              </option>
              <option value="official">{m.importCenter.trustOfficialOpt}</option>
            </select>
          </div>

          {isCustomCtTarget && (
            <div className="row">
              <input
                className="text-input mono"
                style={{ flex: 1 }}
                placeholder={m.importCenter.customGameIdPlaceholder}
                value={ctCustomGameId}
                onChange={(e) => onCtCustomGameIdChange(e.target.value)}
              />
              <input
                className="text-input"
                style={{ flex: 1 }}
                placeholder={m.importCenter.customGameNamePlaceholder}
                value={ctCustomGameName}
                onChange={(e) => onCtCustomGameNameChange(e.target.value)}
              />
            </div>
          )}

          <div className="row">
            <input
              className="text-input mono"
              style={{ flex: 2, minWidth: 200 }}
              placeholder={m.importCenter.ctPathPlaceholder}
              value={ctPath}
              onChange={(e) => onCtPathChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickCtFile}
            >
              <FileCode2 size={14} strokeWidth={1.5} />
              <span>{m.importCenter.chooseCt}</span>
            </button>
            <input
              className="text-input mono"
              style={{ flex: 1 }}
              placeholder={m.importCenter.defaultModulePlaceholder}
              value={ctDefaultModule}
              onChange={(e) => onCtDefaultModuleChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-primary"
              disabled={!ctPath.trim() || busyAction === "import-ct"}
              onClick={onImportCtFile}
            >
              {busyAction === "import-ct" ? (
                <RefreshCw size={14} strokeWidth={1.5} className="spin" />
              ) : (
                <FileCode2 size={14} strokeWidth={1.5} />
              )}
              <span>
                {busyAction === "import-ct"
                  ? m.importCenter.convertingCt
                  : m.importCenter.convertAndImportCt}
              </span>
            </button>
          </div>

          {ctTrust === "unverified" && (
            <div className="muted">
              {m.importCenter.ctUnverifiedHintPrefix}{" "}
              <span className="mono">Community Verified</span>
              {m.importCenter.ctUnverifiedHintSuffix}
            </div>
          )}

          {ctReport && (
            <div
              className="stack"
              style={{
                paddingTop: 10,
                borderTop: "1px solid var(--border)",
              }}
            >
              <div className="row" style={{ justifyContent: "space-between" }}>
                <div className="row" style={{ gap: 6 }}>
                  {ctReport.imported ? (
                    <Check
                      size={15}
                      strokeWidth={1.5}
                      style={{ color: "var(--ok)" }}
                    />
                  ) : (
                    <ShieldAlert
                      size={15}
                      strokeWidth={1.5}
                      style={{ color: "var(--warn)" }}
                    />
                  )}
                  <span style={{ fontWeight: 500 }}>{ctReport.message}</span>
                </div>
                {ctReport.imported && (
                  <button
                    type="button"
                    className="btn btn-outline btn-sm"
                    onClick={() =>
                      onOpenCtImportedGame(ctReport.game_id, ctReport.trainer_id)
                    }
                  >
                    {m.importCenter.openInWorkbench}
                  </button>
                )}
              </div>

              <div className="muted mono">
                trainer_id={ctReport.trainer_id} · game_id={ctReport.game_id}
                {ctReport.modules.length > 0
                  ? ` · modules=${ctReport.modules.join(", ")}`
                  : ""}
              </div>

              {ctReport.converted_features.length > 0 && (
                <div className="stack" style={{ gap: 4 }}>
                  <div className="secondary" style={{ fontWeight: 500 }}>
                    {m.importCenter.convertedFeatures(
                      ctReport.converted_features.length,
                    )}
                  </div>
                  <div className="row">
                    {ctReport.converted_features.map((fid) => (
                      <span key={fid} className="tag official">
                        {fid}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              {ctReport.skipped_entries.length > 0 && (
                <div className="stack" style={{ gap: 4 }}>
                  <div className="secondary" style={{ fontWeight: 500 }}>
                    {m.importCenter.skippedEntries(
                      ctReport.skipped_entries.length,
                    )}
                  </div>
                  {ctReport.skipped_entries.map((entry, idx) => (
                    <div key={idx} className="flat-row">
                      <div style={{ minWidth: 0 }}>
                        <div className="row">
                          <span style={{ fontWeight: 500 }}>
                            {entry.description || m.importCenter.unnamedEntry}
                          </span>
                          <span className="tag unverified">
                            {entry.variable_type}
                          </span>
                        </div>
                        <div className="muted mono" style={{ marginTop: 2 }}>
                          {entry.reason}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* 3. Register GameManifest JSON */}
        <div className="surface-panel stack">
          <div style={{ fontWeight: 600 }}>{m.importCenter.sec3Title}</div>
          <textarea
            className="text-input mono"
            rows={5}
            placeholder={m.importCenter.gameManifestPlaceholder}
            value={gameManifestJson}
            onChange={(e) => onGameManifestJsonChange(e.target.value)}
          />
          <div>
            <button
              type="button"
              className="btn btn-outline"
              disabled={!gameManifestJson.trim()}
              onClick={onRegisterGameManifest}
            >
              {m.importCenter.verifyAndRegisterGame}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
