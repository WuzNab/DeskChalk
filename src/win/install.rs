//! Per-user install, update, autostart and clean uninstall. No admin rights, nothing
//! machine-wide: every Windows user gets their own copy, settings and startup entry.
//!
//! Installed layout:
//!   %LOCALAPPDATA%\Programs\DeskChalk\DeskChalk.exe
//!   %APPDATA%\DeskChalk\config.json                     (boxes and settings)
//!   Start Menu\Programs\DeskChalk.lnk
//!   HKCU\...\Uninstall\DeskChalk                         (Settings > Apps entry)
//!   HKCU\...\Run\DeskChalk                               (start with Windows)

use super::{dialogs, reg, shell, CMD_QUIT, COMMAND_MSG, MAIN_CLASS, REPO_URL, VERSION};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LPARAM, WPARAM};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::{FOLDERID_Programs, FOLDERID_RoamingAppData, FOLDERID_UserProgramFiles};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskChalk";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "DeskChalk";
const EXE_NAME: &str = "DeskChalk.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn install_dir() -> PathBuf {
    shell::known_folder(&FOLDERID_UserProgramFiles)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs")))
        .unwrap_or_default()
        .join("DeskChalk")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join(EXE_NAME)
}

pub fn config_dir() -> PathBuf {
    shell::known_folder(&FOLDERID_RoamingAppData)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_default()
        .join("DeskChalk")
}

