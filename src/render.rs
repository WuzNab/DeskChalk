//! Draws boxes, titles, grid dots and edit-mode decorations into a pixel buffer.
//!
//! Order: grid dots, then every box background (mixed where boxes overlap), then all
//! borders, then all titles. Drawing borders after all backgrounds keeps every outline
//! crisp even where boxes overlap.

use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use crate::model::{BorderStyle, HAlign, Rgb, TitleStyle, VAlign, Zone, ZoneStyle};
use crate::text::{Fonts, Shaped};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, PixmapMut, Stroke, StrokeDash,
    Transform,
};

/// Converts colors for the target pixel layout. Windows DIBs are BGRA, so red and blue swap.
#[derive(Clone, Copy)]
pub struct Painter {
    pub swap_rb: bool,
}

impl Painter {
    pub fn color(&self, c: Rgb, alpha: f32) -> Color {
        let (r, b) = if self.swap_rb { (c.2, c.0) } else { (c.0, c.2) };
        Color::from_rgba8(r, c.1, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    pub fn paint(&self, c: Rgb, alpha: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(self.color(c, alpha));
        p.anti_alias = true;
        p
    }
}

/// Fills a premultiplied 32-bit buffer with black at `alpha` (0 clears it).
pub fn clear(buf: &mut [u8], alpha: f32) {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a == 0 {
        buf.fill(0);
    } else {
        for px in buf.as_chunks_mut::<4>().0 {
            *px = [0, 0, 0, a];
        }
    }
}

pub struct GridLayer<'a> {
    pub grid: &'a Grid,
    pub color: Rgb,
    pub radius: f32,
    pub alpha: f32,
    /// Mouse position and strength (0..1): dots near it drift away.
    pub repel: Option<(Pt, f32)>,
}

pub struct ZoneItem<'a> {
    pub zone: &'a Zone,
    pub rect: Rect,
    pub selected: bool,
}

pub struct EditLayer<'a> {
    pub hint: Option<&'a str>,
    /// Small label near the cursor (e.g. "3 × 2").
    pub label: Option<(String, Pt)>,
    /// A box being drawn right now.
    pub draft: Option<(Rect, &'a ZoneStyle)>,
}

pub struct Scene<'a> {
    /// Monitor-relative position of the buffer's top-left pixel.
    pub origin: Pt,
    pub scale: f32,
    pub painter: Painter,
    pub title_fonts: Option<&'a Fonts>,
    pub ui_fonts: Option<&'a Fonts>,
    pub grid: Option<GridLayer<'a>>,
    pub zones: Vec<ZoneItem<'a>>,
    pub edit: Option<EditLayer<'a>>,
    /// Blend colors where boxes overlap.
    pub mix_colors: bool,
    /// Titles are kept inside this area (the monitor), so one on a box's top line at the
    /// screen edge isn't cut off.
    pub bounds: Rect,
}

/// How far dots near the mouse are pushed (relative to the scale), and the reach.
const REPEL_RADIUS: f32 = 150.0;
const REPEL_PUSH: f32 = 16.0;

/// The part of the screen that changes when the repel point moves (monitor coords).
pub fn repel_bounds(at: Pt, scale: f32) -> Rect {
    let r = (REPEL_RADIUS + REPEL_PUSH + 6.0) * scale;
    Rect::new(at.x - r, at.y - r, 2.0 * r, 2.0 * r)
}

struct BoxDraw<'a> {
    /// The box shape (slightly inset from the stored rect so neighbours don't touch).
    shape: Rect,
    rect: Rect,
    style: &'a ZoneStyle,
    title: Option<TitleLayout>,
    selected: bool,
}

struct TitleLayout {
    shaped: Shaped,
    px: f32,
    x: f32,
    baseline: f32,
    chip: Option<Rect>,
    /// (bottom edge?, x0, x1): where the border is interrupted for the title.
    gap: Option<(bool, f32, f32)>,
}

