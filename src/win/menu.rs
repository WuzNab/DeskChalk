//! Native popup menus and command ids.

use super::util::wide;
use crate::geom::Rect;
use crate::model::{Rgb, ZoneStyle};
use crate::render::{self, Painter};
use tiny_skia::{FillRule, PixmapMut, Stroke, Transform};
use windows::core::PWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, InsertMenuItemW, SetMenuDefaultItem,
    TrackPopupMenuEx, HMENU, MENUITEMINFOW, MENU_ITEM_STATE, MFS_CHECKED, MFS_DISABLED,
    MFT_RADIOCHECK, MFT_SEPARATOR, MFT_STRING, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STATE,
    MIIM_STRING, MIIM_SUBMENU, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

pub mod cmd {
    pub const EDIT: u32 = 1;
    pub const SHOW_ZONES: u32 = 2;
    pub const GRID_SHOW: u32 = 3;
    pub const SNAP: u32 = 4;
    pub const AUTOSTART: u32 = 5;
    pub const MINIMIZE: u32 = 6;
    pub const HOTKEY: u32 = 7;
    pub const SUPPORT: u32 = 8;
    pub const ABOUT: u32 = 9;
    pub const UNINSTALL: u32 = 10;
    pub const EXIT: u32 = 11;
    pub const AUTO_ARRANGE: u32 = 12;
    pub const ALIGN_GRID: u32 = 13;
    pub const RECOVER: u32 = 14;
    pub const REFRESH: u32 = 15;
    pub const UNDO: u32 = 16;
    pub const REDO: u32 = 17;
    pub const DELETE_ALL: u32 = 18;
    pub const APPLY_ALL: u32 = 19;
    pub const GRID_WHITE: u32 = 20;
    pub const GRID_BLACK: u32 = 21;
    pub const Z_RENAME: u32 = 30;
    pub const Z_DUPLICATE: u32 = 31;
    pub const Z_FRONT: u32 = 32;
    pub const Z_BACK: u32 = 33;
    pub const Z_DELETE: u32 = 34;
    pub const Z_MAKE_DEFAULT: u32 = 35;
    pub const Z_RESET_STYLE: u32 = 36;
    /// + index into model::DOT_SIZES
    pub const GRID_SIZE: u32 = 100;
    /// + index into model::DOT_OPACITY
    pub const GRID_OPACITY: u32 = 120;
    /// + index into model::ICON_SIZES
    pub const ICON_SIZE: u32 = 140;
    /// + index into fonts::FONT_CHOICES
    pub const FONT: u32 = 160;
    /// + style offset, applies to the clicked box
    pub const STYLE_ZONE: u32 = 1000;
    /// + style offset, applies to the style for new boxes
    pub const STYLE_DEFAULT: u32 = 2000;
}

pub struct Menu {
    pub h: HMENU,
    bitmaps: Vec<HBITMAP>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { h: unsafe { CreatePopupMenu().unwrap_or_default() }, bitmaps: Vec::new() }
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
        let bmp = swatch(c, scale);
        if let Some(b) = bmp {
            self.bitmaps.push(b);
        }
        self.entry(id, text, checked, true, true, bmp);
    }

    pub fn sep(&mut self) {
        self.insert(
            MENUITEMINFOW { fMask: MIIM_FTYPE, fType: MFT_SEPARATOR, ..Default::default() },
            None,
        );
    }

    pub fn sub(&mut self, text: &str, mut child: Menu) {
        let h = std::mem::take(&mut child.h);
        self.bitmaps.append(&mut child.bitmaps);
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
            for b in self.bitmaps.drain(..) {
                let _ = DeleteObject(b.into());
            }
        }
    }
}

/// A small rounded color square for color menu items.
fn swatch(c: Rgb, scale: f32) -> Option<HBITMAP> {
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
            match e.color {
                Some(c) => sub.color(base + e.offset, &e.label, c, e.checked, scale),
                None => sub.radio(base + e.offset, &e.label, e.checked),
            }
        }
        m.sub(title, sub);
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
