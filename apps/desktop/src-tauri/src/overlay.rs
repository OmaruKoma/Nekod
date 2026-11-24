//! Click-through transparent RGB HUD overlay ("ArrayList") window manager.
//!
//! Renders active modules on the top-right corner of the attached game window (when windowed or
//! borderless-windowed) or primary display using an isolated `hud-overlay` Tauri webview.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl,
    WebviewWindowBuilder,
};

pub const OVERLAY_WINDOW_LABEL: &str = "hud-overlay";
pub const OVERLAY_STATE_EVENT: &str = "nekod://hud-overlay-state";
pub const OVERLAY_WIDTH_PX: u32 = 480;
pub const OVERLAY_HEIGHT_PX: u32 = 760;
const OVERLAY_MARGIN_PX: i32 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudOverlayFeatureItem {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub hotkey: Option<String>,
    #[serde(default)]
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudOverlayState {
    pub enabled: bool,
    pub game_title: String,
    #[serde(default)]
    pub bridge_tag: String,
    #[serde(default)]
    pub target_pid: Option<u32>,
    #[serde(default)]
    pub attached: bool,
    #[serde(default)]
    pub features: Vec<HudOverlayFeatureItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayAnchorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Computes the top-right physical window rectangle for the HUD overlay scaled for display DPI.
pub fn compute_overlay_anchor_rect_scaled(
    game_client_rect: Option<(i32, i32, u32, u32)>,
    screen_width: i32,
    scale_factor: f64,
) -> OverlayAnchorRect {
    let scale = if scale_factor.is_finite() {
        scale_factor.clamp(1.0, 3.0)
    } else {
        1.0
    };
    let width = (f64::from(OVERLAY_WIDTH_PX) * scale).round() as u32;
    let height = (f64::from(OVERLAY_HEIGHT_PX) * scale).round() as u32;
    let margin = (f64::from(OVERLAY_MARGIN_PX) * scale).round() as i32;

    if let Some((gx, gy, gw, gh)) = game_client_rect {
        let max_w = gw.saturating_sub((margin * 2) as u32).max(240);
        let clamped_w = width.min(max_w);
        let x = (gx + (gw as i32) - (clamped_w as i32) - margin).max(gx);
        let y = gy + margin;
        let clamped_h = height.min(gh.saturating_sub((margin * 2) as u32).max(220));
        return OverlayAnchorRect {
            x,
            y,
            width: clamped_w,
            height: clamped_h,
        };
    }

    let safe_screen_w = screen_width.max((width as i32) + margin * 2);
    OverlayAnchorRect {
        x: safe_screen_w - (width as i32) - margin,
        y: margin,
        width,
        height,
    }
}

/// Computes the top-right physical window rectangle for the HUD overlay at 1.0x scale.
#[cfg(test)]
pub fn compute_overlay_anchor_rect(
    game_client_rect: Option<(i32, i32, u32, u32)>,
    screen_width: i32,
) -> OverlayAnchorRect {
    compute_overlay_anchor_rect_scaled(game_client_rect, screen_width, 1.0)
}

pub fn sanitize_overlay_state(mut state: HudOverlayState) -> HudOverlayState {
    state.game_title = state.game_title.trim().chars().take(48).collect();
    state.bridge_tag = state.bridge_tag.trim().chars().take(32).collect();
    state.features.truncate(40);
    for item in &mut state.features {
        item.id = item.id.trim().chars().take(128).collect();
        item.label = item.label.trim().chars().take(48).collect();
        item.tag = item
            .tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(24).collect());
        item.hotkey = item
            .hotkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(16).collect());
    }
    state.features.retain(|item| !item.label.is_empty());
    state
}

#[derive(Default, Clone)]
pub struct HudOverlayService {
    state: Arc<Mutex<HudOverlayState>>,
    window_op_lock: Arc<Mutex<()>>,
}

