//! Windows backend: tray icon, the layer between desktop icons and wallpaper, the
//! edit-mode overlay and the per-user installer.
//!
//! Everything runs on one thread. App state lives in a thread-local RefCell; window
//! procedures borrow it briefly. Modal UI (menus, dialogs) always runs with the borrow
//! released, and re-entrant messages that arrive while it is held fall back to defaults.

#[macro_use]
pub mod util;
mod desktop;
mod diag;
mod comp;
mod dialogs;
mod fonts;
mod install;
mod menu;
mod monitors;
mod reg;
mod shell;
mod sticky;
mod surface;
mod title;
mod tray;

use crate::config;
use crate::editor::{self, Editor, Handle, Hit, Outcome};
use crate::geom::{Pt, Rect};
use crate::grid::{self, Grid};
use crate::model::{self, Config, Rgb, StyleCmd, Zone};
use crate::render::{self, EditLayer, GridLayer, Painter, Scene, ZoneItem};
use crate::text::Fonts;
use desktop::Desktop;
use menu::{cmd, Menu};
use monitors::Monitor;
use shell::IconLayout;
use surface::Surface;
use title::TitleEditor;
use tray::Tray;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::HiDpi::{SetThreadDpiHostingBehavior, DPI_HOSTING_BEHAVIOR_MIXED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, RegisterHotKey, ReleaseCapture, SetCapture, TrackMouseEvent,
    UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_D, VK_DELETE, VK_DOWN,
    VK_ESCAPE, VK_F2, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_Y, VK_Z,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Shell::{FWF_AUTOARRANGE, FWF_SNAPTOGRID, NIN_SELECT};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/WuzNab/DeskChalk";
/// Your Ko-fi page (shown as "Support DeskChalk" in the tray menu).
pub const SUPPORT_URL: &str = "https://ko-fi.com/wuznab";

const MAIN_CLASS: PCWSTR = w!("DeskChalk.Main");
const LAYER_CLASS: PCWSTR = w!("DeskChalk.Layer");
const OVERLAY_CLASS: PCWSTR = w!("DeskChalk.Overlay");

const WM_TRAY: u32 = WM_APP + 1;
const WM_TITLE_DONE: u32 = WM_APP + 2;
const WM_WELCOME: u32 = WM_APP + 3;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

const T_HEALTH: usize = 1;
const T_REATTACH: usize = 2;
const T_SAVE: usize = 3;
const T_TRAY_CLICK: usize = 4;
const T_REFRESH: usize = 5;
const T_FOCUS: usize = 6;
const T_TRIM: usize = 7;
const T_ANIM: usize = 8;
const ANIM_MS: u32 = 15;

const HEALTH_MS: u32 = 2000;
const HOTKEY_ID: i32 = 1;
const HOTKEY_LABEL: &str = "Ctrl+Alt+D";
const EDIT_DIM: f32 = 0.2;
const HINT: &str = "Drag to draw a box  \u{00B7}  Drag edges to resize  \u{00B7}  Double-click to name it  \u{00B7}  Right-click for options  \u{00B7}  Hold Alt to toggle snapping  \u{00B7}  Esc when done";
/// Gap between the top of an icon cell and the icon image, at 100% scaling.
const ICON_TOP_PAD: f32 = 2.0;

pub const CMD_EDIT: usize = 1;
pub const CMD_QUIT: usize = 2;
pub const CMD_SHOW: usize = 3;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static COMMAND_MSG: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).map(|a| a.to_ascii_lowercase()).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        SetThreadDpiHostingBehavior(DPI_HOSTING_BEHAVIOR_MIXED);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Relaxed);
        COMMAND_MSG.store(RegisterWindowMessageW(w!("DeskChalk.Command")), Relaxed);
    }
    if has("--log") {
        util::enable_log(install::config_dir().join("deskchalk.log"));
    }
    if has("--diag") {
        diag::run();
        return;
    }
    if has("--quit") {
        install::signal_running(CMD_QUIT);
        return;
    }
    // One instance per user session. Launching it again (Start menu, desktop shortcut) opens
    // the running one's menu; --edit opens edit mode in it.
    let _mutex = unsafe { CreateMutexW(None, true, w!("Local\\DeskChalk.Instance")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        install::signal_running(if has("--edit") { CMD_EDIT } else { CMD_SHOW });
        return;
    }
    run(RunOpts { edit: has("--edit"), announce: !has("--autostart") });
}

#[derive(Clone, Copy)]
struct RunOpts {
    edit: bool,
    /// Started by hand rather than at sign-in: say so, since the tray icon may be hidden.
    announce: bool,
}

/// Lets popup menus follow the Windows dark/light app theme. Uses uxtheme's
/// SetPreferredAppMode (ordinal 135, Windows 10 1903+); silently does nothing elsewhere.
fn follow_system_menu_theme() {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(ux, PCSTR(135 as *const u8)) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_mode(1); // AllowDark: follow the system setting
        }
        if let Some(f) = GetProcAddress(ux, PCSTR(136 as *const u8)) {
            let flush: extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}

fn run(opts: RunOpts) {
    follow_system_menu_theme();
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        register_class(hinst, MAIN_CLASS, main_proc, false);
        register_class(hinst, LAYER_CLASS, layer_proc, false);
        register_class(hinst, OVERLAY_CLASS, overlay_proc, true);
        title::register(hinst);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("DeskChalk"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        ) else {
            return;
        };
        let app = App::new(hwnd, hinst, opts);
        APP.with(|c| *c.borrow_mut() = Some(app));
        with_app(|a| a.start());
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with_app(|a| a.shutdown());
        APP.with(|c| c.borrow_mut().take());
    }
}

type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

unsafe fn register_class(hinst: HINSTANCE, name: PCWSTR, proc_: WndProc, dblclks: bool) {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: if dblclks { CS_DBLCLKS } else { WNDCLASS_STYLES(0) },
        lpfnWndProc: Some(proc_),
        hInstance: hinst,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: unsafe { LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default() },
        lpszClassName: name,
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

// ---------------------------------------------------------------------------
// App state

struct Layer {
    hwnd: HWND,
    key: String,
    /// DirectComposition content (None when using UpdateLayeredWindow).
    comp: Option<comp::CompLayer>,
}

struct Overlay {
    hwnd: HWND,
    key: String,
    surface: Surface,
    /// Screen rectangle (the monitor's work area).
    rect: RECT,
    /// Work-area origin relative to the monitor (overlay client coords + offset = monitor coords).
    offset: Pt,
}

#[derive(Clone)]
enum Target {
    Global,
    Zone(u64),
    Monitor(String),
}

/// Which menu is open (to refresh its check marks while it stays open).
#[derive(Clone)]
enum MenuKind {
    Tray,
    Zone(u64),
    Canvas(String),
}

#[derive(Clone, Copy)]
enum ColorSlot {
    Box,
    Border,
    Dots,
}

enum Follow {
    Nothing,
    /// `zone` None means the style for new boxes.
    CustomColor { initial: Rgb, slot: ColorSlot, zone: Option<u64> },
    Uninstall,
    ConfirmDeleteAll(String),
}

struct App {
    hwnd: HWND,
    hinst: HINSTANCE,
    opts: RunOpts,
    cfg: Config,
    cfg_path: PathBuf,
    monitors: Vec<Monitor>,
    layout: Option<IconLayout>,
    desktop: Option<Desktop>,
    layers: Vec<Layer>,
    overlays: Vec<Overlay>,
    editing: bool,
    editor: Editor,
    title_edit: Option<TitleEditor>,
    tray: Tray,
    title_fonts: Option<(String, Fonts)>,
    ui_fonts: Option<Fonts>,
    hotkey_on: bool,
    minimized: bool,
    last_dblclk: Option<Instant>,
    /// Last mouse position in edit mode (monitor-relative).
    mouse: Pt,
    uninstalling: bool,
    /// DirectComposition device for the desktop layer (None: UpdateLayeredWindow fallback).
    comp: Option<comp::Compositor>,
    /// The usage hint shows until the first click in an edit session.
    show_hint: bool,
    /// Overlay index and mouse position the dots are currently drifting away from.
    repel: Option<(usize, Pt)>,
    /// How far the drift has eased in (0..1), where it's heading, and the last animation tick.
    repel_strength: f32,
    repel_target: f32,
    anim_tick: Option<Instant>,
    /// Overlay we asked to be told about the mouse leaving.
    tracking_leave: Option<HWND>,
}

/// What the edit overlays are drawn from (borrowed from the App).
struct OverlayView<'a> {
    cfg: &'a Config,
    editor: &'a Editor,
    title_fonts: Option<&'a Fonts>,
    ui_fonts: Option<&'a Fonts>,
    hint: bool,
    snap: bool,
    mouse: Pt,
}

