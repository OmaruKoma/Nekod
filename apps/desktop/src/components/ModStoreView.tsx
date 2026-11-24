import { useMemo, useState } from "react";
import {
  Check,
  ChevronRight,
  Download,
  Globe,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import type {
  CommunityTrainerView,
  RegistrySourceState,
  TrainerUpdateInfo,
} from "../api/nekod";
import { useI18n } from "../i18n";
import { trustLabel, trustTagClass } from "../types/ui";

interface ModStoreViewProps {
  allCommunityTrainers: CommunityTrainerView[];
  registrySources: RegistrySourceState[];
  updates: TrainerUpdateInfo[];
  repoIdInput: string;
  repoUrlInput: string;
  busyAction: string | null;
  onRepoIdChange: (v: string) => void;
  onRepoUrlChange: (v: string) => void;
  onSyncRegistries: () => void;
  onInstallOrUpdate: (repoId: string, trainerId: string) => void;
  onAddThirdPartyRepo: () => void;
  onRemoveRepo: (repoId: string) => void;
}

export function ModStoreView({
  allCommunityTrainers,
  registrySources,
  updates,
  repoIdInput,
  repoUrlInput,
  busyAction,
  onRepoIdChange,
  onRepoUrlChange,
  onSyncRegistries,
  onInstallOrUpdate,
  onAddThirdPartyRepo,
  onRemoveRepo,
}: ModStoreViewProps) {
  const { locale, m } = useI18n();
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<
    "all" | "not_installed" | "installed" | "updates"
  >("all");

  const filteredCatalog = useMemo(() => {
    const q = query.trim().toLowerCase();
    return allCommunityTrainers.filter((ct) => {
      if (
        statusFilter === "not_installed" &&
        ct.install_state.state !== "not_installed"
      ) {
        return false;
      }
      if (
        statusFilter === "installed" &&
        ct.install_state.state !== "installed_up_to_date" &&
        ct.install_state.state !== "update_available"
      ) {
        return false;
      }
      if (
        statusFilter === "updates" &&
        ct.install_state.state !== "update_available"
      ) {
        return false;
      }
      if (!q) return true;
      return (
        ct.trainer_id.toLowerCase().includes(q) ||
        ct.game_id.toLowerCase().includes(q) ||
        ct.repository_id.toLowerCase().includes(q)
      );
    });
  }, [allCommunityTrainers, query, statusFilter]);

  return (
    <div className="page-scroll">
      <div className="page-inner stack" style={{ gap: 16 }}>
        {/* Store Header Banner */}
        <section className="store-hero">
          <div style={{ flex: 1, minWidth: 260 }}>
            <div className="row" style={{ gap: 8 }}>
              <Globe size={17} strokeWidth={1.5} style={{ color: "var(--brand)" }} />
              <h2 style={{ margin: 0, fontSize: 17, fontWeight: 700 }}>
                {m.store.title}
              </h2>
              <span className="tag official">{m.store.badge}</span>
            </div>
            <p className="muted" style={{ margin: "6px 0 0" }}>
              {m.store.subtitlePrefix}{" "}
              <span className="mono">OmaruKoma/Nekod-Mods</span>{" "}
              {m.store.subtitleMiddle}{" "}
              <span className="mono">registry.json</span>{" "}
              {m.store.subtitleSuffix}
            </p>
          </div>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busyAction === "sync"}
            onClick={onSyncRegistries}
          >
            <RefreshCw
              size={14}
              strokeWidth={1.5}
              className={busyAction === "sync" ? "spin" : ""}
            />
            <span>
              {busyAction === "sync" ? m.store.syncingAll : m.store.syncAll}
            </span>
          </button>
        </section>

        {/* Updates Strip */}
        {updates.length > 0 && (
          <div className="surface-panel stack">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <div className="row" style={{ gap: 6 }}>
                <Sparkles size={15} strokeWidth={1.5} style={{ color: "var(--brand)" }} />
                <strong>{m.store.updatesAvailable(updates.length)}</strong>
              </div>
              <span className="muted">{m.store.updatesReconfirmHint}</span>
            </div>
            {updates.map((u) => (
              <div
                key={`${u.repository_id}:${u.trainer_id}`}
                className="flat-row"
              >
                <div>
                  <div className="row">
                    <span style={{ fontWeight: 600 }}>{u.trainer_id}</span>
                    <span className="tag">{u.game_id}</span>
                    <span className={trustTagClass(u.trust_tier)}>
                      {trustLabel(u.trust_tier, locale)}
                    </span>
                  </div>
                  <div className="muted mono" style={{ marginTop: 2 }}>
                    v{u.installed_version} -&gt; v{u.available_version} ·{" "}
                    {u.repository_id}
                  </div>
                </div>
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  disabled={busyAction === `install:${u.trainer_id}`}
                  onClick={() => onInstallOrUpdate(u.repository_id, u.trainer_id)}
                >
                  <Download size={14} strokeWidth={1.5} />
                  <span>
                    {busyAction === `install:${u.trainer_id}`
                      ? m.store.updating
                      : m.store.updateTo(u.available_version)}
                  </span>
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Store Filter Bar */}
        <div className="store-toolbar">
          <div className="store-filter-group">
            <button
              type="button"
              className={`store-filter-tab ${statusFilter === "all" ? "active" : ""}`}
              onClick={() => setStatusFilter("all")}
            >
              <span>{m.store.filterAll}</span>
              <span className="store-filter-count">{allCommunityTrainers.length}</span>
            </button>
            <button
              type="button"
              className={`store-filter-tab ${statusFilter === "not_installed" ? "active" : ""}`}
              onClick={() => setStatusFilter("not_installed")}
            >
              <span>{m.store.filterNotInstalled}</span>
            </button>
            <button
              type="button"
              className={`store-filter-tab ${statusFilter === "installed" ? "active" : ""}`}
              onClick={() => setStatusFilter("installed")}
            >
              <span>{m.store.filterInstalled}</span>
            </button>
            <button
              type="button"
              className={`store-filter-tab ${statusFilter === "updates" ? "active" : ""}`}
              onClick={() => setStatusFilter("updates")}
            >
              <span>{m.store.filterUpdates}</span>
            </button>
          </div>

          <div className="store-search-wrap">
            <Search size={14} strokeWidth={1.5} className="store-search-icon" />
            <input
              className="store-search-input"
              placeholder={m.store.searchPlaceholder}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
        </div>

        {/* Store Mod Cards */}
        {filteredCatalog.length === 0 ? (
          <div className="surface-panel stack">
            <div style={{ fontWeight: 600 }}>{m.store.emptyTitle}</div>
            <div className="muted">
              {m.store.emptyHintPrefix}{" "}
              <span className="mono">Nekod-Mods/registry.json</span>{" "}
              {m.store.emptyHintSuffix}
            </div>
          </div>
        ) : (
          <div className="store-grid">
            {filteredCatalog.map((ct) => {
              const isBusy = busyAction === `install:${ct.trainer_id}`;
              return (
                <article
                  key={`${ct.repository_id}:${ct.trainer_id}`}
                  className="store-card"
                >
                  <div className="store-card-top">
                    <span className="store-game-pill">{ct.game_id}</span>
                    <span className={trustTagClass(ct.trust_tier)}>
                      {trustLabel(ct.trust_tier, locale)}
                    </span>
                  </div>

                  <div className="store-card-body">
                    <h3 className="store-card-title">{ct.trainer_id}</h3>
                    <div className="store-card-meta mono">
                      <span>v{ct.latest_version}</span>
                      <span>·</span>
                      <span>{ct.repository_id}</span>
                    </div>
                    {ct.last_verified_game_version && (
                      <div className="muted" style={{ marginTop: 4 }}>
                        {m.store.verifiedGameVer(ct.last_verified_game_version)}
                      </div>
                    )}
                  </div>

                  <div className="store-card-footer">
                    {ct.compatibility_status && (
                      <span className="tag">{ct.compatibility_status}</span>
                    )}

                    <div style={{ marginLeft: "auto" }}>
                      {ct.install_state.state === "not_installed" && (
                        <button
                          type="button"
                          className="btn btn-primary btn-sm"
                          disabled={isBusy}
                          onClick={() =>
                            onInstallOrUpdate(ct.repository_id, ct.trainer_id)
                          }
                        >
                          <Download size={13} strokeWidth={1.5} />
                          <span>
                            {isBusy ? m.store.installing : m.store.installNmod}
                          </span>
                        </button>
                      )}
                      {ct.install_state.state === "installed_up_to_date" && (
                        <span className="tag official">
                          <Check size={12} strokeWidth={1.5} />
                          <span>{m.store.installed}</span>
                        </span>
                      )}
                      {ct.install_state.state === "update_available" && (
                        <button
                          type="button"
                          className="btn btn-outline btn-sm"
                          disabled={isBusy}
                          onClick={() =>
                            onInstallOrUpdate(ct.repository_id, ct.trainer_id)
                          }
                        >
                          <Download size={13} strokeWidth={1.5} />
                          <span>
                            {isBusy
                              ? m.store.updating
                              : m.store.updateTo(ct.install_state.latest_version)}
                          </span>
                        </button>
                      )}
                      {ct.install_state.state === "revoked" && (
                        <span className="tag unverified">revoked</span>
                      )}
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}

        {/* Collapsible Repository Sources Drawer */}
        <details className="drawer" open={registrySources.length === 0}>
          <summary>
            <span className="drawer-summary-left">
              <Globe size={14} strokeWidth={1.5} />
              <span>{m.store.sourcesDrawer(registrySources.length)}</span>
            </span>
            <ChevronRight size={14} strokeWidth={1.5} className="drawer-chevron" />
          </summary>
          <div className="drawer-body">
            {registrySources.map((s) => (
              <div key={s.source.repository_id} className="flat-row">
                <div style={{ minWidth: 0 }}>
                  <div className="row">
                    <span style={{ fontWeight: 500 }}>
                      {s.source.repository_id}
                    </span>
                    <span className={trustTagClass(s.source.authoritative_trust_tier)}>
                      {trustLabel(s.source.authoritative_trust_tier, locale)}
                    </span>
                    <span className="tag">{s.last_status}</span>
                  </div>
                  <div className="muted mono" style={{ marginTop: 2 }}>
                    {s.source.index_url}
                  </div>
                  {s.last_error && (
                    <div className="mono" style={{ color: "var(--danger)", marginTop: 2 }}>
                      {s.last_error}
                    </div>
                  )}
                </div>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => onRemoveRepo(s.source.repository_id)}
                >
                  <Trash2 size={13} strokeWidth={1.5} />
                  <span>{m.store.removeRepo}</span>
                </button>
              </div>
            ))}

            <div className="stack" style={{ paddingTop: 8 }}>
              <div style={{ fontWeight: 600 }}>{m.store.addThirdPartyTitle}</div>
              <div className="muted">
                {m.store.addThirdPartyDescPrefix}{" "}
                <span className="mono">registry.json</span>{" "}
                {m.store.addThirdPartyDescMiddle}{" "}
                <span className="mono">Unverified</span>
                {m.store.addThirdPartyDescSuffix}
              </div>
              <div className="row">
                <input
                  className="text-input mono"
                  style={{ width: 210 }}
                  placeholder={m.store.repoIdPlaceholder}
                  value={repoIdInput}
                  onChange={(e) => onRepoIdChange(e.target.value)}
                />
                <input
                  className="text-input mono"
                  style={{ flex: 1, minWidth: 220 }}
                  placeholder={m.store.repoUrlPlaceholder}
                  value={repoUrlInput}
                  onChange={(e) => onRepoUrlChange(e.target.value)}
                />
                <button
                  type="button"
                  className="btn btn-outline"
                  disabled={busyAction === "add-repo"}
                  onClick={onAddThirdPartyRepo}
                >
                  {m.store.addRepoBtn}
                </button>
              </div>
            </div>
          </div>
        </details>
      </div>
    </div>
  );
}
