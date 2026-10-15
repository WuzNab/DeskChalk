//! Native popup menus and command ids.

use super::util::wide;
use crate::geom::Rect;
use crate::model::{self, Rgb, StyleCmd, ZoneStyle};
use std::cell::RefCell;
use std::collections::HashMap;
use crate::render::{self, Painter};
use tiny_skia::{FillRule, LineCap, LineJoin, PathBuilder, PixmapMut, Stroke, Transform};
use windows::core::PWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, InsertMenuItemW,
    SetMenuDefaultItem, SetMenuItemInfoW, TrackPopupMenuEx, HMENU, MENUITEMINFOW,
    MENU_ITEM_STATE, MFS_CHECKED, MFS_DISABLED, MFT_RADIOCHECK, MFT_SEPARATOR, MFT_STRING,
    MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

pub(crate) use crate::commands as cmd;

pub struct Menu {
    pub h: HMENU,
    /// Swatch bitmaps owned by this menu (freed with it).
    bitmaps: RefCell<Vec<HBITMAP>>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { h: unsafe { CreatePopupMenu().unwrap_or_default() }, bitmaps: RefCell::new(Vec::new()) }
    }

    fn insert(&mut self, mut mii: MENUITEMINFOW, text: Option<&str>) {
        let mut buf = text.map(wide);
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        if let Some(b) = buf.as_mut() {
            mii.fMask |= MIIM_STRING;
            mii.dwTypeData = PWSTR(b.as_mut_ptr());
            mii.cch = (b.len() - 1) as u32;
        }
        unsafe {
            let pos = GetMenuItemCount(Some(self.h)).max(0) as u32;
            let _ = InsertMenuItemW(self.h, pos, true, &mii);
        }
    }

    fn entry(&mut self, id: u32, text: &str, checked: bool, radio: bool, enabled: bool, bitmap: Option<HBITMAP>) {
        let mut state = MENU_ITEM_STATE(0);
        if checked {
            state |= MFS_CHECKED;
        }
        if !enabled {
            state |= MFS_DISABLED;
        }
        let mut mii = MENUITEMINFOW {
            fMask: MIIM_ID | MIIM_STATE | MIIM_FTYPE,
            fType: if radio { MFT_RADIOCHECK } else { MFT_STRING },
            fState: state,
            wID: id,
            ..Default::default()
        };
        if let Some(b) = bitmap {
            mii.fMask |= MIIM_BITMAP;
            mii.hbmpItem = b;
        }
        self.insert(mii, Some(text));
    }

    pub fn item(&mut self, id: u32, text: &str) {
        self.entry(id, text, false, false, true, None);
    }

    pub fn item_enabled(&mut self, id: u32, text: &str, enabled: bool) {
        self.entry(id, text, false, false, enabled, None);
    }

    pub fn check(&mut self, id: u32, text: &str, checked: bool) {
        self.entry(id, text, checked, false, true, None);
    }

    pub fn radio(&mut self, id: u32, text: &str, checked: bool) {
        self.entry(id, text, checked, true, true, None);
    }

    pub fn color(&mut self, id: u32, text: &str, c: Rgb, checked: bool, scale: f32) {
        let bmp = swatch(c, scale, checked);
        if let Some(b) = bmp {
            self.bitmaps.borrow_mut().push(b);
        }
        self.entry(id, text, checked, true, true, bmp);
    }

    /// A grayed-out caption.
    pub fn heading(&mut self, text: &str) {
        self.entry(0, text, false, false, false, None);
    }

    pub fn sep(&mut self) {
        self.insert(
            MENUITEMINFOW { fMask: MIIM_FTYPE, fType: MFT_SEPARATOR, ..Default::default() },
            None,
        );
    }

    pub fn sub(&mut self, text: &str, mut child: Menu) {
        let h = std::mem::take(&mut child.h);
        self.bitmaps.borrow_mut().append(&mut child.bitmaps.borrow_mut());
        self.insert(MENUITEMINFOW { fMask: MIIM_SUBMENU, hSubMenu: h, ..Default::default() }, Some(text));
    }

    pub fn set_default(&mut self, id: u32) {
        unsafe {
            let _ = SetMenuDefaultItem(self.h, id, 0);
        }
    }

    /// Shows the menu and returns the chosen command (0 if dismissed).
    pub fn track(&self, hwnd: HWND, x: i32, y: i32) -> u32 {
        unsafe {
            TrackPopupMenuEx(self.h, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0, x, y, hwnd, None).0
                as u32
        }
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            if !self.h.is_invalid() {
                let _ = DestroyMenu(self.h);
            }
            for b in self.bitmaps.borrow_mut().drain(..) {
                let _ = DeleteObject(b.into());
            }
        }
    }
}