fn overlay_scene<'a>(v: &OverlayView<'a>, mon: &'a Monitor, origin: Pt, repel: Option<(Pt, f32)>) -> Scene<'a> {
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
        scale: mon.scale(),
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
        // The overlay covers the work area; hints and titles stay inside it.
        bounds: mon.work_rel(),
    }
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

fn find_monitor<'a>(mons: &'a [Monitor], key: &str) -> Option<&'a Monitor> {
    mons.iter().find(|m| m.key == key)
}

fn size_label(r: &Rect, g: &Grid, snap: bool) -> String {
    if snap {
        let (c, rows) = g.cells_in(r);
        format!("{c} \u{00D7} {rows} icons")
    } else {
        format!("{} \u{00D7} {} px", r.w.round(), r.h.round())
    }
}

/// Places the icon grid on every monitor from the icon spacing and where icons actually sit.
pub(crate) fn compute_grids(mons: &mut [Monitor], layout: Option<&IconLayout>) {
    for m in mons.iter_mut() {
        let s = m.scale();
        let (sx, sy) = layout
            .map(|l| (l.spacing.0 as f32, l.spacing.1 as f32))
            .filter(|sp| sp.0 >= 16.0 && sp.1 >= 16.0)
            .unwrap_or((75.0 * s, 75.0 * s));
        let work = m.work_rel();
        let pts: Vec<(f32, f32)> = layout
            .map(|l| {
                l.positions
                    .iter()
                    .filter(|p| m.contains(p.x, p.y))
                    .map(|p| ((p.x - m.rect.left) as f32, (p.y - m.rect.top) as f32))
                    .collect()
            })
            .unwrap_or_default();
        // An item's position is the top-left of its icon image, which Explorer centers
        // horizontally in its cell and draws a couple of pixels below the cell's top.
        let icon_px = layout.map_or(0.0, |l| l.icon_size as f32 * s);
        let inset_x = ((sx - icon_px) / 2.0).max(0.0);
        let inset_y = (ICON_TOP_PAD * s).min(sy / 4.0);
        let px = grid::dominant_phase(pts.iter().map(|p| p.0 - inset_x - work.x), sx).unwrap_or(0.0);
        let py = grid::dominant_phase(pts.iter().map(|p| p.1 - inset_y - work.y), sy).unwrap_or(0.0);
        // Keep the phase small and stable (cells start right at the work area edge).
        let px = if px > sx / 2.0 { px - sx } else { px };
        let py = if py > sy / 2.0 { py - sy } else { py };
        m.grid = Grid {
            origin: Pt::new(work.x + px.round(), work.y + py.round()),
            cell: Pt::new(sx, sy),
            area: work,
        };
    }
}

/// Boxes whose monitor moved to another port find it again by model code.
fn migrate_zones(zones: &mut [Zone], mons: &[Monitor]) -> bool {
    let used: Vec<String> = zones.iter().map(|z| z.monitor.clone()).collect();
    let mut changed = false;
    for z in zones.iter_mut() {
        if z.monitor_hw.is_empty() || mons.iter().any(|m| m.key == z.monitor) {
            continue;
        }
        let candidates: Vec<&Monitor> = mons.iter().filter(|m| m.hw == z.monitor_hw).collect();
        if candidates.len() == 1 && !used.contains(&candidates[0].key) {
            z.monitor = candidates[0].key.clone();
            changed = true;
        }
    }
    changed
}

/// `composition`: content comes from a DirectComposition visual (no GDI surface);
/// otherwise it's a layered window updated with UpdateLayeredWindow.
fn create_layer(hinst: HINSTANCE, parent: HWND, composition: bool) -> Option<HWND> {
    let ex = if composition { WS_EX_NOREDIRECTIONBITMAP } else { WS_EX_LAYERED };
    unsafe {
        CreateWindowExW(
            ex | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            LAYER_CLASS,
            w!(""),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            Some(hinst),
            None,
        )
        .ok()
    }
}

impl App {
    fn new(hwnd: HWND, hinst: HINSTANCE, opts: RunOpts) -> App {
        let cfg_path = install::config_dir().join("config.json");
        let cfg = config::load(&cfg_path);
        App {
            hwnd,
            hinst,
            opts,
            cfg,
            cfg_path,
            monitors: Vec::new(),
            layout: None,
            desktop: None,
            layers: Vec::new(),
            overlays: Vec::new(),
            editing: false,
            editor: Editor::default(),
            title_edit: None,
            tray: Tray::new(hwnd),
            title_fonts: None,
            ui_fonts: None,
            hotkey_on: false,
            minimized: false,
            last_dblclk: None,
            mouse: Pt::default(),
            uninstalling: false,
            comp: None,
            show_hint: true,
            repel: None,
            repel_strength: 0.0,
            repel_target: 0.0,
            anim_tick: None,
            tracking_leave: None,
        }
    }

    fn start(&mut self) {
        self.tray.add(&self.tooltip());
        self.apply_hotkey();
        self.attach();
        self.schedule(T_HEALTH, HEALTH_MS);
        unsafe {
            if !self.cfg.welcomed {
                let _ = PostMessageW(Some(self.hwnd), WM_WELCOME, WPARAM(0), LPARAM(0));
            } else if self.opts.announce && !self.opts.edit {
                self.announce("DeskChalk is running");
            }
            if self.opts.edit {
                let _ = PostMessageW(Some(self.hwnd), COMMAND_MSG.load(Relaxed), WPARAM(CMD_EDIT), LPARAM(0));
            }
        }
    }

