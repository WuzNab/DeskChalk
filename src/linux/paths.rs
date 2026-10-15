//! Where DeskChalk keeps its files on Linux, starting at login, and removing it.

use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// An XDG base directory, or its usual place under the home folder.
fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

/// ~/.config/deskchalk: boxes and settings. DESKCHALK_CONFIG_DIR overrides it (testing).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DESKCHALK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    xdg("XDG_CONFIG_HOME", ".config").join("deskchalk")
}

/// ~/.local/share/deskchalk: the wallpapers DeskChalk paints, and the original wallpaper
/// settings to put back.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DESKCHALK_CONFIG_DIR") {
        return PathBuf::from(dir).join("data");
    }
    xdg("XDG_DATA_HOME", ".local/share").join("deskchalk")
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// The dconf database GSettings desktops write to (changes when the wallpaper changes).
pub fn dconf_db() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("dconf/user")
}

pub fn config_file(rel: &str) -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join(rel)
}

fn autostart_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart/deskchalk.desktop")
}

pub fn autostart_enabled() -> bool {
    std::fs::read_to_string(autostart_file()).is_ok_and(|s| {
        !s.lines().any(|l| matches!(l.trim(), "Hidden=true" | "X-GNOME-Autostart-enabled=false"))
    })
}

pub fn set_autostart(on: bool) {
    let file = autostart_file();
    if !on {
        let _ = std::fs::remove_file(&file);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=DeskChalk\nComment=Draw labeled boxes on your desktop wallpaper\nExec={} --autostart\nIcon=deskchalk\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_arg(&exe)
    );
    let _ = std::fs::write(&file, entry);
}

/// A path as one argument of a .desktop file's Exec= line.
fn exec_arg(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return s.into_owned();
    }
    // Inside quotes, " ` $ and \ are escaped with a backslash, and the key file format
    // escapes backslashes once more.
    let mut q = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' => {
                q.push_str("\\\\");
                q.push(c);
            }
            '\\' => q.push_str("\\\\\\\\"),
            _ => q.push(c),
        }
    }
    q.push('"');
    q
}

/// Installed from the .deb package (the program lives in /usr), not the portable download.
pub fn installed() -> bool {
    std::env::current_exe().is_ok_and(|e| e.starts_with("/usr/"))
}

/// Removes boxes, settings, painted wallpapers and the login entry.
pub fn remove_user_data() {
    set_autostart(false);
    let _ = std::fs::remove_dir_all(config_dir());
    let _ = std::fs::remove_dir_all(data_dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_arguments() {
        assert_eq!(exec_arg(Path::new("/usr/bin/deskchalk")), "/usr/bin/deskchalk");
        assert_eq!(exec_arg(Path::new("/home/a b/DeskChalk")), "\"/home/a b/DeskChalk\"");
        assert_eq!(exec_arg(Path::new("/home/$x/d")), "\"/home/\\\\$x/d\"");
    }
}
