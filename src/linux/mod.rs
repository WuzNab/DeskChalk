//! Linux: DeskChalk paints the boxes into the desktop wallpaper. Linux desktops have no
//! layer between the wallpaper and the icons that other apps can draw in, but every desktop
//! shows a wallpaper, so boxes painted into it sit behind the icons on X11 and Wayland alike.
//! Editing happens in fullscreen windows that show the wallpaper; the tray icon has the
//! settings.
//!
//! The event loop (winit) runs everything on one thread. The tray icon, the single-instance
//! socket and the wallpaper painter run on threads of their own and send user events.

#[macro_use]
pub mod util;
mod bake;
mod edit;
mod fonts;
mod image;
mod ipc;
mod monitors;
mod notify;
mod paths;
mod popup;
mod tray;
mod wallpaper;

use crate::commands as cmd;
use crate::config;
use crate::editor::{self, Editor, Handle, Hit, Outcome};
use crate::geom::{Pt, Rect};
use crate::grid::Grid;
use crate::model::{self, Config, Rgb, StyleCmd, Zone};
use crate::render::{self, EditLayer, GridLayer, Painter, Scene, ZoneItem};
use crate::text::Fonts;
use edit::EditWin;
use monitors::Monitor;
use popup::{Click, Popup};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};
use tiny_skia::{FillRule, PixmapMut, Stroke, Transform};
use wallpaper::{Backend, Pictures, Snapshot};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, WindowId};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO_URL: &str = "https://github.com/WuzNab/DeskChalk";
const SUPPORT_URL: &str = "https://ko-fi.com/wuznab";
const HINT: &str = "Drag to draw a box  \u{00B7}  Drag edges to resize  \u{00B7}  Double-click to name it  \u{00B7}  Right-click for options  \u{00B7}  Hold Shift to toggle snapping  \u{00B7}  Esc when done";
const SUPPORTED: &str = "GNOME, Ubuntu, KDE Plasma, Xfce, Cinnamon, MATE and Budgie";
/// How often screens and the wallpaper setting are checked for changes.
const POLL: Duration = Duration::from_secs(4);
const SAVE_DELAY: Duration = Duration::from_millis(800);
/// Changes are painted into the wallpaper after a short pause (several in a row paint once).
const PAINT_DELAY: Duration = Duration::from_millis(350);
const ANIM: Duration = Duration::from_millis(16);
const DOUBLE_CLICK: Duration = Duration::from_millis(450);
/// Give up waiting for the wallpaper to be put back when exiting after this long.
const EXIT_WAIT: Duration = Duration::from_secs(5);

pub enum UserEvent {
    Command(u32),
    Request(ipc::Request),
    Painted { serial: u64, result: Result<Pictures, String> },
    Restored { serial: u64 },
    Color { color: Rgb, slot: Slot, zone: Option<u64> },
    RemoveConfirmed,
}

#[derive(Clone, Copy)]
pub enum Slot {
    Box,
    Border,
    Dots,
}

/// Jobs for the wallpaper painter thread (only the newest waiting one is done).
enum Work {
    Paint(bake::Job),
    Restore(Snapshot, u64),
}

#[derive(Clone)]
enum Target {
    Global,
    Zone(u64),
    Monitor(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Exit {
    Quit,
    Remove,
}

/// Remembered between runs (in the data folder, next to the painted pictures).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// The wallpaper settings from before DeskChalk painted, put back when it stops.
    original: Option<Snapshot>,
    /// DeskChalk's pictures the desktop shows.
    pictures: Vec<PathBuf>,
}

fn state_path() -> PathBuf {
    paths::data_dir().join("state.json")
}

fn load_state() -> State {
    std::fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn desktop_name() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "this desktop".into())
}

struct Opts {
    edit: bool,
    /// Started by hand (not at login): say so, since the tray icon may be tucked away.
    announce: bool,
    diag: bool,
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).map(|a| a.to_ascii_lowercase()).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--log") {
        util::enable_log(paths::config_dir().join("deskchalk.log"));
    }
    if has("--quit") {
        if ipc::send("quit") {
            ipc::wait_gone(10);
        }
        return;
    }
    let diag = has("--diag");
    let mut owns_socket = false;
    let listener = if diag {
        None
    } else {
        match ipc::claim() {
            ipc::Claim::First(l) => {
                owns_socket = true;
                Some(l)
            }
            ipc::Claim::Taken => {
                // Already running: opening it again shows where it is (or edit mode with --edit).
                ipc::send(if has("--edit") { "edit" } else { "show" });
                return;
            }
            ipc::Claim::Unavailable => {
                log!("no single-instance socket; running anyway");
                None
            }
        }
    };
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("DeskChalk needs a desktop session (X11 or Wayland): {e}");
            if owns_socket {
                ipc::cleanup();
            }
            std::process::exit(1);
        }
    };
    let proxy = event_loop.create_proxy();
    if let Some(l) = listener {
        let p = proxy.clone();
        ipc::serve(l, move |r| {
            let _ = p.send_event(UserEvent::Request(r));
        });
    }
    let mut app = App::new(proxy, Opts { edit: has("--edit"), announce: !has("--autostart"), diag });
    if let Err(e) = event_loop.run_app(&mut app) {
        log!("event loop stopped: {e}");
    }
    if owns_socket {
        ipc::cleanup();
    }
}

struct TitleEntry {
    zone: u64,
    text: String,
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    opts: Opts,
    started: bool,
    cfg: Config,
    cfg_path: PathBuf,
    /// No config existed yet (first run).
    fresh: bool,
    monitors: Vec<Monitor>,
    handles: Vec<MonitorHandle>,
    backend: Backend,
    state: State,
    tray: Option<ksni::blocking::Handle<tray::DeskTray>>,
    can_pick_color: bool,
    worker: Option<mpsc::Sender<Work>>,
    /// Numbers the painter's jobs; only the newest result counts.
    serial: u64,
    /// DeskChalk's pictures are on the desktop (or on their way there).
    showing: bool,
    save_at: Option<Instant>,
    paint_at: Option<Instant>,
    poll_at: Option<Instant>,
    exit_at: Option<Instant>,
    watch_mtime: Option<SystemTime>,
    warned: bool,
    exiting: Option<Exit>,

