//! Monitor enumeration with stable identifiers, so boxes stay on "their" screen.

use super::util::from_wide;
use crate::editor::MonCtx;
use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Device interface path of the physical display (stable across reboots).
    pub key: String,
    /// Model code from the EDID, e.g. "GSM5B7F" (survives a port change).
    pub hw: String,
    pub device: String,
    pub rect: RECT,
    pub work: RECT,
    pub dpi: u32,
    pub primary: bool,
    pub grid: Grid,
}

impl Monitor {
    pub fn size(&self) -> (f32, f32) {
        (
            (self.rect.right - self.rect.left) as f32,
            (self.rect.bottom - self.rect.top) as f32,
        )
    }

    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// Work area (screen minus taskbar) relative to the monitor's top-left.
    pub fn work_rel(&self) -> Rect {
        Rect::new(
            (self.work.left - self.rect.left) as f32,
            (self.work.top - self.rect.top) as f32,
            (self.work.right - self.work.left) as f32,
            (self.work.bottom - self.work.top) as f32,
        )
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.rect.left && x < self.rect.right && y >= self.rect.top && y < self.rect.bottom
    }

    pub fn ctx(&self) -> MonCtx<'_> {
        MonCtx {
            key: &self.key,
            hw: &self.hw,
            size: self.size(),
            scale: self.scale(),
            grid: &self.grid,
            bounds: self.work_rel(),
        }
    }

    /// Same physical layout (ignores the icon grid).
    pub fn same_geometry(&self, o: &Monitor) -> bool {
        self.key == o.key && self.rect == o.rect && self.work == o.work && self.dpi == o.dpi
    }
}

unsafe extern "system" fn collect(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    unsafe { (*(lp.0 as *mut Vec<HMONITOR>)).push(h) };
    true.into()
}

pub fn enumerate() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let mut out: Vec<Monitor> = handles.into_iter().filter_map(describe).collect();
    out.sort_by_key(|m| (m.rect.left, m.rect.top));
    out
}

fn describe(h: HMONITOR) -> Option<Monitor> {
    unsafe {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return None;
        }
        let device = from_wide(&mi.szDevice);
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let dev_w: Vec<u16> = device.encode_utf16().chain(Some(0)).collect();
        let key = if EnumDisplayDevicesW(
            windows::core::PCWSTR(dev_w.as_ptr()),
            0,
            &mut dd,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
        .as_bool()
        {
            from_wide(&dd.DeviceID)
        } else {
            String::new()
        };
        let key = if key.is_empty() { device.clone() } else { key };
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() || dx == 0 {
            dx = 96;
        }
        let rect = mi.monitorInfo.rcMonitor;
        let work = mi.monitorInfo.rcWork;
        let w = (work.right - work.left) as f32;
        let hgt = (work.bottom - work.top) as f32;
        let s = dx as f32 / 96.0;
        Some(Monitor {
            hw: hardware_id(&key),
            key,
            device,
            rect,
            work,
            dpi: dx,
            primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            grid: Grid {
                origin: Pt::new((work.left - rect.left) as f32, (work.top - rect.top) as f32),
                cell: Pt::new(75.0 * s, 75.0 * s),
                area: Rect::new((work.left - rect.left) as f32, (work.top - rect.top) as f32, w, hgt),
            },
        })
    }
}

/// "\\?\DISPLAY#GSM5B7F#5&1a2b&0&UID4352#{...}" -> "GSM5B7F"
fn hardware_id(key: &str) -> String {
    key.split('#').nth(1).unwrap_or_default().to_string()
}
