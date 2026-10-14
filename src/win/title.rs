//! The small text box used to name a box in edit mode.

use super::util::wide;
use super::WM_TITLE_DONE;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW,
    DEFAULT_CHARSET, FW_NORMAL, HBRUSH, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::Controls::{EM_LIMITTEXT, EM_SETCUEBANNER, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowTextW, PostMessageW, RegisterClassExW,
    SendMessageW, SetForegroundWindow, ShowWindow, ES_AUTOHSCROLL, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFOCUS, WM_SETFONT, WNDCLASSEXW,
    WS_BORDER, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

const CLASS: PCWSTR = w!("DeskChalk.Title");

pub struct TitleEditor {
    popup: HWND,
    edit: HWND,
    pub zone: u64,
    pub owner: HWND,
    font: HFONT,
}

pub fn register(hinst: HINSTANCE) {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(popup_proc),
        hInstance: hinst,
        lpszClassName: CLASS,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut _),
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_SETFOCUS {
        // Keep the caret in the text field when the popup is activated.
        if let Ok(edit) = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::GW_CHILD) } {
            let _ = unsafe { SetFocus(Some(edit)) };
            return LRESULT(0);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    main: usize,
) -> LRESULT {
    let main = HWND(main as *mut _);
    let done = |commit: bool| unsafe {
        let _ = PostMessageW(Some(main), WM_TITLE_DONE, WPARAM(commit as usize), LPARAM(0));
    };
    match msg {
        WM_KEYDOWN if wp.0 == VK_RETURN.0 as usize => {
            done(true);
            return LRESULT(0);
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            done(false);
            return LRESULT(0);
        }
        // Swallow the characters so the edit control doesn't beep.
        WM_CHAR if wp.0 == 13 || wp.0 == 27 => return LRESULT(0),
        WM_KILLFOCUS => done(true),
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

impl TitleEditor {
    /// Opens the editor at `r` (screen coordinates). `main` receives WM_TITLE_DONE.
    pub fn open(owner: HWND, main: HWND, r: RECT, text: &str, scale: f32, zone: u64) -> Option<TitleEditor> {
        unsafe {
            let hinst: HINSTANCE = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?.into();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let popup = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS,
                w!("Box title"),
                WS_POPUP | WS_BORDER,
                r.left,
                r.top,
                w,
                h,
                Some(owner),
                None,
                Some(hinst),
                None,
            )
            .ok()?;
            let fh = (15.0 * scale).round() as i32;
            let font = CreateFontW(
                -fh,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let pad = (8.0 * scale) as i32;
            let eh = (fh as f32 * 1.45) as i32;
            let (cw, ch) = (w - 2, h - 2);
            let Ok(edit) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                &HSTRING::from(text),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                pad,
                ((ch - eh) / 2).max(0),
                (cw - 2 * pad).max(20),
                eh,
                Some(popup),
                None,
                Some(hinst),
                None,
            ) else {
                let _ = DestroyWindow(popup);
                let _ = DeleteObject(font.into());
                return None;
            };
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            SendMessageW(edit, EM_LIMITTEXT, Some(WPARAM(120)), None);
            let cue = wide("Name this box, then press Enter");
            SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, main.0 as usize);
            let _ = ShowWindow(popup, SW_SHOW);
            let _ = SetForegroundWindow(popup);
            let _ = SetFocus(Some(edit));
            Some(TitleEditor { popup, edit, zone, owner, font })
        }
    }

    pub fn text(&self) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(self.edit, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    pub fn close(self) {
        unsafe {
            let _ = RemoveWindowSubclass(self.edit, Some(edit_proc), 1);
            let _ = DestroyWindow(self.popup);
            let _ = DeleteObject(self.font.into());
        }
    }
}