    // Edit mode.
    editing: bool,
    wins: Vec<EditWin>,
    editor: Editor,
    title_fonts: Option<Fonts>,
    ui_fonts: Option<Fonts>,
    mods: ModifiersState,
    mouse: Pt,
    show_hint: bool,
    title: Option<TitleEntry>,
    popup: Option<(usize, Popup, Target)>,
    /// Edit window and mouse position the dots are drifting away from.
    repel: Option<(usize, Pt)>,
    repel_strength: f32,
    repel_target: f32,
    anim_at: Option<Instant>,
    anim_tick: Option<Instant>,
    last_press: Option<(Instant, usize, Pt)>,
    cursor: Option<(usize, CursorIcon)>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>, opts: Opts) -> App {
        let cfg_path = paths::config_dir().join("config.json");
        let fresh = !cfg_path.exists();
        let mut cfg = config::load(&cfg_path);
        // A config from Windows names a Windows font; use the Linux default instead.
        cfg.font = fonts::choice_key(&cfg.font).to_string();
        let state = load_state();
        let showing = !state.pictures.is_empty();
        let serial = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64);
        App {
            proxy,
            opts,
            started: false,
            cfg,
            cfg_path,
            fresh,
            monitors: Vec::new(),
            handles: Vec::new(),
            backend: Backend::detect(),
            state,
            tray: None,
            can_pick_color: false,
            worker: None,
            serial,
            showing,
            save_at: None,
            paint_at: None,
            poll_at: None,
            exit_at: None,
            watch_mtime: None,
            warned: false,
            exiting: None,
            editing: false,
            wins: Vec::new(),
            editor: Editor::default(),
            title_fonts: None,
            ui_fonts: None,
            mods: ModifiersState::empty(),
            mouse: Pt::default(),
            show_hint: true,
            title: None,
            popup: None,
            repel: None,
            repel_strength: 0.0,
            repel_target: 0.0,
            anim_at: None,
            anim_tick: None,
            last_press: None,
            cursor: None,
        }
    }

    fn start(&mut self, el: &ActiveEventLoop) {
        if self.opts.diag {
            self.diag(el);
            el.exit();
            return;
        }
        log!("DeskChalk {VERSION}: {} ({}), wallpaper via {}", desktop_name(), std::env::var("XDG_SESSION_TYPE").unwrap_or_default(), self.backend.name());
        self.read_monitors(el);
        self.watch_mtime = self.backend.watch_file().and_then(|f| mtime(&f));
        self.sync_original();
        self.can_pick_color = notify::can_pick_color();
        self.start_worker();
        self.start_tray();
        if self.fresh && paths::installed() {
            paths::set_autostart(true);
        }
        if !self.backend.supported() {
            notify::notify(
                "DeskChalk can't show boxes here yet",
                &format!("DeskChalk paints boxes into the wallpaper, and changing it on {} isn't supported yet. It works on {SUPPORTED}.", desktop_name()),
            );
        }
        if self.wants_paint() {
            self.paint_at = Some(Instant::now());
        } else {
            self.unpaint();
        }
        if !self.cfg.welcomed {
            self.cfg.welcomed = true;
            self.save_soon();
            self.announce("DeskChalk is running");
        } else if self.opts.announce && !self.opts.edit {
            self.announce("DeskChalk is running");
        }
        if self.opts.edit {
            self.enter_edit(el);
        }
        self.poll_at = Some(Instant::now() + POLL);
    }

    fn announce(&self, title: &str) {
        let how = if self.tray.is_some() {
            "Click the pencil in the system tray to draw boxes on your desktop."
        } else {
            "Open DeskChalk again to draw boxes on your desktop. (Your desktop shows no tray icons; on GNOME, the AppIndicator extension adds them.)"
        };
        notify::notify(title, how);
    }

    fn diag(&self, el: &ActiveEventLoop) {
        println!("DeskChalk {VERSION}");
        println!("desktop: {} ({})", desktop_name(), std::env::var("XDG_SESSION_TYPE").unwrap_or_default());
        println!("wallpaper: {}", self.backend.name());
        let mons: Vec<Monitor> = monitors::read(el, self.cfg.grid.spacing).into_iter().map(|(m, _)| m).collect();
        for m in &mons {
            println!("monitor {}: {}x{} at {},{}, scale {}{}", m.key, m.w, m.h, m.x, m.y, m.scale, if m.primary { ", primary" } else { "" });
        }
        match self.backend.snapshot(&mons) {
            Some(s) => {
                for (k, b) in &s.bases {
                    println!("current wallpaper{}: {:?}", if k.is_empty() { String::new() } else { format!(" on {k}") }, b);
                }
            }
            None => println!("current wallpaper: (can't read it)"),
        }
        if let Some(o) = &self.state.original {
            for (k, b) in &o.bases {
                println!("remembered wallpaper{}: {:?}", if k.is_empty() { String::new() } else { format!(" on {k}") }, b);
            }
        }
        println!("tray icons shown: {}", notify::tray_available());
        println!("config: {}", self.cfg_path.display());
        println!("pictures: {}", paths::data_dir().display());
        println!("start at login: {}", paths::autostart_enabled());
        println!("installed (.deb): {}", paths::installed());
    }

    // ---- screens, wallpaper, painting --------------------------------------------------

    /// Re-reads the screens; true if any changed.
    fn read_monitors(&mut self, el: &ActiveEventLoop) -> bool {
        let list = monitors::read(el, self.cfg.grid.spacing);
        let mons: Vec<Monitor> = list.iter().map(|(m, _)| m.clone()).collect();
        let changed = mons.len() != self.monitors.len() || mons.iter().zip(&self.monitors).any(|(a, b)| !a.same_geometry(b));
        self.monitors = mons;
        self.handles = list.into_iter().map(|(_, h)| h).collect();
        changed
    }

    fn regrid(&mut self) {
        let sp = self.cfg.grid.spacing;
        for m in &mut self.monitors {
            m.grid = monitors::grid_for(m.w, m.h, m.scale, sp);
        }
        for w in &mut self.wins {
            w.mon.grid = monitors::grid_for(w.mon.w, w.mon.h, w.mon.scale, sp);
        }
    }

    /// Remembers the wallpaper the user has (from before DeskChalk painted, or picked since).
    /// True if it changed.
    fn sync_original(&mut self) -> bool {
        let Some(now) = self.backend.snapshot(&self.monitors) else { return false };
        let changed = match &mut self.state.original {
            // Showing DeskChalk's picture but nothing remembered: nothing to learn from.
            None if now.all_ours() => false,
            None => {
                self.state.original = Some(now);
                true
            }
            Some(orig) => orig.take_user_changes(&now),
        };
        if changed {
            log!("remembered the wallpaper");
            self.save_state();
        }
        changed
    }

    fn save_state(&self) {
        let _ = std::fs::create_dir_all(paths::data_dir());
        if let Ok(json) = serde_json::to_vec_pretty(&self.state) {
            let tmp = state_path().with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, state_path());
            }
        }
    }

    fn wants_paint(&self) -> bool {
        self.backend.supported() && ((self.cfg.show_zones && !self.cfg.zones.is_empty()) || self.cfg.grid.show_on_desktop)
    }

    fn start_worker(&mut self) {
        let (tx, rx) = mpsc::channel::<Work>();
        let backend = self.backend.clone();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            while let Ok(mut work) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    work = newer;
                }
                match work {
                    Work::Paint(job) => {
                        let result = bake::bake(&job).and_then(|p| backend.apply(&p, &job.monitors).map(|_| p));
                        let _ = proxy.send_event(UserEvent::Painted { serial: job.serial, result });
                    }
                    Work::Restore(snap, serial) => {
                        if let Err(e) = backend.restore(&snap) {
                            log!("putting the wallpaper back failed: {e}");
                        }
                        let _ = proxy.send_event(UserEvent::Restored { serial });
                    }
                }
            }
        });
        self.worker = Some(tx);
    }

    fn paint(&mut self) {
        self.paint_at = None;
        if self.editing || self.exiting.is_some() {
            return;
        }
        if !self.wants_paint() {
            self.unpaint();
            return;
        }
        let Some(tx) = &self.worker else { return };
        self.serial += 1;
        let job = bake::Job {
            serial: self.serial,
            cfg: self.cfg.clone(),
            monitors: self.monitors.clone(),
            original: self.state.original.clone().unwrap_or_default(),
            per_monitor: self.backend.per_monitor(),
            dir: paths::data_dir(),
        };
        if tx.send(Work::Paint(job)).is_ok() {
            self.showing = true;
        }
    }

    /// Puts the original wallpaper back (nothing to show, or DeskChalk is stopping).
    /// Returns false if there was nothing to put back.
    fn unpaint(&mut self) -> bool {
        if !self.showing {
            return false;
        }
        let Some(tx) = &self.worker else { return false };
        let Some(orig) = self.state.original.clone() else {
            self.showing = false;
            return false;
        };
        self.serial += 1;
        tx.send(Work::Restore(orig, self.serial)).is_ok()
    }

    fn painted(&mut self, serial: u64, result: Result<Pictures, String>) {
        match result {
            Ok(p) if serial == self.serial => {
                let files: Vec<PathBuf> = p.spanned.into_iter().chain(p.per_monitor.into_iter().map(|(_, f)| f)).collect();
                bake::clean(&paths::data_dir(), &files);
                self.state.pictures = files;
                self.save_state();
                self.warned = false;
            }
            Ok(_) => {}
            Err(e) => {
                log!("painting the wallpaper failed: {e}");
                if !self.warned {
                    self.warned = true;
                    notify::notify("DeskChalk couldn't change the wallpaper", &e);
                }
            }
        }
    }

    fn restored(&mut self, el: &ActiveEventLoop, serial: u64) {
        if serial != self.serial {
            return;
        }
        bake::clean(&paths::data_dir(), &[]);
        self.state.pictures.clear();
        self.showing = false;
        self.save_state();
        if self.exiting.is_some() {
            self.finish_exit(el);
        }
    }

    /// Every few seconds: new or changed screens, and wallpapers the user picked.
    fn poll(&mut self, el: &ActiveEventLoop) {
        if self.exiting.is_some() {
            return;
        }
        if self.read_monitors(el) {
            log!("screens changed");
            if self.editing {
                self.exit_edit();
            }
            self.paint_at = Some(Instant::now() + PAINT_DELAY);
        }
        if let Some(f) = self.backend.watch_file() {
            let m = mtime(&f);
            if m != self.watch_mtime {
                self.watch_mtime = m;
                if self.sync_original() && self.wants_paint() {
                    self.paint_at = Some(Instant::now() + PAINT_DELAY);
                }
            }
        }
    }

    fn save_soon(&mut self) {
        self.save_at = Some(Instant::now() + SAVE_DELAY);
    }

    fn save_now(&mut self) {
        self.save_at = None;
        if let Err(e) = config::save(&self.cfg_path, &self.cfg) {
            log!("saving settings failed: {e}");
        }
    }

    /// Something visible changed: redraw the edit windows, or paint the wallpaper soon.
    fn changed(&mut self) {
        if self.editing {
            for w in &mut self.wins {
                w.invalidate(None);
            }
        } else {
            self.paint_at = Some(Instant::now() + PAINT_DELAY);
        }
        self.save_soon();
        self.update_tray();
    }

    // ---- tray ----------------------------------------------------------------------------

    fn tray_view(&self) -> tray::View {
        tray::View {
            editing: self.editing,
            show_zones: self.cfg.show_zones,
            mix_colors: self.cfg.mix_colors,
            snap: self.cfg.snap,
            grid: self.cfg.grid.clone(),
            defaults: self.cfg.defaults.clone(),
            font: self.cfg.font.clone(),
            autostart: paths::autostart_enabled(),
            installed: paths::installed(),
            can_pick_color: self.can_pick_color,
            any_zones: !self.cfg.zones.is_empty(),
        }
    }

    fn start_tray(&mut self) {
        use ksni::blocking::TrayMethods;
        match tray::DeskTray::new(self.tray_view(), self.proxy.clone()).spawn() {
            Ok(h) => self.tray = Some(h),
            Err(e) => log!("no tray icon: {e}"),
        }
    }

    fn update_tray(&self) {
        if let Some(h) = &self.tray {
            let view = self.tray_view();
            h.update(move |t: &mut tray::DeskTray| t.view = view);
        }
    }

    // ---- commands --------------------------------------------------------------------------

    fn request(&mut self, el: &ActiveEventLoop, r: ipc::Request) {
        match r {
            ipc::Request::Show => {
                if self.tray.is_some() {
                    self.announce("DeskChalk is already running");
                } else {
                    // Without a tray icon, opening DeskChalk again is the way to edit.
                    self.enter_edit(el);
                }
            }
            ipc::Request::Edit => self.enter_edit(el),
            ipc::Request::Quit => self.begin_exit(el, Exit::Quit),
        }
    }

    fn command(&mut self, el: &ActiveEventLoop, id: u32, target: Target) {
        let zone = match target {
            Target::Zone(z) => Some(z),
            _ => None,
        };
        let in_range = |base: u32, len: usize| (base..base + len as u32).contains(&id);
        match id {
            cmd::EDIT => {
                if self.editing {
                    self.exit_edit();
                } else {
                    self.enter_edit(el);
                }
            }
            cmd::SHOW_ZONES => {
                self.cfg.show_zones = !self.cfg.show_zones;
                self.changed();
            }
            cmd::MIX_COLORS => {
                self.cfg.mix_colors = !self.cfg.mix_colors;
                self.changed();
            }
            cmd::GRID_SHOW => {
                self.cfg.grid.show_on_desktop = !self.cfg.grid.show_on_desktop;
                self.changed();
            }
            cmd::SNAP => {
                self.cfg.snap = !self.cfg.snap;
                self.save_soon();
                self.update_tray();
            }
            cmd::GRID_REPEL => {
                self.cfg.grid.repel = !self.cfg.grid.repel;
                if !self.cfg.grid.repel {
                    self.release_repel();
                }
                self.save_soon();
                self.update_tray();
            }
            cmd::AUTOSTART => {
                paths::set_autostart(!paths::autostart_enabled());
                self.update_tray();
            }
            cmd::SUPPORT => util::open_url(SUPPORT_URL),
            cmd::ABOUT => util::open_url(REPO_URL),
            cmd::UNINSTALL => self.ask_remove(),
            cmd::EXIT => self.begin_exit(el, Exit::Quit),
            cmd::UNDO => {
                if self.editor.undo(&mut self.cfg.zones) {
                    self.changed();
                }
            }
            cmd::REDO => {
                if self.editor.redo(&mut self.cfg.zones) {
                    self.changed();
                }
            }
            cmd::DELETE_ALL => {
                if let Target::Monitor(key) = &target {
                    // Undoable with Ctrl+Z, so no question first.
                    self.editor.checkpoint(&self.cfg.zones);
                    self.cfg.zones.retain(|z| &z.monitor != key);
                    self.editor.selected = None;
                    self.changed();
                }
            }
            cmd::APPLY_ALL => {
                self.editor.checkpoint(&self.cfg.zones);
                let style = self.cfg.defaults.clone();
                for z in &mut self.cfg.zones {
                    z.style = style.clone();
                }
                self.changed();
            }
            cmd::DOT_CUSTOM => self.pick_color(self.cfg.grid.color, Slot::Dots, None),
            cmd::Z_RENAME => {
                if let Some(z) = zone {
                    self.begin_title(z);
                }
            }
            cmd::Z_DUPLICATE => {
                if let Some(z) = zone {
                    self.editor.selected = Some(z);
                    let key = self.cfg.zones.iter().find(|x| x.id == z).map(|x| x.monitor.clone()).unwrap_or_default();
                    if let Some(mon) = self.edit_monitor(&key) {
                        if self.editor.duplicate_selected(&mut self.cfg.zones, &mon.ctx(), self.cfg.snap).is_some() {
                            self.changed();
                        }
                    }
                }
            }
            cmd::Z_FRONT | cmd::Z_BACK => {
                if let Some(pos) = zone.and_then(|z| self.cfg.zones.iter().position(|x| x.id == z)) {
                    self.editor.checkpoint(&self.cfg.zones);
                    let z = self.cfg.zones.remove(pos);
                    if id == cmd::Z_FRONT {
                        self.cfg.zones.push(z);
                    } else {
                        self.cfg.zones.insert(0, z);
                    }
                    self.changed();
                }
            }
            cmd::Z_DELETE => {
                if let Some(z) = zone {
                    self.editor.selected = Some(z);
                    if self.editor.delete_selected(&mut self.cfg.zones) {
                        self.changed();
                    }
                }
            }
            cmd::Z_MAKE_DEFAULT => {
                if let Some(style) = zone.and_then(|z| self.cfg.zones.iter().find(|x| x.id == z)).map(|z| z.style.clone()) {
                    self.cfg.defaults = style;
                    self.save_soon();
                    self.update_tray();
                }
            }
            cmd::Z_RESET_STYLE => {
                if let Some(z) = zone {
                    self.editor.checkpoint(&self.cfg.zones);
                    let style = self.cfg.defaults.clone();
                    if let Some(zz) = self.cfg.zones.iter_mut().find(|x| x.id == z) {
                        zz.style = style;
                    }
                    self.changed();
                }
            }
            _ if in_range(cmd::DOT_COLOR, model::DOT_COLORS.len()) => {
                self.cfg.grid.color = model::DOT_COLORS[(id - cmd::DOT_COLOR) as usize].1;
                self.changed();
            }
            _ if in_range(cmd::GRID_SIZE, model::DOT_SIZES.len()) => {
                self.cfg.grid.size = model::DOT_SIZES[(id - cmd::GRID_SIZE) as usize].1;
                self.changed();
            }
            _ if in_range(cmd::GRID_OPACITY, model::DOT_OPACITY.len()) => {
                self.cfg.grid.opacity = model::DOT_OPACITY[(id - cmd::GRID_OPACITY) as usize].1;
                self.changed();
            }
            _ if in_range(cmd::GRID_SPACING, model::GRID_SPACINGS.len()) => {
                self.cfg.grid.spacing = model::GRID_SPACINGS[(id - cmd::GRID_SPACING) as usize].1;
                self.regrid();
                self.changed();
            }
            _ if in_range(cmd::FONT, fonts::FONT_CHOICES.len()) => {
                self.cfg.font = fonts::FONT_CHOICES[(id - cmd::FONT) as usize].key.into();
                if self.editing {
                    self.title_fonts = fonts::load(&self.cfg.font);
                }
                self.changed();
            }
            _ if in_range(cmd::STYLE_ZONE, model::STYLE_SPAN as usize) => {
                let Some(z) = zone else { return };
                let current = self.cfg.zones.iter().find(|x| x.id == z).map(|x| x.style.clone()).unwrap_or_default();
                match StyleCmd::decode(id - cmd::STYLE_ZONE) {
                    Some(StyleCmd::CustomColor) => self.pick_color(current.color, Slot::Box, Some(z)),
                    Some(StyleCmd::CustomBorderColor) => self.pick_color(current.line_color(), Slot::Border, Some(z)),
                    Some(sc) => {
                        self.editor.checkpoint(&self.cfg.zones);
                        if let Some(zz) = self.cfg.zones.iter_mut().find(|x| x.id == z) {
                            zz.style.apply(sc);
                        }
                        self.changed();
                    }
                    None => {}
                }
            }
            _ if in_range(cmd::STYLE_DEFAULT, model::STYLE_SPAN as usize) => match StyleCmd::decode(id - cmd::STYLE_DEFAULT) {
                Some(StyleCmd::CustomColor) => self.pick_color(self.cfg.defaults.color, Slot::Box, None),
                Some(StyleCmd::CustomBorderColor) => self.pick_color(self.cfg.defaults.line_color(), Slot::Border, None),
                Some(sc) => {
                    self.cfg.defaults.apply(sc);
                    self.save_soon();
                    self.update_tray();
                }
                None => {}
            },
            _ => {}
        }
    }

    /// Opens a color picker (zenity or kdialog) without blocking; the choice comes back as
    /// a user event.
    fn pick_color(&self, initial: Rgb, slot: Slot, zone: Option<u64>) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            if let Some(color) = notify::choose_color(initial) {
                let _ = proxy.send_event(UserEvent::Color { color, slot, zone });
            }
        });
    }

    fn apply_color(&mut self, c: Rgb, slot: Slot, zone: Option<u64>) {
        let set = |style: &mut model::ZoneStyle| match slot {
            Slot::Box => style.color = c,
            Slot::Border => style.border_color = Some(c),
            Slot::Dots => {}
        };
        match (slot, zone) {
            (Slot::Dots, _) => {
                self.cfg.grid.color = c;
                self.changed();
            }
            (_, Some(z)) => {
                self.editor.checkpoint(&self.cfg.zones);
                if let Some(zz) = self.cfg.zones.iter_mut().find(|x| x.id == z) {
                    set(&mut zz.style);
                }
                self.changed();
            }
            (_, None) => {
                set(&mut self.cfg.defaults);
                self.save_soon();
                self.update_tray();
            }
        }
    }

    fn ask_remove(&self) {
        let installed = paths::installed();
        let q = if installed {
            "Uninstall DeskChalk?\n\nThis puts your wallpaper back, deletes all of your boxes and settings, and removes DeskChalk (your password may be asked for). Your desktop icons are not touched."
        } else {
            "Remove DeskChalk?\n\nThis puts your wallpaper back and deletes all of your boxes and settings, then closes DeskChalk. Delete the DeskChalk file yourself afterwards. Your desktop icons are not touched."
        };
        let q = q.to_string();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // Without a dialog program there's no way to ask; the menu choice itself is the answer.
            if notify::ask(&q).unwrap_or(true) {
                let _ = proxy.send_event(UserEvent::RemoveConfirmed);
            }
        });
    }

    fn begin_exit(&mut self, el: &ActiveEventLoop, how: Exit) {
        if self.exiting.is_some() {
            return;
        }
        if self.editing {
            self.exit_edit();
        }
        if how == Exit::Quit {
            self.save_now();
        }
        self.exiting = Some(how);
        self.paint_at = None;
        if self.unpaint() {
            // Wait for the wallpaper to be put back (Restored), but not forever.
            self.exit_at = Some(Instant::now() + EXIT_WAIT);
        } else {
            self.finish_exit(el);
        }
    }

    fn finish_exit(&mut self, el: &ActiveEventLoop) {
        if self.exiting == Some(Exit::Remove) {
            paths::remove_user_data();
            if paths::installed() {
                if util::have("pkexec") && util::have("apt-get") {
                    util::spawn("pkexec", &["apt-get", "remove", "-y", "deskchalk"]);
                } else {
                    notify::notify_now("DeskChalk removed your boxes and settings", "To finish, remove the deskchalk package (for example: sudo apt remove deskchalk).");
                }
            } else {
                notify::notify_now("DeskChalk removed your boxes and settings", "Delete the DeskChalk file to finish.");
            }
        }
        if let Some(t) = self.tray.take() {
            t.shutdown().wait();
        }
        el.exit();
    }

    // ---- edit mode ---------------------------------------------------------------------------

    fn enter_edit(&mut self, el: &ActiveEventLoop) {
        if self.editing || self.exiting.is_some() {
            return;
        }
        self.read_monitors(el);
        self.title_fonts = fonts::load(&self.cfg.font);
        self.ui_fonts = fonts::load_ui();
        let original = self.state.original.clone().unwrap_or_default();
        let mut cache = HashMap::new();
        for (mon, handle) in self.monitors.iter().zip(&self.handles) {
            let Some(wall) = edit::wallpaper_for(original.base_for(&mon.key), mon, &mut cache) else { continue };
            if let Some(w) = EditWin::new(el, handle, mon.clone(), wall) {
                self.wins.push(w);
            }
        }
        if self.wins.is_empty() {
            notify::notify("DeskChalk couldn't open its drawing windows", "Start it with --log and check deskchalk.log in ~/.config/deskchalk.");
            return;
        }
        self.editing = true;
        self.show_hint = true;
        self.editor.selected = None;
        self.paint_at = None;
        self.cursor = None;
        if let Some(w) = self.wins.first() {
            w.window.focus_window();
        }
        self.update_tray();
    }

    fn exit_edit(&mut self) {
        if !self.editing {
            return;
        }
        self.finish_title(true);
        self.editor.cancel();
        self.editor.selected = None;
        self.popup = None;
        self.repel = None;
        self.repel_strength = 0.0;
        self.anim_at = None;
        self.anim_tick = None;
        self.wins.clear();
        self.editing = false;
        self.title_fonts = None;
        self.ui_fonts = None;
        self.paint_at = Some(Instant::now());
        self.save_soon();
        self.update_tray();
    }

    fn win_index(&self, id: WindowId) -> Option<usize> {
        self.wins.iter().position(|w| w.window.id() == id)
    }

    /// The screen as edited (sized like its edit window), or as known otherwise.
    fn edit_monitor(&self, key: &str) -> Option<Monitor> {
        self.wins.iter().find(|w| w.mon.key == key).map(|w| w.mon.clone()).or_else(|| self.monitors.iter().find(|m| m.key == key).cloned())
    }

    fn invalidate(&mut self, i: usize, region: Option<Rect>) {
        if let Some(w) = self.wins.get_mut(i) {
            w.invalidate(region);
        }
    }

    fn invalidate_all(&mut self) {
        for w in &mut self.wins {
            w.invalidate(None);
        }
    }

    fn snapping(&self) -> bool {
        self.cfg.snap != self.mods.shift_key()
    }

    fn redraw(&mut self, i: usize) {
        let snap = self.snapping();
        let at = self.repel.filter(|(j, _)| *j == i).map(|(_, p)| (p, self.repel_strength));
        let App { wins, cfg, editor, title_fonts, ui_fonts, show_hint, title, popup, mouse, .. } = self;
        let Some(w) = wins.get_mut(i) else { return };
        let mon = w.mon.clone();
        let view = View { cfg, editor, title_fonts: title_fonts.as_ref(), ui_fonts: ui_fonts.as_ref(), hint: *show_hint, snap, mouse: *mouse };
        let entry = title.as_ref();
        let menu = popup.as_ref().filter(|(pi, _, _)| *pi == i).map(|(_, p, _)| p);
        w.redraw(|pm, origin| {
            render::draw(pm, &edit_scene(&view, &mon, origin, at));
            if let (Some(t), Some(f)) = (entry, view.ui_fonts) {
                draw_title_entry(pm, origin, &mon, view.cfg, t, f);
            }
            if let (Some(p), Some(f)) = (menu, view.ui_fonts) {
                p.draw(pm, origin, Painter { swap_rb: true }, f);
            }
        });
    }

    fn mouse_move(&mut self, i: usize, p: Pt) {
        if let Some((pi, popup, _)) = self.popup.as_mut() {
            if *pi == i {
                if let (Some(f), Some(w)) = (self.ui_fonts.as_ref(), self.wins.get_mut(i)) {
                    let before = popup.bounds();
                    if popup.hover(p, w.mon.full(), f) {
                        w.invalidate(Some(before.union(&popup.bounds())));
                    }
                }
            }
            self.mouse = p;
            return;
        }
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        if self.editor.drag.is_none() {
            self.mouse = p;
            self.update_cursor(i, &mon, p);
            if self.cfg.grid.repel {
                self.set_repel(i, p);
            }
            return;
        }
        let before = self.drag_area(&mon);
        self.mouse = p;
        if self.editor.update(&mon.ctx(), p, self.snapping()) {
            let after = self.drag_area(&mon);
            let region = match (before, after) {
                (Some(a), Some(b)) => Some(a.union(&b)),
                _ => None,
            };
            self.invalidate(i, region);
        }
    }

    /// What a drag in progress covers: the box (with handles and a title on its line) and
    /// the size label next to the mouse.
    fn drag_area(&self, mon: &Monitor) -> Option<Rect> {
        let d = self.editor.drag.as_ref()?;
        let sc = mon.scale;
        let label = Rect::new(self.mouse.x - 4.0 * sc, self.mouse.y - 4.0 * sc, 280.0 * sc, 64.0 * sc);
        Some(d.rect.inset(-72.0 * sc).union(&label))
    }

    fn update_cursor(&mut self, i: usize, mon: &Monitor, p: Pt) {
        let icon = match Editor::hit_test(&self.cfg.zones, &mon.ctx(), p) {
            Hit::None => CursorIcon::Crosshair,
            Hit::Body(_) => CursorIcon::Move,
            Hit::Handle(_, h) => match h {
                Handle::N | Handle::S => CursorIcon::NsResize,
                Handle::E | Handle::W => CursorIcon::EwResize,
                Handle::NW | Handle::SE => CursorIcon::NwseResize,
                Handle::NE | Handle::SW => CursorIcon::NeswResize,
            },
        };
        if self.cursor != Some((i, icon)) {
            if let Some(w) = self.wins.get(i) {
                w.window.set_cursor(icon);
            }
            self.cursor = Some((i, icon));
        }
    }

    fn press(&mut self, el: &ActiveEventLoop, i: usize, p: Pt) {
        if let Some((pi, mut menu, target)) = self.popup.take() {
            let bounds = menu.bounds();
            let click = if pi == i { menu.click(p) } else { Click::Outside };
            match click {
                Click::Outside => self.invalidate(pi, Some(bounds)),
                Click::Inside => self.popup = Some((pi, menu, target)),
                Click::Chosen(id) => {
                    // Settings keep the menu open (like the Windows menus); actions close it.
                    let keep = sticky(id);
                    if keep {
                        self.popup = Some((pi, menu, target.clone()));
                    } else {
                        self.invalidate(pi, Some(bounds));
                    }
                    self.command(el, id, target);
                    if keep {
                        self.refresh_popup();
                    }
                }
            }
            return;
        }
        if self.title.is_some() {
            self.finish_title(true);
        }
        if self.show_hint {
            self.show_hint = false;
            self.invalidate_all();
        }
        if self.repel.take().is_some() {
            self.repel_strength = 0.0;
            self.invalidate(i, None);
        }
        let now = Instant::now();
        if let Some((t, j, q)) = self.last_press {
            if j == i && now - t < DOUBLE_CLICK && (q.x - p.x).abs() < 6.0 && (q.y - p.y).abs() < 6.0 {
                self.last_press = None;
                self.double_click(i, p);
                return;
            }
        }
        self.last_press = Some((now, i, p));
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        self.mouse = p;
        self.editor.begin(&self.cfg.zones, &mon.ctx(), p);
        self.invalidate(i, None);
    }

    fn release(&mut self, i: usize, p: Pt) {
        if self.editor.drag.is_none() {
            return;
        }
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        let ctx = mon.ctx();
        self.mouse = p;
        self.editor.update(&ctx, p, self.snapping());
        match self.editor.finish(&mut self.cfg.zones, &ctx, &self.cfg.defaults) {
            Outcome::Created(id) => {
                self.save_soon();
                self.update_tray();
                // New boxes ask for a name right away (Esc leaves it untitled).
                self.begin_title(id);
            }
            Outcome::Changed => {
                self.save_soon();
                self.invalidate_all();
            }
            Outcome::Redraw => self.invalidate_all(),
            Outcome::None => {}
        }
    }

    fn double_click(&mut self, i: usize, p: Pt) {
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        if let Hit::Body(id) | Hit::Handle(id, _) = Editor::hit_test(&self.cfg.zones, &mon.ctx(), p) {
            self.begin_title(id);
        }
    }

    fn context_menu(&mut self, i: usize, p: Pt) {
        self.finish_title(true);
        self.editor.cancel();
        if let Some((pi, old, _)) = self.popup.take() {
            self.invalidate(pi, Some(old.bounds()));
        }
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        let (items, target) = match Editor::hit_test(&self.cfg.zones, &mon.ctx(), p) {
            Hit::Body(id) | Hit::Handle(id, _) => {
                self.editor.selected = Some(id);
                (self.zone_menu(id), Target::Zone(id))
            }
            Hit::None => {
                self.editor.selected = None;
                (self.canvas_menu(&mon.key), Target::Monitor(mon.key.clone()))
            }
        };
        let Some(fonts) = self.ui_fonts.as_ref() else { return };
        let menu = Popup::open(items, p, mon.full(), mon.scale, fonts);
        self.popup = Some((i, menu, target));
        self.invalidate(i, None);
    }

    fn style_menus(&self, m: &mut popup::Menu, style: &model::ZoneStyle, base: u32) {
        for (title, entries) in style.menu().sections {
            let mut sub = popup::Menu::default();
            for e in entries {
                if e.separator {
                    sub.sep();
                }
                if let Some(h) = e.heading {
                    sub.heading(h);
                }
                if StyleCmd::decode(e.offset).is_some_and(|c| c.opens_dialog()) && !self.can_pick_color {
                    continue;
                }
                match e.color {
                    Some(c) => sub.color(base + e.offset, &e.label, c, e.checked),
                    None => sub.radio(base + e.offset, &e.label, e.checked),
                }
            }
            m.sub(title, sub);
        }
    }

    fn zone_menu(&self, id: u64) -> Vec<popup::Item> {
        let Some(z) = self.cfg.zones.iter().find(|z| z.id == id) else { return Vec::new() };
        let mut m = popup::Menu::default();
        m.item(cmd::Z_RENAME, "Rename…\tF2");
        m.sep();
        self.style_menus(&mut m, &z.style, cmd::STYLE_ZONE);
        m.sep();
        m.item(cmd::Z_DUPLICATE, "Duplicate\tCtrl+D");
        m.item(cmd::Z_FRONT, "Bring to front");
        m.item(cmd::Z_BACK, "Send to back");
        m.sep();
        m.item(cmd::Z_MAKE_DEFAULT, "Use this style for new boxes");
        m.item(cmd::Z_RESET_STYLE, "Reset to the new-box style");
        m.sep();
        m.item(cmd::Z_DELETE, "Delete\tDel");
        m.items
    }

    fn canvas_menu(&self, key: &str) -> Vec<popup::Item> {
        let any_here = self.cfg.zones.iter().any(|z| z.monitor == key);
        let mut m = popup::Menu::default();
        m.item(cmd::EDIT, "Done editing\tEsc");
        m.sep();
        m.check(cmd::SNAP, "Snap to the grid (hold Shift to toggle)", self.cfg.snap);
        m.check(cmd::GRID_SHOW, "Keep grid dots on the wallpaper", self.cfg.grid.show_on_desktop);
        m.check(cmd::GRID_REPEL, "Dots move away from the mouse", self.cfg.grid.repel);
        m.sep();
        m.item_enabled(cmd::UNDO, "Undo\tCtrl+Z", self.editor.can_undo());
        m.item_enabled(cmd::REDO, "Redo\tCtrl+Y", self.editor.can_redo());
        m.sep();
        m.item_enabled(cmd::DELETE_ALL, "Delete all boxes on this screen", any_here);
        m.items
    }

    /// New check marks for the open menu after a setting changed.
    fn refresh_popup(&mut self) {
        let Some((i, _, target)) = self.popup.as_ref() else { return };
        let (i, target) = (*i, target.clone());
        let items = match &target {
            Target::Zone(id) => self.zone_menu(*id),
            Target::Monitor(key) => self.canvas_menu(key),
            Target::Global => Vec::new(),
        };
        if items.is_empty() {
            if let Some((_, old, _)) = self.popup.take() {
                self.invalidate(i, Some(old.bounds()));
            }
            return;
        }
        let area = self.wins.get(i).map(|w| w.mon.full());
        if let (Some((_, menu, _)), Some(f), Some(area)) = (self.popup.as_mut(), self.ui_fonts.as_ref(), area) {
            let before = menu.bounds();
            menu.refresh(items, area, f);
            let region = before.union(&menu.bounds());
            self.invalidate(i, Some(region));
        }
    }

    fn begin_title(&mut self, id: u64) {
        self.finish_title(true);
        let Some(z) = self.cfg.zones.iter().find(|z| z.id == id) else { return };
        self.title = Some(TitleEntry { zone: id, text: z.title.clone() });
        self.editor.selected = Some(id);
        self.invalidate_all();
    }

    fn finish_title(&mut self, commit: bool) {
        let Some(t) = self.title.take() else { return };
        let text = t.text.trim().to_string();
        if commit {
            if let Some(idx) = self.cfg.zones.iter().position(|z| z.id == t.zone) {
                if self.cfg.zones[idx].title != text {
                    self.editor.checkpoint(&self.cfg.zones);
                    self.cfg.zones[idx].title = text;
                    self.save_soon();
                }
            }
        }
        self.invalidate_all();
    }

    fn key(&mut self, i: usize, ev: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let shift = self.mods.shift_key();
        if let Some(t) = self.title.as_mut() {
            match &ev.logical_key {
                Key::Named(NamedKey::Enter) => self.finish_title(true),
                Key::Named(NamedKey::Escape) => self.finish_title(false),
                Key::Named(NamedKey::Backspace) => {
                    t.text.pop();
                    self.invalidate_all();
                }
                _ => {
                    if let Some(text) = ev.text.as_ref().filter(|_| !ctrl) {
                        for c in text.chars().filter(|c| !c.is_control()) {
                            if t.text.chars().count() < 120 {
                                t.text.push(c);
                            }
                        }
                        self.invalidate_all();
                    }
                }
            }
            return;
        }
        if self.popup.is_some() {
            if ev.logical_key == Key::Named(NamedKey::Escape) {
                if let Some((pi, old, _)) = self.popup.take() {
                    self.invalidate(pi, Some(old.bounds()));
                }
            }
            return;
        }
        let Some(mon) = self.wins.get(i).map(|w| w.mon.clone()) else { return };
        let snap = self.snapping();
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.editor.cancel() {
                    self.invalidate_all();
                } else {
                    self.exit_edit();
                }
            }
            Key::Named(NamedKey::Enter | NamedKey::F2) => {
                if let Some(id) = self.editor.selected {
                    self.begin_title(id);
                }
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                if self.editor.delete_selected(&mut self.cfg.zones) {
                    self.changed();
                }
            }
            Key::Named(NamedKey::Tab) => {
                if self.editor.select_next(&self.cfg.zones, &mon.ctx()) {
                    self.invalidate_all();
                }
            }
            Key::Named(k @ (NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::ArrowUp | NamedKey::ArrowDown)) => {
                let (dx, dy) = match k {
                    NamedKey::ArrowLeft => (-1, 0),
                    NamedKey::ArrowRight => (1, 0),
                    NamedKey::ArrowUp => (0, -1),
                    _ => (0, 1),
                };
                if self.editor.nudge(&mut self.cfg.zones, &mon.ctx(), dx, dy, snap, shift) {
                    self.changed();
                }
            }
            Key::Character(c) if ctrl => match c.to_lowercase().as_str() {
                "z" => {
                    let done = if shift { self.editor.redo(&mut self.cfg.zones) } else { self.editor.undo(&mut self.cfg.zones) };
                    if done {
                        self.changed();
                    }
                }
                "y" => {
                    if self.editor.redo(&mut self.cfg.zones) {
                        self.changed();
                    }
                }
                "d" => {
                    if self.editor.duplicate_selected(&mut self.cfg.zones, &mon.ctx(), snap).is_some() {
                        self.changed();
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Shift pressed or released during a drag: snapping flips, so re-evaluate it.
    fn refresh_drag(&mut self) {
        let Some(key) = self.editor.drag.as_ref().map(|d| d.monitor.clone()) else { return };
        let Some(i) = self.wins.iter().position(|w| w.mon.key == key) else { return };
        let mon = self.wins[i].mon.clone();
        if self.editor.update(&mon.ctx(), self.mouse, self.snapping()) {
            self.invalidate(i, None);
        }
    }

    fn win_scale(&self, i: usize) -> f32 {
        self.wins.get(i).map_or(1.0, |w| w.mon.scale)
    }

    /// Moves the dots' repel point to `p` on edit window `i` and eases the effect in.
    fn set_repel(&mut self, i: usize, p: Pt) {
        if let Some((j, q)) = self.repel {
            if j != i {
                self.repel = None;
                let sc = self.win_scale(j);
                self.invalidate(j, Some(render::repel_bounds(q, sc)));
                self.repel_strength = 0.0;
            }
        }
        let sc = self.win_scale(i);
        let prev = self.repel.map(|(_, q)| q);
        self.repel = Some((i, p));
        self.repel_target = 1.0;
        self.start_anim();
        let dirty = match prev {
            Some(q) => render::repel_bounds(q, sc).union(&render::repel_bounds(p, sc)),
            None => render::repel_bounds(p, sc),
        };
        self.invalidate(i, Some(dirty));
    }

    fn release_repel(&mut self) {
        if self.repel.is_some() && self.repel_target != 0.0 {
            self.repel_target = 0.0;
            self.start_anim();
        }
    }

    fn start_anim(&mut self) {
        if self.anim_at.is_none() && (self.repel_strength - self.repel_target).abs() > 0.001 {
            let now = Instant::now();
            self.anim_tick = Some(now);
            self.anim_at = Some(now + ANIM);
        }
    }

    /// One frame of the dots easing toward (or away from) the mouse.
    fn anim_step(&mut self) {
        let now = Instant::now();
        let dt = self.anim_tick.map_or(0.016, |t| (now - t).as_secs_f32()).min(0.1);
        self.anim_tick = Some(now);
        let k = 1.0 - (-dt / 0.06).exp();
        self.repel_strength += (self.repel_target - self.repel_strength) * k;
        let done = (self.repel_strength - self.repel_target).abs() < 0.01;
        if done {
            self.repel_strength = self.repel_target;
        }
        if let Some((i, p)) = self.repel {
            let sc = self.win_scale(i);
            self.invalidate(i, Some(render::repel_bounds(p, sc)));
            if done && self.repel_target == 0.0 {
                self.repel = None;
            }
        }
        if done || self.repel.is_none() {
            self.anim_at = None;
            self.anim_tick = None;
        } else {
            self.anim_at = Some(now + ANIM);
        }
    }

    fn timers(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if self.exit_at.is_some_and(|t| t <= now) {
            log!("the wallpaper wasn't put back in time; exiting anyway");
            self.finish_exit(el);
            return;
        }
        if self.save_at.is_some_and(|t| t <= now) {
            self.save_now();
        }
        if self.paint_at.is_some_and(|t| t <= now) {
            self.paint();
        }
        if self.anim_at.is_some_and(|t| t <= now) {
            self.anim_step();
        }
        if self.poll_at.is_some_and(|t| t <= now) {
            self.poll_at = Some(now + POLL);
            self.poll(el);
        }
    }
}

/// Commands that change a setting in place: the menu stays open so several can be changed
/// in a row. Everything else closes it.
fn sticky(id: u32) -> bool {
    use cmd::*;
    let within = |base: u32, len: usize| (base..base + len as u32).contains(&id);
    let style = |base: u32| within(base, model::STYLE_SPAN as usize) && StyleCmd::decode(id - base).is_some_and(|c| !c.opens_dialog());
    matches!(id, SHOW_ZONES | GRID_SHOW | SNAP | MIX_COLORS | GRID_REPEL | UNDO | REDO | Z_FRONT | Z_BACK)
        || style(STYLE_ZONE)
        || style(STYLE_DEFAULT)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.start(el);
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Command(id) => {
                if self.exiting.is_none() {
                    self.command(el, id, Target::Global);
                }
            }
            UserEvent::Request(r) => self.request(el, r),
            UserEvent::Painted { serial, result } => self.painted(serial, result),
            UserEvent::Restored { serial } => self.restored(el, serial),
            UserEvent::Color { color, slot, zone } => self.apply_color(color, slot, zone),
            UserEvent::RemoveConfirmed => self.begin_exit(el, Exit::Remove),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(i) = self.win_index(id) else { return };
        match event {
            WindowEvent::RedrawRequested => self.redraw(i),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = self.wins.get_mut(i) {
                    w.sync_size();
                }
            }
            WindowEvent::CloseRequested => self.exit_edit(),
            WindowEvent::ModifiersChanged(m) => {
                let shift_before = self.mods.shift_key();
                self.mods = m.state();
                if self.mods.shift_key() != shift_before {
                    self.refresh_drag();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.mouse_move(i, Pt::new(position.x as f32, position.y as f32)),
            WindowEvent::CursorLeft { .. } => self.release_repel(),
            WindowEvent::MouseInput { state, button, .. } => {
                let p = self.mouse;
                match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => self.press(el, i, p),
                    (MouseButton::Left, ElementState::Released) => self.release(i, p),
                    (MouseButton::Right, ElementState::Pressed) => self.context_menu(i, p),
                    _ => {}
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.key(i, &event),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.timers(el);
        let next = [self.save_at, self.paint_at, self.anim_at, self.poll_at, self.exit_at].into_iter().flatten().min();
        el.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

// ---- drawing the edit windows ----------------------------------------------------------------

struct View<'a> {
    cfg: &'a Config,
    editor: &'a Editor,
    title_fonts: Option<&'a Fonts>,
    ui_fonts: Option<&'a Fonts>,
    hint: bool,
    snap: bool,
    mouse: Pt,
}

fn size_label(r: &Rect, g: &Grid, snap: bool) -> String {
    if snap {
        let (c, rows) = g.cells_in(r);
        format!("{c} \u{00D7} {rows} squares")
    } else {
        format!("{} \u{00D7} {} px", r.w.round(), r.h.round())
    }
}

fn edit_scene<'a>(v: &View<'a>, mon: &'a Monitor, origin: Pt, repel: Option<(Pt, f32)>) -> Scene<'a> {
    let size = mon.size();
    let drag = v.editor.drag.as_ref().filter(|d| d.active && d.monitor == mon.key);
    let zones = v
        .cfg
        .zones
        .iter()
        .filter(|z| z.monitor == mon.key)
        .map(|z| ZoneItem {
            zone: z,
            rect: v.editor.display_rect(z, size),
            selected: v.editor.selected == Some(z.id) && drag.is_none_or(|d| !d.is_create()),
        })
        .collect();
    Scene {
        origin,
        scale: mon.scale,
        painter: Painter { swap_rb: true },
        title_fonts: v.title_fonts,
        ui_fonts: v.ui_fonts,
        grid: Some(GridLayer {
            grid: &mon.grid,
            color: v.cfg.grid.color,
            radius: v.cfg.grid.size,
            alpha: (v.cfg.grid.opacity as f32 / 100.0).max(0.3),
            repel,
        }),
        zones,
        edit: Some(EditLayer {
            hint: v.hint.then_some(HINT),
            label: drag.map(|d| (size_label(&d.rect, &mon.grid, v.snap), v.mouse)),
            draft: drag.filter(|d| d.is_create()).map(|d| (d.rect, &v.cfg.defaults)),
        }),
        mix_colors: v.cfg.mix_colors,
        bounds: mon.full(),
    }
}

/// Where the name field of a box goes (on its title line, like the title itself).
fn title_rect(mon: &Monitor, z: &Zone) -> Rect {
    let sc = mon.scale;
    let r = editor::zone_rect(z, mon.size());
    let h = ((15.0 * 1.45 + 16.0) * sc).round();
    let w = (r.w - 16.0 * sc).clamp(200.0 * sc, 560.0 * sc).round();
    let y = match (z.style.title_on_border, z.style.title_v) {
        (true, model::VAlign::Bottom) => r.bottom() - h / 2.0,
        (true, _) => r.y - h / 2.0,
        (false, model::VAlign::Top) => r.y + 6.0 * sc,
        (false, model::VAlign::Middle) => r.y + (r.h - h) / 2.0,
        (false, model::VAlign::Bottom) => r.bottom() - 6.0 * sc - h,
    };
    let x = match z.style.title_h {
        model::HAlign::Left => r.x + 8.0 * sc,
        model::HAlign::Center => r.x + (r.w - w) / 2.0,
        model::HAlign::Right => r.right() - 8.0 * sc - w,
    };
    Rect::new(x.round(), y.round(), w, h).clamp_into(&mon.full())
}

fn draw_title_entry(pm: &mut PixmapMut, origin: Pt, mon: &Monitor, cfg: &Config, t: &TitleEntry, fonts: &Fonts) {
    let Some(z) = cfg.zones.iter().find(|z| z.id == t.zone && z.monitor == mon.key) else { return };
    let sc = mon.scale;
    let r = title_rect(mon, z);
    let tr = Transform::from_translate(-origin.x, -origin.y);
    let painter = Painter { swap_rb: true };
    let green = model::PASTEL_GREEN;
    if let Some(p) = render::rounded_rect(r, 6.0 * sc) {
        pm.fill_path(&p, &painter.paint(Rgb(0x1E, 0x1E, 0x22), 0.97), FillRule::Winding, tr, None);
        let stroke = Stroke { width: 1.5 * sc, ..Stroke::default() };
        pm.stroke_path(&p, &painter.paint(green, 1.0), &stroke, tr, None);
    }
    let px = 15.0 * sc;
    let baseline = r.y + r.h / 2.0 + fonts.cap_height(px) / 2.0;
    let left = r.x + 10.0 * sc;
    let room = r.w - 22.0 * sc;
    // Show the end of a long name (that's where typing happens).
    let chars: Vec<char> = t.text.chars().collect();
    let mut start = 0;
    let mut shaped = fonts.shape(&t.text, px);
    while shaped.width > room && start < chars.len() {
        start += 1;
        shaped = fonts.shape(&chars[start..].iter().collect::<String>(), px);
    }
    if t.text.is_empty() {
        let hint = fonts.shape("Name this box, then press Enter", px);
        if let Some(p) = fonts.path(&hint, px, left, baseline) {
            pm.fill_path(&p, &painter.paint(Rgb(0x8A, 0x8A, 0x90), 1.0), FillRule::Winding, tr, None);
        }
    } else if let Some(p) = fonts.path(&shaped, px, left, baseline) {
        pm.fill_path(&p, &painter.paint(Rgb(0xF5, 0xF5, 0xF5), 1.0), FillRule::Winding, tr, None);
    }
    let caret_x = if t.text.is_empty() { left } else { left + shaped.width + 1.5 * sc };
    if let Some(caret) = tiny_skia::Rect::from_xywh(caret_x, r.y + 7.0 * sc, 1.5 * sc, r.h - 14.0 * sc) {
        pm.fill_rect(caret, &painter.paint(green, 1.0), tr, None);
    }
}
