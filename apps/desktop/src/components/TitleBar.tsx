import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  Copy as RestoreIcon,
  FolderInput,
  Globe,
  Languages,
  Layers,
  Minus,
  Settings,
  Sparkles,
  Square,
  Terminal,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { isBrowserPreview } from "../api/nekod";
import nekodIcon from "../assets/nekod-icon.png";
import { LOCALE_OPTIONS, useI18n } from "../i18n";
import type { NavTab } from "../types/ui";

interface TitleBarProps {
  appVersion: string;
  tab: NavTab;
  updatesCount: number;
  activeSessionCount: number;
  activeGameTitle: string | null;
  hudOverlayEnabled: boolean;
  soundEnabled: boolean;
  minimizeToTray: boolean;
  onSelectTab: (tab: NavTab) => void;
  onToggleHudOverlay: () => void;
  onToggleSound: () => void;
  onOpenActiveGame: () => void;
}

export function TitleBar({
  appVersion,
  tab,
  updatesCount,
  activeSessionCount,
  activeGameTitle,
  hudOverlayEnabled,
  soundEnabled,
  minimizeToTray,
  onSelectTab,
  onToggleHudOverlay,
  onToggleSound,
  onOpenActiveGame,
}: TitleBarProps) {
  const [maximized, setMaximized] = useState(false);
  const { locale, cycleLocale, m } = useI18n();
  const currentLocaleShort =
    LOCALE_OPTIONS.find((opt) => opt.id === locale)?.shortLabel ?? "中文";

  useEffect(() => {
    if (isBrowserPreview) return;
    let unlisten: (() => void) | null = null;
    const win = getCurrentWindow();
    void win
      .isMaximized()
      .then(setMaximized)
      .catch(() => {});
    void win
      .onResized(() => {
        void win
          .isMaximized()
          .then(setMaximized)
          .catch(() => {});
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, []);

  async function handleMinimize() {
    if (isBrowserPreview) return;
    await getCurrentWindow().minimize().catch(() => {});
  }

  async function handleToggleMaximize() {
    if (isBrowserPreview) return;
    const win = getCurrentWindow();
    await win.toggleMaximize().catch(() => {});
    const max = await win.isMaximized().catch(() => false);
    setMaximized(max);
  }

  async function handleClose() {
    if (isBrowserPreview) return;
    await getCurrentWindow().close().catch(() => {});
  }

  return (
    <header className="titlebar">
      <div className="titlebar-brand" data-tauri-drag-region>
        <img
          className="brand-icon-img"
          src={nekodIcon}
          alt="Nekod"
          draggable={false}
        />
        <span className="brand-wordmark" data-tauri-drag-region>
          Nekod
        </span>
        <span className="brand-version-pill" data-tauri-drag-region>
          v{appVersion}
        </span>
      </div>

      <nav className="titlebar-nav" aria-label={m.titlebar.primaryNavAria}>
        <button
          type="button"
          className={`nav-pill ${tab === "workbench" ? "active" : ""}`}
          onClick={() => onSelectTab("workbench")}
        >
          <Layers size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{m.titlebar.workbench}</span>
        </button>

        <button
          type="button"
          className={`nav-pill ${tab === "community" ? "active" : ""}`}
          onClick={() => onSelectTab("community")}
        >
          <Globe size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{m.titlebar.store}</span>
          {updatesCount > 0 && (
            <span className="nav-pill-badge">{updatesCount}</span>
          )}
        </button>

        <button
          type="button"
          className={`nav-pill ${tab === "import" ? "active" : ""}`}
          onClick={() => onSelectTab("import")}
        >
          <FolderInput size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{m.titlebar.importTab}</span>
        </button>

        <button
          type="button"
          className={`nav-pill ${tab === "settings" ? "active" : ""}`}
          onClick={() => onSelectTab("settings")}
        >
          <Settings size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{m.titlebar.settings}</span>
        </button>

        <button
          type="button"
          className={`nav-pill ${tab === "logs" ? "active" : ""}`}
          onClick={() => onSelectTab("logs")}
        >
          <Terminal size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{m.titlebar.logs}</span>
        </button>
      </nav>

      <div className="titlebar-drag-spacer" data-tauri-drag-region />

      <div className="titlebar-actions">
        {activeSessionCount > 0 && (
          <button
            type="button"
            className="live-session-chip"
            onClick={onOpenActiveGame}
            title={m.titlebar.activeSessionTitle}
          >
            <span className="pulse-dot" />
            <Activity size={13} strokeWidth={1.5} aria-hidden="true" />
            <span>
              {activeGameTitle ? `${activeGameTitle} · ` : ""}
              {m.titlebar.activeModsSuffix(activeSessionCount)}
            </span>
          </button>
        )}

        <button
          type="button"
          className="titlebar-quick-btn"
          onClick={cycleLocale}
          title={m.titlebar.languageTitle}
          aria-label={m.titlebar.languageTitle}
        >
          <Languages size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>{currentLocaleShort}</span>
        </button>

        <button
          type="button"
          className={`titlebar-quick-btn ${hudOverlayEnabled ? "on" : ""}`}
          onClick={onToggleHudOverlay}
          title={m.titlebar.toggleHud}
          aria-label={m.titlebar.toggleHud}
        >
          <Sparkles size={14} strokeWidth={1.5} aria-hidden="true" />
          <span>HUD</span>
        </button>

        <button
          type="button"
          className={`titlebar-quick-btn icon-only ${soundEnabled ? "on" : ""}`}
          onClick={onToggleSound}
          title={soundEnabled ? m.titlebar.soundOn : m.titlebar.soundOff}
          aria-label={m.titlebar.soundToggleAria}
        >
          {soundEnabled ? (
            <Volume2 size={14} strokeWidth={1.5} aria-hidden="true" />
          ) : (
            <VolumeX size={14} strokeWidth={1.5} aria-hidden="true" />
          )}
        </button>

        <div className="window-controls">
          <button
            type="button"
            className="win-btn"
            onClick={() => void handleMinimize()}
            title={m.titlebar.minimize}
            aria-label={m.titlebar.minimizeAria}
          >
            <Minus size={14} strokeWidth={1.5} />
          </button>
          <button
            type="button"
            className="win-btn"
            onClick={() => void handleToggleMaximize()}
            title={maximized ? m.titlebar.restore : m.titlebar.maximize}
            aria-label={
              maximized ? m.titlebar.restoreAria : m.titlebar.maximizeAria
            }
          >
            {maximized ? (
              <RestoreIcon size={12} strokeWidth={1.5} />
            ) : (
              <Square size={12} strokeWidth={1.5} />
            )}
          </button>
          <button
            type="button"
            className="win-btn close"
            onClick={() => void handleClose()}
            title={
              minimizeToTray ? m.titlebar.closeToTray : m.titlebar.closeWindow
            }
            aria-label={m.titlebar.closeWindowAria}
          >
            <X size={14} strokeWidth={1.5} />
          </button>
        </div>
      </div>
    </header>
  );
}
