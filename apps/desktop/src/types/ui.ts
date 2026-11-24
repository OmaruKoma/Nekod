import type {
  GameEnvironmentProfile,
  HudOverlayFeatureItem,
  RuntimeDiagnostics,
  TrainerFeature,
  TrustTier,
} from "../api/nekod";
import {
  translateRuntimeError,
  translateTrust,
  type Locale,
} from "../i18n";

export type NavTab = "workbench" | "community" | "import" | "settings" | "logs";
export type SidebarFilter = "all" | "favorites" | "running" | "modded";

export interface HudToast {
  id: number;
  text: string;
  tone: "info" | "ok" | "warn";
}

export interface ActiveTrainerSession {
  sessionId: string;
  gameKey: string;
  trainerId: string;
  trainerName: string;
  targetPid: number;
  features: TrainerFeature[];
  diag: RuntimeDiagnostics | null;
  toggles: Record<string, boolean>;
  triggeredActions: Record<string, boolean>;
}

export const ENGINE_LABELS: Record<GameEnvironmentProfile["engine"], string> = {
  unity_mono: "Unity (Mono)",
  unity_il2cpp: "Unity (IL2CPP)",
  unreal_engine: "Unreal Engine",
  re_engine: "RE Engine",
  godot: "Godot",
  native_unknown: "Native / Unknown",
};

export const FAV_STORAGE_KEY = "nekod.favoriteGames.v1";
export const SOUND_STORAGE_KEY = "nekod.soundFxEnabled.v1";
export const AUTO_ATTACH_STORAGE_KEY = "nekod.autoAttachEnabled.v1";
export const HUD_OVERLAY_STORAGE_KEY = "nekod.hud_overlay.v1";
export const MINIMIZE_TO_TRAY_STORAGE_KEY = "nekod.minimizeToTray.v1";

export const OVERLAY_FEATURE_LABELS: Record<string, { label: string; tag?: string }> = {
  "god-mode": { label: "GodMode", tag: "Invincible" },
  "infinite-ammo": { label: "InfAmmo", tag: "99/99" },
  "no-recoil": { label: "NoRecoil", tag: "0-Spread" },
  "infinite-stamina": { label: "InfStamina", tag: "Max" },
  "infinite-grenades": { label: "InfGrenades", tag: "Max" },
  "infinite-seiryoku-powerup": { label: "PowerUp", tag: "Seiryoku" },
  "auto-lock-enemy": { label: "AimLock", tag: "Neck/Near" },
  "auto-lock-ads": { label: "ADSAimbot", tag: "Neck/ADS" },
  "auto-fire": { label: "AutoFire", tag: "Rapid Trigger" },
  "spinbot-third-person": { label: "SpinBot", tag: "3rd-Person" },
  "spinbot-armature": { label: "SpinBot", tag: "720° LateUpdate" },
  "god-mode-hp": { label: "GodMode", tag: "99999 HP" },
  "infinite-burst-boost": { label: "InfBurst", tag: "9999" },
  "super-armor": { label: "SuperArmor", tag: "Unbreakable" },
  "paralyze-guard": { label: "ParaGuard", tag: "Immune" },
  "shot-guard": { label: "ShotGuard", tag: "Deflect" },
  "undress-female": { label: "Undress", tag: "Female" },
  "undress-male": { label: "Undress", tag: "Male" },
  "uncensor-material-slot": { label: "UncensorSlot", tag: "Verified PNG" },
  "playable-lily": { label: "PlayableLily", tag: "CharSwap" },
  "lily-god-mode": { label: "LilyGodMode", tag: "Inf HP/MP" },
  "lily-ex-combat-kit": { label: "LilyEXKit", tag: "EX/Homing" },
  "lily-super-armor": { label: "LilyArmor", tag: "SuperArmor" },
  "lily-active-bits": { label: "LilyBits", tag: "Funnels" },
  "lily-move-speed": { label: "LilySpeed" },
  "reload-battle-now": { label: "ReloadBattle" },
  "lily-sword-mode": { label: "SwordMode" },
  "lily-gun-mode": { label: "GunMode" },
  "lily-multiple-homing": { label: "MultiHoming" },
  "lily-defence-homing": { label: "DefHoming" },
  "lily-lockon-all": { label: "LockonAll" },
  "lily-unique-gun": { label: "LilyBeam", tag: "UniqueG" },
  "lily-unique-sword": { label: "LilyNuke", tag: "UniqueS" },
  "refill-hp": { label: "RefillHP" },
  "costume-naked": { label: "CostumeNaked" },
  "costume-combat-heavy": { label: "CostumeHeavy" },
  "costume-swim1": { label: "CostumeSwim" },
  "time-pause": { label: "FreezeTime", tag: "Pause" },
  "kill-aura": { label: "KillAura", tag: "35m Native" },
  "debug-camera": { label: "FreeCam", tag: "Debug" },
  "one-hit-kill": { label: "OneHitKill", tag: "99999" },
  "full-auto-fire": { label: "FullAuto", tag: "Continuous" },
  "no-move-slowdown": { label: "NoSlowdown", tag: "Walk/Fire" },
  "rare-loot-finder": { label: "RareLoot", tag: "5.0x Score" },
  "damage-multiplier": { label: "DamageMult" },
  "move-speed": { label: "MoveSpeed" },
  "reload-speed": { label: "FastReload" },
  "game-speed": { label: "GameSpeed" },
};

