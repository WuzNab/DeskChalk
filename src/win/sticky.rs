//! Menus that stay open after a setting is clicked.
//!
//! While a popup menu is tracked, a thread-local WH_MSGFILTER hook sees its mouse input.
//! A click on a "sticky" item (see menu::is_sticky) is handled right there and swallowed,
//! so the menu stays up; check marks are then refreshed in place. Clicking outside the
//! menu, pressing Esc, or choosing any other item closes it normally.

use super::menu::{self, Menu};
use super::util::lparam_xy;
use super::{rebuild_menu, run_command, MenuKind, Target};
use std::cell::RefCell;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, InvalidateRect};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, EnumThreadWindows, GetClassNameW, GetMenuItemInfoW, MenuItemFromPoint,
    SendMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HMENU, MENUITEMINFOW,
    MFS_DISABLED, MIIM_ID, MIIM_STATE, MIIM_SUBMENU, MN_GETHMENU, MSG, MSGF_MENU, WH_MSGFILTER,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
};

struct Context {
    /// The menu being tracked; only valid during `track`.
    menu: *const Menu,
    target: Target,
    kind: MenuKind,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Shows `menu` like Menu::track, but sticky items don't close it.
pub fn track(menu: &Menu, owner: HWND, x: i32, y: i32, target: Target, kind: MenuKind) -> u32 {
    CONTEXT.with(|c| *c.borrow_mut() = Some(Context { menu, target, kind }));
    let hook = unsafe { SetWindowsHookExW(WH_MSGFILTER, Some(filter), None, GetCurrentThreadId()).ok() };
    let id = menu.track(owner, x, y);
    if let Some(h) = hook {
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        }
    }
    CONTEXT.with(|c| c.borrow_mut().take());
    id
}

unsafe extern "system" fn filter(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == MSGF_MENU as i32 {
        let msg = unsafe { &*(lp.0 as *const MSG) };
        match msg.message {
            // Swallow the press too, so the menu never starts treating it as a selection.
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if sticky_item_at(msg.hwnd, msg.lParam).is_some() => {
                return LRESULT(1);
            }
            WM_LBUTTONUP => {
                if let Some(id) = sticky_item_at(msg.hwnd, msg.lParam) {
                    apply(id);
                    return LRESULT(1);
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

fn is_menu_window(h: HWND) -> bool {
    let mut buf = [0u16; 16];
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n]) == "#32768"
}

/// The sticky item under a mouse message sent to menu window `hwnd` (`lp`: client coords).
fn sticky_item_at(hwnd: HWND, lp: LPARAM) -> Option<u32> {
    unsafe {
        if hwnd.is_invalid() || !is_menu_window(hwnd) {
            return None;
        }
        let (x, y) = lparam_xy(lp);
        let mut pt = POINT { x, y };
        let _ = ClientToScreen(hwnd, &mut pt);
        let hmenu = HMENU(SendMessageW(hwnd, MN_GETHMENU, None, None).0 as *mut _);
        if hmenu.is_invalid() {
            return None;
        }
        let pos = MenuItemFromPoint(None, hmenu, pt);
        if pos < 0 {
            return None;
        }
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_ID | MIIM_STATE | MIIM_SUBMENU,
            ..Default::default()
        };
        if GetMenuItemInfoW(hmenu, pos as u32, true, &mut mii).is_err()
            || !mii.hSubMenu.is_invalid()
            || (mii.fState & MFS_DISABLED).0 != 0
            || !menu::is_sticky(mii.wID)
        {
            return None;
        }
        Some(mii.wID)
    }
}

/// Runs a sticky command and refreshes the open menu's check marks.
fn apply(id: u32) {
    let Some((target, kind, live)) =
        CONTEXT.with(|c| c.borrow().as_ref().map(|c| (c.target.clone(), c.kind.clone(), c.menu)))
    else {
        return;
    };
    run_command(id, target);
    if let Some(fresh) = rebuild_menu(&kind) {
        // SAFETY: `track` keeps the menu alive for as long as the hook can call this.
        menu::sync_states(unsafe { &*live }, fresh);
    }
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(repaint_menus), LPARAM(0));
    }
}

unsafe extern "system" fn repaint_menus(h: HWND, _: LPARAM) -> BOOL {
    if is_menu_window(h) {
        unsafe {
            let _ = InvalidateRect(Some(h), None, true);
        }
    }
    true.into()
}
