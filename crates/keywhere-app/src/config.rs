//! Tiny per-user settings: only the last vault path. Never secrets.

use std::fs;
use std::path::{Path, PathBuf};

fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("keywhere"))
}

pub fn last_vault() -> Option<PathBuf> {
    let p = fs::read_to_string(config_dir()?.join("last-vault")).ok()?;
    let p = PathBuf::from(p.trim());
    p.exists().then_some(p)
}

pub fn remember_vault(path: &Path) {
    if let Some(dir) = config_dir() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("last-vault"), path.to_string_lossy().as_bytes());
    }
}

/// `~/…` shortening for display.
pub fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}
