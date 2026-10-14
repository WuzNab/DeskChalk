//! Edit-mode interaction: drawing, moving and resizing boxes, plus undo/redo.
//! Platform code feeds it mouse positions in monitor-relative pixels.

use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use crate::model::{Zone, ZoneStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    Body(u64),
    Handle(u64, Handle),
}

/// What the editor needs to know about the monitor under the mouse.
pub struct MonCtx<'a> {
    pub key: &'a str,
    pub hw: &'a str,
    /// Monitor size in physical pixels.
    pub size: (f32, f32),
    pub scale: f32,
    pub grid: &'a Grid,
    /// Area boxes are kept inside (the work area), monitor-relative.
    pub bounds: Rect,
}

/// A box's rect on a monitor of the given size (rescaled if the resolution changed).
pub fn zone_rect(z: &Zone, size: (f32, f32)) -> Rect {
    let (rw, rh) = (z.ref_size.0.max(1) as f32, z.ref_size.1.max(1) as f32);
    if (rw - size.0).abs() < 0.5 && (rh - size.1).abs() < 0.5 {
        return z.rect;
    }
    let (sx, sy) = (size.0 / rw, size.1 / rh);
    Rect::new(z.rect.x * sx, z.rect.y * sy, z.rect.w * sx, z.rect.h * sy)
}

pub fn set_zone_rect(z: &mut Zone, r: Rect, ctx: &MonCtx) {
    z.rect = r;
    z.ref_size = (ctx.size.0.round() as u32, ctx.size.1.round() as u32);
    z.monitor = ctx.key.to_string();
    z.monitor_hw = ctx.hw.to_string();
}

pub fn new_zone_id(zones: &[Zone]) -> u64 {
    let mut id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1);
    while zones.iter().any(|z| z.id == id) {
        id += 1;
    }
    id
}

#[derive(Clone, Copy, Debug)]
enum DragKind {
    Create { anchor: Pt },
    Move { id: u64, orig: Rect },
    Resize { id: u64, handle: Handle, orig: Rect },
}

pub struct Drag {
    kind: DragKind,
    start: Pt,
    pub rect: Rect,
    /// False until the mouse moved a few pixels (a plain click doesn't change anything).
    pub active: bool,
    pub monitor: String,
    snapshot: Vec<Zone>,
}

impl Drag {
    pub fn is_create(&self) -> bool {
        matches!(self.kind, DragKind::Create { .. })
    }

