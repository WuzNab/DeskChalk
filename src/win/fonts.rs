//! System fonts for box titles. Files are memory-mapped, so the large fallback fonts
//! (Chinese, Japanese, Korean, symbols) cost almost nothing until a glyph is needed.

use crate::text::{FontData, Fonts};
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileSizeEx, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READONLY,
};

pub struct FontChoice {
    pub key: &'static str,
    pub label: &'static str,
    pub file: &'static str,
}

pub const FONT_CHOICES: &[FontChoice] = &[
    FontChoice { key: "segoe-semibold", label: "Segoe UI Semibold", file: "seguisb.ttf" },
    FontChoice { key: "segoe", label: "Segoe UI", file: "segoeui.ttf" },
    FontChoice { key: "segoe-bold", label: "Segoe UI Bold", file: "segoeuib.ttf" },
    FontChoice { key: "ink-free", label: "Ink Free (handwritten)", file: "Inkfree.ttf" },
    FontChoice { key: "segoe-print", label: "Segoe Print", file: "segoepr.ttf" },
    FontChoice { key: "cascadia", label: "Cascadia Mono", file: "CascadiaMono.ttf" },
    FontChoice { key: "consolas", label: "Consolas", file: "consola.ttf" },
    FontChoice { key: "georgia", label: "Georgia", file: "georgia.ttf" },
];

const FALLBACKS: &[&str] = &[
    "segoeui.ttf",
    "seguisym.ttf",
    "msyh.ttc",
    "YuGothR.ttc",
    "malgun.ttf",
    "Nirmala.ttc",
    "Nirmala.ttf",
    "seguihis.ttf",
];

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(win) = std::env::var("WINDIR") {
        dirs.push(PathBuf::from(win).join("Fonts"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs
}

fn find(file: &str) -> Option<PathBuf> {
    font_dirs().into_iter().map(|d| d.join(file)).find(|p| p.is_file())
}

pub fn available(c: &FontChoice) -> bool {
    find(c.file).is_some()
}

fn open(file: &str) -> Option<FontData> {
    let map = Mmap::open(&find(file)?)?;
    FontData::new(Box::new(map), 0)
}

/// The chosen title font (or Segoe UI Semibold if missing) plus fallbacks.
pub fn load(key: &str) -> Option<Fonts> {
    let primary = FONT_CHOICES
        .iter()
        .find(|c| c.key == key)
        .filter(|c| available(c))
        .unwrap_or(&FONT_CHOICES[0]);
    let mut faces: Vec<FontData> = open(primary.file).into_iter().collect();
    for fb in FALLBACKS.iter().filter(|f| !f.eq_ignore_ascii_case(primary.file)) {
        if let Some(f) = open(fb) {
            faces.push(f);
        }
    }
    Fonts::new(faces)
}

/// Font for hints and labels in edit mode.
pub fn load_ui() -> Option<Fonts> {
    load("segoe")
}

struct Mmap {
    file: HANDLE,
    map: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

impl Mmap {
    fn open(path: &Path) -> Option<Mmap> {
        unsafe {
            let file = CreateFileW(
                &HSTRING::from(path.as_os_str()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .ok()?;
            let mut size = 0i64;
            if GetFileSizeEx(file, &mut size).is_err() || size <= 0 {
                let _ = CloseHandle(file);
                return None;
            }
            let Ok(map) = CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, PCWSTR::null()) else {
                let _ = CloseHandle(file);
                return None;
            };
            let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0);
            if view.Value.is_null() {
                let _ = CloseHandle(map);
                let _ = CloseHandle(file);
                return None;
            }
            Some(Mmap { file, map, view, len: size as usize })
        }
    }
}

impl AsRef<[u8]> for Mmap {
    fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.len) }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.map);
            let _ = CloseHandle(self.file);
        }
    }
}