/// A small rounded color square for color menu items, with a check mark when selected
/// (Windows draws the bitmap where the check mark would go).
fn swatch(c: Rgb, scale: f32, checked: bool) -> Option<HBITMAP> {
    let size = (16.0 * scale).round().max(12.0) as i32;
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let buf = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        buf.fill(0);
        if let Some(mut pm) = PixmapMut::from_bytes(buf, size as u32, size as u32) {
            let p = Painter { swap_rb: true };
            let inset = 1.5 * scale;
            let r = Rect::new(inset, inset, size as f32 - 2.0 * inset, size as f32 - 2.0 * inset);
            if let Some(path) = render::rounded_rect(r, 3.0 * scale) {
                pm.fill_path(&path, &p.paint(c, 1.0), FillRule::Winding, Transform::identity(), None);
                let stroke = Stroke { width: scale.max(1.0), ..Stroke::default() };
                pm.stroke_path(&path, &p.paint(Rgb(0x55, 0x55, 0x55), 0.9), &stroke, Transform::identity(), None);
            }
            if checked {
                let s = size as f32;
                let mut pb = PathBuilder::new();
                pb.move_to(s * 0.27, s * 0.52);
                pb.line_to(s * 0.44, s * 0.69);
                pb.line_to(s * 0.74, s * 0.33);
                if let Some(tick) = pb.finish() {
                    let ink = if c.luminance() > 0.45 { Rgb(0x10, 0x10, 0x10) } else { Rgb(0xFF, 0xFF, 0xFF) };
                    let stroke = Stroke {
                        width: (1.8 * scale).max(1.5),
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        ..Stroke::default()
                    };
                    pm.stroke_path(&tick, &p.paint(ink, 1.0), &stroke, Transform::identity(), None);
                }
            }
        }
        Some(bmp)
    }
}

/// Adds Color / Title position / ... submenus for a style. Ids are `base + style offset`.
pub fn add_style_menus(m: &mut Menu, style: &ZoneStyle, base: u32, scale: f32) {
    for (title, entries) in style.menu().sections {
        let mut sub = Menu::new();
        for e in entries {
            if e.separator {
                sub.sep();
            }
            if let Some(h) = e.heading {
                sub.heading(h);
            }
            match e.color {
                Some(c) => sub.color(base + e.offset, &e.label, c, e.checked, scale),
                None => sub.radio(base + e.offset, &e.label, e.checked),
            }
        }
        m.sub(title, sub);
    }
}

/// Settings that apply instantly; clicking them keeps the menu open so several can be
/// changed in a row. Everything else (dialogs, actions) closes the menu as usual.
pub fn is_sticky(id: u32) -> bool {
    use cmd::*;
    let within = |base: u32, len: usize| (base..base + len as u32).contains(&id);
    let style = |base: u32| within(base, model::STYLE_SPAN as usize)
        && StyleCmd::decode(id - base).is_some_and(|c| !c.opens_dialog());
    matches!(
        id,
        SHOW_ZONES | GRID_SHOW | SNAP | AUTOSTART | MINIMIZE | HOTKEY | AUTO_ARRANGE | ALIGN_GRID
            | MIX_COLORS | GRID_REPEL | UNDO | REDO | Z_FRONT | Z_BACK
    ) || within(GRID_SIZE, model::DOT_SIZES.len())
        || within(GRID_OPACITY, model::DOT_OPACITY.len())
        || within(ICON_SIZE, model::ICON_SIZES.len())
        || within(FONT, 20)
        || within(DOT_COLOR, model::DOT_COLORS.len())
        || style(STYLE_ZONE)
        || style(STYLE_DEFAULT)
}