pub fn draw(pm: &mut PixmapMut, s: &Scene) {
    let t = Transform::from_translate(-s.origin.x, -s.origin.y);
    let visible = Rect::new(s.origin.x, s.origin.y, pm.width() as f32, pm.height() as f32);
    if let Some(g) = &s.grid {
        draw_grid(pm, t, s, g, &visible);
    }

    let mut boxes: Vec<BoxDraw> = Vec::new();
    for z in &s.zones {
        if z.rect.intersect(&visible.inset(-40.0 * s.scale)).is_none() {
            continue;
        }
        boxes.push(make_box(s, z.rect, &z.zone.style, &z.zone.title, z.selected));
    }
    if let Some((r, style)) = s.edit.as_ref().and_then(|e| e.draft) {
        boxes.push(make_box(s, r, style, "", true));
    }

    draw_fills(pm, t, s, &boxes);
    for b in &boxes {
        draw_border(pm, t, s, b);
    }
    for b in boxes.iter().filter(|b| b.selected) {
        draw_selection(pm, t, s, b.rect);
    }
    if let Some(fonts) = s.title_fonts {
        for b in &boxes {
            if let Some(tl) = &b.title {
                draw_title(pm, t, s, fonts, b.style, tl);
            }
        }
    }

    if let Some(e) = &s.edit {
        if let Some((text, p)) = &e.label {
            let at = Pt::new(p.x + 18.0 * s.scale, p.y + 18.0 * s.scale);
            draw_pill(pm, t, s, text, at, false, &visible);
        }
        if let Some(h) = e.hint {
            let at = Pt::new(visible.x + visible.w / 2.0, visible.bottom() - 46.0 * s.scale);
            draw_pill(pm, t, s, h, at, true, &visible);
        }
    }
}

fn make_box<'a>(s: &Scene, rect: Rect, style: &'a ZoneStyle, title: &str, selected: bool) -> BoxDraw<'a> {
    let shape = rect.inset(2.0 * s.scale);
    let title = match s.title_fonts {
        Some(f) if !title.trim().is_empty() && shape.w > 4.0 && shape.h > 4.0 => {
            layout_title(s, f, style, title.trim(), shape)
        }
        _ => None,
    };
    BoxDraw { shape, rect, style, title, selected }
}

pub fn rounded_rect(r: Rect, radius: f32) -> Option<Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    if rad < 0.5 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h)?));
    }
    let k = 0.552_284_8 * rad;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

/// A rounded rectangle outline with an opening on the top or bottom edge (for a title).
fn outline_with_gap(r: Rect, radius: f32, gap: Option<(bool, f32, f32)>) -> Option<Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let Some((bottom, g0, g1)) = gap else { return rounded_rect(r, rad) };
    let g0 = g0.clamp(x0 + rad, x1 - rad);
    let g1 = g1.clamp(x0 + rad, x1 - rad);
    if g1 - g0 < 1.0 {
        return rounded_rect(r, rad);
    }
    let k = 0.552_284_8 * rad;
    let mut pb = PathBuilder::new();
    if !bottom {
        pb.move_to(g1, y0);
        pb.line_to(x1 - rad, y0);
        pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
        pb.line_to(x1, y1 - rad);
        pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
        pb.line_to(x0 + rad, y1);
        pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
        pb.line_to(x0, y0 + rad);
        pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
        pb.line_to(g0, y0);
    } else {
        pb.move_to(g0, y1);
        pb.line_to(x0 + rad, y1);
        pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
        pb.line_to(x0, y0 + rad);
        pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
        pb.line_to(x1 - rad, y0);
        pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
        pb.line_to(x1, y1 - rad);
        pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
        pb.line_to(g1, y1);
    }
    pb.finish()
}

fn draw_grid(pm: &mut PixmapMut, t: Transform, s: &Scene, g: &GridLayer, visible: &Rect) {
    let dots = g.grid.dots(visible);
    if dots.is_empty() {
        return;
    }
    let r = (g.radius * s.scale).max(0.75);
    let (reach, push) = (REPEL_RADIUS * s.scale, REPEL_PUSH * s.scale);
    let mut pb = PathBuilder::new();
    for p in dots {
        let (mut x, mut y, mut rr) = (p.x, p.y, r);
        if let Some((c, strength)) = g.repel {
            let (dx, dy) = (p.x - c.x, p.y - c.y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < reach && d > 0.01 {
                let f = (1.0 - d / reach) * strength.sqrt();
                let m = push * f * f;
                x += dx / d * m;
                y += dy / d * m;
                rr = r * (1.0 + 0.5 * f * f);
            }
        }
        pb.push_circle(x, y, rr);
    }
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &s.painter.paint(g.color, g.alpha), FillRule::Winding, t, None);
    }
}

