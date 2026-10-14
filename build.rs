//! Renders the app icon from src/icon.rs into an .ico and embeds it together with the
//! application manifest and version info.

#[allow(dead_code)]
#[path = "src/icon.rs"]
mod icon;

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=res/app.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("deskchalk.ico");
    write_ico(&ico, &[16, 20, 24, 32, 40, 48, 64, 256]);

    #[cfg(windows)]
    {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico.to_str().unwrap())
            .set_manifest_file("res/app.manifest")
            .set("FileDescription", "DeskChalk")
            .set("ProductName", "DeskChalk")
            .set("InternalName", "DeskChalk")
            .set("OriginalFilename", "DeskChalk.exe")
            .set("LegalCopyright", "PolyForm Strict License 1.0.0")
            .set("FileVersion", version)
            .set("ProductVersion", version);
        res.compile().expect("failed to compile Windows resources");
    }
}

/// Writes a multi-size .ico: BMP entries for small sizes, PNG for 256 px.
fn write_ico(path: &Path, sizes: &[u32]) {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let pm = icon::render(s, icon::WHITE, icon::PASTEL_GREEN, Some(icon::DARK)).unwrap();
            if s >= 256 {
                pm.encode_png().unwrap()
            } else {
                bmp_entry(&pm)
            }
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0u8 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        out.extend_from_slice(img);
    }
    std::fs::write(path, out).unwrap();
}

fn bmp_entry(pm: &tiny_skia::Pixmap) -> Vec<u8> {
    let (w, h) = (pm.width(), pm.height());
    let mask_row = w.div_ceil(32) * 4;
    let mut v = Vec::new();
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(w as i32).to_le_bytes());
    v.extend_from_slice(&((h * 2) as i32).to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&(w * h * 4 + mask_row * h).to_le_bytes());
    v.extend_from_slice(&[0u8; 16]);
    // Bottom-up, straight-alpha BGRA.
    for y in (0..h).rev() {
        for x in 0..w {
            let c = pm.pixel(x, y).unwrap().demultiply();
            v.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
        }
    }
    v.extend(std::iter::repeat_n(0u8, (mask_row * h) as usize));
    v
}
