//! Changing the desktop wallpaper. Every desktop keeps it somewhere else: GNOME, Ubuntu,
//! Budgie, Cinnamon and MATE in GSettings, Xfce in xfconf, and KDE Plasma inside plasmashell
//! (changed with a small script over D-Bus).

use super::monitors::Monitor;
use super::{paths, util};
use crate::model::Rgb;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct Gs {
    schema: &'static str,
    /// "picture-uri" (a file:// URI), or MATE's "picture-filename" (a path).
    key: &'static str,
    path: bool,
    /// GNOME 42+ keeps a second wallpaper for the dark style.
    dark: bool,
}

#[derive(Clone, Debug)]
pub enum Backend {
    Gsettings(Gs),
    Xfce,
    Kde,
    Unsupported(String),
}

/// What's under the boxes: a picture, or a plain color.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Base {
    Image(PathBuf),
    Color(Rgb),
}

/// One setting to put back when DeskChalk stops painting the wallpaper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Restore {
    Gsettings { schema: String, key: String, value: String },
    /// `value` None: the property didn't exist before (it's removed again).
    Xfconf { prop: String, value: Option<String>, kind: String },
    Kde { desktop: usize, plugin: String, image: String },
}

/// What the desktop showed before DeskChalk painted it. Entries belong to a monitor key;
/// "" means every monitor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub bases: Vec<(String, Base)>,
    pub restore: Vec<(String, Restore)>,
}

impl Snapshot {
    pub fn base_for(&self, key: &str) -> Option<&Base> {
        self.bases
            .iter()
            .find(|(k, _)| k == key)
            .or_else(|| self.bases.iter().find(|(k, _)| k.is_empty()))
            .map(|(_, b)| b)
    }

    /// True if every picture shown is one DeskChalk painted.
    pub fn all_ours(&self) -> bool {
        !self.bases.is_empty() && self.bases.iter().all(|(_, b)| is_ours(b))
    }

    /// Takes over wallpapers the user picked since (any monitor not showing one of DeskChalk's
    /// pictures). True if something changed.
    pub fn take_user_changes(&mut self, now: &Snapshot) -> bool {
        let mut changed = false;
        for (key, base) in now.bases.iter().filter(|(_, b)| !is_ours(b)) {
            match self.bases.iter_mut().find(|(k, _)| k == key) {
                Some((_, b)) if b == base => continue,
                Some((_, b)) => *b = base.clone(),
                None => self.bases.push((key.clone(), base.clone())),
            }
            changed = true;
            // "" (every monitor) replaces everything; a monitor replaces its own entries.
            if key.is_empty() {
                self.bases.retain(|(k, _)| k.is_empty());
                self.restore.clear();
            } else {
                self.restore.retain(|(k, _)| k != key);
            }
            self.restore.extend(now.restore.iter().filter(|(k, _)| k == key).cloned());
        }
        changed
    }
}

/// A picture inside DeskChalk's own folder (one it painted).
fn is_ours(b: &Base) -> bool {
    matches!(b, Base::Image(p) if p.starts_with(paths::data_dir()))
}

/// The pictures to show: one across all screens, or one per screen (by monitor key).
pub struct Pictures {
    pub spanned: Option<PathBuf>,
    pub per_monitor: Vec<(String, PathBuf)>,
}

impl Backend {
    pub fn detect() -> Backend {
        let env = |v: &str| std::env::var(v).unwrap_or_default();
        let desk = format!("{}:{}", env("XDG_CURRENT_DESKTOP"), env("DESKTOP_SESSION")).to_ascii_uppercase();
        let has = |s: &str| desk.contains(s);
        let name = env("XDG_CURRENT_DESKTOP");
        if has("KDE") || has("PLASMA") {
            return Backend::Kde;
        }
        if has("XFCE") {
            return Backend::Xfce;
        }
        if !util::have("gsettings") {
            return Backend::Unsupported(name);
        }
        if has("CINNAMON") {
            Backend::Gsettings(Gs { schema: "org.cinnamon.desktop.background", key: "picture-uri", path: false, dark: false })
        } else if has("MATE") {
            Backend::Gsettings(Gs { schema: "org.mate.background", key: "picture-filename", path: true, dark: false })
        } else if ["GNOME", "UNITY", "BUDGIE", "PANTHEON", "UBUNTU", "POP"].iter().any(|d| has(d)) {
            let dark = util::output("gsettings", &["list-keys", "org.gnome.desktop.background"])
                .is_some_and(|k| k.lines().any(|l| l.trim() == "picture-uri-dark"));
            Backend::Gsettings(Gs { schema: "org.gnome.desktop.background", key: "picture-uri", path: false, dark })
        } else {
            Backend::Unsupported(name)
        }
    }

