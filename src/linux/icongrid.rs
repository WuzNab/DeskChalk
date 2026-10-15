//! The desktop's own icon grid, so DeskChalk's dots and snapping match where icons go.
//! Linux desktops draw their icons with different programs, each with its own layout:
//!
//! - GNOME and Ubuntu (Desktop Icons NG): the extension publishes every screen's usable area
//!   over D-Bus; its icon size setting gives the cell size, and the cells fill the area evenly.
//! - Cinnamon (Nemo): fixed-size cells from Nemo's icon size and grid spacing settings,
//!   centered in the work area Cinnamon reports over D-Bus.
//! - Xfce (xfdesktop): square slots from the icon size, spread over the usable area of each
//!   screen (since 4.20) or of the whole desktop (before).
//! - KDE Plasma (folder view): cells from the icon size, label width and text lines settings
//!   and the font size, stretched to fill the screen minus its panels.
//!
//! Everything is read from the running desktop; there is nothing to set up. A screen whose grid
//! can't be worked out keeps DeskChalk's own even grid.

use super::monitors::Monitor;
use super::wallpaper::match_monitor;
use super::{paths, util};
use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Manager {
    Ding,
    Nemo,
    Xfdesktop,
    Plasma,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Manager::Ding => "Desktop Icons NG",
            Manager::Nemo => "Nemo",
            Manager::Xfdesktop => "xfdesktop",
            Manager::Plasma => "Plasma folder view",
        }
    }
}

/// This user's running programs: (name, command line).
fn processes() -> Vec<(String, String)> {
    use std::os::unix::fs::MetadataExt;
    let me = std::fs::metadata(paths::home()).map(|m| m.uid()).ok();
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter(|e| me.is_none() || e.metadata().is_ok_and(|m| Some(m.uid()) == me))
        .filter_map(|e| {
            let p = e.path();
            let comm = std::fs::read_to_string(p.join("comm")).ok()?.trim().to_string();
            let cmd = std::fs::read(p.join("cmdline")).ok().map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
            Some((comm, cmd))
        })
        .collect()
}

fn bus_has_owner(name: &str) -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else { return false };
    conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,))
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// Which program draws the desktop icons, if any DeskChalk knows.
pub fn manager() -> Option<Manager> {
    if bus_has_owner("com.rastersoft.dingextension") {
        return Some(Manager::Ding);
    }
    let procs = processes();
    let running = |name: &str| procs.iter().any(|(c, _)| c == name);
    if procs.iter().any(|(_, cmd)| cmd.contains("ding.js")) {
        Some(Manager::Ding)
    } else if running("nemo-desktop") {
        Some(Manager::Nemo)
    } else if running("xfdesktop") && xfconf_int("/desktop-icons/style").unwrap_or(2) == 2 {
        // style 2 = file icons (0 = none, 1 = minimized windows)
        Some(Manager::Xfdesktop)
    } else if running("plasmashell") {
        Some(Manager::Plasma)
    } else {
        None
    }
}

/// The icon grid of every monitor it can work out (monitor key -> grid).
pub fn detect(mons: &[Monitor]) -> (Option<Manager>, Vec<(String, Grid)>) {
    let m = manager();
    let grids = match m {
        Some(Manager::Ding) => ding(mons),
        Some(Manager::Nemo) => nemo(mons),
        Some(Manager::Xfdesktop) => xfce(mons),
        Some(Manager::Plasma) => plasma(mons),
        None => Vec::new(),
    };
    (m, grids)
}

/// A grid of `cols` x `rows` cells of `cell` size starting at `origin` (monitor pixels).
fn cells(origin: Pt, cell: Pt, cols: f32, rows: f32) -> Grid {
    Grid { origin, cell, area: Rect::new(origin.x, origin.y, cell.x * cols, cell.y * rows) }
}

// ---- GNOME / Ubuntu: Desktop Icons NG ---------------------------------------------------------

/// One screen as Desktop Icons NG describes it (GNOME Shell stage coordinates).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DingArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub zoom: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub window_margin_top: f64,
    pub window_margin_left: f64,
}

/// Cell size (width, height) before spacing, per icon size (Desktop Icons NG's enums.js).
fn ding_cell(size: &str) -> (f64, f64) {
    match size {
        "tiny" => (70.0, 80.0),
        "small" => (90.0, 90.0),
        "large" => (130.0, 138.0),
        _ => (120.0, 106.0),
    }
}

