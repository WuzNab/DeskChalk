//! Loading and saving config.json (per user).

use crate::model::Config;
use std::io;
use std::path::Path;

pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => cfg,
            Err(_) => {
                // Keep the unreadable file around instead of silently losing the user's boxes.
                let _ = std::fs::copy(path, path.with_extension("broken.json"));
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// Writes atomically: a crash mid-write never leaves a truncated config behind.
pub fn save(path: &Path, cfg: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(cfg).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}
