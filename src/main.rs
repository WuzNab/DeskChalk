//! DeskChalk: draw labeled boxes on your desktop wallpaper, behind your icons.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod config;
mod editor;
mod geom;
mod grid;
mod icon;
mod model;
mod render;
mod text;

#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main();

    #[cfg(not(windows))]
    {
        eprintln!("DeskChalk currently supports Windows. Linux support is planned.");
        std::process::exit(1);
    }
}
