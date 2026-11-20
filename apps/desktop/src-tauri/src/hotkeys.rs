//! Lightweight, hookless Windows global hotkey watcher (`GetAsyncKeyState` edge detection).
//!
//! Avoids system-wide DLL hooks and privilege elevation while still allowing players to toggle
//! active mod features without leaving their fullscreen/borderless game window.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalHotkeyBinding {
    pub feature_id: String,
    pub hotkey: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalHotkeyEvent {
    pub feature_id: String,
    pub hotkey: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub vk: u16,
}

/// Parses human-readable hotkey specs like `"F1"`, `"Ctrl+F5"`, `"Alt+Shift+G"`, `"Numpad1"`.
pub fn parse_hotkey_spec(spec: &str) -> Option<ParsedHotkey> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut main_vk: Option<u16> = None;

    for raw_part in trimmed.split('+') {
        let part = raw_part.trim().to_ascii_uppercase();
        if part.is_empty() {
            return None;
        }
        match part.as_str() {
            "CTRL" | "CONTROL" => ctrl = true,
            "ALT" => alt = true,
            "SHIFT" => shift = true,
            other => {
                if main_vk.is_some() {
                    return None;
                }
                main_vk = parse_virtual_key(other);
                main_vk?;
            }
        }
    }

    Some(ParsedHotkey {
        ctrl,
        alt,
        shift,
        vk: main_vk?,
    })
}

fn parse_virtual_key(token: &str) -> Option<u16> {
    // Function keys F1..F12 (VK_F1 = 0x70 .. VK_F12 = 0x7B)
    if let Some(rest) = token.strip_prefix('F')
        && let Ok(n) = rest.parse::<u16>()
        && (1..=12).contains(&n)
    {
        return Some(0x6F + n);
    }

    // Numpad keys NUMPAD0..NUMPAD9 or NUM0..NUM9 (VK_NUMPAD0 = 0x60)
    if let Some(rest) = token
        .strip_prefix("NUMPAD")
        .or_else(|| token.strip_prefix("NUM"))
        && let Ok(n) = rest.parse::<u16>()
        && n <= 9
    {
        return Some(0x60 + n);
    }

    // Single ASCII digit or letter
    let bytes = token.as_bytes();
    if bytes.len() == 1 {
        let b = bytes[0];
        if b.is_ascii_digit() || b.is_ascii_uppercase() {
            return Some(u16::from(b));
        }
    }

    match token {
        "INSERT" | "INS" => Some(0x2D),
        "DELETE" | "DEL" => Some(0x2E),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PGUP" => Some(0x21),
        "PAGEDOWN" | "PGDN" => Some(0x22),
        "PLUS" | "ADD" => Some(0x6B),
        "MINUS" | "SUBTRACT" => Some(0x6D),
        "MULTIPLY" => Some(0x6A),
        "DIVIDE" => Some(0x6F),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ActiveEntry {
    feature_id: String,
    hotkey: String,
    parsed: ParsedHotkey,
}

#[derive(Default, Clone)]
pub struct GlobalHotkeyService {
    bindings: Arc<Mutex<Vec<ActiveEntry>>>,
}

impl GlobalHotkeyService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the currently monitored global hotkey bindings and returns the count of valid bindings.
    pub fn set_bindings(&self, next: Vec<GlobalHotkeyBinding>) -> usize {
        let mut parsed_list = Vec::new();
        for b in next {
            if let Some(parsed) = parse_hotkey_spec(&b.hotkey) {
                parsed_list.push(ActiveEntry {
                    feature_id: b.feature_id,
                    hotkey: b.hotkey,
                    parsed,
                });
            }
        }
        let count = parsed_list.len();
        if let Ok(mut guard) = self.bindings.lock() {
            *guard = parsed_list;
        }
        count
    }

    /// Spawns a single lightweight background polling thread that emits events on hotkey press edges.
    pub fn start_watcher<F>(&self, on_trigger: F)
    where
        F: Fn(GlobalHotkeyEvent) + Send + 'static,
    {
        let bindings = Arc::clone(&self.bindings);
        let _ = thread::Builder::new()
            .name("nekod-global-hotkeys".into())
            .spawn(move || {
                let mut prev_pressed: HashSet<String> = HashSet::new();
                loop {
                    let snapshot = bindings.lock().map(|g| g.clone()).unwrap_or_default();

                    if snapshot.is_empty() {
                        prev_pressed.clear();
                        thread::sleep(Duration::from_millis(200));
                        continue;
                    }

                    let mut currently_pressed = HashSet::new();
                    for entry in &snapshot {
                        if is_hotkey_pressed(&entry.parsed) {
                            currently_pressed.insert(entry.feature_id.clone());
                            if !prev_pressed.contains(&entry.feature_id) {
                                on_trigger(GlobalHotkeyEvent {
                                    feature_id: entry.feature_id.clone(),
                                    hotkey: entry.hotkey.clone(),
                                });
                            }
                        }
                    }
                    prev_pressed = currently_pressed;
                    thread::sleep(Duration::from_millis(45));
                }
            });
    }
}

#[cfg(target_os = "windows")]
fn is_vk_down(vk: u16) -> bool {
    unsafe extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    // SAFETY: `GetAsyncKeyState` is a pure read-only Win32 user32 query taking an integer VK code.
    let state = unsafe { GetAsyncKeyState(i32::from(vk)) };
    (state as u16 & 0x8000) != 0
}

#[cfg(not(target_os = "windows"))]
fn is_vk_down(_vk: u16) -> bool {
    false
}

fn is_hotkey_pressed(hk: &ParsedHotkey) -> bool {
    const VK_SHIFT: u16 = 0x10;
    const VK_CONTROL: u16 = 0x11;
    const VK_MENU: u16 = 0x12;

    if !is_vk_down(hk.vk) {
        return false;
    }
    if hk.ctrl != is_vk_down(VK_CONTROL) {
        return false;
    }
    if hk.alt != is_vk_down(VK_MENU) {
        return false;
    }
    if hk.shift != is_vk_down(VK_SHIFT) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_hotkey_specs_and_rejects_invalid_ones() {
        let f1 = parse_hotkey_spec("F1").expect("F1");
        assert_eq!(f1.vk, 0x70);
        assert!(!f1.ctrl && !f1.alt && !f1.shift);

        let combo = parse_hotkey_spec("Ctrl+Shift+F5").expect("Ctrl+Shift+F5");
        assert_eq!(combo.vk, 0x74);
        assert!(combo.ctrl && combo.shift && !combo.alt);

        let num = parse_hotkey_spec("Alt+Numpad3").expect("Alt+Numpad3");
        assert_eq!(num.vk, 0x63);
        assert!(num.alt && !num.ctrl);

        assert!(parse_hotkey_spec("").is_none());
        assert!(parse_hotkey_spec("Ctrl+").is_none());
        assert!(parse_hotkey_spec("F99").is_none());
        assert!(parse_hotkey_spec("A+B").is_none());
    }
}
