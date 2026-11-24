import { ArrowLeft, RefreshCw, X } from "lucide-react";
import type { DiagnosticEvent } from "../api/nekod";
import { useI18n } from "../i18n";

interface LogsViewProps {
  hasSelectedGame: boolean;
  logs: DiagnosticEvent[];
  diagnosticBundle: string | null;
  onBackToWorkbench: () => void;
  onExportBundle: () => void;
  onCloseBundle: () => void;
  onRefreshLogs: () => void;
}

export function LogsView({
  hasSelectedGame,
  logs,
  diagnosticBundle,
  onBackToWorkbench,
  onExportBundle,
  onCloseBundle,
  onRefreshLogs,
}: LogsViewProps) {
  const { m } = useI18n();

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
                <span>{m.logs.backToWorkbench}</span>
              </button>
            )}
            <h2 style={{ margin: 0, fontSize: 17, fontWeight: 700 }}>
              {m.logs.title}
            </h2>
          </div>
          <div className="row">
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={onExportBundle}
            >
              {m.logs.exportBundle}
            </button>
            <button
              type="button"
              className="btn btn-outline btn-sm"
              onClick={onRefreshLogs}
            >
              <RefreshCw size={14} strokeWidth={1.5} />
              <span>{m.logs.refresh}</span>
            </button>
          </div>
        </div>

        {diagnosticBundle && (
          <div className="surface-panel stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <span className="mono">diagnostic-bundle.json</span>
              <button
                type="button"
                className="icon-btn"
                onClick={onCloseBundle}
                aria-label={m.logs.closeBundleAria}
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

        <div className="surface-panel" style={{ padding: "8px 12px" }}>
          {logs.length === 0 ? (
            <div className="muted" style={{ padding: 10 }}>
              {m.logs.emptyLogs}
            </div>
          ) : (
            logs.map((entry, idx) => (
              <div key={idx} className="log-line">
                [{entry.level.toUpperCase()}] ({entry.category}) {entry.message}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