    fn shutdown(&mut self) {
        if self.editing {
            self.exit_edit();
        }
        if !self.uninstalling {
            self.save_now();
        }
        self.destroy_layers();
        self.tray.remove();
        if self.hotkey_on {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            }
        }
    }

    fn tooltip(&self) -> String {
        if self.editing {
            "DeskChalk: editing boxes (Esc to finish)".into()
        } else {
            "DeskChalk: double-click to draw boxes".into()
        }
    }

    fn schedule(&self, timer: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), timer, ms, None);
        }
    }

    fn kill(&self, timer: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), timer);
        }
    }

    fn save_soon(&self) {
        if !self.uninstalling {
            self.schedule(T_SAVE, 500);
        }
    }

    fn save_now(&mut self) {
        self.kill(T_SAVE);
        if self.uninstalling {
            return;
        }
        if let Err(e) = config::save(&self.cfg_path, &self.cfg) {
            log!("saving settings failed: {e}");
        }
    }

    fn apply_hotkey(&mut self) {
        unsafe {
            if self.hotkey_on {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
                self.hotkey_on = false;
            }
            if self.cfg.hotkey {
                self.hotkey_on = RegisterHotKey(Some(self.hwnd), HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_D.0 as u32).is_ok();
                if !self.hotkey_on {
                    log!("{HOTKEY_LABEL} is already used by another app");
                }
            }
        }
    }

    // ---- monitors, icon grid and the desktop layer --------------------------------

    /// Re-reads monitors and the icon grid. Returns (anything changed, geometry changed).
    fn refresh_monitors(&mut self) -> (bool, bool) {
        let listview = self.desktop.and_then(|d| d.listview).or_else(desktop::locate_listview);
        self.layout = shell::icon_layout(listview);
        let mut mons = monitors::enumerate();
        compute_grids(&mut mons, self.layout.as_ref());
        if migrate_zones(&mut self.cfg.zones, &mons) {
            self.save_soon();
        }
        let geometry = mons.len() != self.monitors.len()
            || mons.iter().zip(&self.monitors).any(|(a, b)| !a.same_geometry(b));
        let changed = mons != self.monitors;
        self.monitors = mons;
        (changed, geometry)
    }

    fn attach(&mut self) {
        self.kill(T_REATTACH);
        self.destroy_layers();
        if self.comp.as_ref().is_some_and(|c| !c.healthy()) {
            self.comp = None;
        }
        // DESKCHALK_PAINT=layered forces the older UpdateLayeredWindow path (troubleshooting).
        if self.comp.is_none() && std::env::var("DESKCHALK_PAINT").map_or(true, |v| v != "layered") {
            self.comp = comp::Compositor::new();
            if self.comp.is_none() {
                log!("DirectComposition unavailable; using layered windows");
            }
        }
        self.desktop = desktop::find();
        match &self.desktop {
            Some(d) => log!("attached to the desktop ({:?} topology)", d.topology),
            None => log!("desktop not found yet; retrying"),
        }
        self.refresh_monitors();
        self.render_layers();
    }

    fn destroy_layers(&mut self) {
        for l in self.layers.drain(..) {
            unsafe {
                let _ = DestroyWindow(l.hwnd);
            }
        }
    }

    /// Runs every couple of seconds; cheap. Reattaches after Explorer restarts or the
    /// wallpaper host windows get recreated.
    fn health_check(&mut self) {
        if self.uninstalling {
            return;
        }
        let Some(d) = self.desktop else {
            self.attach();
            return;
        };
        let layers_ok = self.layers.iter().all(|l| unsafe {
            IsWindow(Some(l.hwnd)).as_bool() && GetParent(l.hwnd).ok() == Some(d.parent)
        });
        if !d.still_valid() || !layers_ok {
            log!("desktop windows changed; reattaching");
            self.attach();
            return;
        }
        if self.comp.as_ref().is_some_and(|c| !c.healthy()) {
            log!("graphics device was reset; rebuilding the desktop layer");
            self.attach();
            return;
        }
        let ours: Vec<HWND> = self.layers.iter().map(|l| l.hwnd).collect();
        if !ours.is_empty() && !d.order_ok(&ours) {
            log!("restoring layer order");
            self.place_layers();
        }
    }

    fn place_layers(&self) {
        let Some(d) = self.desktop else { return };
        for l in &self.layers {
            unsafe {
                let _ = SetWindowPos(l.hwnd, Some(d.insert_after()), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn on_layer_destroyed(&mut self, hwnd: HWND) {
        let before = self.layers.len();
        self.layers.retain(|l| l.hwnd != hwnd);
        if self.layers.len() != before && !self.uninstalling {
            self.schedule(T_REATTACH, 500);
        }
    }

    fn ensure_title_fonts(&mut self) {
        if self.title_fonts.as_ref().is_some_and(|(k, _)| *k == self.cfg.font) {
            return;
        }
        self.title_fonts = fonts::load(&self.cfg.font).map(|f| (self.cfg.font.clone(), f));
    }

    fn ensure_ui_fonts(&mut self) {
        if self.ui_fonts.is_none() {
            self.ui_fonts = fonts::load_ui();
        }
    }

    /// Draws the boxes (and grid dots, if enabled) into the windows behind the icons.
    /// Each window only covers the area that has content, and bitmaps are freed after use.
    fn render_layers(&mut self) {
        if self.editing || self.uninstalling {
            return;
        }
        let Some(desk) = self.desktop else { return };
        if self.cfg.show_zones && self.cfg.zones.iter().any(|z| !z.title.trim().is_empty()) {
            self.ensure_title_fonts();
        }
        let origin = desk.parent_origin();
        let painter = Painter { swap_rb: true };
        let composition = self.comp.is_some();
        let mut kept: Vec<String> = Vec::new();
        for mon in &self.monitors {
            let size = mon.size();
            let full = Rect::new(0.0, 0.0, size.0, size.1);
            let zones: Vec<ZoneItem> = if self.cfg.show_zones {
                self.cfg
                    .zones
                    .iter()
                    .filter(|z| z.monitor == mon.key)
                    .map(|z| ZoneItem { zone: z, rect: editor::zone_rect(z, size), selected: false })
                    .collect()
            } else {
                Vec::new()
            };
            let show_grid = self.cfg.grid.show_on_desktop;
            let mut bounds: Option<Rect> = None;
            for z in &zones {
                // Titles on the border line reach a little outside the box.
                let r = z.rect.inset(-(z.zone.style.title_size * 0.9 + 6.0) * mon.scale());
                bounds = Some(bounds.map_or(r, |b| b.union(&r)));
            }
            if show_grid {
                bounds = Some(bounds.map_or(mon.grid.area, |b| b.union(&mon.grid.area)));
            }
            let Some(b) = bounds.and_then(|b| b.intersect(&full)) else { continue };
            let (bx, by, bw, bh) = b.round_out();
            let hwnd = match self.layers.iter().find(|l| l.key == mon.key) {
                Some(l) => l.hwnd,
                None => match create_layer(self.hinst, desk.parent, composition) {
                    Some(h) => {
                        self.layers.push(Layer { hwnd: h, key: mon.key.clone(), comp: None });
                        h
                    }
                    None => {
                        log!("could not create the desktop layer window");
                        continue;
                    }
                },
            };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    Some(desk.insert_after()),
                    mon.rect.left + bx - origin.x,
                    mon.rect.top + by - origin.y,
                    bw,
                    bh,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            let mut pixels = vec![0u8; (bw * bh * 4) as usize];
            let scene = Scene {
                origin: Pt::new(bx as f32, by as f32),
                scale: mon.scale(),
                painter,
                title_fonts: self.title_fonts.as_ref().map(|(_, f)| f),
                ui_fonts: None,
                grid: show_grid.then_some(GridLayer {
                    grid: &mon.grid,
                    color: self.cfg.grid.color,
                    radius: self.cfg.grid.size,
                    alpha: self.cfg.grid.opacity as f32 / 100.0,
                    repel: None,
                }),
                zones,
                edit: None,
                mix_colors: self.cfg.mix_colors,
                bounds: full,
            };
            if let Some(mut pm) = tiny_skia::PixmapMut::from_bytes(&mut pixels, bw as u32, bh as u32) {
                render::draw(&mut pm, &scene);
            }
            let result = match (&self.comp, self.layers.iter_mut().find(|l| l.hwnd == hwnd)) {
                (Some(c), Some(layer)) => {
                    if layer.comp.is_none() {
                        layer.comp = c.attach(hwnd);
                    }
                    match layer.comp.as_mut() {
                        Some(cl) => c.present(cl, bw as u32, bh as u32, &pixels).map_err(|e| e.to_string()),
                        None => Err("could not attach a composition target".into()),
                    }
                }
                _ => match Surface::from_pixels(bw, bh, &pixels) {
                    Some(surf) => surf.present(hwnd, None).map_err(|e| e.to_string()),
                    None => Err("could not allocate a bitmap".into()),
                },
            };
            if let Err(e) = result {
                log!("drawing the desktop layer failed: {e}");
            }
            kept.push(mon.key.clone());
        }
        let (keep, gone): (Vec<Layer>, Vec<Layer>) = self.layers.drain(..).partition(|l| kept.contains(&l.key));
        for l in gone {
            unsafe {
                let _ = DestroyWindow(l.hwnd);
            }
        }
        self.layers = keep;
        // Fonts are only kept around while editing.
        self.title_fonts = None;
        self.schedule(T_TRIM, 3000);
    }

    /// Hands memory that is no longer needed (bitmaps, fonts) back to Windows.
    fn trim_memory(&self) {
        if !self.editing {
            unsafe {
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            }
        }
    }

    fn orphan_count(&self) -> usize {
        self.cfg.zones.iter().filter(|z| !self.monitors.iter().any(|m| m.key == z.monitor)).count()
    }

    fn recover_orphans(&mut self) {
        let Some(target) = self.monitors.iter().find(|m| m.primary).or(self.monitors.first()) else { return };
        self.editor.checkpoint(&self.cfg.zones);
        let ctx = target.ctx();
        for z in self.cfg.zones.iter_mut() {
            if self.monitors.iter().any(|m| m.key == z.monitor) {
                continue;
            }
            let r = editor::zone_rect(z, ctx.size).clamp_into(&ctx.bounds);
            editor::set_zone_rect(z, r, &ctx);
        }
        self.changed();
    }

    /// Re-render whatever is visible and save soon.
    fn changed(&mut self) {
        if self.editing {
            self.render_overlays(None);
        } else {
            self.render_layers();
        }
        self.save_soon();
    }

    fn refresh(&mut self) {
        self.tray.refresh_icon();
        follow_system_menu_theme();
        let (changed, geometry) = self.refresh_monitors();
        if geometry && self.editing {
            self.exit_edit();
        }
        self.health_check();
        if changed {
            if self.editing {
                self.render_overlays(None);
            } else {
                self.render_layers();
            }
        }
    }

    fn timer(&mut self, id: usize) {
        match id {
            T_HEALTH => self.health_check(),
            T_REATTACH => self.attach(),
            T_SAVE => self.save_now(),
            T_REFRESH => {
                self.kill(T_REFRESH);
                self.refresh();
            }
            T_FOCUS => {
                self.kill(T_FOCUS);
                if self.editing {
                    self.focus_overlay();
                }
            }
            T_TRIM => {
                self.kill(T_TRIM);
                self.trim_memory();
            }
            T_ANIM => self.anim_step(),
            _ => {}
        }
    }

    fn on_taskbar_created(&mut self) {
        // Explorer restarted: the tray icon and the desktop windows are gone.
        log!("Explorer restarted; restoring the tray icon and the desktop layer");
        self.tray.forget();
        self.tray.add(&self.tooltip());
        self.schedule(T_REATTACH, 1500);
    }

    fn tray_select(&mut self) {
        let dbl = unsafe { GetDoubleClickTime() };
        if self.last_dblclk.is_some_and(|t| t.elapsed().as_millis() < dbl as u128 * 2) {
            return;
        }
        self.schedule(T_TRAY_CLICK, dbl);
    }

    fn tray_double_click(&mut self) {
        self.kill(T_TRAY_CLICK);
        self.last_dblclk = Some(Instant::now());
    }

    /// A notification from the tray icon saying where DeskChalk lives and how to start drawing.
    fn announce(&self, title: &str) {
        self.tray.balloon(
            title,
            &format!("Double-click the pencil in the tray (or press {HOTKEY_LABEL}) to draw boxes on your desktop."),
        );
    }

    // ---- edit mode ---------------------------------------------------------------

    fn snapping(&self) -> bool {
        self.cfg.snap != key_down(VK_MENU)
    }

    fn enter_edit(&mut self) {
        if self.editing || self.uninstalling {
            return;
        }
        self.refresh_monitors();
        if self.cfg.minimize_windows {
            self.minimized = shell::minimize_all();
        }
        for l in &self.layers {
            unsafe {
                let _ = ShowWindow(l.hwnd, SW_HIDE);
            }
        }
        self.ensure_title_fonts();
        self.ensure_ui_fonts();
        for mon in &self.monitors {
            let wr = mon.work;
            let (ww, wh) = (wr.right - wr.left, wr.bottom - wr.top);
            let Some(surface) = Surface::new(ww, wh) else { continue };
            let created = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    OVERLAY_CLASS,
                    w!("DeskChalk"),
                    WS_POPUP,
                    wr.left,
                    wr.top,
                    ww,
                    wh,
                    None,
                    None,
                    Some(self.hinst),
                    None,
                )
            };
            let Ok(hwnd) = created else { continue };
            self.overlays.push(Overlay {
                hwnd,
                key: mon.key.clone(),
                surface,
                rect: wr,
                offset: Pt::new((wr.left - mon.rect.left) as f32, (wr.top - mon.rect.top) as f32),
            });
        }
        if self.overlays.is_empty() {
            self.after_edit_cleanup();
            return;
        }
        self.editing = true;
        self.editor.selected = None;
        self.show_hint = true;
        self.render_overlays(None);
        for ov in &self.overlays {
            unsafe {
                let _ = ShowWindow(ov.hwnd, SW_SHOW);
            }
        }
        self.focus_overlay();
        if self.minimized {
            // Minimizing hands focus to the desktop a moment later; take it back.
            self.schedule(T_FOCUS, 450);
        }
        self.tray.set_tip(&self.tooltip());
    }

    fn exit_edit(&mut self) {
        if !self.editing {
            return;
        }
        self.finish_title_edit(true);
        self.editor.cancel();
        self.editor.selected = None;
        self.repel = None;
        self.repel_strength = 0.0;
        self.anim_tick = None;
        self.kill(T_ANIM);
        for ov in self.overlays.drain(..) {
            unsafe {
                let _ = DestroyWindow(ov.hwnd);
            }
        }
        self.editing = false;
        self.after_edit_cleanup();
        self.render_layers();
        self.ui_fonts = None;
        self.save_soon();
        self.tray.set_tip(&self.tooltip());
    }

    fn after_edit_cleanup(&mut self) {
        if self.minimized {
            shell::undo_minimize_all();
            self.minimized = false;
        }
        for l in &self.layers {
            unsafe {
                let _ = ShowWindow(l.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    fn focus_overlay(&self) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let inside = |r: &RECT| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom;
        if let Some(ov) = self.overlays.iter().find(|o| inside(&o.rect)).or(self.overlays.first()) {
            unsafe {
                let _ = SetForegroundWindow(ov.hwnd);
            }
        }
    }

    fn modal_owner(&self) -> Option<HWND> {
        if self.editing {
            self.overlays.first().map(|o| o.hwnd)
        } else {
            Some(self.hwnd)
        }
    }

    fn render_overlays(&mut self, only: Option<usize>) {
        let snap = self.snapping();
        let repel = self.repel;
        let strength = self.repel_strength;
        let view = OverlayView {
            cfg: &self.cfg,
            editor: &self.editor,
            title_fonts: self.title_fonts.as_ref().map(|(_, f)| f),
            ui_fonts: self.ui_fonts.as_ref(),
            hint: self.show_hint,
            snap,
            mouse: self.mouse,
        };
        for (i, ov) in self.overlays.iter_mut().enumerate() {
            if only.is_some_and(|o| o != i) {
                continue;
            }
            let Some(mon) = find_monitor(&self.monitors, &ov.key) else { continue };
            let at = repel.filter(|(j, _)| *j == i).map(|(_, p)| (p, strength));
            let scene = overlay_scene(&view, mon, ov.offset, at);
            render::clear(ov.surface.pixels(), EDIT_DIM);
            if let Some(mut pm) = ov.surface.pixmap() {
                render::draw(&mut pm, &scene);
            }
            let pos = POINT { x: ov.rect.left, y: ov.rect.top };
            if let Err(e) = ov.surface.present(ov.hwnd, Some(pos)) {
                log!("drawing the editor failed: {e}");
            }
        }
    }

    /// Redraws only `region` (monitor coordinates) of one overlay, e.g. around the mouse.
    fn render_overlay_region(&mut self, i: usize, region: Rect) {
        let snap = self.snapping();
        let strength = self.repel_strength;
        let at = self.repel.filter(|(j, _)| *j == i).map(|(_, p)| (p, strength));
        let view = OverlayView {
            cfg: &self.cfg,
            editor: &self.editor,
            title_fonts: self.title_fonts.as_ref().map(|(_, f)| f),
            ui_fonts: self.ui_fonts.as_ref(),
            hint: self.show_hint,
            snap,
            mouse: self.mouse,
        };
        let Some(ov) = self.overlays.get_mut(i) else { return };
        let Some(mon) = find_monitor(&self.monitors, &ov.key) else { return };
        let bitmap = Rect::new(0.0, 0.0, ov.surface.w as f32, ov.surface.h as f32);
        let Some(local) = region.offset(-ov.offset.x, -ov.offset.y).intersect(&bitmap) else { return };
        let (x, y, w, h) = local.round_out();
        let (w, h) = (w.min(ov.surface.w - x), h.min(ov.surface.h - y));
        if w <= 0 || h <= 0 {
            return;
        }
        let mut buf = vec![0u8; (w * h * 4) as usize];
        render::clear(&mut buf, EDIT_DIM);
        let scene = overlay_scene(&view, mon, Pt::new(ov.offset.x + x as f32, ov.offset.y + y as f32), at);
        if let Some(mut pm) = tiny_skia::PixmapMut::from_bytes(&mut buf, w as u32, h as u32) {
            render::draw(&mut pm, &scene);
        }
        ov.surface.write_region(x, y, w, h, &buf);
        let pos = POINT { x: ov.rect.left, y: ov.rect.top };
        ov.surface.present_dirty(ov.hwnd, pos, RECT { left: x, top: y, right: x + w, bottom: y + h });
    }

    fn overlay_scale(&self, i: usize) -> f32 {
        self.overlay_monitor(i).map_or(1.0, |m| m.scale())
    }

    /// Moves the dots' repel point to `p` on overlay `i` and eases the effect in.
    fn set_repel(&mut self, i: usize, p: Pt) {
        if let Some((j, q)) = self.repel {
            if j != i {
                // The mouse moved to another screen: let the old one settle at once.
                self.repel = None;
                let sc = self.overlay_scale(j);
                self.render_overlay_region(j, render::repel_bounds(q, sc));
                self.repel_strength = 0.0;
            }
        }
        let sc = self.overlay_scale(i);
        let prev = self.repel.map(|(_, q)| q);
        self.repel = Some((i, p));
        self.repel_target = 1.0;
        self.start_anim();
        let dirty = match prev {
            Some(q) => render::repel_bounds(q, sc).union(&render::repel_bounds(p, sc)),
            None => render::repel_bounds(p, sc),
        };
        self.render_overlay_region(i, dirty);
    }

    fn overlay_leave(&mut self, hwnd: HWND) {
        self.tracking_leave = None;
        if self.overlay_index(hwnd).is_some() && self.repel.is_some() {
            self.repel_target = 0.0;
            self.start_anim();
        }
    }

    fn start_anim(&mut self) {
        if self.anim_tick.is_none() && (self.repel_strength - self.repel_target).abs() > 0.001 {
            self.anim_tick = Some(Instant::now());
            self.schedule(T_ANIM, ANIM_MS);
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
            let sc = self.overlay_scale(i);
            self.render_overlay_region(i, render::repel_bounds(p, sc));
            if done && self.repel_target == 0.0 {
                self.repel = None;
            }
        }
        if done || self.repel.is_none() || !self.editing {
            self.kill(T_ANIM);
            self.anim_tick = None;
        }
    }

    fn overlay_index(&self, hwnd: HWND) -> Option<usize> {
        self.overlays.iter().position(|o| o.hwnd == hwnd)
    }

    fn overlay_point(&self, i: usize, x: i32, y: i32) -> Pt {
        let o = self.overlays[i].offset;
        Pt::new(x as f32 + o.x, y as f32 + o.y)
    }

    fn overlay_monitor(&self, i: usize) -> Option<&Monitor> {
        let key = &self.overlays.get(i)?.key;
        self.monitors.iter().find(|m| &m.key == key)
    }

    fn overlay_down(&mut self, hwnd: HWND, x: i32, y: i32) {
        self.finish_title_edit(true);
        self.show_hint = false;
        self.repel = None;
        self.repel_strength = 0.0;
        let Some(i) = self.overlay_index(hwnd) else { return };
        let p = self.overlay_point(i, x, y);
        self.mouse = p;
        let Some(mon) = find_monitor(&self.monitors, &self.overlays[i].key) else { return };
        let ctx = mon.ctx();
        self.editor.begin(&self.cfg.zones, &ctx, p);
        self.render_overlays(None);
    }

    fn overlay_move(&mut self, hwnd: HWND, x: i32, y: i32) {
        let Some(i) = self.overlay_index(hwnd) else { return };
        let p = self.overlay_point(i, x, y);
        self.mouse = p;
        if self.editor.drag.is_none() {
            if self.cfg.grid.repel {
                if self.tracking_leave != Some(hwnd) {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    self.tracking_leave = Some(hwnd);
                }
                self.set_repel(i, p);
            }
            return;
        }
        let snap = self.snapping();
        let Some(mon) = find_monitor(&self.monitors, &self.overlays[i].key) else { return };
        if self.editor.update(&mon.ctx(), p, snap) {
            self.render_overlays(Some(i));
        }
    }

    fn overlay_up(&mut self, hwnd: HWND, x: i32, y: i32) {
        let Some(i) = self.overlay_index(hwnd) else { return };
        let p = self.overlay_point(i, x, y);
        self.mouse = p;
        let snap = self.snapping();
        let Some(mon) = self.monitors.iter().find(|m| m.key == self.overlays[i].key) else { return };
        let ctx = mon.ctx();
        self.editor.update(&ctx, p, snap);
        let out = self.editor.finish(&mut self.cfg.zones, &ctx, &self.cfg.defaults);
        match out {
            Outcome::Created(id) => {
                self.save_soon();
                self.render_overlays(None);
                // New boxes ask for a name right away (Esc leaves it untitled).
                self.begin_title_edit(id);
            }
            Outcome::Changed => {
                self.save_soon();
                self.render_overlays(None);
            }
            Outcome::Redraw => self.render_overlays(None),
            Outcome::None => {}
        }
    }

    fn overlay_capture_lost(&mut self) {
        if self.editor.cancel() {
            self.render_overlays(None);
        }
    }

    fn overlay_double_click(&mut self, hwnd: HWND, x: i32, y: i32) {
        let Some(i) = self.overlay_index(hwnd) else { return };
        let p = self.overlay_point(i, x, y);
        let Some(mon) = self.overlay_monitor(i) else { return };
        if let Hit::Body(id) | Hit::Handle(id, _) = Editor::hit_test(&self.cfg.zones, &mon.ctx(), p) {
            self.begin_title_edit(id);
        }
    }

    fn overlay_cursor(&self, hwnd: HWND) -> Option<PCWSTR> {
        let i = self.overlay_index(hwnd)?;
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(hwnd, &mut pt);
        }
        let p = self.overlay_point(i, pt.x, pt.y);
        let mon = self.overlay_monitor(i)?;
        Some(match Editor::hit_test(&self.cfg.zones, &mon.ctx(), p) {
            Hit::None => IDC_CROSS,
            Hit::Body(_) => IDC_SIZEALL,
            Hit::Handle(_, h) => match h {
                Handle::N | Handle::S => IDC_SIZENS,
                Handle::E | Handle::W => IDC_SIZEWE,
                Handle::NW | Handle::SE => IDC_SIZENWSE,
                Handle::NE | Handle::SW => IDC_SIZENESW,
            },
        })
    }

    /// Re-evaluates an active drag (e.g. Alt pressed or released).
    fn refresh_drag(&mut self) {
        let Some(key) = self.editor.drag.as_ref().map(|d| d.monitor.clone()) else { return };
        let snap = self.snapping();
        let Some(mon) = self.monitors.iter().find(|m| m.key == key) else { return };
        if self.editor.update(&mon.ctx(), self.mouse, snap) {
            let idx = self.overlays.iter().position(|o| o.key == key);
            self.render_overlays(idx);
        }
    }

    /// Keyboard shortcuts in edit mode. Returns true if handled.
    fn overlay_key(&mut self, hwnd: HWND, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let snap = self.snapping();
        let Some(i) = self.overlay_index(hwnd) else { return false };
        let key = self.overlays[i].key.clone();
        match vk {
            VK_ESCAPE => {
                if self.editor.cancel() {
                    self.render_overlays(None);
                } else {
                    self.exit_edit();
                }
            }
            VK_RETURN | VK_F2 => {
                if let Some(id) = self.editor.selected {
                    self.begin_title_edit(id);
                }
            }
            VK_DELETE | VK_BACK => {
                if self.editor.delete_selected(&mut self.cfg.zones) {
                    self.changed();
                }
            }
            VK_TAB => {
                if let Some(mon) = self.monitors.iter().find(|m| m.key == key) {
                    if self.editor.select_next(&self.cfg.zones, &mon.ctx()) {
                        self.render_overlays(None);
                    }
                }
            }
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
                let (dx, dy) = match vk {
                    VK_LEFT => (-1, 0),
                    VK_RIGHT => (1, 0),
                    VK_UP => (0, -1),
                    _ => (0, 1),
                };
                if let Some(mon) = self.monitors.iter().find(|m| m.key == key) {
                    if self.editor.nudge(&mut self.cfg.zones, &mon.ctx(), dx, dy, snap, shift) {
                        self.changed();
                    }
                }
            }
            VK_Z if ctrl => {
                let done = if shift {
                    self.editor.redo(&mut self.cfg.zones)
                } else {
                    self.editor.undo(&mut self.cfg.zones)
                };
                if done {
                    self.changed();
                }
            }
            VK_Y if ctrl => {
                if self.editor.redo(&mut self.cfg.zones) {
                    self.changed();
                }
            }
            VK_D if ctrl => {
                if let Some(mon) = self.monitors.iter().find(|m| m.key == key) {
                    if self.editor.duplicate_selected(&mut self.cfg.zones, &mon.ctx(), snap).is_some() {
                        self.changed();
                    }
                }
            }
            VK_MENU => self.refresh_drag(),
            _ => return false,
        }
        true
    }

    fn canvas_menu(&self, key: &str) -> Menu {
        let any_here = self.cfg.zones.iter().any(|z| z.monitor == key);
        menu::canvas_menu(
            self.cfg.snap,
            self.cfg.grid.show_on_desktop,
            self.editor.can_undo(),
            self.editor.can_redo(),
            any_here,
        )
    }

    fn overlay_menu(&mut self, hwnd: HWND, x: i32, y: i32) -> Option<(Menu, Target, MenuKind)> {
        self.finish_title_edit(true);
        self.editor.cancel();
        let i = self.overlay_index(hwnd)?;
        let p = self.overlay_point(i, x, y);
        let mon = self.overlay_monitor(i)?;
        let hit = Editor::hit_test(&self.cfg.zones, &mon.ctx(), p);
        let scale = mon.scale();
        let key = mon.key.clone();
        let result = match hit {
            Hit::Body(id) | Hit::Handle(id, _) => {
                self.editor.selected = Some(id);
                let z = self.cfg.zones.iter().find(|z| z.id == id)?;
                (menu::zone_menu(&z.style, scale), Target::Zone(id), MenuKind::Zone(id))
            }
            Hit::None => {
                self.editor.selected = None;
                (self.canvas_menu(&key), Target::Monitor(key.clone()), MenuKind::Canvas(key))
            }
        };
        self.render_overlays(None);
        Some(result)
    }

    fn begin_title_edit(&mut self, id: u64) {
        self.finish_title_edit(true);
        let Some(z) = self.cfg.zones.iter().find(|z| z.id == id) else { return };
        let Some(mon) = self.monitors.iter().find(|m| m.key == z.monitor) else { return };
        let Some(ov) = self.overlays.iter().find(|o| o.key == mon.key) else { return };
        let sc = mon.scale();
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
        let x = mon.rect.left + x.round() as i32;
        let y = mon.rect.top + y.round() as i32;
        let rect = RECT { left: x, top: y, right: x + w as i32, bottom: y + h as i32 };
        let title = z.title.clone();
        let owner = ov.hwnd;
        self.editor.selected = Some(id);
        self.title_edit = TitleEditor::open(owner, self.hwnd, rect, &title, sc, id);
        self.render_overlays(None);
    }

    fn finish_title_edit(&mut self, commit: bool) {
        let Some(te) = self.title_edit.take() else { return };
        let text = te.text().trim().to_string();
        let (id, owner) = (te.zone, te.owner);
        te.close();
        if commit {
            if let Some(idx) = self.cfg.zones.iter().position(|z| z.id == id) {
                if self.cfg.zones[idx].title != text {
                    self.editor.checkpoint(&self.cfg.zones);
                    self.cfg.zones[idx].title = text;
                    self.save_soon();
                }
            }
        }
        if self.editing {
            self.render_overlays(None);
            unsafe {
                let _ = SetForegroundWindow(owner);
            }
        }
    }

    // ---- menus and commands -------------------------------------------------------

    fn tray_menu(&mut self) -> Menu {
        let scale = self.monitors.iter().find(|m| m.primary).map_or(1.0, |m| m.scale());
        let layout = shell::icon_layout(None);
        let mut m = Menu::new();
        let hk = if self.hotkey_on { format!("\t{HOTKEY_LABEL}") } else { String::new() };
        let edit = if self.editing { "Done editing" } else { "Draw / edit boxes" };
        m.item(cmd::EDIT, &format!("{edit}{hk}"));
        m.set_default(cmd::EDIT);
        m.check(cmd::SHOW_ZONES, "Show boxes", self.cfg.show_zones);
        m.check(cmd::MIX_COLORS, "Mix colors where boxes overlap", self.cfg.mix_colors);
        m.sep();
        m.check(cmd::GRID_SHOW, "Grid view (dots on the desktop)", self.cfg.grid.show_on_desktop);
        m.check(cmd::SNAP, "Snap boxes to the icon grid", self.cfg.snap);

        let g = &self.cfg.grid;
        let mut dots = Menu::new();
        dots.check(cmd::GRID_SHOW, "Always show dots on the desktop", g.show_on_desktop);
        dots.check(cmd::GRID_REPEL, "Dots move away from the mouse (while drawing)", g.repel);
        dots.sep();
        dots.heading("Color");
        for (i, (name, c)) in model::DOT_COLORS.iter().enumerate() {
            dots.color(cmd::DOT_COLOR + i as u32, name, *c, *c == g.color, scale);
        }
        let custom = !model::DOT_COLORS.iter().any(|(_, c)| *c == g.color);
        if custom {
            dots.color(cmd::DOT_CUSTOM, "Custom color…", g.color, true, scale);
        } else {
            dots.radio(cmd::DOT_CUSTOM, "Custom color…", false);
        }
        dots.sep();
        dots.heading("Size");
        for (i, (name, v)) in model::DOT_SIZES.iter().enumerate() {
            dots.radio(cmd::GRID_SIZE + i as u32, name, (v - g.size).abs() < 0.01);
        }
        dots.sep();
        dots.heading("Opacity");
        for (i, (name, v)) in model::DOT_OPACITY.iter().enumerate() {
            dots.radio(cmd::GRID_OPACITY + i as u32, name, *v == g.opacity);
        }
        m.sub("Grid dots", dots);
        m.sep();

        let mut style = Menu::new();
        menu::add_style_menus(&mut style, &self.cfg.defaults, cmd::STYLE_DEFAULT, scale);
        style.sep();
        style.item_enabled(cmd::APPLY_ALL, "Apply this style to all boxes", !self.cfg.zones.is_empty());
        m.sub("New box style", style);

        let mut font_menu = Menu::new();
        for (i, f) in fonts::FONT_CHOICES.iter().enumerate() {
            if fonts::available(f) {
                font_menu.radio(cmd::FONT + i as u32, f.label, f.key == self.cfg.font);
            }
        }
        m.sub("Title font", font_menu);

        let mut icons = Menu::new();
        let current = layout.as_ref().map_or(0, |l| l.icon_size);
        for (i, (name, s)) in model::ICON_SIZES.iter().enumerate() {
            icons.radio(cmd::ICON_SIZE + i as u32, name, *s == current);
        }
        icons.sep();
        icons.check(cmd::AUTO_ARRANGE, "Auto arrange icons (turn off for boxes)", layout.as_ref().is_some_and(|l| l.auto_arrange()));
        icons.check(cmd::ALIGN_GRID, "Align icons to grid", layout.as_ref().is_some_and(|l| l.align_to_grid()));
        icons.sep();
        icons.item(cmd::REFRESH, "Refresh desktop layer");
        m.sub("Desktop icons", icons);

        let orphans = self.orphan_count();
        if orphans > 0 {
            let s = if orphans == 1 { "" } else { "es" };
            m.item(cmd::RECOVER, &format!("Bring back {orphans} box{s} from disconnected screens"));
        }
        m.sep();
        m.check(cmd::AUTOSTART, "Start with Windows", install::autostart_enabled());
        m.check(cmd::MINIMIZE, "Minimize windows while editing", self.cfg.minimize_windows);
        m.check(cmd::HOTKEY, &format!("Hotkey {HOTKEY_LABEL}"), self.cfg.hotkey);
        m.sep();
        m.item(cmd::SUPPORT, "Support DeskChalk on Ko-fi");
        m.item(cmd::ABOUT, &format!("About DeskChalk {VERSION}"));
        if install::uninstaller().is_some() {
            m.item(cmd::UNINSTALL, "Uninstall DeskChalk…");
        } else {
            m.item(cmd::UNINSTALL, "Remove DeskChalk (portable)…");
        }
        m.item(cmd::EXIT, "Exit");
        m
    }

    fn zone_mut(&mut self, id: u64) -> Option<&mut Zone> {
        self.cfg.zones.iter_mut().find(|z| z.id == id)
    }

    fn on_command(&mut self, id: u32, target: &Target) -> Follow {
        let zone = match target {
            Target::Zone(z) => Some(*z),
            _ => None,
        };
        let in_range = |base: u32, len: usize| (base..base + len as u32).contains(&id);
        match id {
            cmd::EDIT => {
                if self.editing {
                    self.exit_edit();
                } else {
                    self.enter_edit();
                }
            }
            cmd::SHOW_ZONES => {
                self.cfg.show_zones = !self.cfg.show_zones;
                self.changed();
            }
            cmd::GRID_SHOW => {
                self.cfg.grid.show_on_desktop = !self.cfg.grid.show_on_desktop;
                self.changed();
            }
            cmd::SNAP => {
                self.cfg.snap = !self.cfg.snap;
                self.save_soon();
            }
            cmd::MIX_COLORS => {
                self.cfg.mix_colors = !self.cfg.mix_colors;
                self.changed();
            }
            cmd::GRID_REPEL => {
                self.cfg.grid.repel = !self.cfg.grid.repel;
                self.save_soon();
            }
            _ if in_range(cmd::DOT_COLOR, model::DOT_COLORS.len()) => {
                self.cfg.grid.color = model::DOT_COLORS[(id - cmd::DOT_COLOR) as usize].1;
                self.changed();
            }
            cmd::DOT_CUSTOM => {
                return Follow::CustomColor { initial: self.cfg.grid.color, slot: ColorSlot::Dots, zone: None };
            }
            _ if in_range(cmd::GRID_SIZE, model::DOT_SIZES.len()) => {
                self.cfg.grid.size = model::DOT_SIZES[(id - cmd::GRID_SIZE) as usize].1;
                self.changed();
            }
            _ if in_range(cmd::GRID_OPACITY, model::DOT_OPACITY.len()) => {
                self.cfg.grid.opacity = model::DOT_OPACITY[(id - cmd::GRID_OPACITY) as usize].1;
                self.changed();
            }
            _ if in_range(cmd::ICON_SIZE, model::ICON_SIZES.len()) => {
                if shell::set_icon_size(model::ICON_SIZES[(id - cmd::ICON_SIZE) as usize].1) {
                    // Spacing changes with the icon size; re-read the grid once Explorer relaid out.
                    self.schedule(T_REFRESH, 700);
                }
            }
            _ if in_range(cmd::FONT, fonts::FONT_CHOICES.len()) => {
                self.cfg.font = fonts::FONT_CHOICES[(id - cmd::FONT) as usize].key.into();
                self.title_fonts = None;
                if self.editing {
                    self.ensure_title_fonts();
                }
                self.changed();
            }
            cmd::AUTO_ARRANGE | cmd::ALIGN_GRID => {
                let flag = if id == cmd::AUTO_ARRANGE { FWF_AUTOARRANGE } else { FWF_SNAPTOGRID };
                let on = shell::icon_layout(None).is_some_and(|l| l.flags & flag.0 as u32 != 0);
                shell::set_folder_flag(flag, !on);
                self.schedule(T_REFRESH, 700);
            }
            cmd::REFRESH => self.attach(),
            cmd::RECOVER => self.recover_orphans(),
            cmd::AUTOSTART => install::set_autostart(!install::autostart_enabled()),
            cmd::MINIMIZE => {
                self.cfg.minimize_windows = !self.cfg.minimize_windows;
                self.save_soon();
            }
            cmd::HOTKEY => {
                self.cfg.hotkey = !self.cfg.hotkey;
                self.apply_hotkey();
                self.save_soon();
            }
            cmd::SUPPORT => shell::open_url(SUPPORT_URL),
            cmd::ABOUT => shell::open_url(REPO_URL),
            cmd::UNINSTALL => return Follow::Uninstall,
            cmd::EXIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
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
                if let Target::Monitor(key) = target {
                    return Follow::ConfirmDeleteAll(key.clone());
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
            cmd::Z_RENAME => {
                if let Some(z) = zone {
                    self.begin_title_edit(z);
                }
            }
            cmd::Z_DUPLICATE => {
                if let Some(z) = zone {
                    self.editor.selected = Some(z);
                    let snap = self.cfg.snap;
                    let key = self.cfg.zones.iter().find(|x| x.id == z).map(|x| x.monitor.clone()).unwrap_or_default();
                    if let Some(mon) = find_monitor(&self.monitors, &key) {
                        let ctx = mon.ctx();
                        if self.editor.duplicate_selected(&mut self.cfg.zones, &ctx, snap).is_some() {
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
                }
            }
            cmd::Z_RESET_STYLE => {
                if let Some(z) = zone {
                    self.editor.checkpoint(&self.cfg.zones);
                    let style = self.cfg.defaults.clone();
                    if let Some(zz) = self.zone_mut(z) {
                        zz.style = style;
                    }
                    self.changed();
                }
            }
            _ if in_range(cmd::STYLE_ZONE, model::STYLE_SPAN as usize) => {
                let Some(z) = zone else { return Follow::Nothing };
                match StyleCmd::decode(id - cmd::STYLE_ZONE) {
                    Some(StyleCmd::CustomColor) => {
                        let initial = self.zone_mut(z).map_or(model::PASTEL_GREEN, |zz| zz.style.color);
                        return Follow::CustomColor { initial, slot: ColorSlot::Box, zone: Some(z) };
                    }
                    Some(StyleCmd::CustomBorderColor) => {
                        let initial = self.zone_mut(z).map_or(model::PASTEL_GREEN, |zz| zz.style.line_color());
                        return Follow::CustomColor { initial, slot: ColorSlot::Border, zone: Some(z) };
                    }
                    Some(sc) => {
                        self.editor.checkpoint(&self.cfg.zones);
                        if let Some(zz) = self.zone_mut(z) {
                            zz.style.apply(sc);
                        }
                        self.changed();
                    }
                    None => {}
                }
            }
            _ if in_range(cmd::STYLE_DEFAULT, model::STYLE_SPAN as usize) => {
                match StyleCmd::decode(id - cmd::STYLE_DEFAULT) {
                    Some(StyleCmd::CustomColor) => {
                        return Follow::CustomColor { initial: self.cfg.defaults.color, slot: ColorSlot::Box, zone: None };
                    }
                    Some(StyleCmd::CustomBorderColor) => {
                        let initial = self.cfg.defaults.line_color();
                        return Follow::CustomColor { initial, slot: ColorSlot::Border, zone: None };
                    }
                    Some(sc) => {
                        self.cfg.defaults.apply(sc);
                        self.save_soon();
                    }
                    None => {}
                }
            }
            _ => {}
        }
        Follow::Nothing
    }

    fn apply_custom_color(&mut self, c: Rgb, slot: ColorSlot, zone: Option<u64>, custom: Vec<Rgb>) {
        self.cfg.custom_colors = custom;
        let set = |style: &mut model::ZoneStyle| match slot {
            ColorSlot::Box => style.color = c,
            ColorSlot::Border => style.border_color = Some(c),
            ColorSlot::Dots => {}
        };
        match (slot, zone) {
            (ColorSlot::Dots, _) => {
                self.cfg.grid.color = c;
                self.changed();
            }
            (_, Some(z)) => {
                self.editor.checkpoint(&self.cfg.zones);
                if let Some(zz) = self.zone_mut(z) {
                    set(&mut zz.style);
                }
                self.changed();
            }
            (_, None) => {
                set(&mut self.cfg.defaults);
                self.save_soon();
            }
        }
    }

    fn delete_all_on(&mut self, key: &str) {
        self.editor.checkpoint(&self.cfg.zones);
        self.cfg.zones.retain(|z| z.monitor != key);
        self.editor.selected = None;
        self.changed();
    }

    fn remove_portable(&mut self) {
        if self.editing {
            self.exit_edit();
        }
        self.uninstalling = true;
        self.kill(T_SAVE);
        self.kill(T_HEALTH);
        self.destroy_layers();
        self.tray.remove();
        install::remove_portable_traces();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands that may need modal UI (run without holding the app borrow)

fn run_command(id: u32, target: Target) {
    let follow = with_app(|a| a.on_command(id, &target)).unwrap_or(Follow::Nothing);
    match follow {
        Follow::Nothing => {}
        Follow::CustomColor { initial, slot, zone } => {
            let Some((owner, custom)) = with_app(|a| (a.modal_owner(), a.cfg.custom_colors.clone())) else {
                return;
            };
            if let Some((c, custom)) = dialogs::choose_color(owner, initial, &custom) {
                with_app(|a| a.apply_custom_color(c, slot, zone, custom));
            }
            with_app(|a| {
                if a.editing {
                    a.focus_overlay();
                }
            });
        }
        Follow::Uninstall => {
            // Installed copy: the setup's uninstaller asks, closes DeskChalk and removes everything.
            if install::launch_uninstaller() {
                return;
            }
            let owner = with_app(|a| a.modal_owner()).flatten();
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "DeskChalk_Portable.exe".into());
            let q = format!(
                "Remove DeskChalk?\n\nThis deletes all of your boxes and settings and the startup entry, then closes DeskChalk. Delete {exe} yourself afterwards. Your desktop icons are not touched."
            );
            if dialogs::ask(owner, &q) {
                with_app(|a| a.remove_portable());
            }
        }
        Follow::ConfirmDeleteAll(key) => {
            let owner = with_app(|a| a.modal_owner()).flatten();
            if dialogs::ask(owner, "Delete all boxes on this screen?\n\nYou can undo this with Ctrl+Z.") {
                with_app(|a| a.delete_all_on(&key));
            }
        }
    }
}

/// A fresh copy of an open menu, to refresh its check marks after a sticky click.
fn rebuild_menu(kind: &MenuKind) -> Option<Menu> {
    with_app(|a| match kind {
        MenuKind::Tray => Some(a.tray_menu()),
        MenuKind::Zone(id) => {
            let z = a.cfg.zones.iter().find(|z| z.id == *id)?;
            let scale = find_monitor(&a.monitors, &z.monitor).map_or(1.0, |m| m.scale());
            Some(menu::zone_menu(&z.style, scale))
        }
        MenuKind::Canvas(key) => Some(a.canvas_menu(key)),
    })
    .flatten()
}

fn show_tray_menu(hwnd: HWND) {
    let Some(m) = with_app(|a| a.tray_menu()) else { return };
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let id = sticky::track(&m, hwnd, pt.x, pt.y, Target::Global, MenuKind::Tray);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        drop(m);
        if id != 0 {
            run_command(id, Target::Global);
        }
    }
}

fn welcome(hwnd: HWND) {
    with_app(|a| {
        a.announce("DeskChalk is running");
        a.cfg.welcomed = true;
        a.save_soon();
    });
    if shell::icon_layout(None).is_some_and(|l| l.auto_arrange())
        && dialogs::ask(
            Some(hwnd),
            "Your desktop icons are set to \"Auto arrange\", which pulls them back into a column, so they can't stay inside your boxes.\n\nTurn off Auto arrange now?\n(You can change this later under Desktop icons in the tray menu.)",
        )
    {
        shell::set_folder_flag(FWF_AUTOARRANGE, false);
        with_app(|a| a.schedule(T_REFRESH, 700));
    }
}

// ---------------------------------------------------------------------------
// Window procedures

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lp.0 & 0xFFFF) as u32 {
                NIN_SELECT => {
                    with_app(|a| a.tray_select());
                }
                NIN_KEYSELECT | WM_CONTEXTMENU => show_tray_menu(hwnd),
                WM_LBUTTONDBLCLK => {
                    with_app(|a| a.tray_double_click());
                    run_command(cmd::EDIT, Target::Global);
                }
                _ => {}
            }
            return LRESULT(0);
        }
        WM_TIMER => {
            if wp.0 == T_TRAY_CLICK {
                unsafe {
                    let _ = KillTimer(Some(hwnd), T_TRAY_CLICK);
                }
                show_tray_menu(hwnd);
            } else {
                with_app(|a| a.timer(wp.0));
            }
            return LRESULT(0);
        }
        WM_HOTKEY => {
            run_command(cmd::EDIT, Target::Global);
            return LRESULT(0);
        }
        WM_TITLE_DONE => {
            with_app(|a| a.finish_title_edit(wp.0 == 1));
            return LRESULT(0);
        }
        WM_WELCOME => {
            welcome(hwnd);
            return LRESULT(0);
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
            with_app(|a| a.schedule(T_REFRESH, 700));
        }
        WM_QUERYENDSESSION => return LRESULT(1),
        WM_ENDSESSION => {
            if wp.0 != 0 {
                with_app(|a| a.save_now());
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    if msg != 0 && msg == TASKBAR_CREATED.load(Relaxed) {
        with_app(|a| a.on_taskbar_created());
        return LRESULT(0);
    }
    if msg != 0 && msg == COMMAND_MSG.load(Relaxed) {
        match wp.0 {
            CMD_EDIT => {
                with_app(|a| {
                    if !a.editing {
                        a.enter_edit();
                    }
                });
            }
            CMD_QUIT => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            // Opened again from the Start menu or a shortcut: show its menu where you clicked,
            // like other apps bring up their window (works even with notifications off).
            CMD_SHOW => {
                if with_app(|a| !a.editing && !a.uninstalling).unwrap_or(false) {
                    show_tray_menu(hwnd);
                }
            }
            _ => {}
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn layer_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => return LRESULT(-1), // HTTRANSPARENT: clicks go to the icons
        WM_ERASEBKGND => return LRESULT(1),
        WM_NCDESTROY => {
            // Explorer destroyed our parent (restart, wallpaper change): rebuild soon.
            with_app(|a| a.on_layer_destroyed(hwnd));
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = util::lparam_xy(lp);
    match msg {
        WM_LBUTTONDOWN => {
            unsafe { SetCapture(hwnd) };
            with_app(|a| a.overlay_down(hwnd, x, y));
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            with_app(|a| a.overlay_move(hwnd, x, y));
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            with_app(|a| a.overlay_up(hwnd, x, y));
            unsafe {
                let _ = ReleaseCapture();
            }
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            with_app(|a| a.overlay_capture_lost());
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with_app(|a| a.overlay_leave(hwnd));
            return LRESULT(0);
        }
        WM_LBUTTONDBLCLK => {
            with_app(|a| a.overlay_double_click(hwnd, x, y));
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            if let Some((m, target, kind)) = with_app(|a| a.overlay_menu(hwnd, x, y)).flatten() {
                let mut pt = POINT { x, y };
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut pt);
                }
                let id = sticky::track(&m, hwnd, pt.x, pt.y, target.clone(), kind);
                drop(m);
                if id != 0 {
                    run_command(id, target);
                }
            }
            return LRESULT(0);
        }
        WM_SETCURSOR => {
            if (lp.0 & 0xFFFF) as u32 == HTCLIENT {
                if let Some(c) = with_app(|a| a.overlay_cursor(hwnd)).flatten() {
                    unsafe {
                        SetCursor(LoadCursorW(None, c).ok());
                    }
                    return LRESULT(1);
                }
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = VIRTUAL_KEY(wp.0 as u16);
            if with_app(|a| a.overlay_key(hwnd, vk)) == Some(true) {
                return LRESULT(0);
            }
        }
        WM_KEYUP | WM_SYSKEYUP => {
            if VIRTUAL_KEY(wp.0 as u16) == VK_MENU {
                with_app(|a| a.refresh_drag());
                return LRESULT(0);
            }
        }
        WM_SYSCOMMAND => {
            // Don't let a lone Alt press enter menu mode (Alt toggles snapping).
            if (wp.0 & 0xFFF0) as u32 == SC_KEYMENU {
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            with_app(|a| a.exit_edit());
            return LRESULT(0);
        }
        WM_DPICHANGED => return LRESULT(0),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
