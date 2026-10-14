//! Explorer automation through documented COM interfaces: desktop icon layout and size,
//! auto-arrange flags, minimize-all, known folders and opening links.

use std::path::PathBuf;
use windows::core::{Interface, GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT, S_OK};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IServiceProvider, CLSCTX_ALL,
    CLSCTX_INPROC_SERVER,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IEnumIDList, IFolderView2, IShellBrowser, IShellDispatch, IShellWindows,
    SHGetKnownFolderPath, Shell, ShellExecuteW, ShellWindows, CSIDL_DESKTOP,
    FOLDERFLAGS, FOLDERVIEWMODE, FVM_ICON, FWF_AUTOARRANGE, FWF_SNAPTOGRID, KF_FLAG_DEFAULT,
    SID_STopLevelBrowser, SVGIO_ALLVIEW, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[derive(Clone, Debug)]
pub struct IconLayout {
    /// Distance between icon slots in physical pixels.
    pub spacing: (i32, i32),
    pub icon_size: i32,
    /// Icon positions (top-left of each item) in screen coordinates.
    pub positions: Vec<POINT>,
    pub flags: u32,
}

impl IconLayout {
    pub fn auto_arrange(&self) -> bool {
        self.flags & FWF_AUTOARRANGE.0 as u32 != 0
    }

    pub fn align_to_grid(&self) -> bool {
        self.flags & FWF_SNAPTOGRID.0 as u32 != 0
    }
}

/// The desktop's folder view, found via IShellWindows (works across processes).
fn desktop_view() -> windows::core::Result<IFolderView2> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let loc = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { lVal: CSIDL_DESKTOP as i32 },
                }),
            },
        };
        let empty = VARIANT::default();
        let mut hwnd = 0i32;
        let disp = windows.FindWindowSW(&loc, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
        let sp: IServiceProvider = disp.cast()?;
        let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser)?;
        browser.QueryActiveShellView()?.cast()
    }
}

/// Reads icon spacing, size, flags and positions. `listview` maps positions to the screen.
pub fn icon_layout(listview: Option<HWND>) -> Option<IconLayout> {
    unsafe {
        let view = desktop_view().ok()?;
        let mut spacing = POINT::default();
        view.GetSpacing(&mut spacing).ok()?;
        let mut mode = FOLDERVIEWMODE(0);
        let mut size = 0i32;
        let _ = view.GetViewModeAndIconSize(&mut mode, &mut size);
        let flags = view.GetCurrentFolderFlags().unwrap_or(0);
        let mut positions = Vec::new();
        if let Ok(items) = view.Items::<IEnumIDList>(SVGIO_ALLVIEW) {
            loop {
                let mut pidl: [*mut ITEMIDLIST; 1] = [std::ptr::null_mut()];
                let mut fetched = 0u32;
                if items.Next(&mut pidl, Some(&mut fetched)) != S_OK || fetched == 0 {
                    break;
                }
                if let Ok(p) = view.GetItemPosition(pidl[0]) {
                    positions.push(p);
                }
                CoTaskMemFree(Some(pidl[0] as *const _));
                if positions.len() >= 4096 {
                    break;
                }
            }
        }
        if let Some(lv) = listview {
            if !positions.is_empty() {
                MapWindowPoints(Some(lv), None, &mut positions);
            }
        }
        Some(IconLayout {
            spacing: (spacing.x, spacing.y),
            icon_size: size,
            positions,
            flags,
        })
    }
}

pub fn set_icon_size(px: i32) -> bool {
    unsafe {
        desktop_view()
            .and_then(|v| v.SetViewModeAndIconSize(FVM_ICON, px))
            .is_ok()
    }
}

pub fn set_folder_flag(flag: FOLDERFLAGS, on: bool) -> bool {
    unsafe {
        desktop_view()
            .and_then(|v| v.SetCurrentFolderFlags(flag.0 as u32, if on { flag.0 as u32 } else { 0 }))
            .is_ok()
    }
}

fn shell_dispatch() -> Option<IShellDispatch> {
    unsafe { CoCreateInstance(&Shell, None, CLSCTX_INPROC_SERVER).ok() }
}

/// Like Win+M. Returns false if it couldn't be done.
pub fn minimize_all() -> bool {
    shell_dispatch().is_some_and(|s| unsafe { s.MinimizeAll().is_ok() })
}

/// Like Win+Shift+M.
pub fn undo_minimize_all() {
    if let Some(s) = shell_dispatch() {
        unsafe {
            let _ = s.UndoMinimizeALL();
        }
    }
}

pub fn open_url(url: &str) {
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