/// The grid of one screen, in pixels of a monitor `mon_w` pixels wide (desktopGrid.js:
/// as many cells as fit with 2 px spacing on each side, then stretched to fill the area).
pub fn ding_grid(a: &DingArea, size: &str, mon_w: f64) -> Option<Grid> {
    if a.width <= 0.0 || a.height <= 0.0 {
        return None;
    }
    let zoom = a.zoom.max(1.0);
    let k = mon_w / a.width;
    let (cw, ch) = ding_cell(size);
    let w = ((a.width - a.margin_left - a.margin_right) / zoom).floor();
    let h = ((a.height - a.margin_top - a.margin_bottom) / zoom).floor();
    let cols = (w / (cw + 8.0)).floor();
    let rows = (h / (ch + 8.0)).floor();
    if cols < 1.0 || rows < 1.0 {
        return None;
    }
    let ox = a.window_margin_left + zoom * ((a.margin_left - a.window_margin_left) / zoom).floor();
    let oy = a.window_margin_top + zoom * ((a.margin_top - a.window_margin_top) / zoom).floor();
    let origin = Pt::new((ox * k) as f32, (oy * k) as f32);
    let cell = Pt::new((zoom * w / cols * k) as f32, (zoom * h / rows * k) as f32);
    Some(cells(origin, cell, cols as f32, rows as f32))
}

/// The screens from the extension's "desktopGeometry" action (exported over D-Bus).
fn ding_areas_dbus() -> Option<Vec<DingArea>> {
    use zbus::zvariant::{OwnedValue, Signature, Value};
    let conn = zbus::blocking::Connection::session().ok()?;
    let reply = conn
        .call_method(
            Some("com.rastersoft.dingextension"),
            "/com/rastersoft/dingextension/control",
            Some("org.gtk.Actions"),
            "Describe",
            &("desktopGeometry",),
        )
        .ok()?;
    let (_enabled, _param, state): (bool, Signature, Vec<OwnedValue>) = reply.body().deserialize().ok()?;
    let list: Value = state.into_iter().next()?.into();
    let list = match list {
        Value::Value(inner) => *inner,
        other => other,
    };
    let Value::Array(items) = list else { return None };
    let mut out = Vec::new();
    for item in items.iter() {
        let item = match item {
            Value::Value(inner) => inner.as_ref(),
            other => other,
        };
        let Ok(map) = item.try_clone().map_err(|_| ()).and_then(|v| HashMap::<String, f64>::try_from(v).map_err(|_| ())) else {
            continue;
        };
        let get = |k: &str| map.get(k).copied().unwrap_or(0.0);
        out.push(DingArea {
            x: get("x"),
            y: get("y"),
            width: get("width"),
            height: get("height"),
            zoom: map.get("scaleFactor").copied().unwrap_or(1.0),
            margin_top: get("marginTop"),
            margin_bottom: get("marginBottom"),
            margin_left: get("marginLeft"),
            margin_right: get("marginRight"),
            window_margin_top: get("windowMarginTop"),
            window_margin_left: get("windowMarginLeft"),
        });
    }
    (!out.is_empty()).then_some(out)
}

/// Older versions get the screens on the command line: -D x:y:w:h:zoom:top:bottom:left:right:index
pub fn ding_areas_cmdline(cmd: &str) -> Vec<DingArea> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    words
        .windows(2)
        .filter(|w| w[0] == "-D")
        .filter_map(|w| {
            let v: Vec<f64> = w[1].split(':').map(|n| n.parse().ok()).collect::<Option<_>>()?;
            (v.len() >= 9).then(|| DingArea {
                x: v[0],
                y: v[1],
                width: v[2],
                height: v[3],
                zoom: v[4],
                margin_top: v[5],
                margin_bottom: v[6],
                margin_left: v[7],
                margin_right: v[8],
                window_margin_top: 0.0,
                window_margin_left: 0.0,
            })
        })
        .collect()
}

fn ding_icon_size() -> String {
    util::output("gsettings", &["get", "org.gnome.shell.extensions.ding", "icon-size"])
        .or_else(|| util::output("dconf", &["read", "/org/gnome/shell/extensions/ding/icon-size"]))
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "standard".into())
}

