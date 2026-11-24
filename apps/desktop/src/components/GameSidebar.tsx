import { Plus, RefreshCw, Search, Star } from "lucide-react";
import type {
  BridgeDeploymentStatus,
  GameArtwork,
  LibraryGameView,
} from "../api/nekod";
import { useI18n } from "../i18n";
import { gameMonogram, type SidebarFilter } from "../types/ui";

interface GameSidebarProps {
  games: LibraryGameView[];
  filteredGames: LibraryGameView[];
  selectedGame: LibraryGameView | null;
  search: string;
  sidebarFilter: SidebarFilter;
  favoriteKeys: Set<string>;
  runningGameKeys: Set<string>;
  activeGameKey: string | null;
  activeSessionCount: number;
  artworkByGame: Record<string, GameArtwork>;
  bridgeStatusByGame: Record<string, BridgeDeploymentStatus>;
  busyAction: string | null;
  onSearchChange: (val: string) => void;
  onFilterChange: (filter: SidebarFilter) => void;
  onOpenAddGame: () => void;
  onRescan: () => void;
  onSelectGame: (entry: LibraryGameView) => void;
  onToggleFavorite: (gameKey: string) => void;
}

export function GameSidebar({
  games,
  filteredGames,
  selectedGame,
  search,
  sidebarFilter,
  favoriteKeys,
  runningGameKeys,
  activeGameKey,
  activeSessionCount,
  artworkByGame,
  bridgeStatusByGame,
  busyAction,
  onSearchChange,
  onFilterChange,
  onOpenAddGame,
  onRescan,
  onSelectGame,
  onToggleFavorite,
}: GameSidebarProps) {
  const { m } = useI18n();

  return (
    <aside className="game-sidebar">
      <div className="sidebar-top">
        <div className="sidebar-back-row">
          <div className="sidebar-heading-group">
            <span className="sidebar-heading-title">{m.sidebar.heading}</span>
            <span className="sidebar-heading-count">{games.length}</span>
          </div>

          <div className="row" style={{ gap: 4 }}>
            <button
              type="button"
              className="icon-btn"
              onClick={onOpenAddGame}
              title={m.sidebar.addLocalGameTitle}
              aria-label={m.sidebar.addLocalGameAria}
            >
              <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="icon-btn"
              disabled={busyAction === "rescan"}
              onClick={onRescan}
              title={m.sidebar.rescanTitle}
              aria-label={m.sidebar.rescanAria}
            >
              <RefreshCw
                size={14}
                strokeWidth={1.5}
                className={busyAction === "rescan" ? "spin" : ""}
                aria-hidden="true"
              />
            </button>
          </div>
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
            placeholder={m.sidebar.searchPlaceholder}
            value={search}
            onChange={(e) => onSearchChange(e.target.value)}
          />
        </div>
      </div>

      <div className="segmented-wrap">
        <div
          className="segmented-control"
          role="tablist"
          aria-label={m.sidebar.filterAria}
        >
          <button
            type="button"
            className={`segmented-btn ${sidebarFilter === "all" ? "active" : ""}`}
            onClick={() => onFilterChange("all")}
          >
            <span>{m.sidebar.filterAll}</span>
            <span className="seg-count">{games.length}</span>
          </button>
          <button
            type="button"
            className={`segmented-btn ${sidebarFilter === "favorites" ? "active" : ""}`}
            onClick={() => onFilterChange("favorites")}
          >
            <span>{m.sidebar.filterFavorites}</span>
            <span className="seg-count">{favoriteKeys.size}</span>
          </button>
          <button
            type="button"
            className={`segmented-btn ${sidebarFilter === "running" ? "active" : ""}`}
            onClick={() => onFilterChange("running")}
          >
            <span>{m.sidebar.filterRunning}</span>
            <span className="seg-count">{runningGameKeys.size}</span>
          </button>
        </div>
      </div>

      <div className="sidebar-section-header">
        <span>{m.sidebar.gamesHeader(filteredGames.length)}</span>
        {activeSessionCount > 0 ? (
          <span className="sidebar-live-label">
            <span className="status-dot ok" />
            <span>
              {activeSessionCount > 1
                ? m.sidebar.injectedMany(activeSessionCount)
                : m.sidebar.injectedOne}
            </span>
          </span>
        ) : null}
      </div>

      <div className="game-list">
        {filteredGames.length === 0 ? (
          <div className="muted" style={{ padding: "10px 6px" }}>
            {m.sidebar.emptyFilter}
          </div>
        ) : (
          filteredGames.map((item) => {
            const isSelected = selectedGame?.library_key === item.library_key;
            const isInjected =
              activeSessionCount > 0 && activeGameKey === item.library_key;
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
                onClick={() => onSelectGame(item)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelectGame(item);
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
                        <span className="game-list-state-ok">live</span>
                      </>
                    )}
                    {!isInjected && isRunning && (
                      <>
                        <span>·</span>
                        <span className="game-list-state-run">run</span>
                      </>
                    )}
                  </div>
                </div>
                <button
                  type="button"
                  className={`fav-btn ${isFav ? "starred" : ""}`}
                  title={isFav ? m.sidebar.unfavorite : m.sidebar.favorite}
                  aria-label={isFav ? m.sidebar.unfavorite : m.sidebar.favorite}
                  onClick={(e) => {
                    e.stopPropagation();
                    onToggleFavorite(item.library_key);
                  }}
                >
                  <Star
                    size={13}
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
    </aside>
  );
}