    pub fn name(&self) -> String {
        match self {
            Backend::Gsettings(g) => format!("GSettings ({})", g.schema),
            Backend::Xfce => "Xfce (xfconf)".into(),
            Backend::Kde => "KDE Plasma".into(),
            Backend::Unsupported(d) => format!("unsupported ({})", if d.is_empty() { "unknown desktop" } else { d }),
        }
    }

    pub fn supported(&self) -> bool {
        !matches!(self, Backend::Unsupported(_))
    }

    /// One picture per screen (otherwise one picture spanning all screens).
    pub fn per_monitor(&self) -> bool {
        matches!(self, Backend::Xfce | Backend::Kde)
    }

    /// A file that changes when the wallpaper setting changes (checked now and then).
    pub fn watch_file(&self) -> Option<PathBuf> {
        match self {
            Backend::Gsettings(_) => Some(paths::dconf_db()),
            Backend::Xfce => Some(paths::config_file("xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml")),
            Backend::Kde => Some(paths::config_file("plasma-org.kde.plasma.desktop-appletsrc")),
            Backend::Unsupported(_) => None,
        }
    }

    /// What the desktop shows now.
    pub fn snapshot(&self, mons: &[Monitor]) -> Option<Snapshot> {
        match self {
            Backend::Gsettings(g) => gs_snapshot(g),
            Backend::Xfce => xfce_snapshot(mons),
            Backend::Kde => kde_snapshot(mons).ok(),
            Backend::Unsupported(_) => None,
        }
    }

    pub fn apply(&self, pics: &Pictures, mons: &[Monitor]) -> Result<(), String> {
        match self {
            Backend::Gsettings(g) => gs_apply(g, pics, mons),
            Backend::Xfce => xfce_apply(pics),
            Backend::Kde => kde_apply(pics, mons),
            Backend::Unsupported(d) => Err(format!("changing the wallpaper on {d} isn't supported yet")),
        }
    }

    pub fn restore(&self, snap: &Snapshot) -> Result<(), String> {
        match self {
            Backend::Gsettings(_) => {
                let mut ok = true;
                for (_, r) in &snap.restore {
                    if let Restore::Gsettings { schema, key, value } = r {
                        ok &= util::run("gsettings", &["set", schema, key, value]);
                    }
                }
                ok.then_some(()).ok_or_else(|| "gsettings failed".into())
            }
            Backend::Xfce => {
                let props = xf_list();
                for (_, r) in &snap.restore {
                    if let Restore::Xfconf { prop, value, kind } = r {
                        match value {
                            Some(v) => xf_set(&props, prop, kind, v),
                            None => util::run("xfconf-query", &["-c", "xfce4-desktop", "-p", prop, "-r"]),
                        };
                    }
                }
                Ok(())
            }
            Backend::Kde => {
                let mut script = String::from("var ds = desktops();\n");
                for (_, r) in &snap.restore {
                    if let Restore::Kde { desktop, plugin, image } = r {
                        script += &format!("if (ds[{desktop}]) {{ ds[{desktop}].wallpaperPlugin = {}; ", js(plugin));
                        if plugin == "org.kde.image" {
                            script += &format!(
                                "ds[{desktop}].currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General']; ds[{desktop}].writeConfig('Image', {}); ",
                                js(image)
                            );
                        }
                        script += "}\n";
                    }
                }
                kde_eval(&script).map(|_| ())
            }
            Backend::Unsupported(_) => Ok(()),
        }
    }
}

// ---- GSettings (GNOME, Ubuntu, Budgie, Cinnamon, MATE) -----------------------------------

fn gs_get(schema: &str, key: &str) -> Option<String> {
    util::output("gsettings", &["get", schema, key])
}

/// A GVariant string as gsettings prints it ('text', with \' and \\ escapes) -> text.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let inner = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).or_else(|| v.strip_prefix('"').and_then(|s| s.strip_suffix('"')));
    let Some(inner) = inner else { return v.to_string() };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The picture file a wallpaper setting points at (first picture of a GNOME slideshow).
fn picture(value: &str, is_path: bool) -> Option<PathBuf> {
    let p = if is_path { PathBuf::from(value) } else { util::uri_path(value)? };
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
        return slideshow_first(&p);
    }
    p.is_file().then_some(p)
}

