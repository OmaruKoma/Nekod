import { FolderOpen, Package, Play, Plus, X } from "lucide-react";
import { useI18n } from "../i18n";

interface AddGameModalProps {
  open: boolean;
  customDir: string;
  customLabel: string;
  busyAction: string | null;
  onCustomDirChange: (v: string) => void;
  onCustomLabelChange: (v: string) => void;
  onPickFolder: () => void;
  onPickExe: () => void;
  onSubmit: () => void;
  onClose: () => void;
}

export function AddGameModal({
  open,
  customDir,
  customLabel,
  busyAction,
  onCustomDirChange,
  onCustomLabelChange,
  onPickFolder,
  onPickExe,
  onSubmit,
  onClose,
}: AddGameModalProps) {
  const { m } = useI18n();
  if (!open) return null;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-local-game-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="row" style={{ justifyContent: "space-between" }}>
          <h3
            id="add-local-game-title"
            style={{ margin: 0, fontSize: 15, fontWeight: 600 }}
          >
            {m.modals.addGameTitle}
          </h3>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            aria-label={m.modals.closeAria}
          >
            <X size={15} strokeWidth={1.5} />
          </button>
        </div>

        <div className="muted">
          {m.modals.addGameDescPrefix}{" "}
          <span className="mono">.exe</span>.
        </div>

        <div className="stack">
          <div className="row">
            <input
              className="text-input mono"
              style={{ flex: 1, minWidth: 220 }}
              placeholder={m.modals.pathPlaceholder}
              value={customDir}
              onChange={(e) => onCustomDirChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickFolder}
            >
              <FolderOpen size={14} strokeWidth={1.5} />
              <span>{m.modals.chooseFolder}</span>
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onPickExe}
            >
              <Play size={14} strokeWidth={1.5} />
              <span>{m.modals.chooseExe}</span>
            </button>
          </div>
          <div className="row">
            <input
              className="text-input"
              style={{ flex: 1 }}
              placeholder={m.modals.labelPlaceholder}
              value={customLabel}
              onChange={(e) => onCustomLabelChange(e.target.value)}
            />
            <button
              type="button"
              className="btn btn-primary"
              disabled={!customDir.trim() || busyAction === "add-game"}
              onClick={onSubmit}
            >
              <Plus size={14} strokeWidth={1.5} />
              <span>
                {busyAction === "add-game"
                  ? m.modals.importing
                  : m.modals.addToLibrary}
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

interface DropZoneOverlayProps {
  visible: boolean;
  paths: string[];
}

export function DropZoneOverlay({ visible, paths }: DropZoneOverlayProps) {
  const { m } = useI18n();
  if (!visible) return null;
  const previewName =
    paths[0]?.split(/[\\/]/).pop() ?? m.modals.dropDefaultPreview;

  return (
    <div className="dropzone-overlay" aria-live="polite">
      <div className="dropzone-card">
        <div className="dropzone-icon">
          <Package size={30} strokeWidth={1.5} />
        </div>
        <div className="dropzone-title">{m.modals.dropTitle}</div>
        <div className="dropzone-sub mono">{previewName}</div>
        <div className="muted">
          {m.modals.dropDesc1} <span className="mono">.nmod</span>{" "}
          {m.modals.dropDesc2} <span className="mono">.ct</span>{" "}
          {m.modals.dropDesc3}
        </div>
      </div>
    </div>
  );
}
