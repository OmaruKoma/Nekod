//! Offline local game artwork resolver (Steam `appcache/librarycache` & local game folder covers).
//!
//! Reads strictly validated image files (<= 2 MiB, JPEG/PNG/WEBP magic bytes) and returns
//! CSP-safe `data:image/...;base64,...` URIs without making external network requests.

use std::fs;
use std::path::{Path, PathBuf};

use nekod_game_detection::SteamDetector;
use serde::Serialize;

const MAX_ARTWORK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
pub struct GameArtwork {
    pub thumbnail_data_uri: Option<String>,
    pub header_data_uri: Option<String>,
}

/// Resolves local cover/header artwork for a discovered game.
pub fn resolve_game_artwork(
    store_source: &str,
    store_app_id: Option<&str>,
    install_dir: &str,
) -> GameArtwork {
    let steam_roots = SteamDetector::from_system().steam_roots().to_vec();
    resolve_game_artwork_with_roots(store_source, store_app_id, install_dir, &steam_roots)
}

pub fn resolve_game_artwork_with_roots(
    store_source: &str,
    store_app_id: Option<&str>,
    install_dir: &str,
    steam_roots: &[PathBuf],
) -> GameArtwork {
    let mut header_candidates: Vec<PathBuf> = Vec::new();
    let mut thumb_candidates: Vec<PathBuf> = Vec::new();

    if store_source.eq_ignore_ascii_case("steam")
        && let Some(app_id) = store_app_id
        && !app_id.is_empty()
        && app_id.chars().all(|c| c.is_ascii_digit())
    {
        for root in steam_roots {
            let cache = root.join("appcache").join("librarycache");
            // Modern Steam nested layout (`librarycache/<appid>/...`) and classic flat layout
            let nested = cache.join(app_id);
            header_candidates.extend([
                nested.join("header.jpg"),
                nested.join("library_hero.jpg"),
                cache.join(format!("{app_id}_header.jpg")),
                cache.join(format!("{app_id}_library_hero.jpg")),
            ]);
            thumb_candidates.extend([
                nested.join("icon.jpg"),
                nested.join("header.jpg"),
                nested.join("library_600x900.jpg"),
                cache.join(format!("{app_id}_icon.jpg")),
                cache.join(format!("{app_id}_header.jpg")),
                cache.join(format!("{app_id}_library_600x900.jpg")),
            ]);
        }
    }

    // Also probe local cover files placed inside the game's install directory
    let dir = Path::new(install_dir);
    if dir.is_dir() {
        for name in [
            "header.jpg",
            "header.png",
            "cover.jpg",
            "cover.png",
            "cover.webp",
            "icon.png",
            "icon.jpg",
        ] {
            let p = dir.join(name);
            header_candidates.push(p.clone());
            thumb_candidates.push(p);
        }
    }

    let header_data_uri = header_candidates
        .iter()
        .find_map(|p| load_image_data_uri(p));
    let thumbnail_data_uri = thumb_candidates
        .iter()
        .find_map(|p| load_image_data_uri(p))
        .or_else(|| header_data_uri.clone());

    GameArtwork {
        thumbnail_data_uri,
        header_data_uri,
    }
}

fn load_image_data_uri(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() as usize > MAX_ARTWORK_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let mime = detect_image_mime(&bytes)?;
    Some(format!("data:{mime};base64,{}", encode_base64(&bytes)))
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_local_steam_cache_jpeg_and_rejects_non_images() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_art_test_{nanos}"));
        let cache = root.join("appcache").join("librarycache");
        fs::create_dir_all(&cache).unwrap();

        // Valid minimal JPEG magic header
        fs::write(
            cache.join("367520_header.jpg"),
            [0xFF, 0xD8, 0xFF, 0xE0, 0x00],
        )
        .unwrap();

        let art = resolve_game_artwork_with_roots(
            "steam",
            Some("367520"),
            root.to_string_lossy().as_ref(),
            std::slice::from_ref(&root),
        );
        assert!(
            art.header_data_uri
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:image/jpeg;base64,")
        );
        assert!(art.thumbnail_data_uri.is_some());

        // Reject invalid non-numeric Steam AppID (path traversal defense)
        let bad = resolve_game_artwork_with_roots(
            "steam",
            Some("../evil"),
            root.to_string_lossy().as_ref(),
            std::slice::from_ref(&root),
        );
        assert!(bad.header_data_uri.is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
