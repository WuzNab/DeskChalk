//! DirectComposition output for the desktop layer.
//!
//! Since Windows 11 24H2, Progman has no redirection surface: the icons (SHELLDLL_DefView)
//! and the wallpaper (WorkerW) are composited by DWM directly. A GDI- or
//! SetLayeredWindowAttributes-painted child ends up opaque, and UpdateLayeredWindow children
//! aren't shown at all. A child window whose content is a DirectComposition visual is
//! composited with real per-pixel alpha, just like Explorer's own wallpaper window.

use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_9_1, D3D_FEATURE_LEVEL_9_3,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;

pub struct Compositor {
    d3d: ID3D11Device,
    ctx: ID3D11DeviceContext,
    dcomp: IDCompositionDevice,
}

/// The composition state of one layer window.
pub struct CompLayer {
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    surface: Option<(IDCompositionSurface, u32, u32)>,
}

fn create_d3d(kind: D3D_DRIVER_TYPE) -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_3,
        D3D_FEATURE_LEVEL_9_1,
    ];
    let mut device = None;
    let mut ctx = None;
    unsafe {
        D3D11CreateDevice(
            None,
            kind,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )
        .ok()?;
    }
    Some((device?, ctx?))
}

impl Compositor {
    pub fn new() -> Option<Compositor> {
        // WARP (software) is plenty for uploading a static bitmap that DWM then composites,
        // and costs a few MB instead of the ~40 MB a hardware GPU device pulls in.
        let (d3d, ctx) = create_d3d(D3D_DRIVER_TYPE_WARP).or_else(|| create_d3d(D3D_DRIVER_TYPE_HARDWARE))?;
        let dxgi: IDXGIDevice = d3d.cast().ok()?;
        let dcomp: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi).ok()? };
        Some(Compositor { d3d, ctx, dcomp })
    }

    /// False after a GPU reset or driver update; the caller then rebuilds everything.
    pub fn healthy(&self) -> bool {
        unsafe { self.d3d.GetDeviceRemovedReason().is_ok() }
    }

    pub fn attach(&self, hwnd: HWND) -> Option<CompLayer> {
        unsafe {
            let target = self.dcomp.CreateTargetForHwnd(hwnd, true).ok()?;
            let visual = self.dcomp.CreateVisual().ok()?;
            target.SetRoot(&visual).ok()?;
            Some(CompLayer { _target: target, visual, surface: None })
        }
    }

    /// Shows premultiplied BGRA pixels (`w` x `h`, tightly packed) in the layer.
    pub fn present(&self, layer: &mut CompLayer, w: u32, h: u32, pixels: &[u8]) -> windows::core::Result<()> {
        unsafe {
            let surface = match &layer.surface {
                Some((s, sw, sh)) if *sw == w && *sh == h => s.clone(),
                _ => {
                    let s = self.dcomp.CreateSurface(w, h, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?;
                    layer.visual.SetContent(&s)?;
                    layer.surface = Some((s.clone(), w, h));
                    s
                }
            };
            let mut offset = POINT::default();
            let tex: ID3D11Texture2D = surface.BeginDraw(None, &mut offset)?;
            let dst = D3D11_BOX {
                left: offset.x as u32,
                top: offset.y as u32,
                front: 0,
                right: offset.x as u32 + w,
                bottom: offset.y as u32 + h,
                back: 1,
            };
            self.ctx.UpdateSubresource(&tex, 0, Some(&dst), pixels.as_ptr() as *const _, w * 4, 0);
            surface.EndDraw()?;
            self.dcomp.Commit()
        }
    }
}