struct FillItem {
    r: Rect,
    rad: f32,
    color: Rgb,
    alpha: f32,
}

impl FillItem {
    /// Anti-aliased coverage of a pixel center by this rounded rectangle (0..1).
    fn coverage(&self, x: f32, y: f32) -> f32 {
        let r = &self.r;
        if x < r.x - 1.0 || x > r.right() + 1.0 || y < r.y - 1.0 || y > r.bottom() + 1.0 {
            return 0.0;
        }
        let (hw, hh) = (r.w / 2.0, r.h / 2.0);
        let rad = self.rad.min(hw).min(hh).max(0.0);
        let qx = (x - (r.x + hw)).abs() - (hw - rad);
        let qy = (y - (r.y + hh)).abs() - (hh - rad);
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
        (0.5 - outside).clamp(0.0, 1.0)
    }
}

fn draw_fills(pm: &mut PixmapMut, t: Transform, s: &Scene, boxes: &[BoxDraw]) {
    let items: Vec<FillItem> = boxes
        .iter()
        .filter(|b| b.style.fill > 0)
        .map(|b| FillItem {
            r: b.shape,
            rad: b.style.radius * s.scale,
            color: b.style.color,
            alpha: b.style.fill as f32 / 100.0,
        })
        .collect();
    if !s.mix_colors {
        for it in &items {
            if let Some(p) = rounded_rect(it.r, it.rad) {
                pm.fill_path(&p, &s.painter.paint(it.color, it.alpha), FillRule::Winding, t, None);
            }
        }
        return;
    }
    mixed_fills(pm, s, &items);
}

/// Where boxes overlap, the colors average (weighted by opacity) and the overlap keeps the
/// opacity of the strongest box instead of getting darker. Blue + red reads as purple.
fn mixed_fills(pm: &mut PixmapMut, s: &Scene, items: &[FillItem]) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let (ox, oy) = (s.origin.x, s.origin.y);
    let swap = s.painter.swap_rb;
    let data = pm.data_mut();
    for (i, it) in items.iter().enumerate() {
        let x0 = ((it.r.x - ox).floor() as i32 - 1).max(0);
        let x1 = ((it.r.right() - ox).ceil() as i32 + 1).min(pw);
        let y0 = ((it.r.y - oy).floor() as i32 - 1).max(0);
        let y1 = ((it.r.bottom() - oy).ceil() as i32 + 1).min(ph);
        // Boxes that can matter on these rows (overlap candidates).
        let later: Vec<&FillItem> = items[i..].iter().filter(|e| e.r.intersect(&it.r.inset(-2.0)).is_some()).collect();
        let earlier: Vec<&FillItem> = items[..i].iter().filter(|e| e.r.intersect(&it.r.inset(-2.0)).is_some()).collect();
        for py in y0..y1 {
            let cy = py as f32 + 0.5 + oy;
            let row = (py * pw) as usize * 4;
            for px in x0..x1 {
                let cx = px as f32 + 0.5 + ox;
                // Each pixel is resolved once, by the first box that covers it.
                if it.coverage(cx, cy) <= 0.0 || earlier.iter().any(|e| e.coverage(cx, cy) > 0.0) {
                    continue;
                }
                let (mut ws, mut rs, mut gs, mut bs, mut amax) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
                for e in &later {
                    let c = e.coverage(cx, cy);
                    if c <= 0.0 {
                        continue;
                    }
                    let w = c * e.alpha;
                    ws += w;
                    rs += w * e.color.0 as f32;
                    gs += w * e.color.1 as f32;
                    bs += w * e.color.2 as f32;
                    amax = amax.max(w);
                }
                if ws <= 0.0 || amax < 0.002 {
                    continue;
                }
                let (r, g, b) = (rs / ws, gs / ws, bs / ws);
                let (c0, c2) = if swap { (b, r) } else { (r, b) };
                let idx = row + px as usize * 4;
                let dst = &mut data[idx..idx + 4];
                let inv = 1.0 - amax;
                let mix = |src: f32, d: u8| (src * amax + d as f32 * inv).round().clamp(0.0, 255.0) as u8;
                dst[0] = mix(c0, dst[0]);
                dst[1] = mix(g, dst[1]);
                dst[2] = mix(c2, dst[2]);
                dst[3] = mix(255.0, dst[3]);
            }
        }
    }
}

