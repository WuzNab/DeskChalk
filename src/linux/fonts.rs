//! Fonts for box titles, found through fontconfig (fc-match), with common font files as a
//! fallback. Files are memory-mapped, so large fallback fonts cost little until used.

use super::util;
use crate::text::{FontData, Fonts};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub struct FontChoice {
    pub key: &'static str,
    pub label: &'static str,
    /// fontconfig pattern
    pub pattern: &'static str,
}

pub const FONT_CHOICES: &[FontChoice] = &[
    FontChoice { key: "sans-semibold", label: "Sans Semibold", pattern: "sans-serif:weight=180" },
    FontChoice { key: "sans", label: "Sans", pattern: "sans-serif" },
    FontChoice { key: "sans-bold", label: "Sans Bold", pattern: "sans-serif:bold" },
    FontChoice { key: "serif", label: "Serif", pattern: "serif" },
    FontChoice { key: "mono", label: "Monospace", pattern: "monospace" },
];

/// Scripts the primary font may lack, looked up as fallbacks.
const FALLBACK_LANGS: &[&str] = &["zh-cn", "ja", "ko", "ar", "he", "hi", "th"];

/// Used when fc-match isn't available.
const KNOWN_FILES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];

/// The key of the chosen font, or the default if it isn't a Linux font (e.g. a config
/// copied from Windows).
pub fn choice_key(key: &str) -> &'static str {
    FONT_CHOICES.iter().find(|c| c.key == key).unwrap_or(&FONT_CHOICES[0]).key
}

fn locate(pattern: &str) -> Option<(PathBuf, u32)> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<(PathBuf, u32)>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().ok()?.get(pattern) {
        return hit.clone();
    }
    let found = util::output("fc-match", &["-f", "%{file}\n%{index}", pattern])
        .and_then(|s| {
            let mut lines = s.lines();
            let file = PathBuf::from(lines.next()?.trim());
            let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
            file.is_file().then_some((file, index))
        })
        .or_else(|| KNOWN_FILES.iter().map(PathBuf::from).find(|p| p.is_file()).map(|p| (p, 0)));
    cache.lock().ok()?.insert(pattern.to_string(), found.clone());
    found
}

fn open(path: &PathBuf, index: u32) -> Option<FontData> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: font files aren't modified while DeskChalk runs; if one is, glyphs may come out
    // wrong, which ttf-parser handles without unsafety of its own.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    FontData::new(Box::new(map), index)
}

/// The chosen title font plus fallbacks for other scripts.
pub fn load(key: &str) -> Option<Fonts> {
    let choice = FONT_CHOICES.iter().find(|c| c.key == choice_key(key))?;
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut faces = Vec::new();
    let patterns = std::iter::once(choice.pattern.to_string())
        .chain(FALLBACK_LANGS.iter().map(|l| format!("sans-serif:lang={l}")));
    for p in patterns {
        let Some((path, index)) = locate(&p) else { continue };
        if seen.contains(&path) {
            continue;
        }
        if let Some(f) = open(&path, index) {
            faces.push(f);
            seen.push(path);
        }
    }
    Fonts::new(faces)
}

/// Font for hints, labels and menus in edit mode.
pub fn load_ui() -> Option<Fonts> {
    load("sans")
}