fn ding(mons: &[Monitor]) -> Vec<(String, Grid)> {
    let areas = ding_areas_dbus().unwrap_or_else(|| {
        processes().iter().find(|(_, c)| c.contains("ding.js")).map(|(_, c)| ding_areas_cmdline(c)).unwrap_or_default()
    });
    let size = ding_icon_size();
    areas
        .iter()
        .filter_map(|a| {
            let m = match_monitor((a.x as f32, a.y as f32, a.width as f32, a.height as f32), mons)?;
            Some((m.key.clone(), ding_grid(a, &size, m.w as f64)?))
        })
        .collect()
}

// ---- Cinnamon: Nemo -------------------------------------------------------------------------

/// The grid inside a work area (monitor pixels), with GTK scale `scale` (nemo-centered-
/// placement-grid.c): cells of 130 x 100 at the standard 48 px icon size, times the grid
/// spacing in percent, as many as fit, centered.
pub fn nemo_grid(work: Rect, scale: f32, zoom: i32, h_adjust: i32, v_adjust: i32) -> Option<Grid> {
    let icon = match zoom.clamp(1, 5) {
        1 => 24.0,
        2 => 32.0,
        3 => 48.0,
        4 => 64.0,
        _ => 96.0,
    };
    let s = icon / 48.0;
    let snap_x = (130.0 * s * h_adjust as f64 / 100.0) as i32;
    let snap_y = (100.0 * s * v_adjust as f64 / 100.0) as i32;
    let scale = scale.max(1.0);
    let (w, h) = ((work.w / scale) as i32, (work.h / scale) as i32);
    if snap_x <= 0 || snap_y <= 0 || w < snap_x || h < snap_y {
        return None;
    }
    let (cols, rows) = (w / snap_x, h / snap_y);
    let left = (w - cols * snap_x) / 2;
    let top = (h - rows * snap_y) / 2;
    let origin = Pt::new(work.x + left as f32 * scale, work.y + top as f32 * scale);
    Some(cells(origin, Pt::new(snap_x as f32 * scale, snap_y as f32 * scale), cols as f32, rows as f32))
}

/// Nemo's per-monitor desktop settings (~/.config/nemo/desktop-metadata): zoom level and
/// grid spacing in percent.
pub fn nemo_settings(metadata: &str, monitor: usize) -> (i32, i32, i32) {
    let group = format!("[desktop-monitor-{monitor}]");
    let mut in_group = false;
    let (mut zoom, mut h, mut v) = (3, 100, 100);
    for line in metadata.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == group;
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some(val) = line.strip_prefix("nemo-icon-view-zoom-level=") {
            zoom = val.trim().parse().unwrap_or(zoom);
        } else if let Some(val) = line.strip_prefix("desktop-grid-adjust=") {
            let mut it = val.split(';').filter_map(|p| p.trim().parse::<i32>().ok());
            h = it.next().unwrap_or(h);
            v = it.next().unwrap_or(v);
        }
    }
    (zoom, h, v)
}

fn nemo(mons: &[Monitor]) -> Vec<(String, Grid)> {
    let Ok(conn) = zbus::blocking::Connection::session() else { return Vec::new() };
    let Ok(reply) = conn.call_method(Some("org.Cinnamon"), "/org/Cinnamon", Some("org.Cinnamon"), "GetMonitors", &()) else {
        return Vec::new();
    };
    let Ok(indices) = reply.body().deserialize::<Vec<i32>>() else { return Vec::new() };
    let metadata = std::fs::read_to_string(paths::config_file("nemo/desktop-metadata")).unwrap_or_default();
    indices
        .into_iter()
        .filter_map(|i| {
            let reply = conn.call_method(Some("org.Cinnamon"), "/org/Cinnamon", Some("org.Cinnamon"), "GetMonitorWorkRect", &(i,)).ok()?;
            let r = reply.body().deserialize::<Vec<i32>>().ok()?;
            let (x, y, w, h) = (*r.first()? as f32, *r.get(1)? as f32, *r.get(2)? as f32, *r.get(3)? as f32);
            // The monitor the work area is on (its center).
            let m = mons.iter().find(|m| {
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                cx >= m.x as f32 && cx < (m.x + m.w as i32) as f32 && cy >= m.y as f32 && cy < (m.y + m.h as i32) as f32
            })?;
            let (zoom, ha, va) = nemo_settings(&metadata, i.max(0) as usize);
            let work = Rect::new(x - m.x as f32, y - m.y as f32, w, h);
            Some((m.key.clone(), nemo_grid(work, m.scale.round(), zoom, ha, va)?))
        })
        .collect()
}

