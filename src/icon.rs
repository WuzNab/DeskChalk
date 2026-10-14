//! The DeskChalk icon: the Material Design "draw" icon (pictogrammers.com/library/mdi,
//! Apache-2.0) with a white pencil and a pastel green line.
//! Shared with build.rs (which renders the .ico), so it depends on tiny-skia only.

use tiny_skia::{FillRule, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

/// The drawn squiggle (first sub-path of mdi "draw").
pub const LINE_PATH: &str = "M9.75 20.85C11.53 20.15 11.14 18.22 10.24 17C9.35 15.75 8.12 14.89 6.88 14.06C6 13.5 5.19 12.8 4.54 12C4.26 11.67 3.69 11.06 4.27 10.94C4.86 10.82 5.88 11.4 6.4 11.62C7.31 12 8.21 12.44 9.05 12.96L10.06 11.26C8.5 10.23 6.5 9.32 4.64 9.05C3.58 8.89 2.46 9.11 2.1 10.26C1.78 11.25 2.29 12.25 2.87 13.03C4.24 14.86 6.37 15.74 7.96 17.32C8.3 17.65 8.71 18.04 8.91 18.5C9.12 18.94 9.07 18.97 8.6 18.97C7.36 18.97 5.81 18 4.8 17.36L3.79 19.06C5.32 20 7.88 21.47 9.75 20.85Z";
/// The pencil (remaining sub-paths of mdi "draw").
pub const PENCIL_PATH: &str = "M20.84 5.25C21.06 5.03 21.06 4.67 20.84 4.46L19.54 3.16C19.33 2.95 18.97 2.95 18.76 3.16L17.74 4.18L19.82 6.26Z M11 10.92V13H13.08L19.23 6.85L17.15 4.77L11 10.92Z";

pub const PASTEL_GREEN: [u8; 3] = [0x77, 0xDD, 0x77];
pub const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
pub const DARK: [u8; 3] = [0x20, 0x20, 0x20];

/// Renders the icon at `size` x `size` pixels (premultiplied RGBA).
/// `outline` draws a thin border around both shapes so a white pencil stays visible on white.
pub fn render(size: u32, pencil: [u8; 3], line: [u8; 3], outline: Option<[u8; 3]>) -> Option<Pixmap> {
    let mut pm = Pixmap::new(size, size)?;
    let s = size as f32 / 24.0;
    let t = Transform::from_scale(s, s);
    let line_path = parse_svg_path(LINE_PATH)?;
    let pencil_path = parse_svg_path(PENCIL_PATH)?;
    let paint = |c: [u8; 3], a: u8| {
        let mut p = Paint::default();
        p.set_color_rgba8(c[0], c[1], c[2], a);
        p.anti_alias = true;
        p
    };
    if let Some(o) = outline {
        let stroke = Stroke {
            width: (1.0 / s).max(1.3),
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        let p = paint(o, 230);
        pm.stroke_path(&line_path, &p, &stroke, t, None);
        pm.stroke_path(&pencil_path, &p, &stroke, t, None);
    }
    pm.fill_path(&line_path, &paint(line, 255), FillRule::Winding, t, None);
    pm.fill_path(&pencil_path, &paint(pencil, 255), FillRule::Winding, t, None);
    Some(pm)
}

/// Parses the subset of SVG path syntax used by Material Design icons
/// (M L H V C S Q Z, absolute and relative).
pub fn parse_svg_path(d: &str) -> Option<Path> {
    let b = d.as_bytes();
    let mut i = 0usize;
    let mut pb = PathBuilder::new();
    let (mut cx, mut cy) = (0f32, 0f32);
    let (mut sx, mut sy) = (0f32, 0f32);
    let (mut lcx, mut lcy) = (0f32, 0f32); // last cubic control point, for S
    let mut cmd = 0u8;
    loop {
        skip_separators(b, &mut i);
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            if cmd == b'Z' || cmd == b'z' {
                pb.close();
                cx = sx;
                cy = sy;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (num(b, &mut i)? + ox, num(b, &mut i)? + oy);
                pb.move_to(x, y);
                (cx, cy, sx, sy) = (x, y, x, y);
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (x, y) = (num(b, &mut i)? + ox, num(b, &mut i)? + oy);
                pb.line_to(x, y);
                (cx, cy) = (x, y);
            }
            b'H' => {
                cx = num(b, &mut i)? + ox;
                pb.line_to(cx, cy);
            }
            b'V' => {
                cy = num(b, &mut i)? + oy;
                pb.line_to(cx, cy);
            }
            b'C' => {
                let x1 = num(b, &mut i)? + ox;
                let y1 = num(b, &mut i)? + oy;
                let x2 = num(b, &mut i)? + ox;
                let y2 = num(b, &mut i)? + oy;
                let x = num(b, &mut i)? + ox;
                let y = num(b, &mut i)? + oy;
                pb.cubic_to(x1, y1, x2, y2, x, y);
                (lcx, lcy, cx, cy) = (x2, y2, x, y);
                continue;
            }
            b'S' => {
                let (x1, y1) = (2.0 * cx - lcx, 2.0 * cy - lcy);
                let x2 = num(b, &mut i)? + ox;
                let y2 = num(b, &mut i)? + oy;
                let x = num(b, &mut i)? + ox;
                let y = num(b, &mut i)? + oy;
                pb.cubic_to(x1, y1, x2, y2, x, y);
                (lcx, lcy, cx, cy) = (x2, y2, x, y);
                continue;
            }
            b'Q' => {
                let x1 = num(b, &mut i)? + ox;
                let y1 = num(b, &mut i)? + oy;
                let x = num(b, &mut i)? + ox;
                let y = num(b, &mut i)? + oy;
                pb.quad_to(x1, y1, x, y);
                (cx, cy) = (x, y);
            }
            _ => return None,
        }
        (lcx, lcy) = (cx, cy);
    }
    pb.finish()
}

fn skip_separators(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
        *i += 1;
    }
}

