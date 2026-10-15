//! The fullscreen windows used while editing. Each shows its screen's wallpaper (dimmed) with
//! the boxes, the dot grid and the editing helpers on top, drawn in software and shown with
//! softbuffer (X11 and Wayland alike).

use super::bake;
use super::monitors::Monitor;
use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use std::num::NonZeroU32;
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapMut};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowLevel};

/// How much the wallpaper is darkened while editing.
const DIM: f32 = 0.2;

pub struct EditWin {
    pub window: Arc<Window>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    _context: softbuffer::Context<Arc<Window>>,
    /// The screen, sized like the window (that is what editing works in).
    pub mon: Monitor,
    /// The wallpaper at the screen's size (rescaled if the window ends up another size).
    wallpaper: Pixmap,
    base: Vec<u32>,
    frame: Vec<u32>,
    surface_size: (u32, u32),
    /// Areas changed in the last few frames, newest last (for buffers that are a few
    /// frames old).
    history: Vec<Rect>,
    /// Waiting to be drawn at the next redraw: None, an area, or everything.
    pending: Option<Rect>,
}

/// 0xFFRRGGBB, darkened: what the window shows under everything.
fn dimmed(pm: &Pixmap) -> Vec<u32> {
    let k = 1.0 - DIM;
    pm.data()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let c = |v: u8| (v as f32 * k).round() as u32;
            0xFF00_0000 | (c(p[0]) << 16) | (c(p[1]) << 8) | c(p[2])
        })
        .collect()
}

/// A grid for a screen `sx` x `sy` times the size.
fn scaled(g: Grid, sx: f32, sy: f32) -> Grid {
    if (sx - 1.0).abs() < 1e-4 && (sy - 1.0).abs() < 1e-4 {
        return g;
    }
    Grid {
        origin: Pt::new(g.origin.x * sx, g.origin.y * sy),
        cell: Pt::new(g.cell.x * sx, g.cell.y * sy),
        area: Rect::new(g.area.x * sx, g.area.y * sy, g.area.w * sx, g.area.h * sy),
    }
}

fn bytes(buf: &mut [u32]) -> &mut [u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8; the length is exact.
    unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 4) }
}