fn draw_border(pm: &mut PixmapMut, t: Transform, s: &Scene, b: &BoxDraw) {
    let st = b.style;
    let bw = st.border_width * s.scale;
    if bw <= 0.0 || st.border_opacity == 0 {
        return;
    }
    let rad = (st.radius * s.scale - bw / 2.0).max(0.0);
    let gap = b.title.as_ref().and_then(|tl| tl.gap);
    let Some(p) = outline_with_gap(b.shape.inset(bw / 2.0), rad, gap) else { return };
    let mut stroke = Stroke {
        width: bw,
        line_join: LineJoin::Round,
        line_cap: if gap.is_some() { LineCap::Round } else { LineCap::Butt },
        ..Stroke::default()
    };
    if st.border_style == BorderStyle::Dashed {
        stroke.dash = StrokeDash::new(vec![bw * 3.0 + 4.0 * s.scale, bw * 2.0 + 3.0 * s.scale], 0.0);
    }
    let paint = s.painter.paint(st.line_color(), st.border_opacity as f32 / 100.0);
    pm.stroke_path(&p, &paint, &stroke, t, None);
}

fn layout_title(s: &Scene, fonts: &Fonts, st: &ZoneStyle, title: &str, r: Rect) -> Option<TitleLayout> {
    let sc = s.scale;
    let px = st.title_size * sc;
    let chip = st.title_style == TitleStyle::Chip;
    let bw = st.border_width * sc;
    if st.title_on_border {
        let bottom = st.title_v == VAlign::Bottom;
        let line_y = if bottom { r.bottom() - bw / 2.0 } else { r.y + bw / 2.0 };
        let margin = (st.radius * sc).max(8.0 * sc) + 6.0 * sc;
        let (cpx, cpy) = if chip { (8.0 * sc, 4.0 * sc) } else { (0.0, 0.0) };
        let gpad = if chip { 3.0 * sc } else { 6.0 * sc };
        let max_w = r.w - 2.0 * margin - 2.0 * cpx - 2.0 * gpad;
        if max_w < px {
            return None;
        }
        let shaped = fonts.fit(title, px, max_w);
        if shaped.is_empty() {
            return None;
        }
        let cap = fonts.cap_height(px);
        let block_w = shaped.width + 2.0 * cpx;
        let bx = match st.title_h {
            HAlign::Left => r.x + margin,
            HAlign::Center => r.x + (r.w - block_w) / 2.0,
            HAlign::Right => r.right() - margin - block_w,
        };
        let chip_h = cap + 2.0 * cpy + 2.0 * sc;
        let (asc, desc) = fonts.vmetrics(px);
        // Centered on the line, but never pushed off the screen.
        let mut baseline = line_y + cap / 2.0;
        let top = if chip { line_y - chip_h / 2.0 } else { baseline - cap - 3.0 * sc };
        let low = if chip { line_y + chip_h / 2.0 } else { baseline + desc.min(asc * 0.3) };
        let limits = s.bounds.inset(1.0);
        if top < limits.y {
            baseline += limits.y - top;
        } else if low > limits.bottom() {
            baseline -= low - limits.bottom();
        }
        let shift = baseline - (line_y + cap / 2.0);
        Some(TitleLayout {
            px,
            x: bx + cpx,
            baseline,
            chip: chip.then(|| Rect::new(bx, line_y - chip_h / 2.0 + shift, block_w, chip_h)),
            gap: Some((bottom, bx - gpad, bx + block_w + gpad)),
            shaped,
        })
    } else {
        let (cpx, cpy) = if chip { (7.0 * sc, 2.0 * sc) } else { (0.0, 0.0) };
        let edge = bw + if chip { 5.0 * sc } else { 8.0 * sc };
        let max_w = r.w - 2.0 * edge - 2.0 * cpx;
        if max_w < px {
            return None;
        }
        let shaped = fonts.fit(title, px, max_w);
        if shaped.is_empty() {
            return None;
        }
        let (asc, desc) = fonts.vmetrics(px);
        let block_w = shaped.width + 2.0 * cpx;
        let block_h = asc + desc + 2.0 * cpy;
        if block_h > r.h - 2.0 * bw {
            return None;
        }
        let bx = match st.title_h {
            HAlign::Left => r.x + edge,
            HAlign::Center => r.x + (r.w - block_w) / 2.0,
            HAlign::Right => r.right() - edge - block_w,
        };
        let by = match st.title_v {
            VAlign::Top => r.y + edge,
            VAlign::Middle => r.y + (r.h - block_h) / 2.0,
            VAlign::Bottom => r.bottom() - edge - block_h,
        };
        Some(TitleLayout {
            px,
            x: bx + cpx,
            baseline: by + cpy + asc,
            chip: chip.then(|| Rect::new(bx, by, block_w, block_h)),
            gap: None,
            shaped,
        })
    }
}

