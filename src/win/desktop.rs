//! Finding the spot between the desktop icons and the wallpaper.
//!
//! Classic topology (Windows 10, Windows 11 before 24H2): sending 0x052C to Progman splits
//! the desktop into a top-level WorkerW holding SHELLDLL_DefView (the icons) and a second
//! top-level WorkerW right behind it that paints the wallpaper. We become a child of that one.
//!
//! Modern topology (Windows 11 24H2+): Progman keeps SHELLDLL_DefView as a child and the
//! wallpaper WorkerW is *also* a child of Progman, below DefView. We become a child of Progman
//! placed between the two. Progman has no redirection surface there, so our content is a
//! DirectComposition visual (see comp.rs).
//!
//! Fallback: no WorkerW at all. Progman paints the wallpaper and holds DefView; we sit right
//! below DefView inside Progman.

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetParent, GetWindow, IsWindow,
    IsWindowVisible, SendMessageTimeoutW, GW_CHILD, GW_HWNDNEXT, HWND_TOP, SMTO_NORMAL,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Modern,
    Classic,
    Fallback,
}

#[derive(Clone, Copy, Debug)]
pub struct Desktop {
    pub topology: Topology,
    pub progman: HWND,
    /// SHELLDLL_DefView, the window that hosts the icons.
    pub defview: HWND,
    /// Parent of `defview` (Progman or a WorkerW).
    pub icon_host: HWND,
    pub listview: Option<HWND>,
    /// The window our layer windows are children of.
    pub parent: HWND,
    /// The window that paints the wallpaper, if separate.
    pub workerw: Option<HWND>,
}

fn child(parent: HWND, class: PCWSTR) -> Option<HWND> {
    unsafe { FindWindowExW(Some(parent), None, class, PCWSTR::null()).ok() }
}

unsafe extern "system" fn find_defview_host(hwnd: HWND, lp: LPARAM) -> BOOL {
    if child(hwnd, w!("SHELLDLL_DefView")).is_some() {
        unsafe { *(lp.0 as *mut Option<HWND>) = Some(hwnd) };
        return false.into();
    }
    true.into()
}

fn defview_host() -> Option<HWND> {
    let mut host: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(find_defview_host), LPARAM(&mut host as *mut _ as isize));
    }
    host
}

/// Detects the current topology at runtime and returns where to attach.
pub fn find() -> Option<Desktop> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        // Ask Explorer to create the wallpaper WorkerW (no-op if it already exists).
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(0x1), SMTO_NORMAL, 1000, None);
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);

        if let Some(defview) = child(progman, w!("SHELLDLL_DefView")) {
            let workerw = child(progman, w!("WorkerW"));
            // We sit right below DefView as its sibling rather than inside the WorkerW, so
            // wallpaper changes that recreate the WorkerW don't take our windows with them.
            let parent = progman;
            return Some(Desktop {
                topology: if workerw.is_some() { Topology::Modern } else { Topology::Fallback },
                progman,
                defview,
                icon_host: progman,
                listview: child(defview, w!("SysListView32")),
                parent,
                workerw,
            });
        }

        let host = defview_host()?;
        let defview = child(host, w!("SHELLDLL_DefView"))?;
        let wallpaper = FindWindowExW(None, Some(host), w!("WorkerW"), PCWSTR::null()).ok()?;
        Some(Desktop {
            topology: Topology::Classic,
            progman,
            defview,
            icon_host: host,
            listview: child(defview, w!("SysListView32")),
            parent: wallpaper,
            workerw: Some(wallpaper),
        })
    }
}

/// The desktop icon list view, without changing anything.
pub fn locate_listview() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        let defview = child(progman, w!("SHELLDLL_DefView")).or_else(|| child(defview_host()?, w!("SHELLDLL_DefView")))?;
        child(defview, w!("SysListView32"))
    }
}

impl Desktop {
    /// False once Explorer restarted, the wallpaper changed topology, or a window died.
    pub fn still_valid(&self) -> bool {
        unsafe {
            let alive = |h: HWND| IsWindow(Some(h)).as_bool();
            if !alive(self.progman) || !alive(self.parent) || !alive(self.defview) {
                return false;
            }
            if FindWindowW(w!("Progman"), PCWSTR::null()).ok() != Some(self.progman) {
                return false;
            }
            if GetParent(self.defview).ok() != Some(self.icon_host) {
                return false;
            }
            match self.topology {
                Topology::Modern => {
                    child(self.progman, w!("WorkerW")) == self.workerw && IsWindowVisible(self.parent).as_bool()
                }
                Topology::Classic => IsWindowVisible(self.parent).as_bool(),
                Topology::Fallback => child(self.progman, w!("WorkerW")).is_none(),
            }
        }
    }

    /// True when our windows are children of the wallpaper window itself.
    fn inside_wallpaper(&self) -> bool {
        Some(self.parent) == self.workerw
    }

    /// The sibling our windows are inserted below (z-order).
    pub fn insert_after(&self) -> HWND {
        if self.inside_wallpaper() {
            HWND_TOP
        } else {
            self.defview
        }
    }

    fn children(&self) -> Vec<HWND> {
        let mut out = Vec::new();
        unsafe {
            let mut h = GetWindow(self.parent, GW_CHILD).ok();
            while let Some(w) = h {
                out.push(w);
                if out.len() > 256 {
                    break;
                }
                h = GetWindow(w, GW_HWNDNEXT).ok();
            }
        }
        out
    }

    /// True when every window in `ours` sits below the icons and above the wallpaper.
    pub fn order_ok(&self, ours: &[HWND]) -> bool {
        if self.inside_wallpaper() {
            return true;
        }
        let order = self.children();
        let pos = |x: HWND| order.iter().position(|&w| w == x);
        let Some(icons) = pos(self.defview) else { return false };
        let wallpaper = self.workerw.and_then(pos).unwrap_or(usize::MAX);
        ours.iter().all(|&o| pos(o).is_some_and(|p| p > icons && p < wallpaper))
    }

    /// Screen position of the parent's client area origin.
    pub fn parent_origin(&self) -> POINT {
        let mut p = POINT::default();
        unsafe {
            let _ = ClientToScreen(self.parent, &mut p);
        }
        p
    }

    /// Human-readable dump for --diag.
    pub fn describe(&self) -> String {
        let mut s = format!(
            "topology: {:?}\nprogman: {:?}\nicon host: {:?} ({})\ndefview: {:?}\nlistview: {:?}\nparent: {:?} ({})\nwallpaper: {:?}\nparent children (top to bottom):\n",
            self.topology,
            self.progman.0,
            self.icon_host.0,
            class_name(self.icon_host),
            self.defview.0,
            self.listview.map(|h| h.0),
            self.parent.0,
            class_name(self.parent),
            self.workerw.map(|h| h.0),
        );
        for c in self.children() {
            s.push_str(&format!("  {:?} {}\n", c.0, class_name(c)));
        }
        s
    }
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}
