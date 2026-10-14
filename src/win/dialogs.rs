//! Message boxes and the color picker.

use crate::model::Rgb;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::UI::Controls::Dialogs::{
    ChooseColorW, CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MESSAGEBOX_RESULT,
    MESSAGEBOX_STYLE,
};

fn show(owner: Option<HWND>, text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(owner, &HSTRING::from(text), w!("DeskChalk"), style | MB_SETFOREGROUND | MB_TOPMOST) }
}

pub fn ask(owner: Option<HWND>, text: &str) -> bool {
    show(owner, text, MB_YESNO | MB_ICONQUESTION) == IDYES
}

fn to_colorref(c: Rgb) -> COLORREF {
    COLORREF(c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16)
}

fn from_colorref(c: COLORREF) -> Rgb {
    Rgb(c.0 as u8, (c.0 >> 8) as u8, (c.0 >> 16) as u8)
}

/// The standard Windows color dialog. Returns the color and the updated custom colors.
pub fn choose_color(owner: Option<HWND>, initial: Rgb, custom: &[Rgb]) -> Option<(Rgb, Vec<Rgb>)> {
    let mut cust = [COLORREF(0x00FF_FFFF); 16];
    for (slot, c) in cust.iter_mut().zip(custom) {
        *slot = to_colorref(*c);
    }
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner.unwrap_or_default(),
        rgbResult: to_colorref(initial),
        lpCustColors: cust.as_mut_ptr(),
        Flags: CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR,
        ..Default::default()
    };
    unsafe {
        if ChooseColorW(&mut cc).as_bool() {
            Some((from_colorref(cc.rgbResult), cust.iter().map(|c| from_colorref(*c)).collect()))
        } else {
            None
        }
    }
}