// ---- Xfce: xfdesktop -----------------------------------------------------------------------

fn xfconf_int(prop: &str) -> Option<i32> {
    util::output("xfconf-query", &["-c", "xfce4-desktop", "-p", prop])?.trim().parse().ok()
}

/// Slot layout over an area of `w` x `h` (logical pixels) for `icon` px icons
/// (xfdesktop-icon-view.c): square slots of 1.9 x icon + 2 x 6 padding, as many as fit inside
/// 8 px margins, with the rest spread between them. Returns the grid relative to the area, in
/// logical pixels, with cell corners in the middle of the gaps.
pub fn xfce_layout(w: i32, h: i32, icon: i32) -> Option<Grid> {
    let slot = 1.9 * icon as f64 + 12.0;
    let axis = |len: i32| -> Option<(i32, i32, i32)> {
        let n = ((len - 16) as f64 / slot) as i32;
        if n <= 0 {
            return None;
        }
        let rest = (len as f64 - n as f64 * slot) as i32;
        let spacing = if n > 1 { (rest - 16) / (n - 1) } else { 1 };
        let margin = (rest - (n - 1) * spacing) / 2;
        Some((n, spacing, margin))
    };
    let (cols, xs, xm) = axis(w)?;
    let (rows, ys, ym) = axis(h)?;
    let cell = Pt::new((slot + xs as f64) as f32, (slot + ys as f64) as f32);
    let origin = Pt::new(xm as f32 - xs as f32 / 2.0, ym as f32 - ys as f32 / 2.0);
    Some(cells(origin, cell, cols as f32, rows as f32))
}

/// The usable area of the whole desktop and of each monitor (X11, from the panels' struts).
struct WorkAreas {
    total: Option<Rect>,
    per_monitor: Vec<Rect>,
}

fn x11_work_areas(mons: &[Monitor]) -> Option<WorkAreas> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = |name: &str| conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok().map(|r| r.atom);
    let cardinals = |win: u32, prop: u32| -> Option<Vec<u32>> {
        let r = conn.get_property(false, win, prop, AtomEnum::CARDINAL, 0, 64).ok()?.reply().ok()?;
        let values: Vec<u32> = r.value32()?.collect();
        Some(values)
    };
    let workarea = atom("_NET_WORKAREA")?;
    let total = cardinals(root, workarea)
        .filter(|v| v.len() >= 4)
        .map(|v| Rect::new(v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32));
    let client_list = atom("_NET_CLIENT_LIST")?;
    let strut_partial = atom("_NET_WM_STRUT_PARTIAL")?;
    let windows: Vec<u32> = conn
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect()))
        .unwrap_or_default();
    let struts: Vec<Vec<u32>> = windows.iter().filter_map(|w| cardinals(*w, strut_partial).filter(|s| s.len() >= 12)).collect();
    let sw = mons.iter().map(|m| m.x + m.w as i32).max().unwrap_or(0) as f32;
    let sh = mons.iter().map(|m| m.y + m.h as i32).max().unwrap_or(0) as f32;
    let per_monitor = mons
        .iter()
        .map(|m| {
            let (x0, y0, x1, y1) = (m.x as f32, m.y as f32, (m.x + m.w as i32) as f32, (m.y + m.h as i32) as f32);
            let overlaps = |a: u32, b: u32, lo: f32, hi: f32| (a as f32) < hi && (b as f32) >= lo;
            let (mut l, mut t, mut r, mut b) = (x0, y0, x1, y1);
            for s in &struts {
                // left, right, top, bottom, then start/end of each along the other axis
                if s[0] > 0 && overlaps(s[4], s[5], y0, y1) && (s[0] as f32) > x0 {
                    l = l.max((s[0] as f32).min(x1));
                }
                if s[1] > 0 && overlaps(s[6], s[7], y0, y1) && sw - (s[1] as f32) < x1 {
                    r = r.min((sw - s[1] as f32).max(x0));
                }
                if s[2] > 0 && overlaps(s[8], s[9], x0, x1) && (s[2] as f32) > y0 {
                    t = t.max((s[2] as f32).min(y1));
                }
                if s[3] > 0 && overlaps(s[10], s[11], x0, x1) && sh - (s[3] as f32) < y1 {
                    b = b.min((sh - s[3] as f32).max(y0));
                }
            }
            Rect::new(l, t, (r - l).max(0.0), (b - t).max(0.0))
        })
        .collect();
    Some(WorkAreas { total, per_monitor })
}