fn item_info(h: HMENU, pos: u32) -> Option<MENUITEMINFOW> {
    let mut mii = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_ID | MIIM_STATE | MIIM_SUBMENU | MIIM_FTYPE | MIIM_BITMAP,
        ..Default::default()
    };
    unsafe { GetMenuItemInfoW(h, pos, true, &mut mii).ok()? };
    Some(mii)
}

fn collect_states(h: HMENU, out: &mut HashMap<u32, (MENU_ITEM_STATE, HBITMAP)>) {
    let n = unsafe { GetMenuItemCount(Some(h)) }.max(0) as u32;
    for pos in 0..n {
        let Some(mii) = item_info(h, pos) else { continue };
        if !mii.hSubMenu.is_invalid() {
            collect_states(mii.hSubMenu, out);
        } else if mii.wID != 0 {
            out.insert(mii.wID, (mii.fState, mii.hbmpItem));
        }
    }
}

/// Copies check marks, enabled states and swatches from `fresh` into the open menu `live`.
pub fn sync_states(live: &Menu, fresh: Menu) {
    let mut states = HashMap::new();
    collect_states(fresh.h, &mut states);
    apply_states(live.h, &states);
    // The live menu may now show some of fresh's swatches: keep them alive with it.
    live.bitmaps.borrow_mut().append(&mut fresh.bitmaps.borrow_mut());
}

fn apply_states(h: HMENU, states: &HashMap<u32, (MENU_ITEM_STATE, HBITMAP)>) {
    let n = unsafe { GetMenuItemCount(Some(h)) }.max(0) as u32;
    for pos in 0..n {
        let Some(mii) = item_info(h, pos) else { continue };
        if !mii.hSubMenu.is_invalid() {
            apply_states(mii.hSubMenu, states);
            continue;
        }
        let Some(&(want, bitmap)) = states.get(&mii.wID) else { continue };
        let mask = MFS_CHECKED | MFS_DISABLED;
        if (mii.fState & mask) == (want & mask) {
            continue;
        }
        let mut set = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STATE,
            fState: (mii.fState & !mask) | (want & mask),
            ..Default::default()
        };
        if !bitmap.is_invalid() {
            set.fMask |= MIIM_BITMAP;
            set.hbmpItem = bitmap;
        }
        unsafe {
            let _ = SetMenuItemInfoW(h, pos, true, &set);
        }
    }
}

/// Right-click on a box in edit mode.
pub fn zone_menu(style: &ZoneStyle, scale: f32) -> Menu {
    let mut m = Menu::new();
    m.item(cmd::Z_RENAME, "Rename…\tF2");
    m.set_default(cmd::Z_RENAME);
    m.sep();
    add_style_menus(&mut m, style, cmd::STYLE_ZONE, scale);
    m.sep();
    m.item(cmd::Z_DUPLICATE, "Duplicate\tCtrl+D");
    m.item(cmd::Z_FRONT, "Bring to front");
    m.item(cmd::Z_BACK, "Send to back");
    m.sep();
    m.item(cmd::Z_MAKE_DEFAULT, "Use this style for new boxes");
    m.item(cmd::Z_RESET_STYLE, "Reset to the new-box style");
    m.sep();
    m.item(cmd::Z_DELETE, "Delete\tDel");
    m
}

/// Right-click on empty space in edit mode.
pub fn canvas_menu(snap: bool, grid: bool, can_undo: bool, can_redo: bool, any_here: bool) -> Menu {
    let mut m = Menu::new();
    m.item(cmd::EDIT, "Done editing\tEsc");
    m.set_default(cmd::EDIT);
    m.sep();
    m.check(cmd::SNAP, "Snap to icon grid (hold Alt to toggle)", snap);
    m.check(cmd::GRID_SHOW, "Keep grid dots visible on the desktop", grid);
    m.sep();
    m.item_enabled(cmd::UNDO, "Undo\tCtrl+Z", can_undo);
    m.item_enabled(cmd::REDO, "Redo\tCtrl+Y", can_redo);
    m.sep();
    m.item_enabled(cmd::DELETE_ALL, "Delete all boxes on this screen…", any_here);
    m
}
