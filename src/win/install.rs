//! Where DeskChalk keeps its data, autostart, and uninstalling.
//!
//! The setup wizard (installer/DeskChalk.iss, built with Inno Setup) installs DeskChalk per
//! user into %LOCALAPPDATA%\Programs\DeskChalk next to its uninstaller (unins000.exe) and
//! registers it in Settings > Apps / Programs and Features. DeskChalk_Portable_v<version>.exe
//! runs from anywhere without installing. Both keep boxes and settings in %APPDATA%\DeskChalk.

use super::{reg, shell, CMD_QUIT, COMMAND_MSG, MAIN_CLASS};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "DeskChalk";
/// The setup's Settings > Apps entry (AppId in installer/DeskChalk.iss, plus Inno's "_is1").
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{3F9C2B7E-8D41-4A6C-B5E2-7A1D9C0E4F68}_is1";

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

/// Puts an installed DeskChalk back into Settings > Apps (and Start with Windows, the setup's
/// default) when the setup's entry is missing. That happens when the setup was started from
/// inside a packaged (MSIX) app: Windows then keeps its registry writes private to that app,
/// so the install works but Windows doesn't know about it. Returns true if it repaired.
pub fn ensure_registered() -> bool {
    let Some(unins) = uninstaller() else { return false };
    if reg::key_exists(UNINSTALL_KEY) {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(dir) = exe.parent() else { return false };
    let size_kb: u64 = std::fs::read_dir(dir)
        .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
        / 1024;
    let t = unsafe { GetLocalTime() };
    let k = UNINSTALL_KEY;
    let (dir_s, exe_s, unins_s) = (dir.display().to_string(), exe.display().to_string(), unins.display().to_string());
    let major: u32 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0);
    let minor: u32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0);
    let ok = reg::set_sz(k, "DisplayName", "DeskChalk")
        && reg::set_sz(k, "DisplayVersion", env!("CARGO_PKG_VERSION"))
        && reg::set_sz(k, "Publisher", "WuzNab")
        && reg::set_sz(k, "DisplayIcon", &exe_s)
        && reg::set_sz(k, "UninstallString", &format!("\"{unins_s}\""))
        && reg::set_sz(k, "QuietUninstallString", &format!("\"{unins_s}\" /SILENT"))
        && reg::set_sz(k, "InstallLocation", &format!("{dir_s}\\"))
        && reg::set_sz(k, "Inno Setup: App Path", &dir_s)
        && reg::set_sz(k, "InstallDate", &format!("{:04}{:02}{:02}", t.wYear, t.wMonth, t.wDay))
        && reg::set_sz(k, "Comments", "Draw labeled boxes on your desktop wallpaper, behind your icons.")
        && reg::set_sz(k, "URLInfoAbout", "https://github.com/WuzNab/DeskChalk")
        && reg::set_sz(k, "HelpLink", "https://github.com/WuzNab/DeskChalk/issues")
        && reg::set_sz(k, "URLUpdateInfo", "https://github.com/WuzNab/DeskChalk/releases")
        && reg::set_dword(k, "NoModify", 1)
        && reg::set_dword(k, "NoRepair", 1)
        && reg::set_dword(k, "EstimatedSize", size_kb as u32)
        && reg::set_dword(k, "MajorVersion", major)
        && reg::set_dword(k, "MinorVersion", minor)
        && reg::set_dword(k, "VersionMajor", major)
        && reg::set_dword(k, "VersionMinor", minor);
    if ok {
        set_autostart_for(&exe);
    }
    ok
}

/// If DeskChalk ran inside a packaged app's container, Windows kept its settings in that app's
/// private copy of AppData (%LOCALAPPDATA%\Packages\<app>\LocalCache\Roaming). When there is
/// no config here yet, brings the newest such copy over (once: the copy is renamed after).
pub fn rescue_config(path: &Path) -> Option<PathBuf> {
    if path.exists() || std::env::var_os("DESKCHALK_CONFIG_DIR").is_some() {
        return None;
    }
    let local = shell::known_folder(&FOLDERID_LocalAppData).or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    rescue_from(&local.join("Packages"), path)
}

fn rescue_from(packages: &Path, path: &Path) -> Option<PathBuf> {
    let (_, src) = std::fs::read_dir(packages)
        .ok()?
        .flatten()
        .map(|d| d.path().join(r"LocalCache\Roaming\DeskChalk\config.json"))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::copy(&src, path).ok()?;
    let _ = std::fs::rename(&src, src.with_file_name("config.moved.json"));
    Some(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brings_back_the_newest_container_copy() {
        let root = std::env::temp_dir().join(format!("deskchalk-rescue-{}", std::process::id()));
        let packages = root.join("Packages");
        for (app, text) in [("Old.App_1", "old"), ("New.App_2", "new")] {
            let dir = packages.join(app).join(r"LocalCache\Roaming\DeskChalk");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.json"), text).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        std::fs::create_dir_all(packages.join("Unrelated_3")).unwrap();
        let target = root.join(r"Roaming\DeskChalk\config.json");
        let src = rescue_from(&packages, &target).unwrap();
        assert!(src.starts_with(packages.join("New.App_2")));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        // Moved aside, so it isn't brought back a second time (e.g. after a clean reinstall).
        assert!(!src.exists() && src.with_file_name("config.moved.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// For the portable copy: removes boxes, settings and the startup entry. The exe itself
/// stays wherever the user put it.
pub fn remove_portable_traces() {
    set_autostart(false);
    let _ = std::fs::remove_dir_all(config_dir());
}