fn shortcut_path() -> Option<PathBuf> {
    shell::known_folder(&FOLDERID_Programs).map(|p| p.join("DeskChalk.lnk"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn parse_version(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

fn launch(exe: &Path, args: &[&str]) {
    let _ = std::process::Command::new(exe).args(args).spawn();
}

#[derive(PartialEq, Eq)]
pub enum Startup {
    Run,
    Exit,
}

/// Runs before the app starts: offers to install, updates an older install, or hands
/// over to the installed copy.
pub fn startup_check() -> Startup {
    let Ok(me) = std::env::current_exe() else { return Startup::Run };
    let target = installed_exe();
    if same_file(&me, &target) {
        return Startup::Run;
    }
    let installed = reg::get_sz(UNINSTALL_KEY, "DisplayVersion").filter(|_| target.is_file());
    match installed {
        Some(old) => {
            if parse_version(VERSION) > parse_version(&old) {
                let q = format!("DeskChalk {old} is installed. Update it to {VERSION}?\n\nYour boxes and settings are kept.");
                if dialogs::ask(None, &q) {
                    if let Err(e) = install_files(&me) {
                        dialogs::warn(&format!("DeskChalk couldn't be updated:\n{e}"));
                        return Startup::Exit;
                    }
                }
            }
            launch(&target, &[]);
            Startup::Exit
        }
        None => {
            let user = std::env::var("USERNAME").unwrap_or_else(|_| "you".into());
            let text = format!(
                "Install DeskChalk for {user}?\n\n\
                 DeskChalk lives in your system tray and draws labeled boxes on your desktop wallpaper, behind your icons.\n\n\
                 \u{2022} Installs to {} (no admin rights needed)\n\
                 \u{2022} Starts with Windows (you can turn this off in the tray menu)\n\
                 \u{2022} Uninstall any time from the tray menu or Settings > Apps\n\n\
                 Yes: install    No: just run it this time    Cancel: quit",
                install_dir().display()
            );
            match dialogs::ask3(&text) {
                dialogs::Answer::Yes => match install_files(&me) {
                    Ok(()) => {
                        set_autostart_for(&target, true);
                        launch(&target, &["--first-run"]);
                        Startup::Exit
                    }
                    Err(e) => {
                        dialogs::warn(&format!("DeskChalk couldn't be installed:\n{e}\n\nIt will run without installing."));
                        Startup::Run
                    }
                },
                dialogs::Answer::No => Startup::Run,
                dialogs::Answer::Cancel => Startup::Exit,
            }
        }
    }
}

/// `DeskChalk.exe --install`: installs (or updates) for this user without asking.
pub fn silent_install(launch_after: bool) {
    let Ok(me) = std::env::current_exe() else { return };
    let target = installed_exe();
    if !same_file(&me, &target) && install_files(&me).is_err() {
        return;
    }
    set_autostart_for(&target, true);
    if launch_after {
        launch(&target, &["--first-run"]);
    }
}

fn install_files(me: &Path) -> std::io::Result<()> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(EXE_NAME);
    if target.exists() {
        signal_running(CMD_QUIT);
    }
    let mut result = std::fs::copy(me, &target).map(|_| ());
    for _ in 0..20 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        result = std::fs::copy(me, &target).map(|_| ());
    }
    result?;
    write_uninstall_entry(&target);
    if let Some(lnk) = shortcut_path() {
        shell::create_shortcut(&lnk, &target, "Draw labeled boxes on your desktop");
    }
    Ok(())
}

fn write_uninstall_entry(exe: &Path) {
    let e = exe.display().to_string();
    let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let size_kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    let k = UNINSTALL_KEY;
    reg::set_sz(k, "DisplayName", "DeskChalk");
    reg::set_sz(k, "DisplayVersion", VERSION);
    reg::set_sz(k, "Publisher", "WuzNab");
    reg::set_sz(k, "DisplayIcon", &format!("{e},0"));
    reg::set_sz(k, "InstallLocation", &dir);
    reg::set_sz(k, "UninstallString", &format!("\"{e}\" --uninstall"));
    reg::set_sz(k, "QuietUninstallString", &format!("\"{e}\" --uninstall --quiet"));
    reg::set_sz(k, "URLInfoAbout", REPO_URL);
    reg::set_sz(k, "HelpLink", REPO_URL);
    reg::set_dword(k, "NoModify", 1);
    reg::set_dword(k, "NoRepair", 1);
    reg::set_dword(k, "EstimatedSize", size_kb);
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
        Ok(exe) => set_autostart_for(&exe, on),
        Err(_) => set_autostart_for(Path::new(""), false),
    }
}

fn set_autostart_for(exe: &Path, on: bool) {
    if on {
        reg::set_sz(RUN_KEY, VALUE_NAME, &format!("\"{}\" --autostart", exe.display()));
    } else {
        reg::delete_value(RUN_KEY, VALUE_NAME);
    }
    reg::delete_value(APPROVED_KEY, VALUE_NAME);
}

/// Removes every trace of DeskChalk for this user. Returns the install folder if it
/// still has to be deleted after this process exits (because we're running from it).
pub fn uninstall_files() -> Option<PathBuf> {
    set_autostart(false);
    reg::delete_tree(UNINSTALL_KEY);
    if let Some(lnk) = shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    let _ = std::fs::remove_dir_all(config_dir());
    let dir = install_dir();
    let running_from_it = std::env::current_exe().is_ok_and(|me| same_file(&me, &dir.join(EXE_NAME)));
    if running_from_it {
        Some(dir)
    } else {
        let _ = std::fs::remove_dir_all(&dir);
        None
    }
}

/// Deletes our install folder a moment after this process has exited.
pub fn schedule_delete(dir: &Path) {
    // Never delete anything that isn't our own folder.
    if dir.file_name().is_none_or(|n| n != "DeskChalk") || !dir.join(EXE_NAME).is_file() {
        return;
    }
    let cmd = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\cmd.exe"))
        .unwrap_or_else(|| PathBuf::from("cmd.exe"));
    let script = format!("/d /c ping -n 3 127.0.0.1 >nul & rd /s /q \"{}\"", dir.display());
    let _ = std::process::Command::new(cmd)
        .raw_arg(script)
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// `DeskChalk.exe --uninstall` (used by Settings > Apps).
pub fn run_uninstaller(quiet: bool) {
    if !quiet
        && !dialogs::ask(
            None,
            "Uninstall DeskChalk?\n\nThis removes DeskChalk, its startup entry, and all of your boxes and settings. Your desktop icons are not touched.",
        )
    {
        return;
    }
    signal_running(CMD_QUIT);
    let pending = uninstall_files();
    if !quiet {
        dialogs::info("DeskChalk has been uninstalled.");
    }
    if let Some(dir) = pending {
        schedule_delete(&dir);
    }
}
