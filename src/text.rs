//! Minimal text rendering: font outlines from ttf-parser turned into tiny-skia paths.
//! No shaping engine; that's plenty for short box titles and keeps the binary small.

use tiny_skia::{Path, PathBuilder};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Font bytes plus the face index inside a collection (.ttc).
pub struct FontData {
    data: Box<dyn AsRef<[u8]>>,
    index: u32,
}

impl FontData {
    pub fn new(data: Box<dyn AsRef<[u8]>>, index: u32) -> Option<FontData> {
        Face::parse((*data).as_ref(), index).ok()?;
        Some(FontData { data, index })
    }

    fn face(&self) -> Face<'_> {
        Face::parse((*self.data).as_ref(), self.index).expect("validated in FontData::new")
    }
}

/// A primary font plus fallbacks for characters it lacks.
pub struct Fonts {
    faces: Vec<FontData>,
}

pub struct Shaped {
    glyphs: Vec<(u8, GlyphId, f32)>,
    pub width: f32,
}

impl Shaped {
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

impl Fonts {
    pub fn new(faces: Vec<FontData>) -> Option<Fonts> {
        (!faces.is_empty()).then_some(Fonts { faces })
    }

    /// (ascent, descent) in pixels for a font size, both positive.
    pub fn vmetrics(&self, px: f32) -> (f32, f32) {
        let f = self.faces[0].face();
        let s = px / f.units_per_em() as f32;
        (f.ascender() as f32 * s, -(f.descender() as f32) * s)
    }

    /// Height of capital letters in pixels (used to center text on a line).
    pub fn cap_height(&self, px: f32) -> f32 {
        let f = self.faces[0].face();
        let upem = f.units_per_em() as f32;
        let cap = f.capital_height().map_or(0.7 * upem, |c| c as f32);
        cap * px / upem
    }

    pub fn shape(&self, text: &str, px: f32) -> Shaped {
        let faces: Vec<Face> = self.faces.iter().map(|f| f.face()).collect();
        let mut x = 0.0;
        let mut glyphs = Vec::with_capacity(text.len());
        for ch in text.chars().filter(|c| !c.is_control()) {
            let (fi, gid) = faces
                .iter()
                .enumerate()
                .find_map(|(i, f)| f.glyph_index(ch).map(|g| (i, g)))
                .unwrap_or((0, GlyphId(0)));
            let face = &faces[fi];
            let s = px / face.units_per_em() as f32;
            glyphs.push((fi as u8, gid, x));
            x += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * s;
        }
        Shaped { glyphs, width: x }
    }

    /// Shapes `text`, cutting it with an ellipsis if it is wider than `max_w`.
    pub fn fit(&self, text: &str, px: f32, max_w: f32) -> Shaped {
        let full = self.shape(text, px);
        if full.width <= max_w {
            return full;
        }
        let chars: Vec<char> = text.chars().collect();
        // Binary search the longest prefix that fits together with the ellipsis.
        let (mut lo, mut hi) = (0usize, chars.len());
        let with_ellipsis = |n: usize| {
            let mut s: String = chars[..n].iter().collect();
            s.truncate(s.trim_end().len());
            s.push('…');
            self.shape(&s, px)
        };
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if with_ellipsis(mid).width <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo == 0 {
            return Shaped { glyphs: Vec::new(), width: 0.0 };
        }
        with_ellipsis(lo)
    }

    /// Glyph outlines for `shaped`, with the left of the text at `x` and the baseline at `baseline`.
    pub fn path(&self, shaped: &Shaped, px: f32, x: f32, baseline: f32) -> Option<Path> {
        let faces: Vec<Face> = self.faces.iter().map(|f| f.face()).collect();
        let mut pb = PathBuilder::new();
        for &(fi, gid, gx) in &shaped.glyphs {
            let face = &faces[fi as usize];
            let mut b = Outliner {
                pb: &mut pb,
                x: x + gx,
                y: baseline,
                s: px / face.units_per_em() as f32,
            };
            face.outline_glyph(gid, &mut b);
        }
        pb.finish()
    }
}

struct Outliner<'a> {
    pb: &'a mut PathBuilder,
    x: f32,
    y: f32,
    s: f32,
}

impl OutlineBuilder for Outliner<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x * self.s, self.y - y * self.s);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x * self.s, self.y - y * self.s);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(
            self.x + x1 * self.s,
            self.y - y1 * self.s,
            self.x + x * self.s,
            self.y - y * self.s,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.x + x1 * self.s,
            self.y - y1 * self.s,
            self.x + x2 * self.s,
            self.y - y2 * self.s,
            self.x + x * self.s,
            self.y - y * self.s,
        );
    }

    fn close(&mut self) {
        self.pb.close();
    }
}