fn draw_title(pm: &mut PixmapMut, t: Transform, s: &Scene, fonts: &Fonts, st: &ZoneStyle, tl: &TitleLayout) {
    let Some(path) = fonts.path(&tl.shaped, tl.px, tl.x, tl.baseline) else { return };
    let ink = st.line_color();
    match st.title_style {
        TitleStyle::Chip => {
            if let Some(chip) = tl.chip {
                let rad = if st.radius * s.scale >= chip.h / 2.0 { chip.h / 2.0 } else { (st.radius * s.scale).max(3.0 * s.scale) };
                if let Some(p) = rounded_rect(chip, rad) {
                    pm.fill_path(&p, &s.painter.paint(ink, 0.92), FillRule::Winding, t, None);
                }
            }
            let text = if ink.luminance() > 0.45 { Rgb(0x1A, 0x1A, 0x1A) } else { Rgb(0xFF, 0xFF, 0xFF) };
            pm.fill_path(&path, &s.painter.paint(text, 1.0), FillRule::Winding, t, None);
        }
        TitleStyle::Outline => {
            let halo = if ink.luminance() > 0.2 { Rgb(0x10, 0x10, 0x10) } else { Rgb(0xF0, 0xF0, 0xF0) };
            let stroke = Stroke {
                width: (tl.px * 0.16).max(2.0),
                line_join: LineJoin::Round,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            pm.stroke_path(&path, &s.painter.paint(halo, 0.55), &stroke, t, None);
            pm.fill_path(&path, &s.painter.paint(ink, 1.0), FillRule::Winding, t, None);
        }
        TitleStyle::Plain => {
            pm.fill_path(&path, &s.painter.paint(ink, 1.0), FillRule::Winding, t, None);
        }
    }
}

fn draw_selection(pm: &mut PixmapMut, t: Transform, s: &Scene, r: Rect) {
    let sc = s.scale;
    let white = Rgb(0xFF, 0xFF, 0xFF);
    let dark = Rgb(0x12, 0x12, 0x12);
    if let Some(tr) = tiny_skia::Rect::from_xywh(r.x, r.y, r.w.max(1.0), r.h.max(1.0)) {
        let p = PathBuilder::from_rect(tr);
        let under = Stroke { width: 3.0 * sc, ..Stroke::default() };
        pm.stroke_path(&p, &s.painter.paint(dark, 0.45), &under, t, None);
        let over = Stroke {
            width: 1.25 * sc,
            dash: StrokeDash::new(vec![5.0 * sc, 4.0 * sc], 0.0),
            ..Stroke::default()
        };
        pm.stroke_path(&p, &s.painter.paint(white, 0.95), &over, t, None);
    }
    let hs = 8.0 * sc;
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let points = [
        (r.x, r.y),
        (cx, r.y),
        (r.right(), r.y),
        (r.right(), cy),
        (r.right(), r.bottom()),
        (cx, r.bottom()),
        (r.x, r.bottom()),
        (r.x, cy),
    ];
    let mut pb = PathBuilder::new();
    for (x, y) in points {
        if let Some(rr) = tiny_skia::Rect::from_xywh(x - hs / 2.0, y - hs / 2.0, hs, hs) {
            pb.push_rect(rr);
        }
    }
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &s.painter.paint(white, 1.0), FillRule::Winding, t, None);
        let edge = Stroke { width: 1.0 * sc, ..Stroke::default() };
        pm.stroke_path(&p, &s.painter.paint(dark, 0.9), &edge, t, None);
    }
}

