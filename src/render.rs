//! Draws boxes, titles, grid dots and edit-mode decorations into a pixel buffer.

use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use crate::model::{BorderStyle, HAlign, Rgb, TitleStyle, VAlign, Zone, ZoneStyle};
use crate::text::Fonts;
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
}

pub fn draw(pm: &mut PixmapMut, s: &Scene) {
    let t = Transform::from_translate(-s.origin.x, -s.origin.y);
    let visible = Rect::new(s.origin.x, s.origin.y, pm.width() as f32, pm.height() as f32);
    if let Some(g) = &s.grid {
        draw_grid(pm, t, s, g, &visible);
    }
    for z in &s.zones {
        if z.rect.intersect(&visible).is_none() {
            continue;
        }
        draw_zone(pm, t, s, &z.zone.style, &z.zone.title, z.rect);
        if z.selected {
            draw_selection(pm, t, s, z.rect);
        }
    }
    if let Some(e) = &s.edit {
        if let Some((r, style)) = e.draft {
            draw_zone(pm, t, s, style, "", r);
            draw_selection(pm, t, s, r);
        }
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

fn draw_grid(pm: &mut PixmapMut, t: Transform, s: &Scene, g: &GridLayer, visible: &Rect) {
    let dots = g.grid.dots(visible);
    if dots.is_empty() {
        return;
    }
    let r = (g.radius * s.scale).max(0.75);
    let mut pb = PathBuilder::new();
    for p in dots {
        pb.push_circle(p.x, p.y, r);
    }
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &s.painter.paint(g.color, g.alpha), FillRule::Winding, t, None);
    }
}

fn draw_zone(pm: &mut PixmapMut, t: Transform, s: &Scene, st: &ZoneStyle, title: &str, rect: Rect) {
    let sc = s.scale;
    // A small gap keeps neighbouring boxes that share a grid line visually apart.
    let r = rect.inset(2.0 * sc);
    if r.w < 2.0 || r.h < 2.0 {
        return;
    }
    let rad = st.radius * sc;
    if st.fill > 0 {
        if let Some(p) = rounded_rect(r, rad) {
            let paint = s.painter.paint(st.color, st.fill as f32 / 100.0);
            pm.fill_path(&p, &paint, FillRule::Winding, t, None);
        }
    }
    let bw = st.border_width * sc;
    if bw > 0.0 {
        if let Some(p) = rounded_rect(r.inset(bw / 2.0), (rad - bw / 2.0).max(0.0)) {
            let mut stroke = Stroke {
                width: bw,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            if st.border_style == BorderStyle::Dashed {
                stroke.dash = StrokeDash::new(vec![bw * 3.0 + 4.0 * sc, bw * 2.0 + 3.0 * sc], 0.0);
            }
            pm.stroke_path(&p, &s.painter.paint(st.color, 0.95), &stroke, t, None);
        }
    }
    if !title.trim().is_empty() {
        if let Some(fonts) = s.title_fonts {
            draw_title(pm, t, s, fonts, st, title.trim(), r, bw);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_title(
    pm: &mut PixmapMut,
    t: Transform,
    s: &Scene,
    fonts: &Fonts,
    st: &ZoneStyle,
    title: &str,
    r: Rect,
    bw: f32,
) {
    let sc = s.scale;
    let px = st.title_size * sc;
    let chip = st.title_style == TitleStyle::Chip;
    let (cpx, cpy) = if chip { (7.0 * sc, 2.0 * sc) } else { (0.0, 0.0) };
    let edge = bw + if chip { 5.0 * sc } else { 8.0 * sc };
    let max_w = r.w - 2.0 * edge - 2.0 * cpx;
    if max_w < px {
        return;
    }
    let shaped = fonts.fit(title, px, max_w);
    if shaped.is_empty() {
        return;
    }
    let (asc, desc) = fonts.vmetrics(px);
    let block_w = shaped.width + 2.0 * cpx;
    let block_h = asc + desc + 2.0 * cpy;
    if block_h > r.h - 2.0 * bw {
        return;
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
    let Some(path) = fonts.path(&shaped, px, bx + cpx, by + cpy + asc) else {
        return;
    };
    match st.title_style {
        TitleStyle::Chip => {
            let chip_r = Rect::new(bx, by, block_w, block_h);
            let rad = if st.radius * sc >= block_h / 2.0 { block_h / 2.0 } else { st.radius * sc };
            if let Some(p) = rounded_rect(chip_r, rad) {
                pm.fill_path(&p, &s.painter.paint(st.color, 0.92), FillRule::Winding, t, None);
            }
            let ink = if st.color.luminance() > 0.45 { Rgb(0x1A, 0x1A, 0x1A) } else { Rgb(0xFF, 0xFF, 0xFF) };
            pm.fill_path(&path, &s.painter.paint(ink, 1.0), FillRule::Winding, t, None);
        }
        TitleStyle::Outline => {
            let halo = if st.color.luminance() > 0.2 { Rgb(0x10, 0x10, 0x10) } else { Rgb(0xF0, 0xF0, 0xF0) };
            let stroke = Stroke {
                width: (px * 0.16).max(2.0),
                line_join: LineJoin::Round,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            pm.stroke_path(&path, &s.painter.paint(halo, 0.55), &stroke, t, None);
            pm.fill_path(&path, &s.painter.paint(st.color, 1.0), FillRule::Winding, t, None);
        }
        TitleStyle::Plain => {
            pm.fill_path(&path, &s.painter.paint(st.color, 1.0), FillRule::Winding, t, None);
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

        let zones = vec![
            zone(1, Rect::new(0.0, 0.0, 380.0, 220.0), "Project Alpha", |_| {}),
            zone(2, Rect::new(380.0, 0.0, 285.0, 220.0), "Invoices", |s| {
                s.color = Rgb(0x8E, 0xC5, 0xFF);
                s.title_style = TitleStyle::Chip;
                s.title_h = HAlign::Center;
                s.fill = 15;
            }),
            zone(3, Rect::new(0.0, 330.0, 475.0, 220.0), "Screenshots to sort", |s| {
                s.color = Rgb(0xFF, 0xB3, 0xC7);
                s.border_style = BorderStyle::Dashed;
                s.title_v = VAlign::Bottom;
                s.title_h = HAlign::Right;
                s.radius = 24.0;
            }),
            zone(4, Rect::new(570.0, 330.0, 380.0, 220.0), "Plain title, square corners", |s| {
                s.color = Rgb(0xFD, 0xF2, 0x8C);
                s.title_style = TitleStyle::Plain;
                s.title_v = VAlign::Middle;
                s.title_h = HAlign::Center;
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
            grid: Some(GridLayer { grid: &grid, color: Rgb(255, 255, 255), radius: 1.5, alpha: 0.75 }),
            zones: zones.iter().map(|z| ZoneItem { zone: z, rect: z.rect, selected: z.id == 2 }).collect(),
            edit: Some(EditLayer {
                hint: Some("Drag to draw a box  ·  Right-click for options  ·  Esc when done"),
                label: Some(("3 × 2 icons".into(), Pt::new(640.0, 200.0))),
                draft: None,
            }),
        };
        draw(&mut layer.as_mut(), &scene);
        bg.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("render-preview.png");
        bg.save_png(&out).unwrap();
    }
}
