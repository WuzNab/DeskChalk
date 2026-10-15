//! Paints the boxes (and grid dots) onto the wallpaper and writes the pictures the desktop
//! shows. Runs on a worker thread: decoding and writing large pictures takes a moment.

use super::monitors::Monitor;
use super::wallpaper::{Base, Pictures, Snapshot};
use super::{fonts, image};
use crate::editor;
use crate::geom::Pt;
use crate::model::{Config, Rgb};
use crate::render::{self, GridLayer, Painter, Scene, ZoneItem};
use crate::text::Fonts;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;

pub struct Job {
    /// Increases with every job; names the files so desktops notice the change.
    pub serial: u64,
    pub cfg: Config,
    pub monitors: Vec<Monitor>,
    pub original: Snapshot,
    pub per_monitor: bool,
    pub dir: PathBuf,
}

/// The wallpaper (or plain color) under the boxes, filling the monitor.
pub fn background(base: Option<&Base>, w: u32, h: u32, cache: &mut HashMap<PathBuf, Option<Pixmap>>) -> Option<Pixmap> {
    match base {
        Some(Base::Image(p)) => {
            let img = cache.entry(p.clone()).or_insert_with(|| image::load(p));
            match img {
                Some(img) => image::cover(img, w, h),
                None => image::solid(w, h, Rgb(0x30, 0x30, 0x30)),
            }
        }
        Some(Base::Color(c)) => image::solid(w, h, *c),
        None => image::solid(w, h, Rgb(0x30, 0x30, 0x30)),
    }
}

/// What the boxes look like on a monitor (drawn from monitor coordinates `origin`).
pub fn scene<'a>(cfg: &'a Config, mon: &'a Monitor, fonts: Option<&'a Fonts>, swap_rb: bool, origin: Pt) -> Scene<'a> {
    let size = mon.size();
    Scene {
        origin,
        scale: mon.scale,
        painter: Painter { swap_rb },
        title_fonts: fonts,
        ui_fonts: None,
        grid: cfg.grid.show_on_desktop.then_some(GridLayer {
            grid: &mon.grid,
            color: cfg.grid.color,
            radius: cfg.grid.size,
            alpha: cfg.grid.opacity as f32 / 100.0,
            repel: None,
        }),
        zones: cfg
            .zones
            .iter()
            .filter(|z| cfg.show_zones && z.monitor == mon.key)
            .map(|z| ZoneItem { zone: z, rect: editor::zone_rect(z, size), selected: false })
            .collect(),
        edit: None,
        mix_colors: cfg.mix_colors,
        bounds: mon.full(),
    }
}

pub fn needs_fonts(cfg: &Config) -> bool {
    cfg.show_zones && cfg.zones.iter().any(|z| !z.title.trim().is_empty())
}

