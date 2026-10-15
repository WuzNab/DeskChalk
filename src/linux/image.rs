//! Wallpaper images: reading PNG, JPEG and WebP, scaling them to fill a screen the way
//! desktops "zoom" a wallpaper, and writing PNG.

use std::io::Cursor;
use std::path::Path;
use tiny_skia::{Color, FilterQuality, Pixmap, PixmapPaint, Transform};

/// Decodes an image file into an opaque pixmap. None for formats DeskChalk can't read
/// (the box picture then goes on a plain color).
pub fn load(path: &Path) -> Option<Pixmap> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"\x89PNG") {
        load_png(&bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        load_jpeg(&bytes)
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        load_webp(&bytes)
    } else {
        None
    }
}

/// Pixels with 1 (gray), 2 (gray + alpha), 3 (RGB) or 4 (RGBA) channels, made opaque
/// over black (wallpapers are shown without transparency).
fn from_channels(w: u32, h: u32, data: &[u8], channels: usize) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    let n = (w * h) as usize;
    if data.len() < n * channels {
        return None;
    }
    let dst = pm.data_mut();
    for i in 0..n {
        let s = &data[i * channels..];
        let (r, g, b, a) = match channels {
            1 => (s[0], s[0], s[0], 255),
            2 => (s[0], s[0], s[0], s[1]),
            3 => (s[0], s[1], s[2], 255),
            _ => (s[0], s[1], s[2], s[3]),
        };
        let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        dst[i * 4..i * 4 + 4].copy_from_slice(&[m(r), m(g), m(b), 255]);
    }
    Some(pm)
}

fn load_png(bytes: &[u8]) -> Option<Pixmap> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    from_channels(frame.width, frame.height, &buf, channels)
}

fn load_jpeg(bytes: &[u8]) -> Option<Pixmap> {
    let mut dec = zune_jpeg::JpegDecoder::new(bytes);
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let channels = dec.get_output_colorspace().map_or(3, |c| c.num_components());
    from_channels(info.width as u32, info.height as u32, &pixels, channels)
}

fn load_webp(bytes: &[u8]) -> Option<Pixmap> {
    let mut dec = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (w, h) = dec.dimensions();
    let mut buf = vec![0u8; dec.output_buffer_size()?];
    dec.read_image(&mut buf).ok()?;
    from_channels(w, h, &buf, if dec.has_alpha() { 4 } else { 3 })
}

/// A plain opaque color.
pub fn solid(w: u32, h: u32, c: crate::model::Rgb) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    pm.fill(Color::from_rgba8(c.0, c.1, c.2, 255));
    Some(pm)
}

/// Halves both sides, averaging 2x2 pixels (good quality when shrinking a lot).
fn halve(src: &Pixmap) -> Option<Pixmap> {
    let (w, h) = ((src.width() / 2).max(1), (src.height() / 2).max(1));
    let mut out = Pixmap::new(w, h)?;
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let s = src.data();
    let d = out.data_mut();
    for y in 0..h as usize {
        let (y0, y1) = ((2 * y).min(sh - 1), (2 * y + 1).min(sh - 1));
        for x in 0..w as usize {
            let (x0, x1) = ((2 * x).min(sw - 1), (2 * x + 1).min(sw - 1));
            for c in 0..4 {
                let sum = s[(y0 * sw + x0) * 4 + c] as u32
                    + s[(y0 * sw + x1) * 4 + c] as u32
                    + s[(y1 * sw + x0) * 4 + c] as u32
                    + s[(y1 * sw + x1) * 4 + c] as u32;
                d[(y * w as usize + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    Some(out)
}

/// Scales `src` to fill `w` x `h`, cropping the overflow evenly ("zoom").
pub fn cover(src: &Pixmap, w: u32, h: u32) -> Option<Pixmap> {
    let mut scale = (w as f32 / src.width() as f32).max(h as f32 / src.height() as f32);
    let mut shrunk: Option<Pixmap> = None;
    while scale <= 0.5 {
        let next = halve(shrunk.as_ref().unwrap_or(src))?;
        shrunk = Some(next);
        scale *= 2.0;
    }
    let img = shrunk.as_ref().unwrap_or(src);
    let scale = (w as f32 / img.width() as f32).max(h as f32 / img.height() as f32);
    let tx = (w as f32 - img.width() as f32 * scale) / 2.0;
    let ty = (h as f32 - img.height() as f32 * scale) / 2.0;
    let mut out = Pixmap::new(w, h)?;
    out.fill(Color::BLACK);
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, img.as_ref(), &paint, Transform::from_row(scale, 0.0, 0.0, scale, tx, ty), None);
    Some(out)
}

/// Draws `src` scaled into the rect (`x`, `y`, `w`, `h`) of `dst`.
pub fn draw_scaled(dst: &mut Pixmap, src: &Pixmap, x: f32, y: f32, w: f32, h: f32) {
    let (sx, sy) = (w / src.width() as f32, h / src.height() as f32);
    let quality = if (sx - 1.0).abs() < 0.001 && (sy - 1.0).abs() < 0.001 { FilterQuality::Nearest } else { FilterQuality::Bicubic };
    let paint = PixmapPaint { quality, ..PixmapPaint::default() };
    dst.draw_pixmap(0, 0, src.as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, x, y), None);
}

/// Writes an opaque pixmap as an RGB PNG (written next to `path` first, then moved into
/// place, so the desktop never reads a half-written file).
pub fn save_png(pm: &Pixmap, path: &Path) -> std::io::Result<()> {
    let rgb: Vec<u8> = pm.data().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let tmp = path.with_extension("tmp");
    {
        let file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let mut enc = png::Encoder::new(file, pm.width(), pm.height());
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(std::io::Error::other)?;
        writer.write_image_data(&rgb).map_err(std::io::Error::other)?;
        writer.finish().map_err(std::io::Error::other)?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_fills_and_crops_evenly() {
        // 400x100: left half red, right half blue. Covering 100x100 keeps the middle.
        let mut src = Pixmap::new(400, 100).unwrap();
        for y in 0..100 {
            for x in 0..400 {
                let c = if x < 200 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                let i = (y * 400 + x) * 4;
                src.data_mut()[i..i + 4].copy_from_slice(&c);
            }
        }
        let out = cover(&src, 100, 100).unwrap();
        assert_eq!((out.width(), out.height()), (100, 100));
        let px = |x: u32, y: u32| out.pixel(x, y).unwrap();
        assert!(px(10, 50).red() > 200 && px(10, 50).blue() < 50);
        assert!(px(90, 50).blue() > 200 && px(90, 50).red() < 50);
        assert_eq!(px(0, 0).alpha(), 255);
    }

    #[test]
    fn shrinks_large_images() {
        let src = solid(3000, 2000, crate::model::Rgb(10, 200, 30)).unwrap();
        let out = cover(&src, 300, 200).unwrap();
        let p = out.pixel(150, 100).unwrap();
        assert_eq!((p.red(), p.green(), p.blue()), (10, 200, 30));
    }

    #[test]
    fn png_round_trip() {
        let dir = std::env::temp_dir().join(format!("deskchalk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        let src = solid(64, 32, crate::model::Rgb(0x77, 0xDD, 0x77)).unwrap();
        save_png(&src, &path).unwrap();
        let back = load(&path).unwrap();
        assert_eq!((back.width(), back.height()), (64, 32));
        let p = back.pixel(5, 5).unwrap();
        assert_eq!((p.red(), p.green(), p.blue(), p.alpha()), (0x77, 0xDD, 0x77, 255));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