    /// The box being moved or resized, if any.
    pub fn zone_id(&self) -> Option<u64> {
        match self.kind {
            DragKind::Create { .. } => None,
            DragKind::Move { id, .. } | DragKind::Resize { id, .. } => Some(id),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Created(u64),
    Changed,
}

#[derive(Default)]
pub struct Editor {
    pub selected: Option<u64>,
    pub drag: Option<Drag>,
    undo: Vec<Vec<Zone>>,
    redo: Vec<Vec<Zone>>,
}

const UNDO_LIMIT: usize = 100;

impl Editor {
    pub fn hit_test(zones: &[Zone], ctx: &MonCtx, p: Pt) -> Hit {
        let tol = 6.0 * ctx.scale;
        for z in zones.iter().rev().filter(|z| z.monitor == ctx.key) {
            let r = zone_rect(z, ctx.size);
            let outer = Rect::new(r.x - tol, r.y - tol, r.w + 2.0 * tol, r.h + 2.0 * tol);
            if !outer.contains(p) {
                continue;
            }
            let l = (p.x - r.x).abs() <= tol;
            let rt = (p.x - r.right()).abs() <= tol;
            let tp = (p.y - r.y).abs() <= tol;
            let bt = (p.y - r.bottom()).abs() <= tol;
            let handle = match (tp, bt, l, rt) {
                (true, _, true, _) => Some(Handle::NW),
                (true, _, _, true) => Some(Handle::NE),
                (_, true, true, _) => Some(Handle::SW),
                (_, true, _, true) => Some(Handle::SE),
                (true, _, _, _) => Some(Handle::N),
                (_, true, _, _) => Some(Handle::S),
                (_, _, true, _) => Some(Handle::W),
                (_, _, _, true) => Some(Handle::E),
                _ => None,
            };
            if let Some(h) = handle {
                return Hit::Handle(z.id, h);
            }
            if r.contains(p) {
                return Hit::Body(z.id);
            }
        }
        Hit::None
    }

    pub fn begin(&mut self, zones: &[Zone], ctx: &MonCtx, p: Pt) {
        let rect_of = |id: u64| zones.iter().find(|z| z.id == id).map(|z| zone_rect(z, ctx.size));
        let (kind, rect) = match Self::hit_test(zones, ctx, p) {
            Hit::Handle(id, handle) => {
                self.selected = Some(id);
                let orig = rect_of(id).unwrap_or_default();
                (DragKind::Resize { id, handle, orig }, orig)
            }
            Hit::Body(id) => {
                self.selected = Some(id);
                let orig = rect_of(id).unwrap_or_default();
                (DragKind::Move { id, orig }, orig)
            }
            Hit::None => {
                self.selected = None;
                (DragKind::Create { anchor: p }, Rect::new(p.x, p.y, 0.0, 0.0))
            }
        };
        self.drag = Some(Drag {
            kind,
            start: p,
            rect,
            active: false,
            monitor: ctx.key.to_string(),
            snapshot: zones.to_vec(),
        });
    }

    /// Updates the drag for a new mouse position. Returns true if a redraw is needed.
    pub fn update(&mut self, ctx: &MonCtx, p: Pt, snap: bool) -> bool {
        let Some(d) = self.drag.as_mut() else { return false };
        if d.monitor != ctx.key {
            return false;
        }
        if !d.active {
            let th = 3.0 * ctx.scale;
            if (p.x - d.start.x).abs() < th && (p.y - d.start.y).abs() < th {
                return false;
            }
            d.active = true;
        }
        let g = ctx.grid;
        let b = ctx.bounds;
        let px = p.x.clamp(b.x, b.right());
        let py = p.y.clamp(b.y, b.bottom());
        let min = if snap { (g.cell.x, g.cell.y) } else { (12.0 * ctx.scale, 12.0 * ctx.scale) };
        let new_rect = match d.kind {
            DragKind::Create { anchor } => {
                if snap {
                    // Every icon slot the drag touches is included.
                    let x0 = g.floor_x(anchor.x.min(px));
                    let y0 = g.floor_y(anchor.y.min(py));
                    let x1 = g.ceil_x(anchor.x.max(px)).max(x0 + g.cell.x);
                    let y1 = g.ceil_y(anchor.y.max(py)).max(y0 + g.cell.y);
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                } else {
                    Rect::from_corners(anchor, Pt::new(px, py))
                }
            }
            DragKind::Move { orig, .. } => {
                let mut r = orig.offset(p.x - d.start.x, p.y - d.start.y);
                if snap {
                    r.x = g.snap_x(r.x);
                    r.y = g.snap_y(r.y);
                }
                r.clamp_into(&b)
            }
            DragKind::Resize { handle, orig, .. } => {
                let sx = if snap { g.snap_x(px) } else { px };
                let sy = if snap { g.snap_y(py) } else { py };
                let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
                let (west, east, north, south) = match handle {
                    Handle::N => (false, false, true, false),
                    Handle::S => (false, false, false, true),
                    Handle::W => (true, false, false, false),
                    Handle::E => (false, true, false, false),
                    Handle::NW => (true, false, true, false),
                    Handle::NE => (false, true, true, false),
                    Handle::SW => (true, false, false, true),
                    Handle::SE => (false, true, false, true),
                };
                if west {
                    x0 = sx.min(x1 - min.0);
                }
                if east {
                    x1 = sx.max(x0 + min.0);
                }
                if north {
                    y0 = sy.min(y1 - min.1);
                }
                if south {
                    y1 = sy.max(y0 + min.1);
                }
                Rect::new(x0, y0, x1 - x0, y1 - y0)
            }
        };
        let changed = new_rect != d.rect;
        d.rect = new_rect;
        changed
    }

    pub fn finish(&mut self, zones: &mut Vec<Zone>, ctx: &MonCtx, defaults: &ZoneStyle) -> Outcome {
        let Some(d) = self.drag.take() else { return Outcome::None };
        if !d.active {
            return Outcome::Redraw;
        }
        match d.kind {
            DragKind::Create { .. } => {
                let min = 8.0 * ctx.scale;
                if d.rect.w < min || d.rect.h < min {
                    return Outcome::Redraw;
                }
                self.push_undo(d.snapshot);
                let id = new_zone_id(zones);
                let mut zone = Zone {
                    id,
                    monitor: String::new(),
                    monitor_hw: String::new(),
                    rect: d.rect,
                    ref_size: (0, 0),
                    title: String::new(),
                    style: defaults.clone(),
                };
                set_zone_rect(&mut zone, d.rect, ctx);
                zones.push(zone);
                self.selected = Some(id);
                Outcome::Created(id)
            }
            DragKind::Move { id, orig } | DragKind::Resize { id, orig, .. } => {
                if d.rect == orig {
                    return Outcome::Redraw;
                }
                if let Some(z) = zones.iter_mut().find(|z| z.id == id) {
                    set_zone_rect(z, d.rect, ctx);
                }
                self.push_undo(d.snapshot);
                Outcome::Changed
            }
        }
    }

    pub fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// Where a box should be drawn right now (follows an active drag).
    pub fn display_rect(&self, z: &Zone, size: (f32, f32)) -> Rect {
        match &self.drag {
            Some(d) if d.active && d.zone_id() == Some(z.id) => d.rect,
            _ => zone_rect(z, size),
        }
    }

    /// Records the current state so the next change can be undone.
    pub fn checkpoint(&mut self, zones: &[Zone]) {
        self.push_undo(zones.to_vec());
    }

    fn push_undo(&mut self, snapshot: Vec<Zone>) {
        self.undo.push(snapshot);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, zones: &mut Vec<Zone>) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(zones, prev));
                self.fix_selection(zones);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, zones: &mut Vec<Zone>) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(zones, next));
                self.fix_selection(zones);
                true
            }
            None => false,
        }
    }

    fn fix_selection(&mut self, zones: &[Zone]) {
        if let Some(id) = self.selected {
            if !zones.iter().any(|z| z.id == id) {
                self.selected = None;
            }
        }
    }

    pub fn delete_selected(&mut self, zones: &mut Vec<Zone>) -> bool {
        let Some(id) = self.selected.take() else { return false };
        if !zones.iter().any(|z| z.id == id) {
            return false;
        }
        self.checkpoint(zones);
        zones.retain(|z| z.id != id);
        true
    }

    /// Moves the selected box by whole grid cells (snap) or pixels.
    pub fn nudge(&mut self, zones: &mut [Zone], ctx: &MonCtx, dx: i32, dy: i32, snap: bool, big: bool) -> bool {
        let Some(id) = self.selected else { return false };
        let Some(idx) = zones.iter().position(|z| z.id == id && z.monitor == ctx.key) else {
            return false;
        };
        self.checkpoint(zones);
        let step = if snap {
            (ctx.grid.cell.x, ctx.grid.cell.y)
        } else if big {
            (10.0 * ctx.scale, 10.0 * ctx.scale)
        } else {
            (1.0, 1.0)
        };
        let r = zone_rect(&zones[idx], ctx.size)
            .offset(dx as f32 * step.0, dy as f32 * step.1)
            .clamp_into(&ctx.bounds);
        set_zone_rect(&mut zones[idx], r, ctx);
        true
    }

    /// Copies the selected box next to the original and selects the copy.
    pub fn duplicate_selected(&mut self, zones: &mut Vec<Zone>, ctx: &MonCtx, snap: bool) -> Option<u64> {
        let id = self.selected?;
        let src = zones.iter().find(|z| z.id == id && z.monitor == ctx.key)?.clone();
        self.checkpoint(zones);
        let (dx, dy) = if snap { (ctx.grid.cell.x, ctx.grid.cell.y) } else { (24.0 * ctx.scale, 24.0 * ctx.scale) };
        let r = zone_rect(&src, ctx.size).offset(dx, dy).clamp_into(&ctx.bounds);
        let mut copy = src;
        copy.id = new_zone_id(zones);
        set_zone_rect(&mut copy, r, ctx);
        let new_id = copy.id;
        zones.push(copy);
        self.selected = Some(new_id);
        Some(new_id)
    }

    /// Selects the next box on this monitor (Tab).
    pub fn select_next(&mut self, zones: &[Zone], ctx: &MonCtx) -> bool {
        let ids: Vec<u64> = zones.iter().filter(|z| z.monitor == ctx.key).map(|z| z.id).collect();
        if ids.is_empty() {
            return false;
        }
        let next = match self.selected.and_then(|s| ids.iter().position(|&i| i == s)) {
            Some(i) => ids[(i + 1) % ids.len()],
            None => ids[0],
        };
        self.selected = Some(next);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(grid: &Grid) -> MonCtx<'_> {
        MonCtx {
            key: "m",
            hw: "hw",
            size: (1000.0, 800.0),
            scale: 1.0,
            grid,
            bounds: Rect::new(0.0, 0.0, 1000.0, 760.0),
        }
    }

    fn grid() -> Grid {
        Grid {
            origin: Pt::new(0.0, 0.0),
            cell: Pt::new(100.0, 100.0),
            area: Rect::new(0.0, 0.0, 1000.0, 760.0),
        }
    }

    #[test]
    fn draw_snapped_box_then_undo() {
        let g = grid();
        let c = ctx(&g);
        let mut zones = Vec::new();
        let mut ed = Editor::default();
        ed.begin(&zones, &c, Pt::new(130.0, 120.0));
        assert!(ed.update(&c, Pt::new(260.0, 150.0), true));
        let out = ed.finish(&mut zones, &c, &ZoneStyle::default());
        assert!(matches!(out, Outcome::Created(_)));
        assert_eq!(zones[0].rect, Rect::new(100.0, 100.0, 200.0, 100.0));
        assert!(ed.undo(&mut zones));
        assert!(zones.is_empty());
        assert!(ed.redo(&mut zones));
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn click_does_not_create() {
        let g = grid();
        let c = ctx(&g);
        let mut zones = Vec::new();
        let mut ed = Editor::default();
        ed.begin(&zones, &c, Pt::new(130.0, 120.0));
        ed.update(&c, Pt::new(131.0, 121.0), true);
        assert_eq!(ed.finish(&mut zones, &c, &ZoneStyle::default()), Outcome::Redraw);
        assert!(zones.is_empty());
    }

    #[test]
    fn move_and_resize_snap() {
        let g = grid();
        let c = ctx(&g);
        let mut zones = Vec::new();
        let mut ed = Editor::default();
        ed.begin(&zones, &c, Pt::new(110.0, 110.0));
        ed.update(&c, Pt::new(290.0, 290.0), true);
        ed.finish(&mut zones, &c, &ZoneStyle::default());
        // Drag the body by 140 px: snaps to one cell over.
        ed.begin(&zones, &c, Pt::new(200.0, 200.0));
        assert!(ed.drag.as_ref().unwrap().zone_id().is_some());
        ed.update(&c, Pt::new(340.0, 200.0), true);
        ed.finish(&mut zones, &c, &ZoneStyle::default());
        assert_eq!(zones[0].rect, Rect::new(200.0, 100.0, 200.0, 200.0));
        // Resize from the east edge.
        ed.begin(&zones, &c, Pt::new(400.0, 200.0));
        ed.update(&c, Pt::new(560.0, 200.0), true);
        ed.finish(&mut zones, &c, &ZoneStyle::default());
        assert_eq!(zones[0].rect, Rect::new(200.0, 100.0, 400.0, 200.0));
    }

    #[test]
    fn rescales_after_resolution_change() {
        let z = Zone {
            id: 1,
            monitor: "m".into(),
            monitor_hw: String::new(),
            rect: Rect::new(100.0, 100.0, 200.0, 100.0),
            ref_size: (1000, 800),
            title: String::new(),
            style: ZoneStyle::default(),
        };
        assert_eq!(zone_rect(&z, (2000.0, 1600.0)), Rect::new(200.0, 200.0, 400.0, 200.0));
    }
}