export function englishOverlayTitle(id: string): string {
  return id
    .split(/[-_.]+/)
    .filter(Boolean)
    .map((seg) => seg[0].toUpperCase() + seg.slice(1))
    .join("");
}

export function formatOverlayFeatureItem(
  feature: TrainerFeature,
  sliderVal: string | undefined,
  hotkey: string | null,
  muted = false,
  compositeId?: string,
): HudOverlayFeatureItem {
  const mapped = OVERLAY_FEATURE_LABELS[feature.id];
  const label = mapped?.label ?? englishOverlayTitle(feature.id);
  let tag = mapped?.tag ?? null;
  if (feature.control_type === "float" || feature.control_type === "integer") {
    const raw = sliderVal?.trim();
    if (raw) {
      tag = feature.control_type === "float" ? `${raw}x` : raw;
    }
  }
  return {
    id: compositeId ?? feature.id,
    label,
    tag,
    hotkey: hotkey?.trim() ? hotkey.trim() : null,
    muted,
  };
}

export function loadStoredFavorites(): Set<string> {
  try {
    const raw = localStorage.getItem(FAV_STORAGE_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw) as unknown;
    return Array.isArray(arr)
      ? new Set(arr.filter((v): v is string => typeof v === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

export function loadStoredBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    return raw === "true";
  } catch {
    return fallback;
  }
}

export function gameMonogram(title: string): string {
  const words = title.trim().split(/\s+/).filter(Boolean);
  if (words.length >= 2) {
    return (words[0][0] + words[1][0]).toUpperCase();
  }
  return title.slice(0, 2).toUpperCase() || "GM";
}

export function trustLabel(tier: TrustTier, locale: Locale = "zh-CN"): string {
  return translateTrust(tier, locale);
}

export function trustTagClass(tier: TrustTier): string {
  if (tier === "official") return "tag official";
  if (tier === "community_verified") return "tag community";
  return "tag unverified";
}

export function normalizePathForCompare(raw: string): string {
  return raw
    .trim()
    .replace(/^"+|"+$/g, "")
    .replace(/[\\/]+$/, "")
    .toLowerCase();
}

export function inferInstallDirFromInput(raw: string): string {
  const cleaned = raw.trim().replace(/^"+|"+$/g, "").replace(/[\\/]+$/, "");
  if (cleaned.toLowerCase().endsWith(".exe")) {
    const sep = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
    return sep > 0 ? cleaned.slice(0, sep) : cleaned;
  }
  return cleaned;
}

export function findDuplicateHotkeys(hotkeys: Record<string, string>): string[] {
  const counts = new Map<string, number>();
  for (const raw of Object.values(hotkeys)) {
    const norm = raw.trim().toLowerCase();
    if (!norm) continue;
    counts.set(norm, (counts.get(norm) ?? 0) + 1);
  }
  return [...counts.entries()].filter(([, c]) => c > 1).map(([k]) => k);
}

/**
 * Formats a KeyboardEvent into a canonical hotkey string compatible with Nekod's
 * global hotkey parser (`Ctrl+Shift+K`, `Alt+1`, `F8`, `Numpad1`, etc.), or
 * returns `null` when only modifier keys were pressed.
 */
export function formatHotkeyFromKeyboardEvent(e: KeyboardEvent): string | null {
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
    return null;
  }
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");

  let keyName = e.key;
  if (e.code.startsWith("Numpad") && e.code.length > 6) {
    const tail = e.code.slice(6);
    if (/^[0-9]$/.test(tail)) {
      keyName = `Numpad${tail}`;
    } else if (tail === "Add") keyName = "NumpadAdd";
    else if (tail === "Subtract") keyName = "NumpadSub";
    else if (tail === "Multiply") keyName = "NumpadMul";
    else if (tail === "Divide") keyName = "NumpadDiv";
  } else if (/^Digit[0-9]$/.test(e.code)) {
    keyName = e.code.slice(5);
  } else if (/^Key[A-Z]$/.test(e.code)) {
    keyName = e.code.slice(3);
  } else if (keyName === " ") {
    keyName = "Space";
  } else if (keyName.length === 1) {
    keyName = keyName.toUpperCase();
  }

  parts.push(keyName);
  return parts.join("+");
}

export function formatRuntimeError(
  raw: string,
  locale: Locale = "zh-CN",
): string {
  return translateRuntimeError(raw, locale);
}