fn xfdesktop_version() -> (u32, u32) {
    util::output("xfdesktop", &["--version"])
        .and_then(|s| {
            let v = s.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?.to_string();
            let mut it = v.split('.').filter_map(|n| n.parse::<u32>().ok());
            Some((it.next()?, it.next().unwrap_or(0)))
        })
        .unwrap_or((4, 20))
}

fn xfce(mons: &[Monitor]) -> Vec<(String, Grid)> {
    let Some(areas) = x11_work_areas(mons) else { return Vec::new() };
    let icon = xfconf_int("/desktop-icons/icon-size").unwrap_or(48).clamp(16, 256);
    let primary_only = util::output("xfconf-query", &["-c", "xfce4-desktop", "-p", "/desktop-icons/primary"]).is_some_and(|v| v.trim() == "true");
    let per_monitor = xfdesktop_version() >= (4, 20);
    let to_monitor = |g: Grid, area: Rect, scale: f32, m: &Monitor| -> Grid {
        let s = scale.max(1.0);
        let o = Pt::new(area.x - m.x as f32 + g.origin.x * s, area.y - m.y as f32 + g.origin.y * s);
        let cell = Pt::new(g.cell.x * s, g.cell.y * s);
        Grid { origin: o, cell, area: Rect::new(o.x, o.y, g.area.w * s, g.area.h * s) }
    };
    let mut out = Vec::new();
    for (i, m) in mons.iter().enumerate() {
        if primary_only && !m.primary {
            continue;
        }
        let scale = m.scale.round();
        let area = if per_monitor || primary_only { areas.per_monitor.get(i).copied() } else { areas.total.or(areas.per_monitor.get(i).copied()) };
        let Some(area) = area.filter(|a| a.w > 0.0 && a.h > 0.0) else { continue };
        let s = scale.max(1.0);
        let Some(g) = xfce_layout((area.w / s) as i32, (area.h / s) as i32, icon) else { continue };
        let mut g = to_monitor(g, area, scale, m);
        // One grid over the whole desktop (before 4.20): keep the part on this monitor.
        if let Some(clip) = g.area.intersect(&m.full()) {
            g.area = clip;
        }
        out.push((m.key.clone(), g));
    }
    out
}

// ---- KDE Plasma: folder view ------------------------------------------------------------------

/// The folder view grid over an available area (monitor pixels, `k` pixels per Plasma
/// layout unit) from its settings (FolderView.qml): icon size, label width (0..2) and text
/// lines, with `grid_unit` = the font's line height.
pub fn plasma_grid(avail: Rect, k: f32, icon: f32, label_width: i32, text_lines: i32, grid_unit: f32) -> Option<Grid> {
    let small_spacing = 4.0;
    let (vw, vh) = ((avail.w / k).ceil(), (avail.h / k).ceil());
    let fill = |min: f32, len: f32| -> Option<(f32, f32)> {
        let n = (len / min).floor();
        if n < 1.0 {
            return None;
        }
        Some((n, min + ((len - n * min) / n).floor()))
    };
    let min_w = (icon + 2.0 * grid_unit + 2.0 * small_spacing).max(16.0 * (label_width as f32 * 2.0 + 4.0));
    let min_h = icon + grid_unit * text_lines as f32 + 3.0 * small_spacing;
    let (cols, cw) = fill(min_w, vw)?;
    let (rows, ch) = fill(min_h, vh)?;
    Some(cells(Pt::new(avail.x, avail.y), Pt::new(cw * k, ch * k), cols, rows))
}

const PLASMA_ICON_SIZES: [f32; 7] = [22.0, 32.0, 48.0, 64.0, 96.0, 128.0, 256.0];