fn num(b: &[u8], i: &mut usize) -> Option<f32> {
    skip_separators(b, i);
    let start = *i;
    if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
        *i += 1;
    }
    let mut seen_dot = false;
    while *i < b.len() {
        match b[*i] {
            b'0'..=b'9' => *i += 1,
            b'.' if !seen_dot => {
                seen_dot = true;
                *i += 1;
            }
            b'e' | b'E' => {
                *i += 1;
                if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
                    *i += 1;
                }
            }
            _ => break,
        }
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Color, PixmapPaint};

    /// Writes target/icon-preview.png: the tray icon on dark and light taskbars at common
    /// sizes, plus the outlined app icon.
    #[test]
    fn preview() {
        let sizes = [16u32, 20, 24, 32, 48, 64, 128];
        let (w, h) = (sizes.iter().sum::<u32>() + 10 * sizes.len() as u32 + 10, 3 * 140);
        let mut out = Pixmap::new(w, h).unwrap();
        let rows: [([u8; 3], [u8; 3], Option<[u8; 3]>, Color); 3] = [
            (WHITE, PASTEL_GREEN, None, Color::from_rgba8(0x20, 0x20, 0x20, 255)),
            (DARK, PASTEL_GREEN, None, Color::from_rgba8(0xF3, 0xF3, 0xF3, 255)),
            (WHITE, PASTEL_GREEN, Some(DARK), Color::from_rgba8(0xFF, 0xFF, 0xFF, 255)),
        ];
        for (r, (pencil, line, outline, bg)) in rows.iter().enumerate() {
            let y = r as i32 * 140;
            let mut paint = Paint::default();
            paint.set_color(*bg);
            out.fill_rect(tiny_skia::Rect::from_xywh(0.0, y as f32, w as f32, 140.0).unwrap(), &paint, Transform::identity(), None);
            let mut x = 10;
            for &s in &sizes {
                let icon = render(s, *pencil, *line, *outline).unwrap();
                out.draw_pixmap(x, y + 6, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                x += s as i32 + 10;
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("icon-preview.png");
        out.save_png(path).unwrap();
    }
}
