//! Where DeskChalk keeps its data, autostart, and uninstalling.
//!
//! The setup wizard (installer/DeskChalk.iss, built with Inno Setup) installs DeskChalk per
//! user into %LOCALAPPDATA%\Programs\DeskChalk next to its uninstaller (unins000.exe) and
//! registers it in Settings > Apps / Programs and Features. DeskChalk_Portable.exe runs from
//! anywhere without installing. Both keep boxes and settings in %APPDATA%\DeskChalk.

use super::{reg, shell, CMD_QUIT, COMMAND_MSG, MAIN_CLASS};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LPARAM, WPARAM};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::FOLDERID_RoamingAppData;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "DeskChalk";

/// %APPDATA%\DeskChalk. DESKCHALK_CONFIG_DIR overrides it (used for testing).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DESKCHALK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    shell::known_folder(&FOLDERID_RoamingAppData)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_default()
        .join("DeskChalk")
}

/// The setup wizard's uninstaller, if this copy was installed with it.
pub fn uninstaller() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join("unins000.exe");
    path.is_file().then_some(path)
}

/// Starts the setup wizard's uninstaller (it asks for confirmation and closes DeskChalk).
pub fn launch_uninstaller() -> bool {
    uninstaller().is_some_and(|u| std::process::Command::new(u).spawn().is_ok())
}

/// Tells an already running DeskChalk (this user, this session) to do something.
pub fn signal_running(command: usize) -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(MAIN_CLASS, PCWSTR::null()) else { return false };
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        let msg = COMMAND_MSG.load(Ordering::Relaxed);
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(command), LPARAM(0));
        if command == CMD_QUIT {
            if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                WaitForSingleObject(h, 5000);
                let _ = CloseHandle(h);
            }
        }
        true
    }
}

pub fn autostart_enabled() -> bool {
    if reg::get_sz(RUN_KEY, VALUE_NAME).is_none_or(|v| v.is_empty()) {
        return false;
    }
    // Task Manager / Settings > Startup can disable it separately (odd first byte = off).
    !matches!(reg::get_binary(APPROVED_KEY, VALUE_NAME), Some(b) if b.first().is_some_and(|x| x & 1 == 1))
}

pub fn set_autostart(on: bool) {
    match std::env::current_exe() {
        Ok(exe) if on => set_autostart_for(&exe),
        _ => {
            reg::delete_value(RUN_KEY, VALUE_NAME);
            reg::delete_value(APPROVED_KEY, VALUE_NAME);
        }
    }
}

fn set_autostart_for(exe: &Path) {
    reg::set_sz(RUN_KEY, VALUE_NAME, &format!("\"{}\" --autostart", exe.display()));
    reg::delete_value(APPROVED_KEY, VALUE_NAME);
}

/// For the portable copy: removes boxes, settings and the startup entry. The exe itself
/// stays wherever the user put it.
pub fn remove_portable_traces() {
    set_autostart(false);
    let _ = std::fs::remove_dir_all(config_dir());
}