const PLASMA_READ: &str = r#"var out = [];
var ds = desktops();
for (var i = 0; i < ds.length; i++) {
  var d = ds[i];
  if (d.screen < 0) continue;
  var g = screenGeometry(d.screen);
  d.currentConfigGroup = ['General'];
  out.push(['D', d.screen, g.x, g.y, g.width, g.height, d.type, d.readConfig('iconSize', 3), d.readConfig('labelWidth', 1), d.readConfig('textLines', 2)].join('\t'));
}
var ps = panels();
for (var j = 0; j < ps.length; j++) {
  var p = ps[j];
  out.push(['P', p.screen, p.location, p.height, p.hiding, (typeof p.floating === 'undefined') ? false : p.floating].join('\t'));
}
print(out.join('\n'));"#;

/// The font's line height in pixels (Kirigami's grid unit), from kdeglobals' font setting.
fn plasma_grid_unit() -> f32 {
    let kdeglobals = std::fs::read_to_string(paths::config_file("kdeglobals")).unwrap_or_default();
    let font = kdeglobals
        .lines()
        .skip_while(|l| l.trim() != "[General]")
        .skip(1)
        .take_while(|l| !l.trim_start().starts_with('['))
        .find_map(|l| l.trim().strip_prefix("font="))
        .unwrap_or("Noto Sans,10");
    let mut parts = font.split(',');
    let family = parts.next().unwrap_or("Noto Sans").trim().to_string();
    let points: f32 = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(10.0);
    let px = points * 96.0 / 72.0;
    let height = util::output("fc-match", &["-f", "%{file}", &family])
        .and_then(|file| std::fs::read(file.trim()).ok())
        .and_then(|data| {
            let face = ttf_parser::Face::parse(&data, 0).ok()?;
            let s = px / face.units_per_em() as f32;
            Some((face.ascender() as f32 * s).round() + (-(face.descender() as f32) * s).round())
        })
        .unwrap_or(18.0);
    // Kirigami keeps the grid unit even.
    let h = height.max(8.0);
    if h as i32 % 2 == 1 { h + 1.0 } else { h }
}

