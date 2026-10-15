//! Small helpers: an opt-in log file and running helper programs.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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
    ($($t:tt)*) => { $crate::linux::util::write_log(&format!($($t)*)) };
}

/// Runs `prog` and returns what it printed (trimmed) if it succeeded.
pub fn output(prog: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(prog).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs `prog` and waits; true if it succeeded.
pub fn run(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Starts `prog` without waiting for it (a helper thread collects its exit status).
pub fn spawn(prog: &str, args: &[&str]) {
    let child = Command::new(prog).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// True if `prog` is on the PATH.
pub fn have(prog: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

pub fn open_url(url: &str) {
    spawn("xdg-open", &[url]);
}

/// `file://` URI for a path (percent-encodes everything but unreserved characters and '/').
pub fn file_uri(path: &std::path::Path) -> String {
    let mut s = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Path from a `file://` URI (or the string itself if it is already a path).
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = match uri.strip_prefix("file://") {
        Some(r) => r,
        None if uri.starts_with('/') => uri,
        None => return None,
    };
    // file://host/path: only the local host makes sense here.
    let rest = if rest.starts_with('/') { rest } else { &rest[rest.find('/')?..] };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip() {
        let p = std::path::Path::new("/home/me/My Pictures/sea (1).jpg");
        let uri = file_uri(p);
        assert_eq!(uri, "file:///home/me/My%20Pictures/sea%20%281%29.jpg");
        assert_eq!(uri_path(&uri).unwrap(), p);
        assert_eq!(uri_path("/usr/share/a.png").unwrap(), std::path::Path::new("/usr/share/a.png"));
        assert_eq!(uri_path("file://localhost/x/y.png").unwrap(), std::path::Path::new("/x/y.png"));
        assert!(uri_path("https://example.com/a.png").is_none());
    }
}