impl HudOverlayService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> HudOverlayState {
        self.state
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Updates HUD overlay state and creates/shows/hides/positions the transparent overlay window.
    pub fn sync_window<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        raw_next: HudOverlayState,
    ) -> Result<HudOverlayState, String> {
        let initial = sanitize_overlay_state(raw_next);
        if let Ok(mut guard) = self.state.lock() {
            *guard = initial;
        }

        let _op_guard = self
            .window_op_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let next = self.snapshot();

        if !next.enabled {
            if let Some(win) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
                let _ = win.hide();
            }
            let _ = app.emit_to(OVERLAY_WINDOW_LABEL, OVERLAY_STATE_EVENT, &next);
            return Ok(next);
        }

        let win = ensure_overlay_window(app)?;
        let _ = win.set_ignore_cursor_events(true);

        let game_rect = next.target_pid.and_then(find_game_client_rect);
        let scale = win.scale_factor().unwrap_or(1.0);
        let anchor = compute_overlay_anchor_rect_scaled(game_rect, primary_screen_width(), scale);
        let _ = win.set_size(Size::Physical(PhysicalSize::new(
            anchor.width,
            anchor.height,
        )));
        let _ = win.set_position(Position::Physical(PhysicalPosition::new(
            anchor.x, anchor.y,
        )));
        let _ = app.emit_to(OVERLAY_WINDOW_LABEL, OVERLAY_STATE_EVENT, &next);

        let should_show = next.target_pid.is_none() || game_rect.is_some();
        if should_show {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }

        Ok(next)
    }

    /// Starts a lightweight background tracker keeping the HUD overlay anchored to the target game window.
    pub fn start_tracker<R: tauri::Runtime>(&self, app: tauri::AppHandle<R>) {
        let state = Arc::clone(&self.state);
        let window_op_lock = Arc::clone(&self.window_op_lock);
        let _ = thread::Builder::new()
            .name("nekod-hud-overlay-tracker".into())
            .spawn(move || {
                let mut last_rect: Option<(Option<OverlayAnchorRect>, bool)> = None;
                loop {
                    let snap = state.lock().map(|g| g.clone()).unwrap_or_default();
                    if !snap.enabled {
                        last_rect = None;
                        thread::sleep(Duration::from_millis(350));
                        continue;
                    }

                    let Ok(_op_guard) = window_op_lock.try_lock() else {
                        thread::sleep(Duration::from_millis(120));
                        continue;
                    };

                    if let Some(win) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
                        let game_rect = snap.target_pid.and_then(find_game_client_rect);
                        let should_show = snap.target_pid.is_none() || game_rect.is_some();
                        let scale = win.scale_factor().unwrap_or(1.0);
                        let anchor = should_show.then(|| {
                            compute_overlay_anchor_rect_scaled(
                                game_rect,
                                primary_screen_width(),
                                scale,
                            )
                        });
                        let next_key = Some((anchor, should_show));
                        if next_key != last_rect {
                            last_rect = next_key;
                            let _ = win.set_ignore_cursor_events(true);
                            if let Some(rect) = anchor {
                                let _ = win.set_size(Size::Physical(PhysicalSize::new(
                                    rect.width,
                                    rect.height,
                                )));
                                let _ = win.set_position(Position::Physical(
                                    PhysicalPosition::new(rect.x, rect.y),
                                ));
                                let _ = win.show();
                            } else {
                                let _ = win.hide();
                            }
                        }
                    } else {
                        last_rect = None;
                    }

                    drop(_op_guard);
                    thread::sleep(Duration::from_millis(120));
                }
            });
    }
}

fn ensure_overlay_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<tauri::WebviewWindow<R>, String> {
    if let Some(existing) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
        return Ok(existing);
    }

    WebviewWindowBuilder::new(
        app,
        OVERLAY_WINDOW_LABEL,
        WebviewUrl::App("index.html?overlay=hud".into()),
    )
    .title("Nekod HUD Overlay")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focusable(false)
    .resizable(false)
    .visible(false)
    .inner_size(f64::from(OVERLAY_WIDTH_PX), f64::from(OVERLAY_HEIGHT_PX))
    .build()
    .map_err(|e| format!("failed creating HUD overlay window: {e}"))
}

#[cfg(target_os = "windows")]
fn primary_screen_width() -> i32 {
    unsafe extern "system" {
        fn GetSystemMetrics(n_index: i32) -> i32;
    }
    // SAFETY: `GetSystemMetrics(0)` is a read-only Win32 user32 query returning primary screen width.
    let w = unsafe { GetSystemMetrics(0) };
    if w > 320 { w } else { 1920 }
}

#[cfg(not(target_os = "windows"))]
fn primary_screen_width() -> i32 {
    1920
}