impl EditWin {
    pub fn new(el: &ActiveEventLoop, handle: &MonitorHandle, mon: Monitor, wallpaper: Pixmap) -> Option<EditWin> {
        let attrs = Window::default_attributes()
            .with_title("DeskChalk")
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_fullscreen(Some(Fullscreen::Borderless(Some(handle.clone()))));
        let window = Arc::new(el.create_window(attrs).map_err(|e| log!("could not open the edit window: {e}")).ok()?);
        let context = softbuffer::Context::new(window.clone()).map_err(|e| log!("softbuffer: {e}")).ok()?;
        let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| log!("softbuffer: {e}")).ok()?;
        let base = dimmed(&wallpaper);
        let frame = base.clone();
        let mut w = EditWin {
            window,
            surface,
            _context: context,
            mon,
            wallpaper,
            base,
            frame,
            surface_size: (0, 0),
            history: Vec::new(),
            pending: None,
        };
        w.sync_size();
        w.invalidate(None);
        Some(w)
    }

    /// Follows the window's real size (fullscreen happens a moment after creation).
    /// True if it changed.
    pub fn sync_size(&mut self) -> bool {
        let size = self.window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        if (w, h) == (self.mon.w, self.mon.h) && self.base.len() == (w * h) as usize {
            return false;
        }
        let (sx, sy) = (w as f32 / self.mon.w.max(1) as f32, h as f32 / self.mon.h.max(1) as f32);
        self.mon.grid = scaled(self.mon.grid, sx, sy);
        self.mon.w = w;
        self.mon.h = h;
        // Scaled from the full-size wallpaper every time (never from an earlier scaled copy).
        let scaled = ((self.wallpaper.width(), self.wallpaper.height()) != (w, h))
            .then(|| super::image::cover(&self.wallpaper, w, h))
            .flatten();
        self.base = dimmed(scaled.as_ref().unwrap_or(&self.wallpaper));
        self.frame = self.base.clone();
        self.history.clear();
        self.invalidate(None);
        true
    }

    /// Uses `grid` (made for a screen of `size` pixels) for this window.
    pub fn set_grid(&mut self, grid: Grid, size: (u32, u32)) {
        let g = scaled(grid, self.mon.w as f32 / size.0.max(1) as f32, self.mon.h as f32 / size.1.max(1) as f32);
        if g != self.mon.grid {
            self.mon.grid = g;
            self.invalidate(None);
        }
    }

    /// Asks for `region` (or everything) to be drawn at the next redraw.
    pub fn invalidate(&mut self, region: Option<Rect>) {
        let full = self.mon.full();
        let r = region.map_or(full, |r| r.intersect(&full).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)));
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        self.pending = Some(self.pending.map_or(r, |p| p.union(&r)));
        self.window.request_redraw();
    }

    /// Draws what was invalidated: `draw` paints on top of the wallpaper into a buffer whose
    /// top-left is at the given monitor position.
    pub fn redraw(&mut self, draw: impl FnOnce(&mut PixmapMut, Pt)) {
        let Some(region) = self.pending.take() else { return };
        let (w, h) = (self.mon.w as i32, self.mon.h as i32);
        let (x, y, rw, rh) = region.round_out();
        let (x, y) = (x.clamp(0, w), y.clamp(0, h));
        let (rw, rh) = (rw.min(w - x), rh.min(h - y));
        if rw <= 0 || rh <= 0 {
            return;
        }
        let mut tmp: Vec<u32> = Vec::with_capacity((rw * rh) as usize);
        for row in y..y + rh {
            let s = (row * w + x) as usize;
            tmp.extend_from_slice(&self.base[s..s + rw as usize]);
        }
        if let Some(mut pm) = PixmapMut::from_bytes(bytes(&mut tmp), rw as u32, rh as u32) {
            draw(&mut pm, Pt::new(x as f32, y as f32));
        }
        for row in 0..rh {
            let d = ((y + row) * w + x) as usize;
            let s = (row * rw) as usize;
            self.frame[d..d + rw as usize].copy_from_slice(&tmp[s..s + rw as usize]);
        }
        self.present(Rect::new(x as f32, y as f32, rw as f32, rh as f32));
    }

    fn present(&mut self, dirty: Rect) {
        let (w, h) = (self.mon.w, self.mon.h);
        if self.surface_size != (w, h) {
            let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else { return };
            if self.surface.resize(nw, nh).is_err() {
                return;
            }
            self.surface_size = (w, h);
            self.history.clear();
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        if buf.len() != self.frame.len() {
            return;
        }
        self.history.push(dirty);
        if self.history.len() > 4 {
            self.history.remove(0);
        }
        // The buffer may be a few frames old: bring over everything that changed since.
        let age = buf.age() as usize;
        let copy = if age == 0 || age > self.history.len() {
            self.mon.full()
        } else {
            self.history[self.history.len() - age..].iter().fold(dirty, |a, r| a.union(r))
        };
        let (cx, cy, cw, ch) = copy.round_out();
        let (cx, cy) = (cx.clamp(0, w as i32) as usize, cy.clamp(0, h as i32) as usize);
        let cw = (cw.max(0) as usize).min(w as usize - cx);
        let ch = (ch.max(0) as usize).min(h as usize - cy);
        for row in cy..cy + ch {
            let s = row * w as usize + cx;
            buf[s..s + cw].copy_from_slice(&self.frame[s..s + cw]);
        }
        let damage = [softbuffer::Rect {
            x: cx as u32,
            y: cy as u32,
            width: NonZeroU32::new(cw.max(1) as u32).unwrap_or(NonZeroU32::MIN),
            height: NonZeroU32::new(ch.max(1) as u32).unwrap_or(NonZeroU32::MIN),
        }];
        if let Err(e) = buf.present_with_damage(&damage) {
            log!("presenting the edit window failed: {e}");
        }
    }
}

/// The wallpaper for an edit window (the original wallpaper, never DeskChalk's own picture;
/// a plain gray one if it can't be read).
pub fn wallpaper_for(base: Option<&super::wallpaper::Base>, mon: &Monitor, cache: &mut std::collections::HashMap<std::path::PathBuf, Option<Pixmap>>) -> Option<Pixmap> {
    bake::background(base, mon.w, mon.h, cache).ok().or_else(|| super::image::solid(mon.w, mon.h, crate::model::Rgb(0x30, 0x30, 0x30)))
}
