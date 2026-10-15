//! DeskChalk: draw labeled boxes on your desktop wallpaper, behind your icons.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod commands;
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

#[cfg(target_os = "linux")]
mod linux;

fn main() {
    #[cfg(windows)]
    win::main();

    #[cfg(target_os = "linux")]
    linux::main();

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        eprintln!("DeskChalk runs on Windows and Linux.");
        std::process::exit(1);
    }
}
