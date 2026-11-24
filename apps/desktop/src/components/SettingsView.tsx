import {
  ArrowLeft,
  FolderOpen,
  Languages,
  Layers,
  Play,
  Plus,
  Shield,
} from "lucide-react";
import type {
  CompanionBridgeCatalogEntry,
  CustomGamePathRecord,
  SecurityPolicyConfig,
} from "../api/nekod";
import { LOCALE_OPTIONS, useI18n } from "../i18n";
import { ENGINE_LABELS } from "../types/ui";

interface SettingsViewProps {
  hasSelectedGame: boolean;
  appVersion: string;
  policy: SecurityPolicyConfig;
  minimizeToTray: boolean;
  autoAttachEnabled: boolean;
  hudOverlayEnabled: boolean;
  soundEnabled: boolean;
  customPaths: CustomGamePathRecord[];
  customDir: string;
  customLabel: string;
  bridgeCatalog: CompanionBridgeCatalogEntry[];
  busyAction: string | null;
  onBackToWorkbench: () => void;
  onTogglePolicy: (next: SecurityPolicyConfig) => void;
  onToggleMinimizeToTray: () => void;
  onToggleAutoAttach: () => void;
  onToggleHudOverlay: () => void;
  onToggleSound: () => void;
  onCustomDirChange: (v: string) => void;
  onCustomLabelChange: (v: string) => void;
  onPickCustomFolder: () => void;
  onPickCustomExe: () => void;
  onAddCustomPath: () => void;
  onRemoveCustomPath: (path: string) => void;
}