#[cfg(target_os = "windows")]
fn find_game_client_rect(target_pid: u32) -> Option<(i32, i32, u32, u32)> {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct WinPoint {
        x: i32,
        y: i32,
    }

    struct SearchCtx {
        target_pid: u32,
        best_area: i64,
        best_rect: Option<(i32, i32, u32, u32)>,
    }

    unsafe extern "system" {
        fn EnumWindows(
            lp_enum_func: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            l_param: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, lpdw_process_id: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn IsIconic(hwnd: *mut c_void) -> i32;
        fn GetClientRect(hwnd: *mut c_void, lp_rect: *mut WinRect) -> i32;
        fn ClientToScreen(hwnd: *mut c_void, lp_point: *mut WinPoint) -> i32;
    }

    unsafe extern "system" fn enum_cb(hwnd: *mut c_void, l_param: isize) -> i32 {
        // SAFETY: `l_param` points to `SearchCtx` live for the synchronous duration of `EnumWindows`.
        let ctx = unsafe { &mut *(l_param as *mut SearchCtx) };
        let mut pid = 0u32;
        // SAFETY: Valid `hwnd` provided by `EnumWindows` and valid stack pointer `&mut pid`.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid != ctx.target_pid {
            return 1;
        }
        // SAFETY: Read-only Win32 queries on an enumerated top-level HWND.
        if unsafe { IsWindowVisible(hwnd) } == 0 || unsafe { IsIconic(hwnd) } != 0 {
            return 1;
        }
        let mut rc = WinRect::default();
        let mut pt = WinPoint::default();
        // SAFETY: Valid stack buffers passed to Win32 client-rect queries for an existing HWND.
        if unsafe { GetClientRect(hwnd, &mut rc) } == 0
            || unsafe { ClientToScreen(hwnd, &mut pt) } == 0
        {
            return 1;
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w < 320 || h < 240 {
            return 1;
        }
        let area = i64::from(w) * i64::from(h);
        if area > ctx.best_area {
            ctx.best_area = area;
            ctx.best_rect = Some((pt.x, pt.y, w as u32, h as u32));
        }
        1
    }

    if target_pid == 0 {
        return None;
    }
    let mut ctx = SearchCtx {
        target_pid,
        best_area: 0,
        best_rect: None,
    };
    // SAFETY: `ctx` lives on the stack for the entire synchronous `EnumWindows` call.
    unsafe {
        EnumWindows(enum_cb, (&raw mut ctx) as isize);
    }
    ctx.best_rect
}

#[cfg(not(target_os = "windows"))]
fn find_game_client_rect(_target_pid: u32) -> Option<(i32, i32, u32, u32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_overlay_state_sanitizes_and_computes_anchor_rect() {
        let raw = HudOverlayState {
            enabled: true,
            game_title: "   Seed of the Dead: Sweet Home   ".to_owned(),
            bridge_tag: "  UE4SS v0.7 · 2M  ".to_owned(),
            target_pid: Some(1234),
            attached: true,
            features: vec![
                HudOverlayFeatureItem {
                    id: "spinbot-third-person".to_owned(),
                    label: "  SpinBot  ".to_owned(),
                    tag: Some("  3rd-Person  ".to_owned()),
                    hotkey: Some(" ".to_owned()),
                    muted: false,
                },
                HudOverlayFeatureItem {
                    id: "empty".to_owned(),
                    label: "   ".to_owned(),
                    tag: None,
                    hotkey: None,
                    muted: false,
                },
            ],
        };

        let clean = sanitize_overlay_state(raw);
        assert_eq!(clean.game_title, "Seed of the Dead: Sweet Home");
        assert_eq!(clean.bridge_tag, "UE4SS v0.7 · 2M");
        assert_eq!(clean.features.len(), 1);
        assert_eq!(clean.features[0].label, "SpinBot");
        assert_eq!(clean.features[0].tag.as_deref(), Some("3rd-Person"));
        assert!(clean.features[0].hotkey.is_none());
        assert!(!clean.features[0].muted);

        let attached_rect = compute_overlay_anchor_rect(Some((100, 50, 1280, 720)), 1920);
        assert_eq!(attached_rect.x, 100 + 1280 - (OVERLAY_WIDTH_PX as i32) - 16);
        assert_eq!(attached_rect.y, 50 + 16);
        assert_eq!(attached_rect.width, OVERLAY_WIDTH_PX);
        assert_eq!(attached_rect.height, 720 - 32);

        let fallback_rect = compute_overlay_anchor_rect(None, 2560);
        assert_eq!(fallback_rect.x, 2560 - (OVERLAY_WIDTH_PX as i32) - 16);
        assert_eq!(fallback_rect.y, 16);
    }

    #[test]
    fn computes_dpi_scaled_overlay_rect_and_preserves_muted_action_items() {
        let composite_id = "org.nekod.aidealrays-playable-lily:lily-multiple-homing";
        let state = sanitize_overlay_state(HudOverlayState {
            enabled: true,
            game_title: "AIdealRays".to_owned(),
            bridge_tag: "IL2CPP · 2M".to_owned(),
            target_pid: Some(42),
            attached: true,
            features: vec![HudOverlayFeatureItem {
                id: composite_id.to_owned(),
                label: "MultiHoming".to_owned(),
                tag: Some("Action".to_owned()),
                hotkey: Some("F6".to_owned()),
                muted: true,
            }],
        });

        assert_eq!(state.features.len(), 1);
        assert_eq!(state.features[0].id, composite_id);
        assert!(state.features[0].muted);

        let scaled = compute_overlay_anchor_rect_scaled(Some((0, 0, 1920, 1080)), 2560, 1.5);
        assert_eq!(scaled.width, 720);
        assert_eq!(scaled.height, 1080 - 48);
        assert_eq!(scaled.x, 1920 - 720 - 24);
        assert_eq!(scaled.y, 24);
    }

    #[test]
    fn snapshot_remains_non_blocking_while_window_op_lock_is_held() {
        let service = HudOverlayService::new();
        let _op_guard = service.window_op_lock.lock().expect("window_op_lock");

        {
            let mut guard = service.state.lock().expect("state lock");
            guard.enabled = true;
            guard.game_title = "AIdealRays".to_owned();
            guard.attached = true;
        }

        let clone = service.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(clone.snapshot());
        });

        let snap = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("snapshot must not block on window_op_lock");
        assert!(snap.enabled);
        assert!(snap.attached);
        assert_eq!(snap.game_title, "AIdealRays");
    }
}
