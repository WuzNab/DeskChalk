//! A 32-bit premultiplied BGRA bitmap that tiny-skia draws into directly and that is
//! pushed to a layered window with UpdateLayeredWindow (per-pixel transparency).

use tiny_skia::PixmapMut;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
    HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

pub struct Surface {
    pub w: i32,
    pub h: i32,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Surface> {
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(b) if !bits.is_null() => b,
                _ => {
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
            let old = SelectObject(dc, bmp.into());
            Some(Surface { w, h, dc, bmp, old, bits: bits as *mut u8 })
        }
    }

    /// A bitmap holding a copy of tightly packed premultiplied BGRA pixels.
    pub fn from_pixels(w: i32, h: i32, pixels: &[u8]) -> Option<Surface> {
        let mut s = Surface::new(w, h)?;
        let dst = s.pixels();
        if dst.len() != pixels.len() {
            return None;
        }
        dst.copy_from_slice(pixels);
        Some(s)
    }

    pub fn pixels(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h * 4) as usize) }
    }

    pub fn pixmap(&mut self) -> Option<PixmapMut<'_>> {
        let (w, h) = (self.w as u32, self.h as u32);
        PixmapMut::from_bytes(self.pixels(), w, h)
    }

    /// Shows the bitmap on a layered window. `pos` moves top-level windows (screen coords);
    /// child windows are positioned separately and pass None.
    pub fn present(&self, hwnd: HWND, pos: Option<POINT>) -> windows::core::Result<()> {
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                pos.as_ref().map(|p| p as *const POINT),
                Some(&size as *const SIZE),
                Some(self.dc),
                Some(&src as *const POINT),
                COLORREF(0),
                Some(&blend as *const BLENDFUNCTION),
                ULW_ALPHA,
            )
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