/// Paints every monitor and writes the pictures.
pub fn bake(job: &Job) -> Result<Pictures, String> {
    std::fs::create_dir_all(&job.dir).map_err(|e| format!("can't create {}: {e}", job.dir.display()))?;
    let fonts = if needs_fonts(&job.cfg) { fonts::load(&job.cfg.font) } else { None };
    let mut cache = HashMap::new();
    let mut painted: Vec<(&Monitor, Pixmap)> = Vec::new();
    for mon in &job.monitors {
        let mut pm = background(job.original.base_for(&mon.key), mon.w, mon.h, &mut cache).ok_or("out of memory")?;
        render::draw(&mut pm.as_mut(), &scene(&job.cfg, mon, fonts.as_ref(), false, Pt::new(0.0, 0.0)));
        painted.push((mon, pm));
    }
    drop(cache);
    let file = |suffix: &str| job.dir.join(format!("wall-{}{suffix}.png", job.serial));
    if job.per_monitor || painted.len() == 1 {
        let mut per_monitor = Vec::new();
        for (i, (mon, pm)) in painted.iter().enumerate() {
            let path = file(&format!("-{i}"));
            image::save_png(pm, &path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
            per_monitor.push((mon.key.clone(), path));
        }
        let spanned = (painted.len() == 1).then(|| per_monitor[0].1.clone());
        return Ok(Pictures { spanned, per_monitor });
    }
    let path = file("");
    let span = spanned(&painted).ok_or("out of memory")?;
    image::save_png(&span, &path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    Ok(Pictures { spanned: Some(path), per_monitor: Vec::new() })
}

/// One picture across all screens, laid out like the desktop's screen layout (that is what
/// "spanned" wallpapers follow), sharp on the screen with the most pixels per layout unit.
fn spanned(painted: &[(&Monitor, Pixmap)]) -> Option<Pixmap> {
    let k = painted.iter().map(|(m, _)| m.w as f32 / m.layout().2.max(1.0)).fold(0.1f32, f32::max);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (m, _) in painted {
        let (x, y, w, h) = m.layout();
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x + w);
        y1 = y1.max(y + h);
    }
    let mut out = Pixmap::new(((x1 - x0) * k).round() as u32, ((y1 - y0) * k).round() as u32)?;
    out.fill(tiny_skia::Color::BLACK);
    for (m, pm) in painted {
        let (x, y, w, h) = m.layout();
        image::draw_scaled(&mut out, pm, ((x - x0) * k).round(), ((y - y0) * k).round(), (w * k).round(), (h * k).round());
    }
    Some(out)
}

/// Deletes pictures from earlier jobs (keeps `keep`).
pub fn clean(dir: &Path, keep: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("wall-") && (name.ends_with(".png") || name.ends_with(".tmp")) && !keep.contains(&p) {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::linux::monitors::grid_for;
    use crate::model::{Zone, ZoneStyle};

    fn mon(key: &str, x: i32, w: u32, h: u32, scale: f32, wayland: bool) -> Monitor {
        Monitor { key: key.into(), x, y: 0, w, h, scale, primary: x == 0, grid: grid_for(w, h, scale, 96.0), wayland }
    }

    #[test]
    fn bakes_boxes_onto_each_monitor() {
        let dir = std::env::temp_dir().join(format!("deskchalk-bake-{}", std::process::id()));
        let mut cfg = Config::default();
        let mut style = ZoneStyle::default();
        style.color = Rgb(255, 0, 0);
        style.fill = 100;
        style.border_width = 0.0;
        cfg.zones.push(Zone {
            id: 1,
            monitor: "B".into(),
            monitor_hw: String::new(),
            rect: Rect::new(10.0, 10.0, 50.0, 30.0),
            ref_size: (200, 100),
            title: String::new(),
            style,
        });
        let original = Snapshot { bases: vec![(String::new(), Base::Color(Rgb(0, 0, 255)))], restore: Vec::new() };
        let job = Job {
            serial: 7,
            cfg,
            monitors: vec![mon("A", 0, 200, 100, 1.0, false), mon("B", 200, 200, 100, 1.0, false)],
            original,
            per_monitor: false,
            dir: dir.clone(),
        };
        let pics = bake(&job).unwrap();
        let span = image::load(pics.spanned.as_ref().unwrap()).unwrap();
        assert_eq!((span.width(), span.height()), (400, 100));
        let px = |x: u32, y: u32| span.pixel(x, y).unwrap();
        // The box is on the second screen only.
        assert!(px(230, 25).red() > 200 && px(230, 25).blue() < 60);
        assert!(px(30, 25).blue() > 200 && px(30, 25).red() < 60);

        let per = bake(&Job { per_monitor: true, serial: 8, ..job }).unwrap();
        assert_eq!(per.per_monitor.len(), 2);
        let b = image::load(&per.per_monitor[1].1).unwrap();
        assert!(b.pixel(30, 25).unwrap().red() > 200);
        clean(&dir, &[]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spans_mixed_scaling_like_wayland_desktops() {
        // Wayland: a 4K screen at 200% (logical 0..100) left of a screen at 100% (logical 100..200).
        // winit reports the second one's position as logical x * its own scale = 100.
        let (a, b) = (mon("A", 0, 200, 100, 2.0, true), mon("B", 100, 100, 50, 1.0, true));
        let red = image::solid(200, 100, Rgb(255, 0, 0)).unwrap();
        let blue = image::solid(100, 50, Rgb(0, 0, 255)).unwrap();
        let span = spanned(&[(&a, red), (&b, blue)]).unwrap();
        // Two layout units per pixel of the sharper screen: 200 x 50 layout units -> 400 x 100.
        assert_eq!((span.width(), span.height()), (400, 100));
        assert!(span.pixel(150, 50).unwrap().red() > 200);
        assert!(span.pixel(250, 50).unwrap().blue() > 200);
    }
}