fn slideshow_first(xml: &Path) -> Option<PathBuf> {
    let s = std::fs::read_to_string(xml).ok()?;
    let i = s.find("<file>")? + "<file>".len();
    let j = s[i..].find("</file>")? + i;
    let mut inner = s[i..j].trim();
    if let Some(k) = inner.find("<size") {
        let a = inner[k..].find('>')? + k + 1;
        let b = inner[a..].find("</size>")? + a;
        inner = inner[a..b].trim();
    }
    let p = PathBuf::from(inner);
    p.is_file().then_some(p)
}

fn prefer_dark() -> bool {
    gs_get("org.gnome.desktop.interface", "color-scheme").is_some_and(|v| unquote(&v) == "prefer-dark")
}

fn gs_snapshot(g: &Gs) -> Option<Snapshot> {
    let value = gs_get(g.schema, g.key)?;
    let options = gs_get(g.schema, "picture-options").unwrap_or_else(|| "'zoom'".into());
    let color = gs_get(g.schema, "primary-color")
        .and_then(|c| Rgb::from_hex(&unquote(&c)))
        .unwrap_or(Rgb(0x30, 0x30, 0x30));
    let entry = |key: &str, value: &str| (String::new(), Restore::Gsettings { schema: g.schema.into(), key: key.into(), value: value.into() });
    let mut restore = vec![entry(g.key, &value), entry("picture-options", &options)];
    let mut active = value.clone();
    if g.dark {
        if let Some(d) = gs_get(g.schema, "picture-uri-dark") {
            restore.push(entry("picture-uri-dark", &d));
            if prefer_dark() && !unquote(&d).is_empty() {
                active = d;
            }
        }
    }
    let base = if unquote(&options) == "none" {
        Base::Color(color)
    } else {
        picture(&unquote(&active), g.path).map_or(Base::Color(color), Base::Image)
    };
    Some(Snapshot { bases: vec![(String::new(), base)], restore })
}

fn gs_apply(g: &Gs, pics: &Pictures, mons: &[Monitor]) -> Result<(), String> {
    let file = pics
        .spanned
        .clone()
        .or_else(|| pics.per_monitor.first().map(|(_, p)| p.clone()))
        .ok_or("nothing to show")?;
    let value = if g.path { file.display().to_string() } else { util::file_uri(&file) };
    // One picture covering all screens; with one screen it is exactly that screen's size.
    let options = if mons.len() > 1 { "'spanned'" } else { "'zoom'" };
    let mut ok = util::run("gsettings", &["set", g.schema, "picture-options", options]);
    ok &= util::run("gsettings", &["set", g.schema, g.key, &quote(&value)]);
    if g.dark {
        ok &= util::run("gsettings", &["set", g.schema, "picture-uri-dark", &quote(&value)]);
    }
    ok.then_some(()).ok_or_else(|| "gsettings failed".into())
}

// ---- Xfce --------------------------------------------------------------------------------