export function SettingsView({
  hasSelectedGame,
  appVersion,
  policy,
  minimizeToTray,
  autoAttachEnabled,
  hudOverlayEnabled,
  soundEnabled,
  customPaths,
  customDir,
  customLabel,
  bridgeCatalog,
  busyAction,
  onBackToWorkbench,
  onTogglePolicy,
  onToggleMinimizeToTray,
  onToggleAutoAttach,
  onToggleHudOverlay,
  onToggleSound,
  onCustomDirChange,
  onCustomLabelChange,
  onPickCustomFolder,
  onPickCustomExe,
  onAddCustomPath,
  onRemoveCustomPath,
}: SettingsViewProps) {
  const { locale, setLocale, m } = useI18n();

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
                <span>{m.settings.backToWorkbench}</span>
              </button>
            )}
            <h2 style={{ margin: 0, fontSize: 17, fontWeight: 700 }}>
              {m.settings.title}
            </h2>
          </div>
          <span className="muted mono">Nekod Desktop v{appVersion}</span>
        </div>

        {/* Language Section */}
        <div className="surface-panel stack">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="stack" style={{ gap: 2 }}>
              <div className="row" style={{ gap: 6 }}>
                <Languages
                  size={15}
                  strokeWidth={1.5}
                  style={{ color: "var(--brand)" }}
                />
                <span style={{ fontWeight: 600 }}>
                  {m.settings.languageSection}
                </span>
              </div>
              <div className="muted">{m.settings.languageDesc}</div>
            </div>

            <div className="locale-selector" role="group" aria-label={m.settings.languageLabel}>
              {LOCALE_OPTIONS.map((opt) => (
                <button
                  key={opt.id}
                  type="button"
                  className={`locale-pill ${locale === opt.id ? "active" : ""}`}
                  onClick={() => setLocale(opt.id)}
                >
                  {opt.nativeLabel}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Desktop Behavior */}
        <div className="surface-panel stack">
          <div style={{ fontWeight: 600 }}>{m.settings.desktopBehavior}</div>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={minimizeToTray}
              onChange={onToggleMinimizeToTray}
            />
            <div>
              <div>{m.settings.minimizeToTrayTitle}</div>
              <div className="muted">{m.settings.minimizeToTrayDesc}</div>
            </div>
          </label>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={autoAttachEnabled}
              onChange={onToggleAutoAttach}
            />
            <div>
              <div>{m.settings.autoAttachTitle}</div>
              <div className="muted">{m.settings.autoAttachDesc}</div>
            </div>
          </label>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={hudOverlayEnabled}
              onChange={onToggleHudOverlay}
            />
            <div>
              <div>{m.settings.hudOverlayTitle}</div>
              <div className="muted">{m.settings.hudOverlayDesc}</div>
            </div>
          </label>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={soundEnabled}
              onChange={onToggleSound}
            />
            <div>
              <div>{m.settings.soundTitle}</div>
              <div className="muted">{m.settings.soundDesc}</div>
            </div>
          </label>
        </div>

        {/* Security Policy */}
        <div className="surface-panel stack">
          <div className="row" style={{ gap: 6 }}>
            <Shield size={15} strokeWidth={1.5} style={{ color: "var(--brand)" }} />
            <span style={{ fontWeight: 600 }}>{m.settings.securitySection}</span>
          </div>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={policy.allow_unverified_sources}
              onChange={(e) =>
                onTogglePolicy({
                  ...policy,
                  allow_unverified_sources: e.target.checked,
                })
              }
            />
            <div>
              <div>{m.settings.allowUnverifiedTitle}</div>
              <div className="muted">{m.settings.allowUnverifiedDesc}</div>
            </div>
          </label>

          <label className="setting-toggle-row">
            <input
              type="checkbox"
              checked={policy.allow_third_party_native_modules}
              onChange={(e) =>
                onTogglePolicy({
                  ...policy,
                  allow_third_party_native_modules: e.target.checked,
                })
              }
            />
            <div>
              <div>{m.settings.allowNativeTitle}</div>
              <div className="muted">{m.settings.allowNativeDesc}</div>
            </div>
          </label>
        </div>

        {/* Custom Game Folders */}
        <div className="surface-panel stack">
          <div style={{ fontWeight: 600 }}>{m.settings.customFoldersTitle}</div>
          <div className="row">
            <input
              className="text-input mono"
              style={{ flex: 1, minWidth: 220 }}
              placeholder={m.settings.pathPlaceholder}
              value={customDir}
              onChange={(e) => onCustomDirChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickCustomFolder}
            >
              <FolderOpen size={14} strokeWidth={1.5} />
              <span>{m.settings.chooseFolder}</span>
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickCustomExe}
            >
              <Play size={14} strokeWidth={1.5} />
              <span>{m.settings.chooseExe}</span>
            </button>
          </div>
          <div className="row">
            <input
              className="text-input"
              style={{ flex: 1 }}
              placeholder={m.settings.labelPlaceholder}
              value={customLabel}
              onChange={(e) => onCustomLabelChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-primary"
              disabled={!customDir.trim() || busyAction === "add-game"}
              onClick={onAddCustomPath}
            >
              <Plus size={14} strokeWidth={1.5} />
              <span>{m.settings.addFolderBtn}</span>
            </button>
          </div>

          {customPaths.map((cp) => (
            <div key={cp.path} className="flat-row">
              <div>
                <div style={{ fontWeight: 500 }}>{cp.label ?? cp.path}</div>
                <div className="muted mono">{cp.path}</div>
              </div>
              <button
                type="button"
                className="btn btn-danger-outline btn-sm"
                onClick={() => onRemoveCustomPath(cp.path)}
              >
                {m.settings.removeBtn}
              </button>
            </div>
          ))}
        </div>

        {/* Builtin Companion Bridges Overview */}
        <div className="surface-panel stack">
          <div className="row" style={{ gap: 6 }}>
            <Layers size={15} strokeWidth={1.5} />
            <span style={{ fontWeight: 600 }}>
              {m.settings.bridgesMatrixTitle(bridgeCatalog.length)}
            </span>
          </div>
          {bridgeCatalog.map((b) => (
            <div key={b.id} className="flat-row">
              <div>
                <div className="row">
                  <span style={{ fontWeight: 500 }}>{b.display_name}</span>
                  <span className="mono muted">
                    {ENGINE_LABELS[b.engine]} · v{b.version}
                  </span>
                  <span
                    className={`tag ${b.ready_locally ? "official" : "unverified"}`}
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
      </div>
    </div>
  );
}
