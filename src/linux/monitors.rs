//! Screens, with stable names so boxes stay on "their" screen.

use crate::editor::MonCtx;
use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::platform::wayland::ActiveEventLoopExtWayland;

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Connector name ("DP-1"), or size and position if the system gives none.
    pub key: String,
    /// Position and size in physical pixels.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    pub primary: bool,
    pub grid: Grid,
    /// A Wayland session (screens are arranged in scaled units, see `layout`).
    pub wayland: bool,
}

impl Monitor {
    pub fn size(&self) -> (f32, f32) {
        (self.w as f32, self.h as f32)
    }

    pub fn full(&self) -> Rect {
        Rect::new(0.0, 0.0, self.w as f32, self.h as f32)
    }

    /// Position and size in the units the desktop arranges screens in. Wayland desktops use
    /// scaled (logical) units, and winit reports a position as that times the screen's own
    /// scale; X11 arranges screens in pixels.
    pub fn layout(&self) -> (f32, f32, f32, f32) {
        let s = if self.wayland { self.scale.max(0.1) } else { 1.0 };
        (self.x as f32 / s, self.y as f32 / s, self.w as f32 / s, self.h as f32 / s)
    }

    pub fn ctx(&self) -> MonCtx<'_> {
        MonCtx { key: &self.key, hw: &self.key, size: self.size(), scale: self.scale, grid: &self.grid, bounds: self.full() }
    }

    /// Same screen in the same place (ignores the dot spacing).
    pub fn same_geometry(&self, o: &Monitor) -> bool {
        self.key == o.key && (self.x, self.y, self.w, self.h) == (o.x, o.y, o.w, o.h) && self.scale == o.scale
    }
}

/// A dot grid with `spacing` (device-independent pixels), centered on the screen.
pub fn grid_for(w: u32, h: u32, scale: f32, spacing: f32) -> Grid {
    let cell = (spacing * scale).max(8.0);
    let (w, h) = (w as f32, h as f32);
    let ox = (w - (w / cell).floor() * cell) / 2.0;
    let oy = (h - (h / cell).floor() * cell) / 2.0;
    Grid { origin: Pt::new(ox, oy), cell: Pt::new(cell, cell), area: Rect::new(0.0, 0.0, w, h) }
}

pub fn read(el: &ActiveEventLoop, spacing: f32) -> Vec<(Monitor, MonitorHandle)> {
    let primary = el.primary_monitor();
    let wayland = el.is_wayland();
    let mut out: Vec<(Monitor, MonitorHandle)> = el
        .available_monitors()
        .filter_map(|h| {
            let size = h.size();
            if size.width == 0 || size.height == 0 {
                return None;
            }
            let pos = h.position();
            let scale = h.scale_factor() as f32;
            let key = h
                .name()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("{}x{}@{},{}", size.width, size.height, pos.x, pos.y));
            let m = Monitor {
                key,
                x: pos.x,
                y: pos.y,
                w: size.width,
                h: size.height,
                scale,
                primary: primary.as_ref() == Some(&h),
                grid: grid_for(size.width, size.height, scale, spacing),
                wayland,
            };
            Some((m, h))
        })
        .collect();
    out.sort_by_key(|(m, _)| (m.x, m.y));
    if !out.iter().any(|(m, _)| m.primary) {
        if let Some((m, _)) = out.iter_mut().min_by_key(|(m, _)| (m.x.abs() + m.y.abs(), m.x)) {
            m.primary = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_centered() {
        let g = grid_for(1000, 500, 1.0, 96.0);
        assert_eq!(g.cell, Pt::new(96.0, 96.0));
        assert!((g.origin.x - 20.0).abs() < 0.01);
        assert!((g.origin.y - 10.0).abs() < 0.01);
    }
}