/// A dark rounded label. `centered` centers it horizontally on `at`, else `at` is its top-left.
fn draw_pill(pm: &mut PixmapMut, t: Transform, s: &Scene, text: &str, at: Pt, centered: bool, visible: &Rect) {
    let Some(fonts) = s.ui_fonts else { return };
    let sc = s.scale;
    let px = 13.0 * sc;
    let (padx, pady) = (10.0 * sc, 5.0 * sc);
    let shaped = fonts.fit(text, px, (visible.w - 4.0 * padx).max(px));
    if shaped.is_empty() {
        return;
    }
    let (asc, desc) = fonts.vmetrics(px);
    let w = shaped.width + 2.0 * padx;
    let h = asc + desc + 2.0 * pady;
    let mut box_r = Rect::new(if centered { at.x - w / 2.0 } else { at.x }, at.y, w, h);
    box_r = box_r.clamp_into(&visible.inset(4.0 * sc));
    if let Some(p) = rounded_rect(box_r, h / 2.0) {
        pm.fill_path(&p, &s.painter.paint(Rgb(0x18, 0x18, 0x18), 0.86), FillRule::Winding, t, None);
    }
    if let Some(p) = fonts.path(&shaped, px, box_r.x + padx, box_r.y + pady + asc) {
        pm.fill_path(&p, &s.painter.paint(Rgb(0xF5, 0xF5, 0xF5), 1.0), FillRule::Winding, t, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BorderStyle, HAlign, TitleStyle, VAlign, Zone, ZoneStyle};
    use crate::text::FontData;
    use tiny_skia::{GradientStop, LinearGradient, Pixmap, PixmapPaint, Point, SpreadMode};

    fn font(file: &str) -> Option<FontData> {
        let data = std::fs::read(format!(r"C:\Windows\Fonts\{file}")).ok()?;
        FontData::new(Box::new(data), 0)
    }

    fn zone(id: u64, r: Rect, title: &str, f: impl FnOnce(&mut ZoneStyle)) -> Zone {
        let mut style = ZoneStyle::default();
        f(&mut style);
        Zone { id, monitor: "m".into(), monitor_hw: String::new(), rect: r, ref_size: (1000, 640), title: title.into(), style }
    }

    #[test]
    fn overlap_mixes_to_purple_without_darkening() {
        let blue = zone(1, Rect::new(0.0, 0.0, 60.0, 40.0), "", |s| {
            s.color = Rgb(0, 0, 255);
            s.fill = 50;
            s.border_width = 0.0;
            s.radius = 12.0;
        });
        let red = zone(2, Rect::new(30.0, 0.0, 60.0, 40.0), "", |s| {
            s.color = Rgb(255, 0, 0);
            s.fill = 50;
            s.border_width = 0.0;
        });
        let mut pm = Pixmap::new(100, 40).unwrap();
        let scene = Scene {
            origin: Pt::new(0.0, 0.0),
            scale: 1.0,
            painter: Painter { swap_rb: false },
            title_fonts: None,
            ui_fonts: None,
            grid: None,
            zones: vec![
                ZoneItem { zone: &blue, rect: blue.rect, selected: false },
                ZoneItem { zone: &red, rect: red.rect, selected: false },
            ],
            edit: None,
            mix_colors: true,
            bounds: Rect::new(0.0, 0.0, 100.0, 40.0),
        };
        draw(&mut pm.as_mut(), &scene);
        let p = pm.pixel(45, 20).unwrap();
        // Premultiplied purple at the same 50% opacity as each box on its own.
        assert_eq!(p.alpha(), 128);
        assert!((p.red() as i32 - 64).abs() <= 1 && (p.blue() as i32 - 64).abs() <= 1, "{p:?}");
        let only_blue = pm.pixel(10, 20).unwrap();
        assert_eq!((only_blue.red(), only_blue.blue(), only_blue.alpha()), (0, 128, 128));
        // Just outside blue's rounded corner, inside red: plain red, blended only once.
        let corner = pm.pixel(57, 37).unwrap();
        assert_eq!((corner.red(), corner.blue(), corner.alpha()), (128, 0, 128));
    }

    /// Renders sample boxes over a fake wallpaper into target/render-preview.png.
    #[test]
    fn preview() {
        let (Some(title), Some(ui)) = (font("seguisb.ttf"), font("segoeui.ttf")) else { return };
        let title = Fonts::new(vec![title]).unwrap();
        let ui = Fonts::new(vec![ui]).unwrap();
        let (w, h) = (1000u32, 640u32);
        let mut bg = Pixmap::new(w, h).unwrap();
        let mut paint = Paint::default();
        paint.shader = LinearGradient::new(
            Point::from_xy(0.0, 0.0),
            Point::from_xy(w as f32, h as f32),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(0x1d, 0x35, 0x57, 255)),
                GradientStop::new(0.6, Color::from_rgba8(0x45, 0x7b, 0x9d, 255)),
                GradientStop::new(1.0, Color::from_rgba8(0xe6, 0x9f, 0x6b, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        bg.fill_rect(tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(), &paint, Transform::identity(), None);

        let zones = [
            zone(1, Rect::new(20.0, 30.0, 380.0, 220.0), "Project Alpha", |s| {
                s.color = Rgb(0x8E, 0xC5, 0xFF);
                s.fill = 30;
            }),
            zone(2, Rect::new(300.0, 140.0, 330.0, 200.0), "Invoices", |s| {
                s.color = Rgb(0xFF, 0x7F, 0x6E);
                s.fill = 30;
                s.title_style = TitleStyle::Chip;
                s.title_h = HAlign::Right;
            }),
            zone(3, Rect::new(20.0, 380.0, 475.0, 220.0), "Screenshots to sort", |s| {
                s.color = Rgb(0xFF, 0xB3, 0xC7);
                s.border_style = BorderStyle::Dashed;
                s.title_v = VAlign::Bottom;
                s.title_h = HAlign::Left;
                s.radius = 24.0;
            }),
            zone(4, Rect::new(570.0, 380.0, 380.0, 220.0), "Inside, centered", |s| {
                s.color = Rgb(0xFD, 0xF2, 0x8C);
                s.border_color = Some(Rgb(0xFF, 0xFF, 0xFF));
                s.border_opacity = 50;
                s.title_on_border = false;
                s.title_style = TitleStyle::Plain;
                s.title_v = VAlign::Middle;
                s.radius = 0.0;
                s.border_width = 3.0;
                s.title_size = 21.0;
            }),
        ];
        let grid = Grid { origin: Pt::new(0.0, 0.0), cell: Pt::new(95.0, 110.0), area: Rect::new(0.0, 0.0, w as f32, h as f32) };
        let mut layer = Pixmap::new(w, h).unwrap();
        let scene = Scene {
            origin: Pt::new(0.0, 0.0),
            scale: 1.25,
            painter: Painter { swap_rb: false },
            title_fonts: Some(&title),
            ui_fonts: Some(&ui),
            grid: Some(GridLayer { grid: &grid, color: Rgb(255, 255, 255), radius: 1.5, alpha: 0.75, repel: Some((Pt::new(760.0, 120.0), 1.0)) }),
            zones: zones.iter().map(|z| ZoneItem { zone: z, rect: z.rect, selected: z.id == 2 }).collect(),
            edit: Some(EditLayer { hint: None, label: Some(("3 × 2 icons".into(), Pt::new(640.0, 300.0))), draft: None }),
            mix_colors: true,
            bounds: Rect::new(0.0, 0.0, w as f32, h as f32),
        };
        draw(&mut layer.as_mut(), &scene);
        bg.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("render-preview.png");
        bg.save_png(&out).unwrap();
    }
}
