import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import { HudOverlayFeatureItem, HudOverlayState, nekodApi } from "./api/nekod";

function displayScore(item: HudOverlayFeatureItem): number {
  const tagLen = item.tag ? item.tag.length + 3 : 0;
  const hkLen = item.hotkey ? item.hotkey.length + 3 : 0;
  return item.label.length + tagLen + hkLen * 0.5;
}

export default function HudOverlay() {
  const [state, setState] = useState<HudOverlayState>({
    enabled: true,
    game_title: "Seed of the Dead",
    bridge_tag: "UE4SS v0.7",
    target_pid: null,
    attached: false,
    features: [],
  });

  useEffect(() => {
    let mounted = true;
    void nekodApi
      .getHudOverlayState()
      .then((snap) => {
        if (mounted && snap) {
          setState(snap);
        }
      })
      .catch(() => {
        // Ignore initial snapshot read failure outside Tauri runtime
      });

    let unlisten: (() => void) | null = null;
    void nekodApi
      .onHudOverlayState((next) => {
        if (mounted) {
          setState(next);
        }
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        // Ignore listener setup failure
      });

    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  const sortedFeatures = useMemo(() => {
    return [...state.features].sort((a, b) => {
      const aMuted = a.muted ? 1 : 0;
      const bMuted = b.muted ? 1 : 0;
      if (aMuted !== bMuted) return aMuted - bMuted;
      const diff = displayScore(b) - displayScore(a);
      if (Math.abs(diff) > 0.01) return diff;
      return a.label.localeCompare(b.label);
    });
  }, [state.features]);

  const gameBadge = state.game_title
    ? state.game_title.slice(0, 22)
    : "NEKOD";

  return (
    <div className="hud-overlay-root" aria-hidden="true">
      <div className="hud-watermark">
        <span className="hud-rgb-text hud-brand-word">NEKOD</span>
        <span className="hud-wm-sep">|</span>
        <span className="hud-wm-game">{gameBadge}</span>
        {state.bridge_tag ? (
          <>
            <span className="hud-wm-sep">|</span>
            <span className="hud-wm-bridge">{state.bridge_tag}</span>
          </>
        ) : null}
        <span className={`hud-wm-status ${state.attached ? "live" : "idle"}`}>
          {state.attached ? "LIVE" : "IDLE"}
        </span>
      </div>

      {sortedFeatures.length > 0 ? (
        <ul className="hud-arraylist">
          {sortedFeatures.map((item, idx) => (
            <li
              key={item.id}
              className={`hud-array-item ${item.muted ? "is-muted" : ""}`}
              style={{ "--row-idx": idx } as CSSProperties}
            >
              {item.hotkey ? (
                <span className="hud-item-hotkey">{item.hotkey}</span>
              ) : null}
              {item.tag ? (
                <span className="hud-item-tag">[{item.tag}]</span>
              ) : null}
              {item.muted ? (
                <>
                  <span className="hud-muted-label">{item.label}</span>
                  <span className="hud-muted-bar" />
                </>
              ) : (
                <>
                  <span className="hud-rgb-text hud-item-label">
                    {item.label}
                  </span>
                  <span className="hud-rgb-bar" />
                </>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <div className="hud-empty-hint">
          <span className="hud-rgb-text">READY</span>
          <span>Toggle modules in Nekod</span>
        </div>
      )}
    </div>
  );
}
