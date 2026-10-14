//! `DeskChalk.exe --diag`: prints what DeskChalk sees (desktop windows, monitors, icon
//! grid). Changes nothing. Useful for bug reports.

use super::{compute_grids, desktop, install, monitors, shell};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

pub fn run() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let mut out = String::new();
    out.push_str(&format!("DeskChalk {} diagnostics\n\n", super::VERSION));
    let listview = match desktop::find() {
        Some(d) => {
            out.push_str(&d.describe());
            d.listview
        }
        None => {
            out.push_str("desktop: NOT FOUND\n");
            desktop::locate_listview()
        }
    };
    let layout = shell::icon_layout(listview);
    let mut mons = monitors::enumerate();
    compute_grids(&mut mons, layout.as_ref());
    out.push_str("\nmonitors:\n");
    for m in &mons {
        out.push_str(&format!(
            "  {} primary={} dpi={} rect=({},{})-({},{}) work=({},{})-({},{})\n    key={}\n    grid origin=({:.1},{:.1}) cell=({:.1},{:.1})\n",
            m.device, m.primary, m.dpi,
            m.rect.left, m.rect.top, m.rect.right, m.rect.bottom,
            m.work.left, m.work.top, m.work.right, m.work.bottom,
            m.key, m.grid.origin.x, m.grid.origin.y, m.grid.cell.x, m.grid.cell.y,
        ));
    }
    match &layout {
        Some(l) => {
            out.push_str(&format!(
                "\nicons: size={} spacing={:?} auto_arrange={} align_to_grid={} count={}\n",
                l.icon_size, l.spacing, l.auto_arrange(), l.align_to_grid(), l.positions.len()
            ));
            for p in l.positions.iter().take(40) {
                out.push_str(&format!("  ({}, {})\n", p.x, p.y));
            }
        }
        None => out.push_str("\nicons: could not read the desktop view\n"),
    }
    out.push_str(&format!(
        "\nexe: {}\nuninstaller: {}\nconfig dir: {}\nautostart: {}\n",
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        install::uninstaller().map_or("none (portable)".into(), |p| p.display().to_string()),
        install::config_dir().display(),
        install::autostart_enabled()
    ));
    println!("{out}");
}
