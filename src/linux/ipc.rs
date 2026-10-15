//! One DeskChalk per user: the first one listens on a socket, and starting DeskChalk again
//! hands the request (show the menu, edit, quit) to the running one.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub enum Request {
    Show,
    Edit,
    Quit,
}

fn socket_path() -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(super::paths::home()).map(|m| m.uid()).unwrap_or(0);
    super::paths::runtime_dir().join(format!("deskchalk-{uid}.sock"))
}

pub enum Claim {
    First(UnixListener),
    /// Another DeskChalk is running.
    Taken,
    /// No socket could be made; run without (a second copy can't be prevented).
    Unavailable,
}

pub fn claim() -> Claim {
    let path = socket_path();
    if let Ok(l) = UnixListener::bind(&path) {
        return Claim::First(l);
    }
    if UnixStream::connect(&path).is_ok() {
        return Claim::Taken;
    }
    // Left over from a DeskChalk that didn't exit cleanly.
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(l) => Claim::First(l),
        Err(_) => Claim::Unavailable,
    }
}

pub fn send(request: &str) -> bool {
    UnixStream::connect(socket_path()).and_then(|mut s| s.write_all(format!("{request}\n").as_bytes())).is_ok()
}

/// Waits (up to `secs`) until the running DeskChalk is gone.
pub fn wait_gone(secs: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < until && UnixStream::connect(socket_path()).is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn serve(listener: UnixListener, on_request: impl Fn(Request) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            match line.trim() {
                "show" => on_request(Request::Show),
                "edit" => on_request(Request::Edit),
                "quit" => on_request(Request::Quit),
                _ => {}
            }
        }
    });
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
