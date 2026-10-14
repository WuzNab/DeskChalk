//! Small helpers: UTF-16 strings, message parameters and an opt-in log file.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use windows::Win32::Foundation::LPARAM;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
const LOG_LIMIT: u64 = 512 * 1024;

/// Enables logging to `path` (only when started with --log).
pub fn enable_log(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::remove_file(&path);
    }
    let _ = LOG_PATH.set(path);
}

pub fn write_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("{msg}");
    }
    if let Some(p) = LOG_PATH.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{t}] {msg}");
        }
    }
}

macro_rules! log {
    ($($t:tt)*) => { $crate::win::util::write_log(&format!($($t)*)) };
}

/// NUL-terminated UTF-16.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Copies `s` into a fixed-size UTF-16 buffer, truncating and NUL-terminating.
pub fn fill_wide(dst: &mut [u16], s: &str) {
    if dst.is_empty() {
        return;
    }
    let max = dst.len() - 1;
    let mut n = 0;
    for c in s.encode_utf16().take(max) {
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}

/// Signed client coordinates packed in an LPARAM.
pub fn lparam_xy(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}