fn xf_list() -> Vec<String> {
    util::output("xfconf-query", &["-c", "xfce4-desktop", "-l"])
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn xf_get(prop: &str) -> Option<String> {
    util::output("xfconf-query", &["-c", "xfce4-desktop", "-p", prop])
}

fn xf_set(existing: &[String], prop: &str, kind: &str, value: &str) -> bool {
    if existing.iter().any(|p| p == prop) {
        util::run("xfconf-query", &["-c", "xfce4-desktop", "-p", prop, "-s", value])
    } else {
        util::run("xfconf-query", &["-c", "xfce4-desktop", "-p", prop, "-n", "-t", kind, "-s", value])
    }
}

/// The "last-image" properties for a monitor (one per workspace), or the one to create.
fn xf_targets(props: &[String], key: &str) -> Vec<String> {
    let prefix = format!("/backdrop/screen0/monitor{key}/workspace");
    let found: Vec<String> = props.iter().filter(|p| p.starts_with(&prefix) && p.ends_with("/last-image")).cloned().collect();
    if found.is_empty() {
        vec![format!("{prefix}0/last-image")]
    } else {
        found
    }
}

fn xfce_snapshot(mons: &[Monitor]) -> Option<Snapshot> {
    let props = xf_list();
    if props.is_empty() {
        return None;
    }
    let mut snap = Snapshot::default();
    for m in mons {
        let mut base = None;
        for li in xf_targets(&props, &m.key) {
            let value = xf_get(&li);
            if base.is_none() {
                base = value.as_deref().map(PathBuf::from).filter(|p| p.is_file()).map(Base::Image);
            }
            let style = li.replace("/last-image", "/image-style");
            snap.restore.push((m.key.clone(), Restore::Xfconf { prop: li, value, kind: "string".into() }));
            snap.restore.push((m.key.clone(), Restore::Xfconf { value: xf_get(&style), prop: style, kind: "int".into() }));
        }
        // Older Xfce names monitors by number: fall back to any picture it uses.
        let base = base.or_else(|| {
            props.iter().filter(|p| p.ends_with("/last-image")).find_map(|p| xf_get(p)).map(PathBuf::from).filter(|p| p.is_file()).map(Base::Image)
        });
        snap.bases.push((m.key.clone(), base.unwrap_or(Base::Color(Rgb(0x30, 0x30, 0x30)))));
    }
    Some(snap)
}

fn xfce_apply(pics: &Pictures) -> Result<(), String> {
    let props = xf_list();
    let mut ok = true;
    for (key, file) in &pics.per_monitor {
        for li in xf_targets(&props, key) {
            ok &= xf_set(&props, &li, "string", &file.display().to_string());
            // 3 = stretched: the picture is exactly the screen's size.
            ok &= xf_set(&props, &li.replace("/last-image", "/image-style"), "int", "3");
        }
    }
    ok.then_some(()).ok_or_else(|| "xfconf-query failed".into())
}

// ---- KDE Plasma ----------------------------------------------------------------------------

fn js(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n"))
}

fn kde_eval(script: &str) -> Result<String, String> {
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let reply = conn
        .call_method(Some("org.kde.plasmashell"), "/PlasmaShell", Some("org.kde.PlasmaShell"), "evaluateScript", &(script,))
        .map_err(|e| e.to_string())?;
    reply.body().deserialize::<String>().map_err(|e| e.to_string())
}

struct KdeDesktop {
    index: usize,
    geometry: (f32, f32, f32, f32),
    plugin: String,
    image: String,
}

const KDE_READ: &str = r#"var ds = desktops(); var out = [];
for (var i = 0; i < ds.length; i++) {
  var d = ds[i];
  if (d.screen < 0) continue;
  var g = screenGeometry(d.screen);
  d.currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General'];
  out.push([i, g.x, g.y, g.width, g.height, d.wallpaperPlugin, d.readConfig('Image')].join('\t'));
}
print(out.join('\n'));"#;

fn kde_desktops() -> Result<Vec<KdeDesktop>, String> {
    let out = kde_eval(KDE_READ)?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 6 {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<f32>().ok();
            Some(KdeDesktop {
                index: f[0].trim().parse().ok()?,
                geometry: (n(1)?, n(2)?, n(3)?, n(4)?),
                plugin: f[5].trim().to_string(),
                image: f.get(6).map(|s| s.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect())
}

/// The monitor a Plasma desktop is on: its geometry matches the screen layout (or the
/// physical pixels); otherwise the nearest one.
pub fn match_monitor<'a>(g: (f32, f32, f32, f32), mons: &'a [Monitor]) -> Option<&'a Monitor> {
    let close = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
        (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0 && (a.2 - b.2).abs() < 2.0 && (a.3 - b.3).abs() < 2.0
    };
    let physical = |m: &Monitor| (m.x as f32, m.y as f32, m.w as f32, m.h as f32);
    mons.iter()
        .find(|m| close(g, m.layout()) || close(g, physical(m)))
        .or_else(|| {
            let c = (g.0 + g.2 / 2.0, g.1 + g.3 / 2.0);
            mons.iter().min_by(|a, b| {
                let d = |m: &Monitor| {
                    let l = m.layout();
                    (l.0 + l.2 / 2.0 - c.0).powi(2) + (l.1 + l.3 / 2.0 - c.1).powi(2)
                };
                d(a).total_cmp(&d(b))
            })
        })
}

/// A Plasma wallpaper package folder -> its largest picture.
fn kde_package_image(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("contents/images"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next()?;
            let (w, h) = stem.split_once('x')?;
            Some((w.parse::<u64>().ok()? * h.parse::<u64>().ok()?, e.path()))
        })
        .max_by_key(|(px, _)| *px)
        .map(|(_, p)| p)
}

fn kde_snapshot(mons: &[Monitor]) -> Result<Snapshot, String> {
    let mut snap = Snapshot::default();
    for d in kde_desktops()? {
        let Some(m) = match_monitor(d.geometry, mons) else { continue };
        let base = if d.plugin == "org.kde.image" {
            util::uri_path(&d.image)
                .and_then(|p| if p.is_dir() { kde_package_image(&p) } else { p.is_file().then_some(p) })
                .map(Base::Image)
        } else {
            None
        };
        if !snap.bases.iter().any(|(k, _)| *k == m.key) {
            snap.bases.push((m.key.clone(), base.unwrap_or(Base::Color(Rgb(0x1D, 0x1F, 0x24)))));
        }
        snap.restore.push((m.key.clone(), Restore::Kde { desktop: d.index, plugin: d.plugin, image: d.image }));
    }
    Ok(snap)
}

fn kde_apply(pics: &Pictures, mons: &[Monitor]) -> Result<(), String> {
    let mut script = String::from("var ds = desktops();\n");
    for d in kde_desktops()? {
        let Some(m) = match_monitor(d.geometry, mons) else { continue };
        let Some((_, file)) = pics.per_monitor.iter().find(|(k, _)| *k == m.key) else { continue };
        let i = d.index;
        script += &format!(
            "ds[{i}].wallpaperPlugin = 'org.kde.image'; ds[{i}].currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General']; ds[{i}].writeConfig('Image', {});\n",
            js(&util::file_uri(file))
        );
    }
    kde_eval(&script).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::monitors::grid_for;

    fn mon(key: &str, x: i32, y: i32, w: u32, h: u32, scale: f32, wayland: bool) -> Monitor {
        Monitor { key: key.into(), x, y, w, h, scale, primary: x == 0, grid: grid_for(w, h, scale, 96.0), wayland }
    }

    #[test]
    fn gvariant_strings() {
        assert_eq!(unquote("'file:///a b.png'"), "file:///a b.png");
        assert_eq!(unquote(r"'it\'s'"), "it's");
        assert_eq!(quote("it's"), r"'it\'s'");
        assert_eq!(unquote("'zoom'\n"), "zoom");
    }

    #[test]
    fn plasma_screens_match_monitors() {
        // A 4K screen at 200% left of a 1080p screen at 100%. On Wayland, Plasma reports
        // logical geometry and winit puts HDMI-1 at its logical x (1920) times its scale (1).
        let wl = [mon("DP-1", 0, 0, 3840, 2160, 2.0, true), mon("HDMI-1", 1920, 0, 1920, 1080, 1.0, true)];
        assert_eq!(match_monitor((0.0, 0.0, 1920.0, 1080.0), &wl).unwrap().key, "DP-1");
        assert_eq!(match_monitor((1920.0, 0.0, 1920.0, 1080.0), &wl).unwrap().key, "HDMI-1");
        // On X11 everything is in pixels.
        let x11 = [mon("DP-1", 0, 0, 3840, 2160, 2.0, false), mon("HDMI-1", 3840, 0, 1920, 1080, 1.0, false)];
        assert_eq!(match_monitor((0.0, 0.0, 3840.0, 2160.0), &x11).unwrap().key, "DP-1");
        assert_eq!(match_monitor((3840.0, 0.0, 1920.0, 1080.0), &x11).unwrap().key, "HDMI-1");
        // Off by a bit: the nearest screen.
        assert_eq!(match_monitor((3900.0, 10.0, 1900.0, 1000.0), &x11).unwrap().key, "HDMI-1");
    }

    #[test]
    fn user_changes_replace_the_original() {
        let ours = paths::data_dir().join("wall-1.png");
        let mut orig = Snapshot {
            bases: vec![("A".into(), Base::Image("/pics/a.jpg".into())), ("B".into(), Base::Image("/pics/b.jpg".into()))],
            restore: vec![
                ("A".into(), Restore::Kde { desktop: 0, plugin: "org.kde.image".into(), image: "file:///pics/a.jpg".into() }),
                ("B".into(), Restore::Kde { desktop: 1, plugin: "org.kde.image".into(), image: "file:///pics/b.jpg".into() }),
            ],
        };
        // A still shows DeskChalk's picture, B got a new wallpaper.
        let now = Snapshot {
            bases: vec![("A".into(), Base::Image(ours.clone())), ("B".into(), Base::Image("/pics/new.jpg".into()))],
            restore: vec![
                ("A".into(), Restore::Kde { desktop: 0, plugin: "org.kde.image".into(), image: util::file_uri(&ours) }),
                ("B".into(), Restore::Kde { desktop: 1, plugin: "org.kde.image".into(), image: "file:///pics/new.jpg".into() }),
            ],
        };
        assert!(!now.all_ours());
        assert!(orig.take_user_changes(&now));
        assert_eq!(orig.base_for("A"), Some(&Base::Image("/pics/a.jpg".into())));
        assert_eq!(orig.base_for("B"), Some(&Base::Image("/pics/new.jpg".into())));
        assert!(orig.restore.iter().any(|(k, r)| k == "B" && matches!(r, Restore::Kde { image, .. } if image.ends_with("new.jpg"))));
        assert!(orig.restore.iter().any(|(k, r)| k == "A" && matches!(r, Restore::Kde { image, .. } if image.ends_with("a.jpg"))));
        assert!(!orig.take_user_changes(&now));
    }
}
