//! The notification-area (tray) icon. The pencil is white on a dark taskbar and dark
//! on a light one; the line is always pastel green.

use super::reg;
use super::util::fill_wide;
use crate::icon;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
    NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, FindWindowW, HICON, ICONINFO, SM_CXSMICON,
};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

pub struct Tray {
    hwnd: HWND,
    icon: Option<HICON>,
    added: bool,
    light: bool,
    size: i32,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        Tray { hwnd, icon: None, added: false, light: false, size: 0 }
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        }
    }

    /// (Re)creates the icon bitmap if the taskbar theme or DPI changed.
    fn ensure_icon(&mut self) -> bool {
        let light = taskbar_is_light();
        let size = tray_icon_size();
        if self.icon.is_some() && light == self.light && size == self.size {
            return false;
        }
        let pencil = if light { icon::DARK } else { icon::WHITE };
        match make_icon(size, pencil, icon::PASTEL_GREEN, None) {
            Some(new) => {
                if let Some(old) = self.icon.replace(new) {
                    unsafe {
                        let _ = DestroyIcon(old);
                    }
                }
                self.light = light;
                self.size = size;
                true
            }
            None => false,
        }
    }

    pub fn add(&mut self, tip: &str) {
        self.ensure_icon();
        let mut n = self.base();
        n.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        n.uCallbackMessage = super::WM_TRAY;
        n.hIcon = self.icon.unwrap_or_default();
        fill_wide(&mut n.szTip, tip);
        unsafe {
            // Explorer may still hold an entry from before it restarted.
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.base());
            self.added = Shell_NotifyIconW(NIM_ADD, &n).as_bool();
            n.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &n);
        }
    }

    pub fn refresh_icon(&mut self) {
        if self.ensure_icon() && self.added {
            let mut n = self.base();
            n.uFlags = NIF_ICON;
            n.hIcon = self.icon.unwrap_or_default();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
            }
        }
    }

    pub fn set_tip(&self, tip: &str) {
        if !self.added {
            return;
        }
        let mut n = self.base();
        n.uFlags = NIF_TIP | NIF_SHOWTIP;
        fill_wide(&mut n.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
        }
    }

    pub fn balloon(&self, title: &str, text: &str) {
        if !self.added {
            return;
        }
        let mut n = self.base();
        n.uFlags = NIF_INFO;
        n.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
        fill_wide(&mut n.szInfoTitle, title);
        fill_wide(&mut n.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.base());
            }
            self.added = false;
        }
    }

    pub fn forget(&mut self) {
        self.added = false;
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        if let Some(i) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(i);
            }
        }
    }
}

fn taskbar_is_light() -> bool {
    reg::get_dword(PERSONALIZE, "SystemUsesLightTheme") == Some(1)
}

fn tray_icon_size() -> i32 {
    unsafe {
        let dpi = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())
            .map(|h| GetDpiForWindow(h))
            .unwrap_or(96);
        GetSystemMetricsForDpi(SM_CXSMICON, if dpi == 0 { 96 } else { dpi }).max(16)
    }
}

/// Builds an alpha-blended HICON from the vector icon.
pub fn make_icon(size: i32, pencil: [u8; 3], line: [u8; 3], outline: Option<[u8; 3]>) -> Option<HICON> {
    let pm = icon::render(size as u32, pencil, line, outline)?;
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
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for (px, out) in pm.pixels().iter().zip(dst.as_chunks_mut::<4>().0) {
            let c = px.demultiply();
            *out = [c.blue(), c.green(), c.red(), c.alpha()];
        }
        let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
