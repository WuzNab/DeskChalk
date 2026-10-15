//! Menus drawn inside the edit windows (right-click a box or empty space). A tray app can't
//! open a native menu over a fullscreen window on every Linux desktop, so DeskChalk draws its
//! own: items, check marks, color swatches and submenus.

use crate::geom::{Pt, Rect};
use crate::model::Rgb;
use crate::render::{self, Painter};
use crate::text::Fonts;
use tiny_skia::{FillRule, LineCap, LineJoin, PathBuilder, PixmapMut, Stroke, Transform};

#[derive(Clone)]
pub enum Kind {
    Action,
    Check(bool),
    Radio(bool),
    Swatch(Rgb, bool),
    Heading,
    Separator,
    Sub(Vec<Item>),
}

#[derive(Clone)]
pub struct Item {
    pub id: u32,
    pub label: String,
    pub shortcut: String,
    pub kind: Kind,
    pub enabled: bool,
}

/// Builds a menu with the same calls as the Windows menus.
#[derive(Default)]
pub struct Menu {
    pub items: Vec<Item>,
}

impl Menu {
    fn push(&mut self, id: u32, label: &str, kind: Kind, enabled: bool) {
        let (label, shortcut) = label.split_once('\t').unwrap_or((label, ""));
        self.items.push(Item { id, label: label.into(), shortcut: shortcut.into(), kind, enabled });
    }

    pub fn item(&mut self, id: u32, label: &str) {
        self.push(id, label, Kind::Action, true);
    }

    pub fn item_enabled(&mut self, id: u32, label: &str, enabled: bool) {
        self.push(id, label, Kind::Action, enabled);
    }

    pub fn check(&mut self, id: u32, label: &str, on: bool) {
        self.push(id, label, Kind::Check(on), true);
    }

    pub fn radio(&mut self, id: u32, label: &str, on: bool) {
        self.push(id, label, Kind::Radio(on), true);
    }

    pub fn color(&mut self, id: u32, label: &str, c: Rgb, on: bool) {
        self.push(id, label, Kind::Swatch(c, on), true);
    }

    pub fn heading(&mut self, label: &str) {
        self.push(0, label, Kind::Heading, false);
    }

    pub fn sep(&mut self) {
        self.push(0, "", Kind::Separator, false);
    }

    pub fn sub(&mut self, label: &str, child: Menu) {
        self.push(0, label, Kind::Sub(child.items), true);
    }
}

pub enum Click {
    /// Outside every open menu (closes it).
    Outside,
    Inside,
    Chosen(u32),
}

const ROW: f32 = 28.0;
const SEP: f32 = 9.0;
const HEAD: f32 = 24.0;
const PAD: f32 = 5.0;
const TEXT: f32 = 13.5;
const LEFT: f32 = 34.0;
const MIN_W: f32 = 180.0;

pub struct Popup {
    items: Vec<Item>,
    origin: Pt,
    width: f32,
    scale: f32,
    hover: Option<usize>,
    sub: Option<(usize, Box<Popup>)>,
}

impl Popup {
    /// Opens at `at` (monitor coordinates), moved or flipped to stay inside `area`.
    pub fn open(items: Vec<Item>, at: Pt, area: Rect, scale: f32, fonts: &Fonts) -> Popup {
        let mut p = Popup { items, origin: at, width: 0.0, scale, hover: None, sub: None };
        p.width = p.measure(fonts);
        let (w, h) = (p.width, p.height());
        let x = if at.x + w > area.right() { (at.x - w).max(area.x) } else { at.x };
        let y = if at.y + h > area.bottom() { (area.bottom() - h).max(area.y) } else { at.y };
        p.origin = Pt::new(x, y);
        p
    }

    fn measure(&self, fonts: &Fonts) -> f32 {
        let sc = self.scale;
        let px = TEXT * sc;
        let widest = self
            .items
            .iter()
            .map(|i| {
                let label = fonts.shape(&i.label, px).width;
                let short = if i.shortcut.is_empty() { 0.0 } else { fonts.shape(&i.shortcut, px).width + 28.0 * sc };
                label + short
            })
            .fold(0.0f32, f32::max);
        (widest + (LEFT + 34.0) * sc).max(MIN_W * sc)
    }

    fn row_h(&self, i: &Item) -> f32 {
        self.scale
            * match i.kind {
                Kind::Separator => SEP,
                Kind::Heading => HEAD,
                _ => ROW,
            }
    }