fn plasma(mons: &[Monitor]) -> Vec<(String, Grid)> {
    let Ok(conn) = zbus::blocking::Connection::session() else { return Vec::new() };
    let Ok(reply) = conn.call_method(Some("org.kde.plasmashell"), "/PlasmaShell", Some("org.kde.PlasmaShell"), "evaluateScript", &(PLASMA_READ,)) else {
        return Vec::new();
    };
    let Ok(text) = reply.body().deserialize::<String>() else { return Vec::new() };
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    let grid_unit = plasma_grid_unit();
    let mut out = Vec::new();
    for d in lines.iter().filter(|f| f.len() >= 10 && f[0] == "D") {
        // Only the folder view lays icons out on the desktop.
        if d[6] != "org.kde.plasma.folder" {
            continue;
        }
        let n = |s: &str| s.trim().parse::<f32>().ok();
        let (Some(gx), Some(gy), Some(gw), Some(gh)) = (n(d[2]), n(d[3]), n(d[4]), n(d[5])) else { continue };
        let Some(m) = match_monitor((gx, gy, gw, gh), mons) else { continue };
        // Panels that always show take their thickness (plus the gap, when floating).
        let (mut l, mut t, mut r, mut b) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for p in lines.iter().filter(|f| f.len() >= 6 && f[0] == "P" && f[1] == d[1]) {
            if p[4] != "none" {
                continue;
            }
            let thick = n(p[3]).unwrap_or(0.0) + if p[5] == "true" { 8.0 } else { 0.0 };
            match p[2] {
                "top" => t = t.max(thick),
                "bottom" => b = b.max(thick),
                "left" => l = l.max(thick),
                "right" => r = r.max(thick),
                _ => {}
            }
        }
        let k = m.w as f32 / gw.max(1.0);
        let avail = Rect::new(l * k, t * k, (gw - l - r) * k, (gh - t - b) * k);
        let icon = PLASMA_ICON_SIZES[(n(d[7]).unwrap_or(3.0) as usize).min(PLASMA_ICON_SIZES.len() - 1)];
        let label = n(d[8]).unwrap_or(1.0) as i32;
        let text_lines = n(d[9]).unwrap_or(2.0) as i32;
        if let Some(g) = plasma_grid(avail, k, icon, label, text_lines, grid_unit) {
            out.push((m.key.clone(), g));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ding_fills_the_usable_area() {
        // 1920 x 1080, top bar 32, dock on the left 64, standard icons, no scaling.
        let a = DingArea { width: 1920.0, height: 1080.0, zoom: 1.0, margin_top: 32.0, margin_left: 64.0, ..Default::default() };
        let g = ding_grid(&a, "standard", 1920.0).unwrap();
        // 1856 / 128 = 14 columns of 132.57; 1048 / 114 = 9 rows of 116.44.
        assert_eq!(g.origin, Pt::new(64.0, 32.0));
        assert!((g.cell.x - 1856.0 / 14.0).abs() < 0.01);
        assert!((g.cell.y - 1048.0 / 9.0).abs() < 0.01);
        assert!((g.area.right() - 1920.0).abs() < 0.5 && (g.area.bottom() - 1080.0).abs() < 0.5);
        // The same screen at 200%: half as many logical pixels, cells twice the pixels.
        let a2 = DingArea { width: 3840.0, height: 2160.0, zoom: 2.0, margin_top: 64.0, margin_left: 128.0, ..Default::default() };
        let g2 = ding_grid(&a2, "standard", 3840.0).unwrap();
        assert!((g2.cell.x - 2.0 * g.cell.x).abs() < 0.05);
    }

    #[test]
    fn ding_command_line() {
        let a = ding_areas_cmdline("gjs /usr/share/ding/app/ding.js -E -P /x -D 0:0:1920:1080:1:32:0:64:0:0 -D 1920:0:1280:1024:1:0:0:0:0:1");
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].margin_top, a[0].margin_left), (32.0, 64.0));
        assert_eq!(a[1].x, 1920.0);
    }

    #[test]
    fn nemo_cells_are_centered() {
        // Standard size at 100%: 130 x 100 cells; 1920 x 1040 fits 14 x 10, centered.
        let g = nemo_grid(Rect::new(0.0, 40.0, 1920.0, 1040.0), 1.0, 3, 100, 100).unwrap();
        assert_eq!(g.cell, Pt::new(130.0, 100.0));
        assert_eq!(g.origin, Pt::new((1920.0 - 14.0 * 130.0) / 2.0, 40.0 + 20.0));
        let wide = nemo_grid(Rect::new(0.0, 0.0, 1920.0, 1080.0), 1.0, 3, 150, 100).unwrap();
        assert_eq!(wide.cell.x, 195.0);
    }

    #[test]
    fn nemo_metadata() {
        let meta = "[desktop-monitor-0]\nnemo-icon-view-zoom-level=4\ndesktop-grid-adjust=120;90;@x-nemo-desktop-metadata-term@;\n\n[desktop-monitor-1]\nnemo-icon-view-zoom-level=2\n";
        assert_eq!(nemo_settings(meta, 0), (4, 120, 90));
        assert_eq!(nemo_settings(meta, 1), (2, 100, 100));
        assert_eq!(nemo_settings(meta, 5), (3, 100, 100));
    }

    #[test]
    fn xfce_slots() {
        // 48 px icons: slots of 103.2; 1920 wide fits 18, the rest spread between them.
        let g = xfce_layout(1920, 1050, 48).unwrap();
        let cols = (g.area.w / g.cell.x).round();
        assert_eq!(cols, 18.0);
        assert!(g.origin.x >= 0.0 && g.area.right() <= 1920.0 + 1.0);
        assert!(xfce_layout(10, 10, 48).is_none());
    }

    #[test]
    fn plasma_cells_stretch_to_fill() {
        // 64 px icons, medium labels, 2 lines, 18 px font: 108 x 112 minimum.
        let g = plasma_grid(Rect::new(0.0, 0.0, 1920.0, 1032.0), 1.0, 64.0, 1, 2, 18.0).unwrap();
        assert_eq!((g.area.w / g.cell.x).round(), 17.0);
        assert!((g.cell.x - (108.0 + ((1920.0 - 17.0 * 108.0) / 17.0f32).floor())).abs() < 0.01);
        assert_eq!((g.area.h / g.cell.y).round(), 9.0);
    }
}