    fn height(&self) -> f32 {
        self.items.iter().map(|i| self.row_h(i)).sum::<f32>() + 2.0 * PAD * self.scale
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.origin.x, self.origin.y, self.width, self.height())
    }

    /// Everything that is drawn, open submenus and shadow included.
    pub fn bounds(&self) -> Rect {
        let r = self.rect().inset(-6.0 * self.scale);
        match &self.sub {
            Some((_, s)) => r.union(&s.bounds()),
            None => r,
        }
    }

    fn row_rect(&self, idx: usize) -> Rect {
        let mut y = self.origin.y + PAD * self.scale;
        for (i, it) in self.items.iter().enumerate() {
            let h = self.row_h(it);
            if i == idx {
                return Rect::new(self.origin.x, y, self.width, h);
            }
            y += h;
        }
        Rect::new(self.origin.x, y, self.width, 0.0)
    }

    fn row_at(&self, p: Pt) -> Option<usize> {
        if !self.rect().contains(p) {
            return None;
        }
        (0..self.items.len()).find(|&i| self.row_rect(i).contains(p))
    }

    fn contains(&self, p: Pt) -> bool {
        self.rect().contains(p) || self.sub.as_ref().is_some_and(|(_, s)| s.contains(p))
    }

    /// Tracks the mouse; opens submenus on hover. True if anything changed.
    pub fn hover(&mut self, p: Pt, area: Rect, fonts: &Fonts) -> bool {
        if let Some((_, s)) = &mut self.sub {
            if s.contains(p) {
                return s.hover(p, area, fonts);
            }
        }
        if !self.rect().contains(p) {
            // Leaving towards nothing keeps an open submenu (so it can be reached diagonally).
            let changed = self.hover.is_some() && self.sub.is_none();
            if self.sub.is_none() {
                self.hover = None;
            }
            return changed;
        }
        let row = self.row_at(p).filter(|&i| !matches!(self.items[i].kind, Kind::Separator | Kind::Heading));
        let mut changed = row != self.hover;
        self.hover = row;
        match row {
            Some(i) if matches!(self.items[i].kind, Kind::Sub(_)) => {
                if self.sub.as_ref().map(|(j, _)| *j) != Some(i) {
                    self.open_sub(i, area, fonts);
                    changed = true;
                }
            }
            _ => {
                if self.sub.take().is_some() {
                    changed = true;
                }
            }
        }
        changed
    }

    fn open_sub(&mut self, i: usize, area: Rect, fonts: &Fonts) {
        let Kind::Sub(items) = &self.items[i].kind else { return };
        let row = self.row_rect(i);
        let mut s = Popup { items: items.clone(), origin: Pt::new(0.0, 0.0), width: 0.0, scale: self.scale, hover: None, sub: None };
        s.width = s.measure(fonts);
        let overlap = 4.0 * self.scale;
        let x = if row.right() - overlap + s.width <= area.right() { row.right() - overlap } else { (row.x - s.width + overlap).max(area.x) };
        let h = s.height();
        let y = (row.y - PAD * self.scale).min(area.bottom() - h).max(area.y);
        s.origin = Pt::new(x, y);
        self.sub = Some((i, Box::new(s)));
    }

    pub fn click(&mut self, p: Pt) -> Click {
        if let Some((_, s)) = &mut self.sub {
            match s.click(p) {
                Click::Outside => {}
                other => return other,
            }
        }
        let Some(i) = self.row_at(p) else {
            return if self.rect().contains(p) { Click::Inside } else { Click::Outside };
        };
        let it = &self.items[i];
        match it.kind {
            Kind::Sub(_) | Kind::Separator | Kind::Heading => Click::Inside,
            _ if it.enabled => Click::Chosen(it.id),
            _ => Click::Inside,
        }
    }

    /// Replaces the items (new check marks after a change) and keeps the menu where it is,
    /// with the same submenu open.
    pub fn refresh(&mut self, items: Vec<Item>, area: Rect, fonts: &Fonts) {
        let same_shape = items.len() == self.items.len();
        self.items = items;
        self.width = self.width.max(self.measure(fonts));
        if !same_shape {
            self.hover = None;
            self.sub = None;
            return;
        }
        if let Some((i, s)) = &mut self.sub {
            match &self.items[*i].kind {
                Kind::Sub(children) => s.refresh(children.clone(), area, fonts),
                _ => self.sub = None,
            }
        }
    }

    pub fn draw(&self, pm: &mut PixmapMut, origin: Pt, painter: Painter, fonts: &Fonts) {
        let t = Transform::from_translate(-origin.x, -origin.y);
        let sc = self.scale;
        let r = self.rect();
        let radius = 8.0 * sc;
        if let Some(shadow) = render::rounded_rect(r.offset(0.0, 2.0 * sc).inset(-2.0 * sc), radius + 2.0 * sc) {
            pm.fill_path(&shadow, &painter.paint(Rgb(0, 0, 0), 0.28), FillRule::Winding, t, None);
        }
        if let Some(bg) = render::rounded_rect(r, radius) {
            pm.fill_path(&bg, &painter.paint(Rgb(0x2B, 0x2B, 0x2E), 0.98), FillRule::Winding, t, None);
            let stroke = Stroke { width: sc.max(1.0), ..Stroke::default() };
            pm.stroke_path(&bg, &painter.paint(Rgb(0x55, 0x55, 0x5A), 0.9), &stroke, t, None);
        }
        let px = TEXT * sc;
        let cap = fonts.cap_height(px);
        for (i, it) in self.items.iter().enumerate() {
            let row = self.row_rect(i);
            let mid = row.y + row.h / 2.0;
            match &it.kind {
                Kind::Separator => {
                    if let Some(line) = tiny_skia::Rect::from_xywh(row.x + 10.0 * sc, mid - 0.5 * sc, row.w - 20.0 * sc, sc.max(1.0)) {
                        pm.fill_rect(line, &painter.paint(Rgb(0x55, 0x55, 0x5A), 1.0), t, None);
                    }
                    continue;
                }
                Kind::Heading => {
                    let small = 11.5 * sc;
                    let shaped = fonts.shape(&it.label, small);
                    if let Some(p) = fonts.path(&shaped, small, row.x + 12.0 * sc, mid + fonts.cap_height(small) / 2.0) {
                        pm.fill_path(&p, &painter.paint(Rgb(0x9A, 0x9A, 0xA0), 1.0), FillRule::Winding, t, None);
                    }
                    continue;
                }
                _ => {}
            }
            let hovered = self.hover == Some(i) || self.sub.as_ref().is_some_and(|(j, _)| *j == i);
            if hovered && it.enabled {
                if let Some(p) = render::rounded_rect(Rect::new(row.x + 4.0 * sc, row.y + 1.0 * sc, row.w - 8.0 * sc, row.h - 2.0 * sc), 5.0 * sc) {
                    pm.fill_path(&p, &painter.paint(Rgb(0x45, 0x45, 0x4C), 1.0), FillRule::Winding, t, None);
                }
            }
            let ink = if it.enabled { Rgb(0xF2, 0xF2, 0xF2) } else { Rgb(0x7A, 0x7A, 0x80) };
            let icon = Pt::new(row.x + 17.0 * sc, mid);
            match it.kind {
                Kind::Check(true) => tick(pm, t, painter, icon, 5.5 * sc, ink, sc),
                Kind::Radio(true) => {
                    let mut pb = PathBuilder::new();
                    pb.push_circle(icon.x, icon.y, 3.5 * sc);
                    if let Some(p) = pb.finish() {
                        pm.fill_path(&p, &painter.paint(ink, 1.0), FillRule::Winding, t, None);
                    }
                }
                Kind::Swatch(c, on) => {
                    let s = 15.0 * sc;
                    let sw = Rect::new(icon.x - s / 2.0, icon.y - s / 2.0, s, s);
                    if let Some(p) = render::rounded_rect(sw, 3.0 * sc) {
                        pm.fill_path(&p, &painter.paint(c, 1.0), FillRule::Winding, t, None);
                        let stroke = Stroke { width: sc.max(1.0), ..Stroke::default() };
                        pm.stroke_path(&p, &painter.paint(Rgb(0x88, 0x88, 0x88), 0.9), &stroke, t, None);
                    }
                    if on {
                        let mark = if c.luminance() > 0.45 { Rgb(0x10, 0x10, 0x10) } else { Rgb(0xFF, 0xFF, 0xFF) };
                        tick(pm, t, painter, icon, 4.5 * sc, mark, sc);
                    }
                }
                _ => {}
            }
            let shaped = fonts.fit(&it.label, px, row.w - (LEFT + 30.0) * sc);
            if let Some(p) = fonts.path(&shaped, px, row.x + LEFT * sc, mid + cap / 2.0) {
                pm.fill_path(&p, &painter.paint(ink, 1.0), FillRule::Winding, t, None);
            }
            if !it.shortcut.is_empty() {
                let s = fonts.shape(&it.shortcut, px);
                if let Some(p) = fonts.path(&s, px, row.right() - 14.0 * sc - s.width, mid + cap / 2.0) {
                    pm.fill_path(&p, &painter.paint(Rgb(0x9A, 0x9A, 0xA0), 1.0), FillRule::Winding, t, None);
                }
            }
            if matches!(it.kind, Kind::Sub(_)) {
                let x = row.right() - 16.0 * sc;
                let mut pb = PathBuilder::new();
                pb.move_to(x - 3.0 * sc, mid - 5.0 * sc);
                pb.line_to(x + 2.0 * sc, mid);
                pb.line_to(x - 3.0 * sc, mid + 5.0 * sc);
                if let Some(p) = pb.finish() {
                    let stroke = Stroke { width: 1.6 * sc, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
                    pm.stroke_path(&p, &painter.paint(ink, 1.0), &stroke, t, None);
                }
            }
        }
        if let Some((_, s)) = &self.sub {
            s.draw(pm, origin, painter, fonts);
        }
    }
}

fn tick(pm: &mut PixmapMut, t: Transform, painter: Painter, c: Pt, s: f32, ink: Rgb, sc: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(c.x - s, c.y);
    pb.line_to(c.x - s * 0.3, c.y + s * 0.7);
    pb.line_to(c.x + s, c.y - s * 0.75);
    if let Some(p) = pb.finish() {
        let stroke = Stroke { width: 1.8 * sc, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(&p, &painter.paint(ink, 1.0), &stroke, t, None);
    }
}
